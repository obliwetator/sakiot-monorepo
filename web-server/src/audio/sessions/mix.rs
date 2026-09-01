//! Timestamp-aligned channel mixes for finalized logical recordings.
//!
//! A mix is deliberately a filesystem cache rather than a database object. The
//! database remains the source of truth for the selected scope's timeline,
//! contributors, authorization, and cache fingerprint. A process-local job
//! container deduplicates concurrent renders while the cache makes successful
//! renders rebuildable after a restart.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use actix_files::NamedFile;
use actix_web::{HttpRequest, HttpResponse, Responder, get, http::header, post, route, web};
use sakiot_paths::RecordingKey;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres, Row};
use tokio::sync::{Mutex, RwLock};

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::media_archive::MediaArchive;
use crate::permissions::require_channel_access;

use super::super::live::mark_cache_access;
use super::{AudioFragment, SessionAccess, fragment_path, load_fragments, require_session_access};

mod cache;
mod occupancy;
mod render;

use cache::{
    anchor_wait_reason, cache_has_current_source, canonical_generation_settings,
    default_generation_settings, mix_cache_dir, mix_fingerprint, mix_response,
    mix_source_fingerprint,
};
use occupancy::{MixWindow, fallback_mix_windows, load_bot_occupancy_windows};
use render::start_mix_job;

#[cfg(test)]
use cache::{MixCacheMetadata, cache_is_valid};
#[cfg(test)]
use occupancy::{BotConnectionEvent, build_occupancy_windows, select_occupancy_windows};
#[cfg(test)]
use render::{build_mix_filter, run_mix_ffmpeg};

const MIX_OUTPUT: &str = "combined.ogg";
const MIX_FINGERPRINT: &str = "source-fingerprint";
const MIX_SETTINGS: &str = "generation-settings.json";

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

#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMixScope {
    #[default]
    AllRecordings,
    SelectedSession,
}

impl ChannelMixScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::AllRecordings => "all_recordings",
            Self::SelectedSession => "selected_session",
        }
    }
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, utoipa::ToSchema)]
pub struct ChannelMixParticipantSettings {
    pub user_id: String,
    pub gain_db: f32,
    pub muted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ChannelMixGenerationSettings {
    pub participants: Vec<ChannelMixParticipantSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ChannelMixSourceSegment {
    pub id: String,
    pub audio_file_id: String,
    pub recording_session_id: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub source_offset_ms: i64,
    pub source_duration_ms: i64,
    pub live: bool,
    pub media_url: String,
    pub hls_playlist_url: String,
    pub waveform_url: String,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ChannelMixTrack {
    pub user_id: String,
    pub display_name: Option<String>,
    pub is_anchor: bool,
    pub segments: Vec<ChannelMixSourceSegment>,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ChannelMixResponse {
    pub scope: ChannelMixScope,
    pub status: ChannelMixStatus,
    pub reason: Option<ChannelMixReason>,
    pub progress: i16,
    pub duration_ms: i64,
    pub participants: Vec<ChannelMixParticipant>,
    pub source_count: i32,
    pub media_url: Option<String>,
    pub can_generate: bool,
    pub tracks: Vec<ChannelMixTrack>,
    pub generation_settings: Option<ChannelMixGenerationSettings>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct GenerateChannelMixBody {
    #[serde(default, alias = "settings", alias = "participant_settings")]
    pub participants: Vec<ChannelMixParticipantSettings>,
}

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct ChannelMixQuery {
    pub scope: Option<ChannelMixScope>,
}

impl ChannelMixQuery {
    fn scope(&self) -> ChannelMixScope {
        self.scope.unwrap_or_default()
    }
}

#[derive(Debug)]
struct MixJob {
    source_fingerprint: String,
    settings: ChannelMixGenerationSettings,
    progress: i16,
    failed: Option<String>,
}

type MixJobKey = (i64, ChannelMixScope);
type MixJobHandle = Arc<Mutex<MixJob>>;
type MixJobs = HashMap<MixJobKey, MixJobHandle>;

/// A separate job container prevents a session mix from sharing progress or
/// retry state with waveform, silence-removal, or per-recording HLS jobs.
#[derive(Default, Debug)]
pub struct SessionMixContainer {
    jobs: RwLock<MixJobs>,
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

    async fn job(&self, session_id: i64, scope: ChannelMixScope) -> Option<Arc<Mutex<MixJob>>> {
        self.jobs.read().await.get(&(session_id, scope)).cloned()
    }

    async fn remove_if_same(
        &self,
        session_id: i64,
        scope: ChannelMixScope,
        expected: &Arc<Mutex<MixJob>>,
    ) {
        let mut jobs = self.jobs.write().await;
        if jobs
            .get(&(session_id, scope))
            .is_some_and(|current| Arc::ptr_eq(current, expected))
        {
            jobs.remove(&(session_id, scope));
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
    recording_session_id: Option<i64>,
    participant_user_id: i64,
    guild_id: i64,
    channel_id: i64,
    year: i32,
    month: i32,
    file_name: String,
    path: PathBuf,
    fragment_start_ms: i64,
    fragment_end_ms: i64,
    live: bool,
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
    scope: ChannelMixScope,
    duration_ms: i64,
    contributors: Vec<MixContributor>,
    sources: Vec<MixSource>,
    participants: Vec<ChannelMixParticipant>,
    tracks: Vec<ChannelMixTrack>,
    cache_dir: PathBuf,
    source_fingerprint: String,
    fingerprint: String,
    settings: ChannelMixGenerationSettings,
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

    fn can_generate(&self, access: &SessionAccess) -> bool {
        self.blocking_reason(access).is_none() && !self.sources.is_empty()
    }

    fn blocking_reason(&self, access: &SessionAccess) -> Option<ChannelMixReason> {
        if self.scope == ChannelMixScope::SelectedSession
            && let Some(reason) = anchor_wait_reason(access)
        {
            return Some(reason);
        }
        self.has_active_contributor().then_some(ChannelMixReason {
            code: "active_contributors".into(),
            message: "One or more channel mix recordings are still active.".into(),
        })
    }

    fn renderable_sources(&self) -> Vec<&MixSource> {
        self.sources
            .iter()
            .filter(|source| {
                !self
                    .settings
                    .participants
                    .iter()
                    .find(|participant| {
                        participant.user_id == source.participant_user_id.to_string()
                    })
                    .is_some_and(|participant| participant.muted)
            })
            .collect()
    }

    fn with_settings(&self, settings: ChannelMixGenerationSettings) -> Self {
        let mut plan = self.clone();
        plan.fingerprint = mix_fingerprint(&self.source_fingerprint, &settings);
        plan.settings = settings;
        plan
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
    params(
        ("recording_session_id" = i64, Path, description = "Logical recording session id"),
        ("scope" = Option<ChannelMixScope>, Query, description = "Timeline scope; all recordings during the bot's connected presence by default"),
    ),
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
    query: web::Query<ChannelMixQuery>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    container: web::Data<SessionMixContainer>,
) -> Result<web::Json<ChannelMixResponse>, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let plan = build_mix_plan(&pool, &access, token.user_id, query.scope()).await?;
    Ok(web::Json(
        mix_response(&plan, &access, &container, true).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/api/audio/sessions/{recording_session_id}/channel-mix",
    tag = "audio",
    params(
        ("recording_session_id" = i64, Path, description = "Logical recording session id"),
        ("scope" = Option<ChannelMixScope>, Query, description = "Timeline scope; all recordings during the bot's connected presence by default"),
    ),
    responses(
        (status = 200, description = "Mix is ready or cannot currently be generated", body = ChannelMixResponse),
        (status = 202, description = "Mix render started or is already running", body = ChannelMixResponse),
        (status = 409, description = "Mix generation is not allowed while a source is live or pending", body = ChannelMixResponse),
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
    query: web::Query<ChannelMixQuery>,
    body: Option<web::Json<GenerateChannelMixBody>>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    container: web::Data<SessionMixContainer>,
    media: web::Data<MediaArchive>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let base_plan = build_mix_plan(&pool, &access, token.user_id, query.scope()).await?;
    let settings = canonical_generation_settings(
        &base_plan,
        body.map(|body| body.into_inner().participants)
            .unwrap_or_default(),
    )?;
    let plan = base_plan.with_settings(settings);
    let status = mix_response(&plan, &access, &container, false).await?;

    // A live/pending mix is useful for browser preview, but server rendering
    // must never snapshot it. Return the full status document so clients can
    // keep showing the newly discovered tracks while respecting the conflict.
    if plan.blocking_reason(&access).is_some() {
        return Ok(HttpResponse::Conflict().json(status));
    }
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
    let response = mix_response(&plan, &access, &container, false).await?;
    Ok(HttpResponse::Accepted().json(response))
}

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/channel-mix/media",
    tag = "audio",
    params(
        ("recording_session_id" = i64, Path, description = "Logical recording session id"),
        ("download" = Option<bool>, Query, description = "Download instead of inline playback"),
        ("scope" = Option<ChannelMixScope>, Query, description = "Timeline scope; all recordings during the bot's connected presence by default"),
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
    let plan = build_mix_plan(
        &pool,
        &access,
        token.user_id,
        query.scope.unwrap_or_default(),
    )
    .await?;
    let cache_file = plan.cache_dir.join(MIX_OUTPUT);
    if !cache_has_current_source(&plan).await {
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
    pub scope: Option<ChannelMixScope>,
}

async fn build_mix_plan(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    viewer_user_id: i64,
    scope: ChannelMixScope,
) -> Result<MixPlan, AppError> {
    let selected_timeline_end_ms = super::timeline_end_ms(access);
    let selected_fragments = load_fragments(pool, access.session_id)
        .await?
        .into_iter()
        .filter(|fragment| {
            fragment.channel_id != 0
                && fragment.start_ms < selected_timeline_end_ms
                && fragment.end_ms.unwrap_or(selected_timeline_end_ms) > access.started_at_ms
        })
        .collect::<Vec<_>>();
    let (timeline_start_ms, timeline_end_ms, windows, source_anchor_fragments, inputs) = match scope
    {
        ChannelMixScope::SelectedSession => {
            let windows =
                fallback_mix_windows(access, &selected_fragments, selected_timeline_end_ms);
            let inputs = load_mix_candidates(
                pool,
                access,
                &windows,
                access.started_at_ms,
                selected_timeline_end_ms,
                Some(access.user_id),
            )
            .await?;
            (
                access.started_at_ms,
                selected_timeline_end_ms,
                windows,
                selected_fragments.clone(),
                inputs,
            )
        }
        ChannelMixScope::AllRecordings => {
            let mut windows = load_bot_occupancy_windows(
                pool,
                access,
                &selected_fragments,
                selected_timeline_end_ms,
            )
            .await?;
            if windows.is_empty() {
                windows =
                    fallback_mix_windows(access, &selected_fragments, selected_timeline_end_ms);
            }
            let timeline_start_ms = windows
                .iter()
                .map(|window| window.start_ms)
                .min()
                .unwrap_or(access.started_at_ms);
            let timeline_end_ms = windows
                .iter()
                .map(|window| window.end_ms)
                .max()
                .unwrap_or(selected_timeline_end_ms);
            let inputs = load_mix_candidates(
                pool,
                access,
                &windows,
                timeline_start_ms,
                timeline_end_ms,
                None,
            )
            .await?;
            (
                timeline_start_ms,
                timeline_end_ms,
                windows,
                Vec::new(),
                inputs,
            )
        }
    };

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
            scope == ChannelMixScope::AllRecordings
                || candidate.fragment.recording_session_id != Some(access.session_id)
        })
        .map(|candidate| MixContributor {
            session_id: candidate.fragment.recording_session_id,
            user_id: candidate.fragment.user_id,
            state: candidate.state.clone(),
            fragment: candidate.fragment.clone(),
        })
        .collect::<Vec<_>>();

    let duration_ms = timeline_end_ms.saturating_sub(timeline_start_ms);
    let mut sources = Vec::new();
    if scope == ChannelMixScope::SelectedSession {
        add_window_sources(
            &mut sources,
            &windows,
            &source_anchor_fragments,
            access.user_id,
            timeline_start_ms,
            timeline_end_ms,
        );
        for contributor in &contributors {
            add_window_sources(
                &mut sources,
                &windows,
                std::slice::from_ref(&contributor.fragment),
                contributor.user_id,
                timeline_start_ms,
                timeline_end_ms,
            );
        }
    } else {
        for candidate in &inputs.candidates {
            add_window_sources(
                &mut sources,
                &windows,
                std::slice::from_ref(&candidate.fragment),
                candidate.fragment.user_id,
                timeline_start_ms,
                timeline_end_ms,
            );
        }
    }

    let participants = participant_metadata(
        pool,
        access.guild_id,
        (scope == ChannelMixScope::SelectedSession).then_some((
            access.user_id,
            access.session_id,
            source_anchor_fragments.as_slice(),
        )),
        &contributors,
    )
    .await?;
    let tracks = build_tracks(
        &participants,
        &sources,
        (scope == ChannelMixScope::SelectedSession).then_some(access.user_id),
        timeline_start_ms,
    );
    let cache_dir = mix_cache_dir(access, &selected_fragments);
    let source_fingerprint = mix_source_fingerprint(
        access,
        scope,
        timeline_start_ms,
        duration_ms,
        &windows,
        &source_anchor_fragments,
        &contributors,
    );
    let settings = default_generation_settings(&tracks);
    let fingerprint = mix_fingerprint(&source_fingerprint, &settings);

    Ok(MixPlan {
        session_id: access.session_id,
        scope,
        duration_ms,
        contributors,
        sources,
        participants,
        tracks,
        cache_dir,
        source_fingerprint,
        fingerprint,
        settings,
    })
}

async fn load_mix_candidates(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    windows: &[MixWindow],
    timeline_start_ms: i64,
    timeline_end_ms: i64,
    excluded_user_id: Option<i64>,
) -> Result<MixPlanInputs, AppError> {
    if windows.is_empty() || timeline_end_ms <= timeline_start_ms {
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
                (
                    af.end_ts IS NULL
                    AND af.reaped IS FALSE
                    AND EXISTS (
                        SELECT 1
                          FROM bot_instances bi
                         WHERE bi.instance_id = af.recording_owner_instance_id
                           AND af.recording_heartbeat_at > now() - interval '120 seconds'
                           AND bi.heartbeat_at > now() - interval '120 seconds'
                           AND bi.state <> 'stopped'
                    )
                ) AS live,
                COALESCE(rs.state,
                         CASE WHEN af.end_ts IS NULL THEN 'active' ELSE 'finalized' END) AS session_state
           FROM audio_files af
           LEFT JOIN recording_sessions rs ON rs.id = af.recording_session_id
          WHERE af.guild_id = $1
            AND ($2::bigint IS NULL OR af.user_id <> $2)
            AND af.start_ts IS NOT NULL
            AND af.start_ts < $4
            AND COALESCE(af.end_ts, $4) > $3
          ORDER BY af.start_ts, af.id",
    )
    .bind(access.guild_id)
    .bind(excluded_user_id)
    .bind(timeline_start_ms)
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
            live: row.try_get::<Option<bool>, _>("live")?.unwrap_or(false),
        };
        let overlaps_window = windows.iter().any(|window| {
            if window.channel_id != fragment.channel_id {
                return false;
            }
            let Some(fragment_interval) = MixInterval::new(
                fragment.start_ms,
                fragment.end_ms.unwrap_or(timeline_end_ms),
            ) else {
                return false;
            };
            intersect_mix_intervals(window.interval(), fragment_interval).is_some()
        });
        if overlaps_window {
            candidates.push(CandidateRow {
                fragment,
                state: row.try_get("session_state")?,
            });
        }
    }
    Ok(MixPlanInputs { candidates })
}

#[cfg(test)]
fn add_fragment_sources(
    sources: &mut Vec<MixSource>,
    anchor_fragments: &[AudioFragment],
    source_fragments: &[AudioFragment],
    participant_user_id: i64,
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
            add_source_overlap(
                sources,
                source,
                participant_user_id,
                overlap,
                anchor_started_at_ms,
                timeline_end_ms,
            );
        }
    }
}

fn add_window_sources(
    sources: &mut Vec<MixSource>,
    windows: &[MixWindow],
    source_fragments: &[AudioFragment],
    participant_user_id: i64,
    timeline_start_ms: i64,
    timeline_end_ms: i64,
) {
    for source in source_fragments {
        for window in windows {
            if source.channel_id != window.channel_id {
                continue;
            }
            let Some(source_interval) =
                MixInterval::new(source.start_ms, source.end_ms.unwrap_or(timeline_end_ms))
            else {
                continue;
            };
            let Some(overlap) = intersect_mix_intervals(window.interval(), source_interval) else {
                continue;
            };
            add_source_overlap(
                sources,
                source,
                participant_user_id,
                overlap,
                timeline_start_ms,
                timeline_end_ms,
            );
        }
    }
}

fn add_source_overlap(
    sources: &mut Vec<MixSource>,
    source: &AudioFragment,
    participant_user_id: i64,
    overlap: MixInterval,
    timeline_start_ms: i64,
    timeline_end_ms: i64,
) {
    sources.push(MixSource {
        audio_file_id: source.id,
        recording_session_id: source.recording_session_id,
        participant_user_id,
        guild_id: source.guild_id,
        channel_id: source.channel_id,
        year: source.year,
        month: source.month,
        file_name: source.file_name.clone(),
        path: fragment_path(source),
        fragment_start_ms: source.start_ms,
        fragment_end_ms: source.end_ms.unwrap_or(timeline_end_ms),
        live: source.live,
        source_start_ms: source.start_ms,
        overlap_start_ms: overlap.start_ms,
        overlap_end_ms: overlap.end_ms,
        delay_ms: overlap.start_ms.saturating_sub(timeline_start_ms),
    });
}

async fn participant_metadata(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    anchor: Option<(i64, i64, &[AudioFragment])>,
    contributors: &[MixContributor],
) -> Result<Vec<ChannelMixParticipant>, AppError> {
    let mut by_user: HashMap<i64, ChannelMixParticipant> = HashMap::new();
    if let Some((anchor_user_id, anchor_session_id, anchor_fragments)) = anchor {
        let entry = by_user
            .entry(anchor_user_id)
            .or_insert_with(|| ChannelMixParticipant {
                user_id: anchor_user_id.to_string(),
                display_name: None,
                session_ids: vec![anchor_session_id.to_string()],
                source_count: 0,
            });
        let mut anchor_sources = HashSet::new();
        for fragment in anchor_fragments {
            anchor_sources.insert(fragment.id);
        }
        entry.source_count = anchor_sources.len().try_into().unwrap_or(i32::MAX);
    }
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

fn build_tracks(
    participants: &[ChannelMixParticipant],
    sources: &[MixSource],
    anchor_user_id: Option<i64>,
    timeline_start_ms: i64,
) -> Vec<ChannelMixTrack> {
    let mut tracks = participants
        .iter()
        .filter_map(|participant| {
            let user_id = participant.user_id.parse::<i64>().ok()?;
            let mut segments = sources
                .iter()
                .filter(|source| source.participant_user_id == user_id)
                .map(|source| source_segment(source, timeline_start_ms))
                .collect::<Vec<_>>();
            segments.sort_by_key(|segment| {
                (
                    segment.start_ms,
                    segment.end_ms,
                    segment.audio_file_id.clone(),
                )
            });
            Some(ChannelMixTrack {
                user_id: participant.user_id.clone(),
                display_name: participant.display_name.clone(),
                is_anchor: anchor_user_id == Some(user_id),
                segments,
            })
        })
        .collect::<Vec<_>>();
    tracks.sort_by_key(|track| (if track.is_anchor { 0 } else { 1 }, track.user_id.clone()));
    tracks
}

fn source_segment(source: &MixSource, anchor_started_at_ms: i64) -> ChannelMixSourceSegment {
    let key = RecordingKey::new(
        source.guild_id,
        source.channel_id,
        source.year,
        source.month as u32,
        source.file_name.clone(),
    );
    let (media_url, hls_playlist_url) = match source.recording_session_id {
        Some(session_id) => (
            format!(
                "/api/audio/sessions/{session_id}/segments/{}",
                source.audio_file_id
            ),
            format!(
                "/api/audio/sessions/{session_id}/live/{}/playlist.m3u8",
                source.audio_file_id
            ),
        ),
        None => (
            key.audio_url(),
            format!(
                "/api/audio/live/{}/{}/{:04}/{:02}/{}/playlist.m3u8",
                key.guild_id, key.channel_id, key.year, key.month, key.stem
            ),
        ),
    };
    let start_ms = source.overlap_start_ms.saturating_sub(anchor_started_at_ms);
    let end_ms = source.overlap_end_ms.saturating_sub(anchor_started_at_ms);
    ChannelMixSourceSegment {
        id: format!("{}:{}", source.audio_file_id, source.overlap_start_ms),
        audio_file_id: source.audio_file_id.to_string(),
        recording_session_id: source.recording_session_id.map(|id| id.to_string()),
        start_ms,
        end_ms,
        source_offset_ms: source
            .overlap_start_ms
            .saturating_sub(source.source_start_ms),
        source_duration_ms: source
            .fragment_end_ms
            .saturating_sub(source.fragment_start_ms),
        live: source.live,
        media_url,
        hls_playlist_url,
        waveform_url: key.waveform_url(),
    }
}

#[cfg(test)]
mod tests;
