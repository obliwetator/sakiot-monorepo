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
use sqlx::{Pool, Postgres};
use tokio::sync::{Mutex, RwLock};

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::media_jobs::JobAttempt;
use crate::permissions::require_channel_access;
use crate::server_timing::measure;

use super::super::live::mark_cache_access;
use super::{AudioFragment, SessionAccess, fragment_path, load_fragments, require_session_access};

mod cache;
mod occupancy;
mod plan;
mod render;
mod tracks;

use cache::{
    anchor_wait_reason, cache_has_current_source, canonical_generation_settings,
    default_generation_settings, mix_cache_dir, mix_fingerprint, mix_response,
    mix_source_fingerprint,
};
use occupancy::{MixWindow, fallback_mix_windows, load_bot_occupancy_windows};
use plan::*;
use tracks::*;

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
    /// The media job rendering the mix, while one is: realtime `jobs` events
    /// name it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
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
}

impl SessionMixContainer {
    async fn job(&self, session_id: i64, scope: ChannelMixScope) -> Option<Arc<Mutex<MixJob>>> {
        self.jobs.read().await.get(&(session_id, scope)).cloned()
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
    let access =
        require_session_access(&pool, session_id, crate::permissions::Viewer::of(&token)).await?;
    let plan = measure(
        "plan",
        build_mix_plan(
            &pool,
            &access,
            crate::permissions::Viewer::of(&token),
            query.scope(),
        ),
    )
    .await?;
    let mut response = measure("status", mix_response(&plan, &access, &container, true)).await?;
    let resource = format!("session-mix:{session_id}:{}", query.scope().as_str());
    if let Some(job) = measure(
        "jobs",
        crate::media_jobs::active_for_resource(&pool, token.user_id, "session_mix", &resource),
    )
    .await?
    {
        response.status = ChannelMixStatus::Processing;
        response.progress = job.progress;
        response.job_id = Some(job.id);
    }
    Ok(web::Json(response))
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
    request: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<ChannelMixQuery>,
    body: Option<web::Json<GenerateChannelMixBody>>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    container: web::Data<SessionMixContainer>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access =
        require_session_access(&pool, session_id, crate::permissions::Viewer::of(&token)).await?;
    let base_plan = build_mix_plan(
        &pool,
        &access,
        crate::permissions::Viewer::of(&token),
        query.scope(),
    )
    .await?;
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

    let participants =
        serde_json::to_value(&plan.settings.participants).map_err(|_| AppError::InternalError)?;
    let job_request = crate::media_jobs::MediaJobRequest::SessionMix {
        session_id,
        scope: plan.scope.as_str().to_owned(),
        participants,
    };
    let resource = format!("session-mix:{session_id}:{}", plan.scope.as_str());
    let key = request
        .headers()
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let job = crate::media_jobs::enqueue(
        pool.get_ref(),
        Some(access.guild_id),
        crate::permissions::Viewer::of(&token),
        &key,
        &resource,
        &job_request,
    )
    .await?;
    Ok(HttpResponse::Accepted()
        .insert_header((header::LOCATION, format!("/api/media-jobs/{}", job.id)))
        .json(job))
}

pub(crate) async fn run_session_mix_job(
    attempt: &JobAttempt<'_>,
    session_id: i64,
    scope: &str,
    participants: serde_json::Value,
) -> Result<(Option<String>, Option<PathBuf>), AppError> {
    let JobAttempt {
        pool,
        media,
        requester,
        id: job_id,
        token: attempt_token,
    } = *attempt;
    let scope = match scope {
        "all_recordings" => ChannelMixScope::AllRecordings,
        "selected_session" => ChannelMixScope::SelectedSession,
        _ => return Err(AppError::BadRequest("Unknown channel mix scope".into())),
    };
    let pool_data = web::Data::new(pool.clone());
    let access = require_session_access(&pool_data, session_id, requester).await?;
    let base = build_mix_plan(&pool_data, &access, requester, scope).await?;
    let requested: Vec<ChannelMixParticipantSettings> = serde_json::from_value(participants)
        .map_err(|_| AppError::BadRequest("Invalid channel mix settings".into()))?;
    let settings = canonical_generation_settings(&base, requested)?;
    let plan = base.with_settings(settings);
    if plan.blocking_reason(&access).is_some() {
        return Err(AppError::Conflict(
            "Channel mix sources are not finalized".into(),
        ));
    }
    let job = Arc::new(Mutex::new(MixJob {
        source_fingerprint: plan.source_fingerprint.clone(),
        settings: plan.settings.clone(),
        progress: 0,
        failed: None,
    }));
    crate::media_jobs::report_progress(pool, job_id, attempt_token, "rendering", 5).await?;
    let rendering = render::render_mix(&pool_data, media, &plan, &job, Some(attempt));
    tokio::pin!(rendering);
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    interval.tick().await;
    loop {
        tokio::select! {
            biased;
            result = &mut rendering => { result?; break; }
            _ = interval.tick() => {
                let value = job.lock().await.progress;
                if !crate::media_jobs::report_progress(pool, job_id, attempt_token, "rendering", value).await? {
                    return Err(AppError::JobLeaseLost);
                }
            }
        }
    }
    Ok((
        Some(format!(
            "/api/audio/sessions/{session_id}/channel-mix/media?scope={}",
            scope.as_str()
        )),
        None,
    ))
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
    let access =
        require_session_access(&pool, session_id, crate::permissions::Viewer::of(&token)).await?;
    let plan = build_mix_plan(
        &pool,
        &access,
        crate::permissions::Viewer::of(&token),
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

#[cfg(test)]
mod tests;
