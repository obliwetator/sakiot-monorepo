//! Durable queue shared by CPU- and I/O-heavy media operations.
//!
//! PostgreSQL owns lifecycle and admission. Every attempt receives a fencing
//! token and final publication is conditional on that token still owning a
//! live lease, so an expired worker cannot overwrite a newer result.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use actix_files::NamedFile;
use actix_web::{HttpRequest, HttpResponse, get, web};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres};

use crate::audio::WaveformProgressContainer;
use crate::auth::{Access, Token};
use crate::errors::{AppError, ErrorKind, JobError};
use crate::media_archive::MediaArchive;

mod lease;
mod worker;

pub use lease::{begin_publication, complete_publication, report_progress, track_progress};
pub use worker::spawn_worker;

use lease::*;

/// The composition queue uses the same transaction lock for cross-queue limits.
pub const QUEUE_LOCK: i64 = 0x53414b4d45444941;
const MAX_ATTEMPTS: i32 = 3;
const GLOBAL_RUNNING_LIMIT: i64 = 4;
/// Active media plus composition jobs one user may have queued or running.
pub(crate) const PER_USER_ACTIVE_LIMIT: i64 = 3;
/// A recording's waveform. A page builds one for each recording it shows (the
/// channel mix, for every source of a session) without the viewer choosing how
/// many, so these jobs neither count towards nor are refused by the per-user
/// limit: the fourth source of a cold mix would otherwise show as failed.
/// They are shared per file, and [`GLOBAL_RUNNING_LIMIT`] still bounds how many
/// run at once.
pub(crate) const UNMETERED_KIND: &str = "recording_waveform";

/// The attempt a job runner works inside: where it reads and writes, who
/// asked (access is re-checked as them), and the fencing token its progress
/// reports and final publication must still hold.
#[derive(Clone, Copy)]
pub(crate) struct JobAttempt<'a> {
    pub pool: &'a Pool<Postgres>,
    pub media: &'a MediaArchive,
    pub requester: crate::permissions::Viewer,
    pub id: &'a str,
    pub token: &'a str,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaJobRequest {
    RecordingSilence {
        guild_id: i64,
        channel_id: i64,
        year: i32,
        month: i32,
        file_name: String,
    },
    RecordingWaveform {
        guild_id: i64,
        channel_id: i64,
        year: i32,
        month: i32,
        file_name: String,
        silence_free: bool,
    },
    ClipWaveform {
        guild_id: i64,
        clip_id: String,
    },
    SessionWaveform {
        session_id: i64,
        silence_free: bool,
    },
    SessionSilence {
        session_id: i64,
    },
    SessionMix {
        session_id: i64,
        scope: String,
        participants: serde_json::Value,
    },
    SessionDownload {
        session_id: i64,
        start: Option<f64>,
        end: Option<f64>,
        remove_silence: bool,
    },
}

impl MediaJobRequest {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::RecordingSilence { .. } => "recording_silence",
            Self::RecordingWaveform { .. } => "recording_waveform",
            Self::ClipWaveform { .. } => "clip_waveform",
            Self::SessionWaveform { .. } => "session_waveform",
            Self::SessionSilence { .. } => "session_silence",
            Self::SessionMix { .. } => "session_mix",
            Self::SessionDownload { .. } => "session_download",
        }
    }
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct MediaJobStatus {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub stage: String,
    pub progress: i16,
    pub result_url: Option<String>,
    /// Safe explanation of the last failed attempt; never internal detail.
    pub error: Option<String>,
    /// Stable classification of `error`. Absent for records that predate it.
    pub error_kind: Option<ErrorKind>,
}

#[derive(Clone, Debug)]
pub struct ClaimedMediaJob {
    pub id: String,
    /// Who requested the job; access is re-checked as them when it runs.
    pub requester: crate::permissions::Viewer,
    pub request: MediaJobRequest,
    pub token: String,
}

pub async fn enqueue(
    pool: &Pool<Postgres>,
    guild_id: Option<i64>,
    requester: crate::permissions::Viewer,
    idempotency_key: &str,
    resource_key: &str,
    request: &MediaJobRequest,
) -> Result<MediaJobStatus, AppError> {
    let user_id = requester.user_id;
    if idempotency_key.is_empty() || idempotency_key.len() > 128 {
        return Err(AppError::BadRequest("Invalid media job request key".into()));
    }
    let request_json = serde_json::to_value(request).map_err(|_| AppError::InternalError)?;
    let mut tx = pool.begin().await?;
    sqlx::query!("SELECT pg_advisory_xact_lock($1)", QUEUE_LOCK)
        .execute(&mut *tx)
        .await?;

    if let Some(row) = sqlx::query!(
        "SELECT id, request FROM media_jobs WHERE user_id = $1 AND kind = $2 AND idempotency_key = $3",
        user_id,
        request.kind(),
        idempotency_key
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        if row.request != request_json {
            return Err(AppError::Conflict(
                "This media request key was already used for different work".into(),
            ));
        }
        sqlx::query!(
            "INSERT INTO media_job_viewers (job_id,user_id) VALUES ($1,$2) ON CONFLICT DO NOTHING",
            row.id,
            user_id
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return load_status(pool, user_id, &row.id).await;
    }

    if let Some(row) = sqlx::query!(
        "SELECT id, request FROM media_jobs WHERE kind = $1 AND resource_key = $2 AND state IN ('queued', 'running') ORDER BY created_at LIMIT 1",
        request.kind(),
        resource_key
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        if row.request != request_json {
            return Err(AppError::Conflict(
                "This media resource is already being processed with different settings".into(),
            ));
        }
        sqlx::query!(
            "INSERT INTO media_job_viewers (job_id,user_id) VALUES ($1,$2) ON CONFLICT DO NOTHING",
            row.id,
            user_id
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return load_status(pool, user_id, &row.id).await;
    }

    if request.kind() != UNMETERED_KIND {
        let active = sqlx::query_scalar!(
            r#"SELECT (SELECT count(*) FROM media_jobs WHERE user_id = $1 AND kind <> $2 AND state IN ('queued','running')) + (SELECT count(*) FROM composition_jobs WHERE user_id = $1 AND state IN ('queued','running')) AS "active!""#,
            user_id,
            UNMETERED_KIND
        )
        .fetch_one(&mut *tx)
        .await?;
        if active >= PER_USER_ACTIVE_LIMIT {
            return Err(AppError::UserJobLimitReached);
        }
    }

    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query!(
        "INSERT INTO media_jobs (id, kind, guild_id, user_id, idempotency_key, resource_key, request, requester_dev) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        id,
        request.kind(),
        guild_id,
        user_id,
        idempotency_key,
        resource_key,
        request_json,
        requester.dev
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO media_job_viewers (job_id,user_id) VALUES ($1,$2)",
        id,
        user_id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    load_status(pool, user_id, &id).await
}

pub async fn load_status(
    pool: &Pool<Postgres>,
    user_id: i64,
    id: &str,
) -> Result<MediaJobStatus, AppError> {
    let row = sqlx::query!(
        "SELECT j.id, j.kind, j.state, j.stage, j.progress, j.result_url, j.error, j.error_kind FROM media_jobs j JOIN media_job_viewers v ON v.job_id=j.id WHERE j.id = $1 AND v.user_id = $2",
        id,
        user_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let error = JobError::from_columns(row.error_kind.as_deref(), row.error.as_deref());
    Ok(MediaJobStatus {
        id: row.id,
        kind: row.kind,
        status: row.state,
        stage: row.stage,
        progress: row.progress,
        result_url: row.result_url,
        error_kind: error.as_ref().and_then(|error| error.kind),
        error: error.map(|error| error.message),
    })
}

pub async fn active_for_resource(
    pool: &Pool<Postgres>,
    user_id: i64,
    kind: &str,
    resource_key: &str,
) -> Result<Option<MediaJobStatus>, AppError> {
    let id = sqlx::query_scalar!(
        "SELECT j.id FROM media_jobs j JOIN media_job_viewers v ON v.job_id=j.id WHERE v.user_id=$1 AND j.kind=$2 AND j.resource_key=$3 AND j.state IN ('queued','running') ORDER BY j.created_at DESC LIMIT 1",
        user_id,
        kind,
        resource_key
    )
    .fetch_optional(pool)
    .await?;
    match id {
        Some(id) => load_status(pool, user_id, &id).await.map(Some),
        None => Ok(None),
    }
}

pub async fn latest_failed_for_resource(
    pool: &Pool<Postgres>,
    user_id: i64,
    kind: &str,
    resource_key: &str,
) -> Result<Option<MediaJobStatus>, AppError> {
    let id = sqlx::query_scalar!(
        "SELECT j.id FROM media_jobs j JOIN media_job_viewers v ON v.job_id=j.id WHERE v.user_id=$1 AND j.kind=$2 AND j.resource_key=$3 AND j.state='failed' ORDER BY j.created_at DESC LIMIT 1",
        user_id,
        kind,
        resource_key
    )
    .fetch_optional(pool)
    .await?;
    match id {
        Some(id) => load_status(pool, user_id, &id).await.map(Some),
        None => Ok(None),
    }
}

#[utoipa::path(
    get,
    path = "/api/media-jobs/{job_id}",
    tag = "media",
    responses(
        (status = 200, body = MediaJobStatus),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 404, description = "Job not found, expired, or not visible to the caller", body = crate::errors::ApiError),
    )
)]
#[get("/media-jobs/{job_id}")]
pub async fn get_media_job(
    path: web::Path<String>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<web::Json<MediaJobStatus>, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    Ok(web::Json(load_status(&pool, token.user_id, &path).await?))
}

#[utoipa::path(get, path = "/api/media-jobs/{job_id}/result", tag = "media", responses((status = 200, description = "Completed media job result")))]
#[actix_web::route("/media-jobs/{job_id}/result", method = "GET", method = "HEAD")]
pub async fn get_media_job_result(
    request: HttpRequest,
    path: web::Path<String>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let row = sqlx::query!(
        "SELECT j.state,j.result_path,j.request FROM media_jobs j JOIN media_job_viewers v ON v.job_id=j.id WHERE j.id=$1 AND v.user_id=$2",
        path.as_str(),
        token.user_id
    )
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or(AppError::NotFound)?;
    if row.state != "ready" {
        return Err(AppError::Conflict("Media result is not ready".into()));
    }
    let job_request: MediaJobRequest =
        serde_json::from_value(row.request).map_err(|_| AppError::InternalError)?;
    if let MediaJobRequest::SessionDownload { session_id, .. } = job_request {
        crate::audio::sessions::require_session_access(
            &pool,
            session_id,
            crate::permissions::Viewer::of(&token),
        )
        .await?;
    } else {
        return Err(AppError::FileNotFound);
    }
    let result_path = row.result_path.ok_or(AppError::FileNotFound)?;
    let file = NamedFile::open_async(Path::new(&result_path))
        .await
        .map_err(AppError::IoError)?;
    Ok(file
        .set_content_disposition(actix_web::http::header::ContentDisposition {
            disposition: actix_web::http::header::DispositionType::Attachment,
            parameters: vec![],
        })
        .into_response(&request))
}

#[cfg(test)]
mod tests;
