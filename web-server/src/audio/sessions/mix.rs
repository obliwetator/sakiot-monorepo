//! Timestamp-aligned channel mixes for finalized logical recordings.
//!
//! A mix is deliberately a filesystem cache rather than a database object. The
//! database remains the source of truth for the anchor timeline, contributors,
//! authorization, and the cache fingerprint. A process-local job container
//! deduplicates concurrent renders while the cache makes successful renders
//! rebuildable after a restart.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use actix_files::NamedFile;
use actix_web::{HttpRequest, HttpResponse, Responder, get, http::header, post, route, web};
use chrono::{Datelike, Utc};
use sakiot_paths::SessionKey;
use serde::Serialize;
use sqlx::{Pool, Postgres, Row};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{Mutex, RwLock};

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::media_archive::MediaArchive;
use crate::permissions::require_channel_access;

use super::super::live::mark_cache_access;
use super::super::paths::recording_path;
use super::{AudioFragment, SessionAccess, fragment_path, load_fragments, require_session_access};

const MIX_OUTPUT: &str = "combined.ogg";
const MIX_FINGERPRINT: &str = "source-fingerprint";

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMixStatus {
    Unavailable,
    Waiting,
    Idle,
    Processing,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ChannelMixReason {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ChannelMixParticipant {
    pub user_id: String,
    pub display_name: Option<String>,
    pub session_ids: Vec<String>,
    pub source_count: i32,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ChannelMixResponse {
    pub status: ChannelMixStatus,
    pub reason: Option<ChannelMixReason>,
    pub progress: i16,
    pub duration_ms: i64,
    pub participants: Vec<ChannelMixParticipant>,
    pub source_count: i32,
    pub media_url: Option<String>,
}

#[derive(Debug)]
struct MixJob {
    fingerprint: String,
    progress: i16,
    failed: Option<String>,
}

/// A separate job container prevents a session mix from sharing progress or
/// retry state with waveform, silence-removal, or per-recording HLS jobs.
#[derive(Default, Debug)]
pub struct SessionMixContainer {
    jobs: RwLock<HashMap<i64, Arc<Mutex<MixJob>>>>,
    locks: Mutex<HashMap<i64, Arc<Mutex<()>>>>,
}

impl SessionMixContainer {
    async fn key_lock(&self, session_id: i64) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().await;
        locks
            .entry(session_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn job(&self, session_id: i64) -> Option<Arc<Mutex<MixJob>>> {
        self.jobs.read().await.get(&session_id).cloned()
    }

    async fn remove_if_same(&self, session_id: i64, expected: &Arc<Mutex<MixJob>>) {
        let mut jobs = self.jobs.write().await;
        if jobs
            .get(&session_id)
            .is_some_and(|current| Arc::ptr_eq(current, expected))
        {
            jobs.remove(&session_id);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MixInterval {
    pub start_ms: i64,
    pub end_ms: i64,
}

impl MixInterval {
    fn new(start_ms: i64, end_ms: i64) -> Option<Self> {
        (end_ms > start_ms).then_some(Self { start_ms, end_ms })
    }
}

/// Half-open interval intersection. Keeping this small and pure is useful for
/// both the database-backed planner and the exact-boundary test cases.
pub fn intersect_mix_intervals(left: MixInterval, right: MixInterval) -> Option<MixInterval> {
    MixInterval::new(
        left.start_ms.max(right.start_ms),
        left.end_ms.min(right.end_ms),
    )
}

#[derive(Clone, Debug)]
struct MixSource {
    audio_file_id: i64,
    path: PathBuf,
    source_start_ms: i64,
    overlap_start_ms: i64,
    overlap_end_ms: i64,
    delay_ms: i64,
}

#[derive(Clone, Debug)]
struct MixContributor {
    session_id: Option<i64>,
    user_id: i64,
    state: String,
    fragment: AudioFragment,
}

#[derive(Clone, Debug)]
struct MixPlan {
    session_id: i64,
    duration_ms: i64,
    contributors: Vec<MixContributor>,
    sources: Vec<MixSource>,
    participants: Vec<ChannelMixParticipant>,
    cache_dir: PathBuf,
    fingerprint: String,
}

impl MixPlan {
    fn source_count(&self) -> i32 {
        self.sources
            .iter()
            .map(|source| source.audio_file_id)
            .collect::<HashSet<_>>()
            .len()
            .try_into()
            .unwrap_or(i32::MAX)
    }

    fn has_active_contributor(&self) -> bool {
        self.contributors.iter().any(|contributor| {
            contributor.state != "finalized" || contributor.fragment.end_ms.is_none()
        })
    }
}

#[derive(Clone, Debug)]
struct CandidateRow {
    fragment: AudioFragment,
    state: String,
}

#[derive(Clone, Debug)]
struct MixPlanInputs {
    candidates: Vec<CandidateRow>,
}

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/channel-mix",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Timestamp-aligned channel mix status", body = ChannelMixResponse),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more contributor sessions are inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Session not found", body = crate::errors::ApiError),
        (status = 500, description = "Mix status failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/sessions/{recording_session_id}/channel-mix")]
pub async fn get_session_channel_mix(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    container: web::Data<SessionMixContainer>,
) -> Result<web::Json<ChannelMixResponse>, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let plan = build_mix_plan(&pool, &access, token.user_id).await?;
    Ok(web::Json(mix_response(&plan, &access, &container).await?))
}

#[utoipa::path(
    post,
    path = "/api/audio/sessions/{recording_session_id}/channel-mix",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Mix is ready or cannot currently be generated", body = ChannelMixResponse),
        (status = 202, description = "Mix render started or is already running", body = ChannelMixResponse),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more contributor sessions are inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Session not found", body = crate::errors::ApiError),
        (status = 500, description = "Mix render could not be started", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/audio/sessions/{recording_session_id}/channel-mix")]
pub async fn generate_session_channel_mix(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    container: web::Data<SessionMixContainer>,
    media: web::Data<MediaArchive>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let plan = build_mix_plan(&pool, &access, token.user_id).await?;
    let status = mix_response(&plan, &access, &container).await?;
    if !matches!(
        status.status,
        ChannelMixStatus::Idle | ChannelMixStatus::Failed
    ) {
        let mut response = if matches!(&status.status, ChannelMixStatus::Ready) {
            HttpResponse::Ok()
        } else {
            HttpResponse::Accepted()
        };
        return Ok(response.json(status));
    }

    start_mix_job(&pool, &container, media.get_ref(), plan.clone()).await?;
    let response = mix_response(&plan, &access, &container).await?;
    Ok(HttpResponse::Accepted().json(response))
}

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/channel-mix/media",
    tag = "audio",
    params(
        ("recording_session_id" = i64, Path, description = "Logical recording session id"),
        ("download" = Option<bool>, Query, description = "Download instead of inline playback"),
    ),
    responses(
        (status = 200, description = "Timestamp-aligned Ogg/Opus mix", content_type = "audio/ogg"),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more contributor sessions are inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Mix has not been generated", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[route(
    "/audio/sessions/{recording_session_id}/channel-mix/media",
    method = "GET",
    method = "HEAD"
)]
pub async fn get_session_channel_mix_media(
    request: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<ChannelMixMediaQuery>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<impl Responder, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let plan = build_mix_plan(&pool, &access, token.user_id).await?;
    let cache_file = plan.cache_dir.join(MIX_OUTPUT);
    if !cache_is_valid(&plan).await {
        return Err(AppError::FileNotFound);
    }
    mark_cache_access(&plan.cache_dir).await;
    let file = NamedFile::open_async(cache_file).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::FileNotFound
        } else {
            AppError::IoError(error)
        }
    })?;
    let disposition = if query.download.unwrap_or(false) {
        header::DispositionType::Attachment
    } else {
        header::DispositionType::Inline
    };
    Ok(file
        .set_content_disposition(header::ContentDisposition {
            disposition,
            parameters: vec![],
        })
        .into_response(&request))
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct ChannelMixMediaQuery {
    pub download: Option<bool>,
}

async fn build_mix_plan(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    viewer_user_id: i64,
) -> Result<MixPlan, AppError> {
    let timeline_end_ms = super::timeline_end_ms(access);
    let anchor_fragments = load_fragments(pool, access.session_id)
        .await?
        .into_iter()
        .filter(|fragment| {
            fragment.channel_id != 0
                && fragment.start_ms < timeline_end_ms
                && fragment.end_ms.unwrap_or(timeline_end_ms) > access.started_at_ms
        })
        .collect::<Vec<_>>();
    let inputs = load_mix_candidates(pool, access, &anchor_fragments, timeline_end_ms).await?;

    // Authorize every complete logical contributor before resolving display
    // names or returning the participant list. Legacy rows have no logical
    // parent; their channel permission is the only safe authorization scope.
    let mut authorized_sessions = HashSet::new();
    for candidate in &inputs.candidates {
        if let Some(session_id) = candidate.fragment.recording_session_id {
            if session_id == access.session_id || !authorized_sessions.insert(session_id) {
                continue;
            }
            require_session_access(pool, session_id, viewer_user_id).await?;
        } else {
            require_channel_access(
                pool,
                candidate.fragment.guild_id,
                candidate.fragment.channel_id,
                viewer_user_id,
            )
            .await?;
        }
    }

    let contributors = inputs
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.fragment.recording_session_id != Some(access.session_id)
                && candidate.fragment.user_id != access.user_id
        })
        .map(|candidate| MixContributor {
            session_id: candidate.fragment.recording_session_id,
            user_id: candidate.fragment.user_id,
            state: candidate.state.clone(),
            fragment: candidate.fragment.clone(),
        })
        .collect::<Vec<_>>();

    let duration_ms = timeline_end_ms.saturating_sub(access.started_at_ms);
    let mut sources = Vec::new();
    add_fragment_sources(
        &mut sources,
        &anchor_fragments,
        &anchor_fragments,
        access.started_at_ms,
        timeline_end_ms,
    );
    for contributor in &contributors {
        add_fragment_sources(
            &mut sources,
            &anchor_fragments,
            std::slice::from_ref(&contributor.fragment),
            access.started_at_ms,
            timeline_end_ms,
        );
    }

    let participants = participant_metadata(pool, access.guild_id, &contributors).await?;
    let cache_dir = mix_cache_dir(access, &anchor_fragments);
    let fingerprint = mix_fingerprint(access, duration_ms, &anchor_fragments, &contributors);

    Ok(MixPlan {
        session_id: access.session_id,
        duration_ms,
        contributors,
        sources,
        participants,
        cache_dir,
        fingerprint,
    })
}

async fn load_mix_candidates(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    anchor_fragments: &[AudioFragment],
    timeline_end_ms: i64,
) -> Result<MixPlanInputs, AppError> {
    if anchor_fragments.is_empty() || timeline_end_ms <= access.started_at_ms {
        return Ok(MixPlanInputs {
            candidates: Vec::new(),
        });
    }

    let rows = sqlx::query(
        "SELECT af.id,
                af.guild_id,
                af.channel_id,
                af.user_id,
                af.recording_session_id,
                af.file_name,
                af.year,
                af.month,
                af.start_ts,
                af.end_ts,
                af.segment_index,
                COALESCE(rs.state,
                         CASE WHEN af.end_ts IS NULL THEN 'active' ELSE 'finalized' END) AS session_state
           FROM audio_files af
           LEFT JOIN recording_sessions rs ON rs.id = af.recording_session_id
          WHERE af.guild_id = $1
            AND af.user_id <> $2
            AND af.start_ts IS NOT NULL
            AND af.start_ts < $4
            AND COALESCE(af.end_ts, $4) > $3
          ORDER BY af.start_ts, af.id",
    )
    .bind(access.guild_id)
    .bind(access.user_id)
    .bind(access.started_at_ms)
    .bind(timeline_end_ms)
    .fetch_all(pool.get_ref())
    .await?;

    let mut candidates = Vec::new();
    for row in rows {
        let fragment = AudioFragment {
            id: row.try_get("id")?,
            guild_id: row.try_get("guild_id")?,
            channel_id: row.try_get("channel_id")?,
            user_id: row.try_get("user_id")?,
            recording_session_id: row.try_get("recording_session_id")?,
            file_name: row.try_get("file_name")?,
            year: row.try_get("year")?,
            month: row.try_get("month")?,
            start_ms: row.try_get::<Option<i64>, _>("start_ts")?.unwrap_or(0),
            end_ms: row.try_get("end_ts")?,
            segment_index: row.try_get("segment_index")?,
            live: false,
        };
        let overlaps_anchor = anchor_fragments.iter().any(|anchor| {
            if anchor.channel_id != fragment.channel_id {
                return false;
            }
            let Some(anchor_interval) = MixInterval::new(
                anchor.start_ms.max(access.started_at_ms),
                anchor
                    .end_ms
                    .unwrap_or(timeline_end_ms)
                    .min(timeline_end_ms),
            ) else {
                return false;
            };
            let Some(fragment_interval) = MixInterval::new(
                fragment.start_ms,
                fragment.end_ms.unwrap_or(timeline_end_ms),
            ) else {
                return false;
            };
            intersect_mix_intervals(anchor_interval, fragment_interval).is_some()
        });
        if overlaps_anchor {
            candidates.push(CandidateRow {
                fragment,
                state: row.try_get("session_state")?,
            });
        }
    }
    Ok(MixPlanInputs { candidates })
}

fn add_fragment_sources(
    sources: &mut Vec<MixSource>,
    anchor_fragments: &[AudioFragment],
    source_fragments: &[AudioFragment],
    anchor_started_at_ms: i64,
    timeline_end_ms: i64,
) {
    for anchor in anchor_fragments {
        let Some(anchor_interval) = MixInterval::new(
            anchor.start_ms.max(anchor_started_at_ms),
            anchor
                .end_ms
                .unwrap_or(timeline_end_ms)
                .min(timeline_end_ms),
        ) else {
            continue;
        };
        for source in source_fragments {
            if source.channel_id != anchor.channel_id {
                continue;
            }
            let Some(source_interval) = MixInterval::new(
                source.start_ms,
                source.end_ms.unwrap_or(anchor_interval.end_ms),
            ) else {
                continue;
            };
            let Some(overlap) = intersect_mix_intervals(anchor_interval.clone(), source_interval)
            else {
                continue;
            };
            sources.push(MixSource {
                audio_file_id: source.id,
                path: fragment_path(source),
                source_start_ms: source.start_ms,
                overlap_start_ms: overlap.start_ms,
                overlap_end_ms: overlap.end_ms,
                delay_ms: overlap.start_ms.saturating_sub(anchor_started_at_ms),
            });
        }
    }
}

async fn participant_metadata(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    contributors: &[MixContributor],
) -> Result<Vec<ChannelMixParticipant>, AppError> {
    let mut by_user: HashMap<i64, ChannelMixParticipant> = HashMap::new();
    for contributor in contributors {
        let entry = by_user
            .entry(contributor.user_id)
            .or_insert_with(|| ChannelMixParticipant {
                user_id: contributor.user_id.to_string(),
                display_name: None,
                session_ids: Vec::new(),
                source_count: 0,
            });
        if let Some(session_id) = contributor.session_id
            && !entry.session_ids.contains(&session_id.to_string())
        {
            entry.session_ids.push(session_id.to_string());
        }
        entry.source_count = entry.source_count.saturating_add(1);
    }
    if by_user.is_empty() {
        return Ok(Vec::new());
    }

    let user_ids = by_user.keys().copied().collect::<Vec<_>>();
    let rows = sqlx::query(
        "SELECT un.user_id,
                COALESCE(nn.nickname, un.global_name, un.username) AS display_name
           FROM user_names un
           LEFT JOIN user_nicknames nn
             ON nn.user_id = un.user_id AND nn.guild_id = $1
          WHERE un.user_id = ANY($2)",
    )
    .bind(guild_id)
    .bind(&user_ids)
    .fetch_all(pool.get_ref())
    .await?;
    for row in rows {
        let user_id: i64 = row.try_get("user_id")?;
        if let Some(participant) = by_user.get_mut(&user_id) {
            participant.display_name = row.try_get("display_name")?;
        }
    }

    let mut result = by_user.into_values().collect::<Vec<_>>();
    result.sort_by_key(|participant| participant.user_id.parse::<i64>().unwrap_or(i64::MAX));
    for participant in &mut result {
        participant.session_ids.sort();
    }
    Ok(result)
}

fn mix_cache_dir(access: &SessionAccess, anchor_fragments: &[AudioFragment]) -> PathBuf {
    let (channel_id, year, month) = anchor_fragments
        .first()
        .map(|fragment| (fragment.channel_id, fragment.year, fragment.month as u32))
        .unwrap_or_else(|| {
            let date = chrono::DateTime::<Utc>::from_timestamp_millis(access.started_at_ms)
                .unwrap_or_else(Utc::now);
            (access.starting_channel_id, date.year(), date.month())
        });
    SessionKey::new(
        access.guild_id,
        channel_id,
        year,
        month,
        access.started_at_ms,
    )
    .mix_dir(&recording_path())
}

fn mix_fingerprint(
    access: &SessionAccess,
    duration_ms: i64,
    anchors: &[AudioFragment],
    contributors: &[MixContributor],
) -> String {
    let mut parts = vec![format!(
        "v2;session={};started={};duration={}",
        access.session_id, access.started_at_ms, duration_ms
    )];
    for fragment in anchors {
        parts.push(format!(
            "anchor:{}:{}:{}:{}:{}",
            fragment.id,
            fragment.channel_id,
            fragment.start_ms,
            fragment.end_ms.unwrap_or(0),
            fragment.file_name
        ));
    }
    for contributor in contributors {
        let fragment = &contributor.fragment;
        parts.push(format!(
            "source:{}:{}:{}:{}:{}:{}:{}",
            fragment.id,
            contributor.session_id.unwrap_or(0),
            contributor.user_id,
            fragment.channel_id,
            fragment.start_ms,
            fragment.end_ms.unwrap_or(0),
            contributor.state
        ));
    }
    parts.join("\n")
}

async fn cache_is_valid(plan: &MixPlan) -> bool {
    let output = plan.cache_dir.join(MIX_OUTPUT);
    let fingerprint = plan.cache_dir.join(MIX_FINGERPRINT);
    if !tokio::fs::try_exists(&output).await.unwrap_or(false)
        || !tokio::fs::try_exists(&fingerprint).await.unwrap_or(false)
    {
        return false;
    }
    matches!(
        tokio::fs::read_to_string(fingerprint).await,
        Ok(value) if value == plan.fingerprint
    )
}

async fn mix_response(
    plan: &MixPlan,
    access: &SessionAccess,
    container: &SessionMixContainer,
) -> Result<ChannelMixResponse, AppError> {
    if tokio::fs::try_exists(&plan.cache_dir)
        .await
        .unwrap_or(false)
    {
        mark_cache_access(&plan.cache_dir).await;
    }

    let common = |status, reason, progress, media_url| ChannelMixResponse {
        status,
        reason,
        progress,
        duration_ms: plan.duration_ms,
        participants: plan.participants.clone(),
        source_count: plan.source_count(),
        media_url,
    };
    if let Some(job) = container.job(plan.session_id).await {
        let job = job.lock().await;
        if job.fingerprint == plan.fingerprint {
            if let Some(error) = &job.failed {
                return Ok(common(
                    ChannelMixStatus::Failed,
                    Some(ChannelMixReason {
                        code: "render_failed".into(),
                        message: error.clone(),
                    }),
                    0,
                    None,
                ));
            }
            return Ok(common(
                ChannelMixStatus::Processing,
                None,
                job.progress,
                None,
            ));
        }
    }

    if let Some(reason) = anchor_wait_reason(access) {
        return Ok(common(ChannelMixStatus::Waiting, Some(reason), 0, None));
    }
    if plan.contributors.is_empty() {
        return Ok(common(
            ChannelMixStatus::Unavailable,
            Some(ChannelMixReason {
                code: "no_overlap".into(),
                message: "No other recorded participant overlaps the anchor timeline.".into(),
            }),
            0,
            None,
        ));
    }
    if plan.has_active_contributor() {
        return Ok(common(
            ChannelMixStatus::Waiting,
            Some(ChannelMixReason {
                code: "active_contributors".into(),
                message: "One or more overlapping contributor recordings are still active.".into(),
            }),
            0,
            None,
        ));
    }
    if cache_is_valid(plan).await {
        return Ok(common(
            ChannelMixStatus::Ready,
            None,
            100,
            Some(format!(
                "/api/audio/sessions/{}/channel-mix/media",
                plan.session_id
            )),
        ));
    }
    Ok(common(ChannelMixStatus::Idle, None, 0, None))
}

fn anchor_wait_reason(access: &SessionAccess) -> Option<ChannelMixReason> {
    match access.state.as_str() {
        "finalized" => None,
        "active" => Some(ChannelMixReason {
            code: "active_anchor".into(),
            message: "The anchor recording is still active.".into(),
        }),
        "pending" => Some(ChannelMixReason {
            code: "pending_anchor".into(),
            message: "The anchor recording is paused and awaiting finalization.".into(),
        }),
        _ => Some(ChannelMixReason {
            code: "unfinalized_anchor".into(),
            message: "The anchor recording is not finalized yet.".into(),
        }),
    }
}

async fn start_mix_job(
    pool: &web::Data<Pool<Postgres>>,
    container: &web::Data<SessionMixContainer>,
    media: &MediaArchive,
    plan: MixPlan,
) -> Result<(), AppError> {
    let lock = container.key_lock(plan.session_id).await;
    let _guard = lock.lock().await;
    if let Some(existing) = container.job(plan.session_id).await {
        let job = existing.lock().await;
        if job.fingerprint == plan.fingerprint && job.failed.is_none() {
            return Ok(());
        }
    }
    if cache_is_valid(&plan).await {
        return Ok(());
    }

    let job = Arc::new(Mutex::new(MixJob {
        fingerprint: plan.fingerprint.clone(),
        progress: 0,
        failed: None,
    }));
    container
        .jobs
        .write()
        .await
        .insert(plan.session_id, job.clone());
    let pool = pool.clone();
    let container = container.clone();
    let media = media.clone();
    tokio::spawn(async move {
        let result = render_mix(&pool, &media, &plan, &job).await;
        match result {
            Ok(()) => container.remove_if_same(plan.session_id, &job).await,
            Err(error) => {
                tracing::error!(
                    session_id = plan.session_id,
                    "channel mix render failed: {}",
                    error
                );
                let mut state = job.lock().await;
                state.progress = 0;
                state.failed = Some("Channel mix generation failed. Try again.".into());
            }
        }
    });
    Ok(())
}

async fn render_mix(
    pool: &web::Data<Pool<Postgres>>,
    media: &MediaArchive,
    plan: &MixPlan,
    job: &Arc<Mutex<MixJob>>,
) -> Result<(), AppError> {
    if plan.duration_ms <= 0 || plan.sources.is_empty() {
        return Err(AppError::BadRequest(
            "Channel mix has no renderable audio".into(),
        ));
    }
    tokio::fs::create_dir_all(&plan.cache_dir).await?;
    let mut materialized = HashSet::new();
    for source in &plan.sources {
        if materialized.insert(source.audio_file_id) {
            media
                .ensure_recording_local(pool.get_ref(), source.audio_file_id, &source.path)
                .await?;
        }
    }

    let temporary = plan
        .cache_dir
        .join(format!(".combined.{}.tmp.ogg", uuid::Uuid::new_v4()));
    let fingerprint_temporary = plan
        .cache_dir
        .join(format!(".fingerprint.{}.tmp", uuid::Uuid::new_v4()));
    let result = run_mix_ffmpeg(plan, job, &temporary).await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&temporary).await;
        let _ = tokio::fs::remove_file(&fingerprint_temporary).await;
        return Err(error);
    }
    if let Err(error) = tokio::fs::write(&fingerprint_temporary, &plan.fingerprint).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        let _ = tokio::fs::remove_file(&fingerprint_temporary).await;
        return Err(error.into());
    }
    if let Err(error) = tokio::fs::rename(&temporary, plan.cache_dir.join(MIX_OUTPUT)).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        let _ = tokio::fs::remove_file(&fingerprint_temporary).await;
        return Err(error.into());
    }
    if let Err(error) =
        tokio::fs::rename(&fingerprint_temporary, plan.cache_dir.join(MIX_FINGERPRINT)).await
    {
        let _ = tokio::fs::remove_file(plan.cache_dir.join(MIX_OUTPUT)).await;
        let _ = tokio::fs::remove_file(&fingerprint_temporary).await;
        return Err(error.into());
    }
    job.lock().await.progress = 100;
    Ok(())
}

async fn run_mix_ffmpeg(
    plan: &MixPlan,
    job: &Arc<Mutex<MixJob>>,
    output: &Path,
) -> Result<(), AppError> {
    let duration_seconds = super::milliseconds_as_seconds(plan.duration_ms);
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .arg("-y")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    for source in &plan.sources {
        command.args(["-i"]).arg(&source.path);
    }

    let filter = build_mix_filter(plan);
    command
        .args(["-filter_complex", &filter, "-map", "[out]"])
        .args([
            "-ar",
            "48000",
            "-ac",
            "1",
            "-c:a",
            "libopus",
            "-b:a",
            "96k",
            "-t",
            &duration_seconds,
            "-progress",
            "pipe:2",
            "-nostats",
        ])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::ServiceUnavailable(
                "ffmpeg executable is unavailable; install FFmpeg on the web server".into(),
            )
        } else {
            AppError::IoError(error)
        }
    })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::FfmpegError("FFmpeg stderr pipe unavailable".into()))?;
    let mut lines = BufReader::new(stderr).lines();
    let mut error_output = Vec::new();
    while let Some(line) = lines.next_line().await? {
        if let Some(value) = line.strip_prefix("out_time_us=")
            && let Ok(elapsed_us) = value.parse::<u64>()
        {
            let progress = elapsed_us
                .saturating_mul(99)
                .checked_div((plan.duration_ms.max(1) as u64).saturating_mul(1_000))
                .unwrap_or(0)
                .clamp(1, 99) as i16;
            let mut state = job.lock().await;
            state.progress = state.progress.max(progress);
        } else if !super::is_ffmpeg_progress_line(&line) && error_output.len() < 4_096 {
            let remaining = 4_096 - error_output.len();
            error_output.extend_from_slice(&line.as_bytes()[..line.len().min(remaining)]);
            error_output.push(b'\n');
        }
    }
    let status = child.wait().await?;
    if !status.success() {
        return Err(AppError::FfmpegError(
            String::from_utf8_lossy(&error_output).into_owned(),
        ));
    }
    Ok(())
}

fn build_mix_filter(plan: &MixPlan) -> String {
    let duration_seconds = super::milliseconds_as_seconds(plan.duration_ms);
    let duration_samples = plan.duration_ms.saturating_mul(48);
    let mut filter = String::new();
    for (index, source) in plan.sources.iter().enumerate() {
        let trim_start = super::milliseconds_as_seconds(
            source
                .overlap_start_ms
                .saturating_sub(source.source_start_ms),
        );
        let trim_end = super::milliseconds_as_seconds(
            source.overlap_end_ms.saturating_sub(source.source_start_ms),
        );
        let delay_samples = source.delay_ms.saturating_mul(48);
        filter.push_str(&format!(
            "[{index}:a]aresample=48000, aformat=sample_fmts=fltp:channel_layouts=mono, atrim=start={trim_start}:end={trim_end}, asetpts=PTS-STARTPTS, adelay={delay_samples}S|{delay_samples}S, apad=whole_len={duration_samples}, atrim=duration={duration_seconds}[mix{index}];"
        ));
    }
    for index in 0..plan.sources.len() {
        filter.push_str(&format!("[mix{index}]"));
    }
    filter.push_str(&format!(
        "amix=inputs={}:duration=longest:normalize=0:dropout_transition=0, alimiter=limit=0.95:attack=5:release=50:level=false, atrim=duration={duration_seconds}, asetpts=N/SR/TB[out]",
        plan.sources.len()
    ));
    filter
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(start_ms: i64, end_ms: i64) -> MixInterval {
        MixInterval { start_ms, end_ms }
    }

    fn session_access(state: &str) -> SessionAccess {
        SessionAccess {
            session_id: 1,
            guild_id: 2,
            user_id: 3,
            starting_channel_id: 4,
            state: state.into(),
            started_at_ms: 1_000,
            ended_at_ms: Some(9_000),
            pause_started_at_ms: Some(5_000),
        }
    }

    #[test]
    fn anchor_wait_reason_distinguishes_active_and_pending_sessions() {
        assert!(anchor_wait_reason(&session_access("finalized")).is_none());
        assert_eq!(
            anchor_wait_reason(&session_access("active"))
                .expect("active reason")
                .code,
            "active_anchor"
        );
        assert_eq!(
            anchor_wait_reason(&session_access("pending"))
                .expect("pending reason")
                .message,
            "The anchor recording is paused and awaiting finalization."
        );
    }

    #[test]
    fn staggered_intervals_are_trimmed_to_the_shared_half_open_range() {
        assert_eq!(
            intersect_mix_intervals(interval(1_000, 4_000), interval(2_500, 5_000)),
            Some(interval(2_500, 4_000))
        );
    }

    #[test]
    fn exact_boundaries_do_not_overlap() {
        assert_eq!(
            intersect_mix_intervals(interval(1_000, 2_000), interval(2_000, 3_000)),
            None
        );
    }

    #[test]
    fn a_repeated_anchor_visit_has_independent_overlap_windows() {
        let anchor = [interval(1_000, 2_000), interval(4_000, 5_000)];
        let contributor = interval(1_500, 4_500);
        let overlaps = anchor
            .iter()
            .filter_map(|window| intersect_mix_intervals(window.clone(), contributor.clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            overlaps,
            vec![interval(1_500, 2_000), interval(4_000, 4_500)]
        );
    }

    #[test]
    fn delay_is_relative_to_the_anchor_start_even_across_a_gap() {
        let overlap_start = 4_000;
        let anchor_start = 1_000;
        assert_eq!(overlap_start - anchor_start, 3_000);
    }

    #[test]
    fn channel_switches_never_contribute_to_the_wrong_anchor_visit() {
        let anchor = AudioFragment {
            id: 1,
            guild_id: 1,
            channel_id: 10,
            user_id: 20,
            recording_session_id: Some(1),
            file_name: "anchor".into(),
            year: 2026,
            month: 8,
            start_ms: 1_000,
            end_ms: Some(2_000),
            segment_index: Some(0),
            live: false,
        };
        let other_channel = AudioFragment {
            channel_id: 11,
            ..anchor.clone()
        };
        let mut sources = Vec::new();
        add_fragment_sources(&mut sources, &[anchor], &[other_channel], 1_000, 2_000);
        assert!(sources.is_empty());
    }

    #[test]
    fn output_duration_includes_anchor_handoff_gaps() {
        let anchor_start = 1_000;
        let anchor_end = 9_000;
        let fragments = [interval(1_000, 3_000), interval(6_000, 9_000)];
        let covered: i64 = fragments
            .iter()
            .map(|fragment| fragment.end_ms - fragment.start_ms)
            .sum();
        assert_eq!(anchor_end - anchor_start, 8_000);
        assert_eq!(covered, 5_000);
    }

    #[test]
    fn ffmpeg_graph_uses_timestamp_delay_mono_mix_and_limiter() {
        let plan = MixPlan {
            session_id: 1,
            duration_ms: 1_000,
            contributors: Vec::new(),
            sources: vec![MixSource {
                audio_file_id: 2,
                path: PathBuf::from("source.ogg"),
                source_start_ms: 1_000,
                overlap_start_ms: 1_500,
                overlap_end_ms: 2_000,
                delay_ms: 500,
            }],
            participants: Vec::new(),
            cache_dir: PathBuf::from("cache"),
            fingerprint: "test".into(),
        };
        let filter = build_mix_filter(&plan);
        assert!(filter.contains("atrim=start=0.500:end=1.000"));
        assert!(filter.contains("adelay=24000S|24000S"));
        assert!(filter.contains("amix=inputs=1:duration=longest:normalize=0"));
        assert!(filter.contains("alimiter=limit=0.95"));
        assert!(filter.contains("apad=whole_len=48000"));
        assert!(filter.contains("atrim=duration=1.000"));
    }

    #[tokio::test]
    async fn cached_mix_requires_the_current_source_fingerprint() {
        let directory = tempfile::tempdir().expect("temporary mix directory");
        let output = directory.path().join(MIX_OUTPUT);
        let fingerprint = directory.path().join(MIX_FINGERPRINT);
        tokio::fs::write(&output, b"ogg")
            .await
            .expect("write output");
        tokio::fs::write(&fingerprint, "current")
            .await
            .expect("write fingerprint");
        let plan = MixPlan {
            session_id: 1,
            duration_ms: 1_000,
            contributors: Vec::new(),
            sources: Vec::new(),
            participants: Vec::new(),
            cache_dir: directory.path().to_path_buf(),
            fingerprint: "current".into(),
        };
        assert!(cache_is_valid(&plan).await);
        let stale = MixPlan {
            fingerprint: "stale".into(),
            ..plan
        };
        assert!(!cache_is_valid(&stale).await);
    }

    #[tokio::test]
    async fn ffmpeg_fixture_starts_delayed_audio_at_the_expected_offset() {
        let Ok(version) = tokio::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .await
        else {
            return;
        };
        if !version.status.success() {
            return;
        }
        let directory = tempfile::tempdir().expect("temporary mix directory");
        let source = directory.path().join("source.ogg");
        let generated = tokio::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=880:sample_rate=48000:duration=1",
                "-ar",
                "48000",
                "-ac",
                "1",
                "-c:a",
                "libopus",
                "-b:a",
                "96k",
            ])
            .arg(&source)
            .output()
            .await
            .expect("generate fixture");
        assert!(generated.status.success());

        let output = directory.path().join("combined.ogg");
        let job = Arc::new(Mutex::new(MixJob {
            fingerprint: "fixture".into(),
            progress: 0,
            failed: None,
        }));
        let plan = MixPlan {
            session_id: 1,
            duration_ms: 1_000,
            contributors: Vec::new(),
            sources: vec![MixSource {
                audio_file_id: 2,
                path: source,
                source_start_ms: 0,
                overlap_start_ms: 0,
                overlap_end_ms: 1_000,
                delay_ms: 500,
            }],
            participants: Vec::new(),
            cache_dir: directory.path().to_path_buf(),
            fingerprint: "fixture".into(),
        };
        run_mix_ffmpeg(&plan, &job, &output)
            .await
            .expect("render delayed fixture");

        let probe = tokio::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(&output)
            .output()
            .await
            .expect("probe fixture");
        let duration = String::from_utf8_lossy(&probe.stdout)
            .trim()
            .parse::<f64>()
            .expect("fixture duration");
        assert!((0.98..=1.03).contains(&duration), "duration={duration}");

        let silence = tokio::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-nostats",
                "-i",
                output.to_str().expect("output path"),
                "-af",
                "silencedetect=n=-45dB:d=0.1",
                "-f",
                "null",
                "-",
            ])
            .output()
            .await
            .expect("inspect fixture");
        let stderr = String::from_utf8_lossy(&silence.stderr);
        let silence_end = stderr
            .split("silence_end: ")
            .nth(1)
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<f64>().ok())
            .expect("silence end");
        assert!(
            (0.4..=0.6).contains(&silence_end),
            "silence_end={silence_end}"
        );
    }
}
