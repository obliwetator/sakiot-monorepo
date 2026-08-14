//! Timestamp-aligned channel mixes for finalized logical recordings.
//!
//! A mix is deliberately a filesystem cache rather than a database object. The
//! database remains the source of truth for the selected scope's timeline,
//! contributors, authorization, and cache fingerprint. A process-local job
//! container deduplicates concurrent renders while the cache makes successful
//! renders rebuildable after a restart.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use actix_files::NamedFile;
use actix_web::{HttpRequest, HttpResponse, Responder, get, http::header, post, route, web};
use chrono::{Datelike, Utc};
use sakiot_paths::{RecordingKey, SessionKey};
use serde::{Deserialize, Serialize};
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct MixWindow {
    channel_id: i64,
    start_ms: i64,
    end_ms: i64,
}

impl MixWindow {
    fn new(channel_id: i64, start_ms: i64, end_ms: i64) -> Option<Self> {
        (end_ms > start_ms).then_some(Self {
            channel_id,
            start_ms,
            end_ms,
        })
    }

    fn interval(&self) -> MixInterval {
        MixInterval {
            start_ms: self.start_ms,
            end_ms: self.end_ms,
        }
    }
}

#[derive(Clone, Debug)]
struct BotConnectionEvent {
    started_ms: i64,
    completed_ms: i64,
    to_channel_id: Option<i64>,
    outcome: String,
}

#[derive(Clone, Debug)]
struct OccupancyWindow {
    channel_id: i64,
    start_ms: i64,
    end_ms: i64,
    episode_id: i64,
}

fn is_successful_connection_outcome(outcome: &str) -> bool {
    matches!(
        outcome,
        "joined" | "rejoined" | "switched" | "already_in_channel" | "disconnected"
    ) || outcome == "switched_after_join_error"
}

fn build_occupancy_windows(
    events: &[BotConnectionEvent],
    horizon_end_ms: i64,
) -> Vec<OccupancyWindow> {
    let mut windows = Vec::new();
    let mut current: Option<(i64, i64, i64)> = None;
    let mut next_episode_id = 0;

    for event in events {
        if !is_successful_connection_outcome(&event.outcome) {
            continue;
        }
        let operation_start_ms = event.started_ms.min(event.completed_ms);
        let operation_end_ms = event.completed_ms.max(event.started_ms);
        match event.to_channel_id {
            Some(channel_id) => {
                let episode_id =
                    if let Some((current_channel_id, episode_id, current_start_ms)) = current {
                        if current_channel_id == channel_id {
                            // A successful health-check/rejoin to the same channel
                            // does not split the bot's connected presence.
                            continue;
                        }
                        if let Some(window) =
                            MixWindow::new(current_channel_id, current_start_ms, operation_start_ms)
                        {
                            windows.push(OccupancyWindow {
                                channel_id: window.channel_id,
                                start_ms: window.start_ms,
                                end_ms: window.end_ms,
                                episode_id,
                            });
                        }
                        episode_id
                    } else {
                        next_episode_id += 1;
                        next_episode_id
                    };
                current = Some((channel_id, episode_id, operation_end_ms));
            }
            None => {
                if let Some((channel_id, episode_id, current_start_ms)) = current.take()
                    && let Some(window) =
                        MixWindow::new(channel_id, current_start_ms, operation_start_ms)
                {
                    windows.push(OccupancyWindow {
                        channel_id: window.channel_id,
                        start_ms: window.start_ms,
                        end_ms: window.end_ms,
                        episode_id,
                    });
                }
            }
        }
    }

    if let Some((channel_id, episode_id, current_start_ms)) = current
        && let Some(window) = MixWindow::new(channel_id, current_start_ms, horizon_end_ms)
    {
        windows.push(OccupancyWindow {
            channel_id: window.channel_id,
            start_ms: window.start_ms,
            end_ms: window.end_ms,
            episode_id,
        });
    }
    windows
}

fn select_occupancy_windows(
    occupancy: Vec<OccupancyWindow>,
    selected_start_ms: i64,
    selected_end_ms: i64,
    selected_channels: &HashSet<i64>,
) -> Vec<MixWindow> {
    let selected_episodes = occupancy
        .iter()
        .filter(|window| {
            selected_channels.contains(&window.channel_id)
                && window.start_ms < selected_end_ms
                && window.end_ms > selected_start_ms
        })
        .map(|window| window.episode_id)
        .collect::<HashSet<_>>();
    occupancy
        .into_iter()
        .filter(|window| {
            selected_episodes.contains(&window.episode_id)
                && selected_channels.contains(&window.channel_id)
        })
        .filter_map(|window| MixWindow::new(window.channel_id, window.start_ms, window.end_ms))
        .collect()
}

async fn load_bot_occupancy_windows(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    selected_fragments: &[AudioFragment],
    selected_timeline_end_ms: i64,
) -> Result<Vec<MixWindow>, AppError> {
    let now_ms = Utc::now().timestamp_millis();
    let rows = sqlx::query(
        "SELECT ((EXTRACT(EPOCH FROM started_at) * 1000)::bigint) AS started_ms,
                ((EXTRACT(EPOCH FROM completed_at) * 1000)::bigint) AS completed_ms,
                to_channel_id,
                outcome
           FROM voice_connection_events
          WHERE guild_id = $1
            AND completed_at <= now()
          ORDER BY completed_at, id",
    )
    .bind(access.guild_id)
    .fetch_all(pool.get_ref())
    .await?;
    let events = rows
        .into_iter()
        .map(|row| {
            Ok(BotConnectionEvent {
                started_ms: row.try_get("started_ms")?,
                completed_ms: row.try_get("completed_ms")?,
                to_channel_id: row.try_get("to_channel_id")?,
                outcome: row.try_get("outcome")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let occupancy = build_occupancy_windows(&events, now_ms.max(selected_timeline_end_ms));
    let selected_start_ms = access.started_at_ms;
    let selected_channels = selected_fragments
        .iter()
        .filter(|fragment| fragment.channel_id != 0)
        .map(|fragment| fragment.channel_id)
        .chain(std::iter::once(access.starting_channel_id))
        .collect::<HashSet<_>>();
    Ok(select_occupancy_windows(
        occupancy,
        selected_start_ms,
        selected_timeline_end_ms,
        &selected_channels,
    ))
}

fn fallback_mix_windows(
    access: &SessionAccess,
    fragments: &[AudioFragment],
    timeline_end_ms: i64,
) -> Vec<MixWindow> {
    fragments
        .iter()
        .filter(|fragment| fragment.channel_id != 0)
        .filter_map(|fragment| {
            MixWindow::new(
                fragment.channel_id,
                fragment.start_ms.max(access.started_at_ms),
                fragment
                    .end_ms
                    .unwrap_or(timeline_end_ms)
                    .min(timeline_end_ms),
            )
        })
        .collect()
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

fn default_generation_settings(tracks: &[ChannelMixTrack]) -> ChannelMixGenerationSettings {
    let mut participants = tracks
        .iter()
        .map(|track| ChannelMixParticipantSettings {
            user_id: track.user_id.clone(),
            gain_db: 0.0,
            muted: false,
        })
        .collect::<Vec<_>>();
    participants.sort_by_key(|participant| participant.user_id.parse::<i64>().unwrap_or(i64::MAX));
    ChannelMixGenerationSettings {
        participants,
        source_fingerprint: None,
    }
}

fn canonical_generation_settings(
    plan: &MixPlan,
    requested: Vec<ChannelMixParticipantSettings>,
) -> Result<ChannelMixGenerationSettings, AppError> {
    let known = plan
        .tracks
        .iter()
        .filter_map(|track| track.user_id.parse::<i64>().ok())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    for participant in &requested {
        let user_id = participant.user_id.parse::<i64>().map_err(|_| {
            AppError::BadRequest(format!(
                "Unknown channel mix participant {}",
                participant.user_id
            ))
        })?;
        if !known.contains(&user_id) {
            return Err(AppError::BadRequest(format!(
                "Unknown channel mix participant {}",
                participant.user_id
            )));
        }
        if !seen.insert(user_id) {
            return Err(AppError::BadRequest(format!(
                "Duplicate channel mix participant {}",
                participant.user_id
            )));
        }
        if !participant.gain_db.is_finite() || !(-60.0..=12.0).contains(&participant.gain_db) {
            return Err(AppError::BadRequest(format!(
                "Channel mix gain for {} must be between -60 dB and +12 dB",
                participant.user_id
            )));
        }
    }

    let mut settings = default_generation_settings(&plan.tracks);
    let mut values = requested
        .into_iter()
        .map(|participant| {
            let user_id = participant.user_id.parse::<i64>().unwrap_or_default();
            (
                user_id,
                ChannelMixParticipantSettings {
                    user_id: user_id.to_string(),
                    gain_db: if participant.gain_db == 0.0 {
                        0.0
                    } else {
                        participant.gain_db
                    },
                    muted: participant.muted,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    for participant in &mut settings.participants {
        let user_id = participant.user_id.parse::<i64>().unwrap_or_default();
        if let Some(value) = values.remove(&user_id) {
            *participant = value;
        }
    }
    if !settings.participants.is_empty()
        && settings
            .participants
            .iter()
            .all(|participant| participant.muted)
    {
        return Err(AppError::BadRequest(
            "At least one channel mix participant must be unmuted".into(),
        ));
    }
    Ok(settings)
}

fn mix_source_fingerprint(
    access: &SessionAccess,
    scope: ChannelMixScope,
    timeline_start_ms: i64,
    duration_ms: i64,
    windows: &[MixWindow],
    anchors: &[AudioFragment],
    contributors: &[MixContributor],
) -> String {
    let mut parts = vec![format!(
        "v4;scope={};session={};timeline_start={};duration={}",
        scope.as_str(),
        access.session_id,
        timeline_start_ms,
        duration_ms
    )];
    for window in windows {
        parts.push(format!(
            "window:{}:{}:{}",
            window.channel_id, window.start_ms, window.end_ms
        ));
    }
    for fragment in anchors {
        parts.push(format!(
            "anchor:{}:{}:{}:{}:{}:{}:{}:{}",
            fragment.id,
            fragment.channel_id,
            fragment.start_ms,
            fragment.end_ms.unwrap_or(0),
            fragment.file_name,
            fragment.year,
            fragment.month,
            fragment.live
        ));
    }
    for contributor in contributors {
        let fragment = &contributor.fragment;
        parts.push(format!(
            "source:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            fragment.id,
            contributor.session_id.unwrap_or(0),
            contributor.user_id,
            fragment.channel_id,
            fragment.start_ms,
            fragment.end_ms.unwrap_or(0),
            contributor.state,
            fragment.file_name,
            fragment.year,
            fragment.month,
            fragment.live
        ));
    }
    parts.join("\n")
}

fn mix_fingerprint(source_fingerprint: &str, settings: &ChannelMixGenerationSettings) -> String {
    let settings = serde_json::to_string(&settings.participants).unwrap_or_default();
    format!("{source_fingerprint}\nsettings:{settings}")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct MixCacheMetadata {
    source_fingerprint: String,
    fingerprint: String,
    settings: ChannelMixGenerationSettings,
}

fn response_settings(
    settings: &ChannelMixGenerationSettings,
    source_fingerprint: &str,
) -> ChannelMixGenerationSettings {
    let mut settings = settings.clone();
    settings.source_fingerprint = Some(source_fingerprint.to_owned());
    settings
}

async fn read_cache_metadata(plan: &MixPlan) -> Option<MixCacheMetadata> {
    let output = plan.cache_dir.join(MIX_OUTPUT);
    let fingerprint = plan.cache_dir.join(MIX_FINGERPRINT);
    let settings = plan.cache_dir.join(MIX_SETTINGS);
    if !tokio::fs::try_exists(&output).await.unwrap_or(false)
        || !tokio::fs::try_exists(&fingerprint).await.unwrap_or(false)
        || !tokio::fs::try_exists(&settings).await.unwrap_or(false)
    {
        return None;
    }
    let fingerprint_value = tokio::fs::read_to_string(fingerprint).await.ok()?;
    let metadata = tokio::fs::read_to_string(settings)
        .await
        .ok()
        .and_then(|value| serde_json::from_str::<MixCacheMetadata>(&value).ok())?;
    (metadata.fingerprint == fingerprint_value).then_some(metadata)
}

async fn cache_has_current_source(plan: &MixPlan) -> bool {
    read_cache_metadata(plan)
        .await
        .is_some_and(|metadata| metadata.source_fingerprint == plan.source_fingerprint)
}

async fn cache_is_valid(plan: &MixPlan) -> bool {
    read_cache_metadata(plan).await.is_some_and(|metadata| {
        metadata.source_fingerprint == plan.source_fingerprint
            && metadata.fingerprint == plan.fingerprint
    })
}

async fn mix_response(
    plan: &MixPlan,
    access: &SessionAccess,
    container: &SessionMixContainer,
    include_existing_artifact: bool,
) -> Result<ChannelMixResponse, AppError> {
    if tokio::fs::try_exists(&plan.cache_dir)
        .await
        .unwrap_or(false)
    {
        mark_cache_access(&plan.cache_dir).await;
    }

    let common = |status,
                  reason,
                  progress,
                  media_url,
                  generation_settings: Option<ChannelMixGenerationSettings>| {
        ChannelMixResponse {
            scope: plan.scope,
            status,
            reason,
            progress,
            duration_ms: plan.duration_ms,
            participants: plan.participants.clone(),
            source_count: plan.source_count(),
            media_url,
            can_generate: plan.can_generate(access),
            tracks: plan.tracks.clone(),
            generation_settings,
        }
    };
    if let Some(reason) = plan.blocking_reason(access) {
        return Ok(common(
            ChannelMixStatus::Waiting,
            Some(reason),
            0,
            None,
            None,
        ));
    }
    if plan.sources.is_empty() {
        return Ok(common(
            ChannelMixStatus::Unavailable,
            Some(ChannelMixReason {
                code: "no_overlap".into(),
                message: match plan.scope {
                    ChannelMixScope::AllRecordings => {
                        "No recordings were found while the bot was connected to this channel."
                            .into()
                    }
                    ChannelMixScope::SelectedSession => {
                        "No other recorded participant overlaps the selected session timeline."
                            .into()
                    }
                },
            }),
            0,
            None,
            None,
        ));
    }
    if let Some(job) = container.job(plan.session_id, plan.scope).await {
        let job = job.lock().await;
        if job.source_fingerprint != plan.source_fingerprint {
            return Ok(common(ChannelMixStatus::Idle, None, 0, None, None));
        }
        let settings = Some(response_settings(&job.settings, &job.source_fingerprint));
        if let Some(error) = &job.failed {
            return Ok(common(
                ChannelMixStatus::Failed,
                Some(ChannelMixReason {
                    code: "render_failed".into(),
                    message: error.clone(),
                }),
                0,
                None,
                settings,
            ));
        }
        return Ok(common(
            ChannelMixStatus::Processing,
            None,
            job.progress,
            None,
            settings,
        ));
    }
    if cache_is_valid(plan).await {
        return Ok(common(
            ChannelMixStatus::Ready,
            None,
            100,
            Some(format!(
                "/api/audio/sessions/{}/channel-mix/media?scope={}",
                plan.session_id,
                plan.scope.as_str()
            )),
            Some(response_settings(&plan.settings, &plan.source_fingerprint)),
        ));
    }
    if include_existing_artifact
        && let Some(metadata) = read_cache_metadata(plan).await
        && metadata.source_fingerprint == plan.source_fingerprint
    {
        return Ok(common(
            ChannelMixStatus::Ready,
            None,
            100,
            Some(format!(
                "/api/audio/sessions/{}/channel-mix/media?scope={}",
                plan.session_id,
                plan.scope.as_str()
            )),
            Some(response_settings(
                &metadata.settings,
                &metadata.source_fingerprint,
            )),
        ));
    }
    Ok(common(ChannelMixStatus::Idle, None, 0, None, None))
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
    if let Some(existing) = container.job(plan.session_id, plan.scope).await {
        let job = existing.lock().await;
        if job.failed.is_none() {
            return Ok(());
        }
    }
    if cache_is_valid(&plan).await {
        return Ok(());
    }

    let job = Arc::new(Mutex::new(MixJob {
        source_fingerprint: plan.source_fingerprint.clone(),
        settings: plan.settings.clone(),
        progress: 0,
        failed: None,
    }));
    container
        .jobs
        .write()
        .await
        .insert((plan.session_id, plan.scope), job.clone());
    let pool = pool.clone();
    let container = container.clone();
    let media = media.clone();
    tokio::spawn(async move {
        let render_lock = container.key_lock(plan.session_id).await;
        let _render_guard = render_lock.lock().await;
        let result = render_mix(&pool, &media, &plan, &job).await;
        match result {
            Ok(()) => {
                container
                    .remove_if_same(plan.session_id, plan.scope, &job)
                    .await
            }
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
    for source in plan.renderable_sources() {
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
    let settings_temporary = plan
        .cache_dir
        .join(format!(".settings.{}.tmp", uuid::Uuid::new_v4()));
    let result = run_mix_ffmpeg(plan, job, &temporary).await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&temporary).await;
        let _ = tokio::fs::remove_file(&fingerprint_temporary).await;
        let _ = tokio::fs::remove_file(&settings_temporary).await;
        return Err(error);
    }
    if let Err(error) = tokio::fs::write(&fingerprint_temporary, &plan.fingerprint).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        let _ = tokio::fs::remove_file(&fingerprint_temporary).await;
        let _ = tokio::fs::remove_file(&settings_temporary).await;
        return Err(error.into());
    }
    let metadata = MixCacheMetadata {
        source_fingerprint: plan.source_fingerprint.clone(),
        fingerprint: plan.fingerprint.clone(),
        settings: plan.settings.clone(),
    };
    if let Err(error) = tokio::fs::write(
        &settings_temporary,
        serde_json::to_vec_pretty(&metadata).map_err(std::io::Error::other)?,
    )
    .await
    {
        let _ = tokio::fs::remove_file(&temporary).await;
        let _ = tokio::fs::remove_file(&fingerprint_temporary).await;
        let _ = tokio::fs::remove_file(&settings_temporary).await;
        return Err(error.into());
    }
    if let Err(error) = tokio::fs::rename(&temporary, plan.cache_dir.join(MIX_OUTPUT)).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        let _ = tokio::fs::remove_file(&fingerprint_temporary).await;
        let _ = tokio::fs::remove_file(&settings_temporary).await;
        return Err(error.into());
    }
    if let Err(error) =
        tokio::fs::rename(&fingerprint_temporary, plan.cache_dir.join(MIX_FINGERPRINT)).await
    {
        let _ = tokio::fs::remove_file(plan.cache_dir.join(MIX_OUTPUT)).await;
        let _ = tokio::fs::remove_file(&fingerprint_temporary).await;
        let _ = tokio::fs::remove_file(&settings_temporary).await;
        return Err(error.into());
    }
    if let Err(error) =
        tokio::fs::rename(&settings_temporary, plan.cache_dir.join(MIX_SETTINGS)).await
    {
        let _ = tokio::fs::remove_file(plan.cache_dir.join(MIX_OUTPUT)).await;
        let _ = tokio::fs::remove_file(plan.cache_dir.join(MIX_FINGERPRINT)).await;
        let _ = tokio::fs::remove_file(&settings_temporary).await;
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
    for source in plan.renderable_sources() {
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
    let sources = plan.renderable_sources();
    for (index, source) in sources.iter().enumerate() {
        let trim_start = super::milliseconds_as_seconds(
            source
                .overlap_start_ms
                .saturating_sub(source.source_start_ms),
        );
        let trim_end = super::milliseconds_as_seconds(
            source.overlap_end_ms.saturating_sub(source.source_start_ms),
        );
        let delay_samples = source.delay_ms.saturating_mul(48);
        let gain_db = plan
            .settings
            .participants
            .iter()
            .find(|participant| participant.user_id == source.participant_user_id.to_string())
            .map(|participant| participant.gain_db)
            .unwrap_or(0.0);
        filter.push_str(&format!(
            "[{index}:a]aresample=48000, aformat=sample_fmts=fltp:channel_layouts=mono, atrim=start={trim_start}:end={trim_end}, asetpts=PTS-STARTPTS, volume={gain_db}dB, adelay={delay_samples}S|{delay_samples}S, apad=whole_len={duration_samples}, atrim=duration={duration_seconds}[mix{index}];"
        ));
    }
    for index in 0..sources.len() {
        filter.push_str(&format!("[mix{index}]"));
    }
    filter.push_str(&format!(
        "amix=inputs={}:duration=longest:normalize=0:dropout_transition=0, alimiter=limit=0.95:attack=5:release=50:level=false, atrim=duration={duration_seconds}, asetpts=N/SR/TB[out]",
        sources.len()
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

    fn settings_plan() -> MixPlan {
        let tracks = vec![
            ChannelMixTrack {
                user_id: "3".into(),
                display_name: Some("Anchor".into()),
                is_anchor: true,
                segments: Vec::new(),
            },
            ChannelMixTrack {
                user_id: "7".into(),
                display_name: Some("Contributor".into()),
                is_anchor: false,
                segments: Vec::new(),
            },
        ];
        let settings = default_generation_settings(&tracks);
        MixPlan {
            session_id: 1,
            scope: ChannelMixScope::SelectedSession,
            duration_ms: 1_000,
            contributors: Vec::new(),
            sources: Vec::new(),
            participants: Vec::new(),
            tracks,
            cache_dir: PathBuf::from("cache"),
            source_fingerprint: "source".into(),
            fingerprint: mix_fingerprint("source", &settings),
            settings,
        }
    }

    #[test]
    fn generation_settings_default_missing_participants_and_normalize_ids() {
        let plan = settings_plan();
        let settings = canonical_generation_settings(
            &plan,
            vec![ChannelMixParticipantSettings {
                user_id: "07".into(),
                gain_db: 3.5,
                muted: true,
            }],
        )
        .expect("valid settings");
        assert_eq!(
            settings.participants,
            vec![
                ChannelMixParticipantSettings {
                    user_id: "3".into(),
                    gain_db: 0.0,
                    muted: false,
                },
                ChannelMixParticipantSettings {
                    user_id: "7".into(),
                    gain_db: 3.5,
                    muted: true,
                },
            ]
        );
    }

    #[test]
    fn generation_settings_reject_duplicate_unknown_invalid_and_all_muted() {
        let plan = settings_plan();
        let duplicate = canonical_generation_settings(
            &plan,
            vec![
                ChannelMixParticipantSettings {
                    user_id: "3".into(),
                    gain_db: 0.0,
                    muted: false,
                },
                ChannelMixParticipantSettings {
                    user_id: "03".into(),
                    gain_db: 0.0,
                    muted: false,
                },
            ],
        );
        assert!(
            matches!(duplicate, Err(AppError::BadRequest(message)) if message.contains("Duplicate"))
        );

        let unknown = canonical_generation_settings(
            &plan,
            vec![ChannelMixParticipantSettings {
                user_id: "99".into(),
                gain_db: 0.0,
                muted: false,
            }],
        );
        assert!(
            matches!(unknown, Err(AppError::BadRequest(message)) if message.contains("Unknown"))
        );

        let invalid = canonical_generation_settings(
            &plan,
            vec![ChannelMixParticipantSettings {
                user_id: "3".into(),
                gain_db: 12.1,
                muted: false,
            }],
        );
        assert!(
            matches!(invalid, Err(AppError::BadRequest(message)) if message.contains("between"))
        );

        let all_muted = canonical_generation_settings(
            &plan,
            vec![
                ChannelMixParticipantSettings {
                    user_id: "3".into(),
                    gain_db: 0.0,
                    muted: true,
                },
                ChannelMixParticipantSettings {
                    user_id: "7".into(),
                    gain_db: 0.0,
                    muted: true,
                },
            ],
        );
        assert!(
            matches!(all_muted, Err(AppError::BadRequest(message)) if message.contains("unmuted"))
        );
    }

    #[test]
    fn tracks_include_anchor_and_aligned_authenticated_source_metadata() {
        let anchor = AudioFragment {
            id: 11,
            guild_id: 1,
            channel_id: 10,
            user_id: 3,
            recording_session_id: Some(1),
            file_name: "1000-3".into(),
            year: 2026,
            month: 8,
            start_ms: 1_500,
            end_ms: Some(3_000),
            segment_index: Some(0),
            live: false,
        };
        let contributor = AudioFragment {
            id: 22,
            user_id: 7,
            recording_session_id: Some(2),
            file_name: "1200-7".into(),
            start_ms: 1_000,
            end_ms: Some(3_000),
            live: true,
            ..anchor.clone()
        };
        let participants = vec![
            ChannelMixParticipant {
                user_id: "3".into(),
                display_name: Some("Anchor".into()),
                session_ids: vec!["1".into()],
                source_count: 1,
            },
            ChannelMixParticipant {
                user_id: "7".into(),
                display_name: Some("Contributor".into()),
                session_ids: vec!["2".into()],
                source_count: 1,
            },
        ];
        let mut sources = Vec::new();
        add_fragment_sources(
            &mut sources,
            std::slice::from_ref(&anchor),
            std::slice::from_ref(&contributor),
            7,
            1_000,
            3_000,
        );
        let tracks = build_tracks(&participants, &sources, Some(3), 1_000);
        assert_eq!(tracks.len(), 2);
        assert!(tracks[0].is_anchor);
        let segment = &tracks[1].segments[0];
        assert_eq!(segment.start_ms, 500);
        assert_eq!(segment.end_ms, 2_000);
        assert_eq!(segment.source_offset_ms, 500);
        assert!(segment.live);
        assert_eq!(segment.id, "22:1500");
        assert_eq!(segment.recording_session_id.as_deref(), Some("2"));
        assert_eq!(segment.media_url, "/api/audio/sessions/2/segments/22");
        assert_eq!(
            segment.hls_playlist_url,
            "/api/audio/sessions/2/live/22/playlist.m3u8"
        );
        assert_eq!(
            segment.waveform_url,
            "/api/audio/waveform/1/10/2026/08/1200-7"
        );
        let mut extended_source = sources[0].clone();
        extended_source.overlap_end_ms += 1_000;
        assert_eq!(
            source_segment(&sources[0], 1_000).id,
            source_segment(&extended_source, 1_000).id
        );
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
    fn occupancy_windows_expand_to_the_selected_connected_episode() {
        let events = vec![
            BotConnectionEvent {
                started_ms: 100,
                completed_ms: 110,
                to_channel_id: Some(10),
                outcome: "joined".into(),
            },
            BotConnectionEvent {
                started_ms: 200,
                completed_ms: 210,
                to_channel_id: Some(10),
                outcome: "already_in_channel".into(),
            },
            BotConnectionEvent {
                started_ms: 500,
                completed_ms: 520,
                to_channel_id: Some(11),
                outcome: "switched".into(),
            },
            BotConnectionEvent {
                started_ms: 900,
                completed_ms: 910,
                to_channel_id: None,
                outcome: "disconnected".into(),
            },
            BotConnectionEvent {
                started_ms: 2_000,
                completed_ms: 2_010,
                to_channel_id: Some(10),
                outcome: "joined".into(),
            },
        ];
        let windows = build_occupancy_windows(&events, 3_000);
        assert_eq!(
            windows
                .iter()
                .map(|window| (
                    window.channel_id,
                    window.start_ms,
                    window.end_ms,
                    window.episode_id
                ))
                .collect::<Vec<_>>(),
            vec![(10, 110, 500, 1), (11, 520, 900, 1), (10, 2_010, 3_000, 2)]
        );
        let selected_channels = HashSet::from([10, 11]);
        let selected = select_occupancy_windows(windows, 300, 600, &selected_channels);
        assert_eq!(
            selected,
            vec![
                MixWindow {
                    channel_id: 10,
                    start_ms: 110,
                    end_ms: 500,
                },
                MixWindow {
                    channel_id: 11,
                    start_ms: 520,
                    end_ms: 900,
                },
            ]
        );
    }

    #[test]
    fn failed_connection_audits_do_not_create_occupancy() {
        let events = vec![BotConnectionEvent {
            started_ms: 100,
            completed_ms: 110,
            to_channel_id: Some(10),
            outcome: "join_failed".into(),
        }];
        assert!(build_occupancy_windows(&events, 3_000).is_empty());
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
        add_fragment_sources(&mut sources, &[anchor], &[other_channel], 20, 1_000, 2_000);
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
            scope: ChannelMixScope::SelectedSession,
            duration_ms: 1_000,
            contributors: Vec::new(),
            sources: vec![MixSource {
                audio_file_id: 2,
                recording_session_id: None,
                participant_user_id: 20,
                guild_id: 1,
                channel_id: 10,
                year: 2026,
                month: 8,
                file_name: "source".into(),
                path: PathBuf::from("source.ogg"),
                fragment_start_ms: 1_000,
                fragment_end_ms: 2_000,
                live: false,
                source_start_ms: 1_000,
                overlap_start_ms: 1_500,
                overlap_end_ms: 2_000,
                delay_ms: 500,
            }],
            participants: Vec::new(),
            tracks: Vec::new(),
            cache_dir: PathBuf::from("cache"),
            source_fingerprint: "test".into(),
            fingerprint: "test".into(),
            settings: ChannelMixGenerationSettings {
                participants: vec![ChannelMixParticipantSettings {
                    user_id: "20".into(),
                    gain_db: -6.0,
                    muted: false,
                }],
                source_fingerprint: None,
            },
        };
        let filter = build_mix_filter(&plan);
        assert!(filter.contains("volume=-6dB, adelay="));
        assert!(filter.contains("atrim=start=0.500:end=1.000"));
        assert!(filter.contains("adelay=24000S|24000S"));
        assert!(filter.contains("amix=inputs=1:duration=longest:normalize=0"));
        assert!(filter.contains("alimiter=limit=0.95"));
        assert!(filter.contains("apad=whole_len=48000"));
        assert!(filter.contains("atrim=duration=1.000"));

        let muted = plan.with_settings(ChannelMixGenerationSettings {
            participants: vec![ChannelMixParticipantSettings {
                user_id: "20".into(),
                gain_db: -6.0,
                muted: true,
            }],
            source_fingerprint: None,
        });
        assert!(muted.renderable_sources().is_empty());
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
        tokio::fs::write(
            directory.path().join(MIX_SETTINGS),
            serde_json::to_vec(&MixCacheMetadata {
                source_fingerprint: "current".into(),
                fingerprint: "current".into(),
                settings: ChannelMixGenerationSettings {
                    participants: Vec::new(),
                    source_fingerprint: None,
                },
            })
            .expect("serialize settings"),
        )
        .await
        .expect("write settings");
        let plan = MixPlan {
            session_id: 1,
            scope: ChannelMixScope::SelectedSession,
            duration_ms: 1_000,
            contributors: Vec::new(),
            sources: Vec::new(),
            participants: Vec::new(),
            tracks: Vec::new(),
            cache_dir: directory.path().to_path_buf(),
            source_fingerprint: "current".into(),
            fingerprint: "current".into(),
            settings: ChannelMixGenerationSettings {
                participants: Vec::new(),
                source_fingerprint: None,
            },
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
            source_fingerprint: "fixture".into(),
            settings: ChannelMixGenerationSettings {
                participants: Vec::new(),
                source_fingerprint: None,
            },
            progress: 0,
            failed: None,
        }));
        let plan = MixPlan {
            session_id: 1,
            scope: ChannelMixScope::SelectedSession,
            duration_ms: 1_000,
            contributors: Vec::new(),
            sources: vec![MixSource {
                audio_file_id: 2,
                recording_session_id: None,
                participant_user_id: 20,
                guild_id: 1,
                channel_id: 10,
                year: 2026,
                month: 8,
                file_name: "source".into(),
                path: source,
                fragment_start_ms: 0,
                fragment_end_ms: 1_000,
                live: false,
                source_start_ms: 0,
                overlap_start_ms: 0,
                overlap_end_ms: 1_000,
                delay_ms: 500,
            }],
            participants: Vec::new(),
            tracks: Vec::new(),
            cache_dir: directory.path().to_path_buf(),
            source_fingerprint: "fixture".into(),
            fingerprint: "fixture".into(),
            settings: ChannelMixGenerationSettings {
                participants: Vec::new(),
                source_fingerprint: None,
            },
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
