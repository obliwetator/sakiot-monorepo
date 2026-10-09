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

async fn claim(pool: &Pool<Postgres>) -> Result<Option<ClaimedMediaJob>, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query!("SELECT pg_advisory_xact_lock($1)", QUEUE_LOCK)
        .execute(&mut *tx)
        .await?;
    let interrupted = ErrorKind::WorkerInterrupted;
    sqlx::query!(
        "UPDATE media_jobs SET state='failed', stage='failed', error=$2, error_kind=$3, attempt_token=NULL, lease_expires_at=NULL, finished_at=now(), updated_at=now() WHERE state='running' AND lease_expires_at < now() AND attempts >= $1",
        MAX_ATTEMPTS,
        interrupted.default_message(),
        interrupted.as_str()
    )
    .execute(&mut *tx)
    .await?;
    let running = sqlx::query_scalar!(
        r#"SELECT (SELECT count(*) FROM media_jobs WHERE state='running' AND lease_expires_at >= now()) + (SELECT count(*) FROM composition_jobs WHERE state='running' AND lease_expires_at >= now()) AS "running!""#
    )
    .fetch_one(&mut *tx)
    .await?;
    if running >= GLOBAL_RUNNING_LIMIT {
        tx.commit().await?;
        return Ok(None);
    }
    let token = uuid::Uuid::new_v4().to_string();
    let row = sqlx::query!(
        "WITH candidate AS (SELECT id FROM media_jobs WHERE attempts < $2 AND ((state='queued' AND retry_at <= now()) OR (state='running' AND lease_expires_at < now())) ORDER BY created_at,id FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE media_jobs j SET state='running',stage='preparing',progress=0,attempts=attempts+1,attempt_token=$1,lease_expires_at=now()+interval '60 seconds',error=NULL,error_kind=NULL,updated_at=now() FROM candidate WHERE j.id=candidate.id RETURNING j.id,j.user_id,j.requester_dev,j.request",
        token,
        MAX_ATTEMPTS
    )
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    row.map(|row| {
        Ok(ClaimedMediaJob {
            id: row.id,
            requester: crate::permissions::Viewer {
                user_id: row.user_id,
                dev: row.requester_dev,
            },
            request: serde_json::from_value(row.request).map_err(|_| AppError::InternalError)?,
            token,
        })
    })
    .transpose()
}

// Lease renewals and progress reports skip the job row while it is locked
// instead of waiting for it. The only lock on a running attempt's row is its
// own publication (`begin_publication` until commit), and the loops that
// renew and report are the same tasks that must keep polling that
// publication: waiting there deadlocked the job and held its connections
// until a restart. Both still answer whether the attempt owns the lease; a
// locked row counts as owned, so only a lost or expired lease returns false.

async fn renew(pool: &Pool<Postgres>, job: &ClaimedMediaJob) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar!(
        r#"WITH target AS (
               SELECT id FROM media_jobs
                WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at > now()
                  FOR UPDATE SKIP LOCKED
           ), renewed AS (
               UPDATE media_jobs j SET lease_expires_at=now()+interval '60 seconds',updated_at=now()
                 FROM target WHERE j.id=target.id
           )
           SELECT EXISTS (
               SELECT 1 FROM media_jobs
                WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at > now()
           ) AS "owned!""#,
        job.id,
        job.token
    )
    .fetch_one(pool)
    .await?)
}

pub async fn report_progress(
    pool: &Pool<Postgres>,
    id: &str,
    token: &str,
    stage: &str,
    progress: i16,
) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar!(
        r#"WITH target AS (
               SELECT id FROM media_jobs
                WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at > now()
                  FOR UPDATE SKIP LOCKED
           ), reported AS (
               UPDATE media_jobs j SET stage=$3,progress=$4,updated_at=now()
                 FROM target WHERE j.id=target.id
           )
           SELECT EXISTS (
               SELECT 1 FROM media_jobs
                WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at > now()
           ) AS "owned!""#,
        id,
        token,
        stage,
        progress.clamp(0, 99)
    )
    .fetch_one(pool)
    .await?)
}

/// Mirror the existing FFmpeg/audiowaveform progress source into the durable
/// row while the operation is running. Losing the lease stops the operation.
pub async fn track_progress<T, F>(
    pool: &Pool<Postgres>,
    id: &str,
    token: &str,
    stage: &str,
    values: &web::Data<WaveformProgressContainer>,
    key: &str,
    future: F,
) -> Result<T, AppError>
where
    F: Future<Output = Result<T, AppError>>,
{
    tokio::pin!(future);
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.tick().await;
    loop {
        tokio::select! {
            biased;
            result = &mut future => return result,
            _ = interval.tick() => {
                let value = values.0.read().await.get(key).copied();
                if let Some(value) = value.filter(|value| *value >= 0)
                    && !report_progress(pool, id, token, stage, value).await?
                {
                    return Err(AppError::JobLeaseLost);
                }
            }
        }
    }
}

/// Lock the job row through publication. A replacement attempt cannot claim
/// the expired row until the rename and DB result commit finish; a stale
/// attempt cannot obtain this lock after a new token has been assigned.
pub async fn begin_publication<'a>(
    pool: &'a Pool<Postgres>,
    id: &str,
    token: &str,
) -> Result<sqlx::Transaction<'a, Postgres>, AppError> {
    let mut tx = pool.begin().await?;
    let owned = sqlx::query_scalar!(
        "SELECT id FROM media_jobs WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at > now() FOR UPDATE",
        id,
        token
    )
    .fetch_optional(&mut *tx)
    .await?;
    if owned.is_none() {
        return Err(AppError::JobLeaseLost);
    }
    Ok(tx)
}

pub async fn complete_publication(
    mut tx: sqlx::Transaction<'_, Postgres>,
    id: &str,
    token: &str,
    result_url: &str,
    result_path: Option<&Path>,
) -> Result<(), AppError> {
    let result_path = result_path.map(|path| path.to_string_lossy().into_owned());
    let updated = sqlx::query!(
        "UPDATE media_jobs SET state='ready',stage='ready',progress=100,result_url=$3,result_path=$4,error=NULL,error_kind=NULL,attempt_token=NULL,lease_expires_at=NULL,finished_at=now(),updated_at=now() WHERE id=$1 AND attempt_token=$2 AND state='running'",
        id,
        token,
        result_url,
        result_path
    )
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(AppError::JobLeaseLost);
    }
    tx.commit().await?;
    Ok(())
}

async fn finish(
    pool: &Pool<Postgres>,
    job: &ClaimedMediaJob,
    result: Result<(Option<String>, Option<PathBuf>), AppError>,
) -> Result<(), AppError> {
    let (state, stage, progress, result_url, result_path, error, retry) = match result {
        Ok((url, path)) => (
            "ready",
            "ready",
            100i16,
            url,
            path.map(|p| p.to_string_lossy().into_owned()),
            None,
            false,
        ),
        // The new owner reports this job; a stale attempt must not.
        Err(AppError::JobLeaseLost) => {
            tracing::warn!(job_id = %job.id, "media job lease lost; leaving the job to its new owner");
            return Ok(());
        }
        Err(error) => {
            let retry = job_attempts(pool, &job.id).await? < MAX_ATTEMPTS;
            let kind = error.kind();
            tracing::warn!(job_id = %job.id, kind = kind.as_str(), retry, ?error, "media job attempt failed");
            if retry {
                ("queued", "queued", 0, None, None, Some(kind), true)
            } else {
                ("failed", "failed", 0, None, None, Some(kind), false)
            }
        }
    };
    // A retry waits five seconds before it can be claimed again.
    sqlx::query!(
        "UPDATE media_jobs SET state=$3,stage=$4,progress=$5,result_url=$6,result_path=$7,error=$8,error_kind=$9,attempt_token=NULL,lease_expires_at=NULL,retry_at=CASE WHEN $10 THEN now()+interval '5 seconds' ELSE now() END,finished_at=CASE WHEN $3 IN ('ready','failed') THEN now() ELSE NULL END,updated_at=now() WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at > now()",
        job.id,
        job.token,
        state,
        stage,
        progress,
        result_url,
        result_path,
        error.map(ErrorKind::default_message),
        error.map(ErrorKind::as_str),
        retry
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn job_attempts(pool: &Pool<Postgres>, id: &str) -> Result<i32, AppError> {
    Ok(
        sqlx::query_scalar!("SELECT attempts FROM media_jobs WHERE id=$1", id)
            .fetch_one(pool)
            .await?,
    )
}

async fn run_attempt(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    job: &ClaimedMediaJob,
) -> Result<(Option<String>, Option<PathBuf>), AppError> {
    report_progress(pool, &job.id, &job.token, "processing", 1).await?;
    let attempt = JobAttempt {
        pool,
        media,
        requester: job.requester,
        id: &job.id,
        token: &job.token,
    };
    match &job.request {
        MediaJobRequest::RecordingSilence {
            guild_id,
            channel_id,
            year,
            month,
            file_name,
        } => {
            crate::audio::silence::run_recording_silence_job(
                &attempt,
                *guild_id,
                *channel_id,
                *year,
                *month,
                file_name,
            )
            .await
        }
        MediaJobRequest::RecordingWaveform {
            guild_id,
            channel_id,
            year,
            month,
            file_name,
            silence_free,
        } => {
            crate::audio::peaks::run_recording_waveform_job(
                &attempt,
                *guild_id,
                *channel_id,
                *year,
                *month,
                file_name,
                *silence_free,
            )
            .await
        }
        MediaJobRequest::ClipWaveform { guild_id, clip_id } => {
            crate::audio::peaks::run_clip_waveform_job(&attempt, *guild_id, clip_id).await
        }
        MediaJobRequest::SessionWaveform {
            session_id,
            silence_free,
        } => {
            crate::audio::sessions::run_session_waveform_job(&attempt, *session_id, *silence_free)
                .await
        }
        MediaJobRequest::SessionSilence { session_id } => {
            crate::audio::sessions::run_session_silence_job(&attempt, *session_id).await
        }
        MediaJobRequest::SessionMix {
            session_id,
            scope,
            participants,
        } => {
            crate::audio::sessions::run_session_mix_job(
                &attempt,
                *session_id,
                scope,
                participants.clone(),
            )
            .await
        }
        MediaJobRequest::SessionDownload {
            session_id,
            start,
            end,
            remove_silence,
        } => {
            crate::audio::sessions::run_download_job(
                &attempt,
                *session_id,
                *start,
                *end,
                *remove_silence,
            )
            .await
        }
    }
}

async fn execute(pool: Pool<Postgres>, media: MediaArchive, job: ClaimedMediaJob) {
    let started = std::time::Instant::now();
    let result = {
        let mut interval = tokio::time::interval(Duration::from_secs(20));
        interval.tick().await;
        let mut lease_confirmed_at = tokio::time::Instant::now();
        let deadline = tokio::time::sleep(Duration::from_secs(30 * 60));
        tokio::pin!(deadline);
        let attempt = run_attempt(&pool, &media, &job);
        tokio::pin!(attempt);
        loop {
            tokio::select! {
                result = &mut attempt => break result,
                _ = &mut deadline => break Err(AppError::ExecutionTimedOut),
                _ = interval.tick() => match renew(&pool, &job).await {
                    Ok(true) => lease_confirmed_at = tokio::time::Instant::now(),
                    Ok(false) => break Err(AppError::JobLeaseLost),
                    Err(error) => {
                        tracing::warn!(job_id=%job.id, ?error, "media lease renewal failed");
                        if lease_confirmed_at.elapsed() >= Duration::from_secs(40) {
                            break Err(AppError::WorkerInterrupted(
                                "media job lease could not be renewed".into(),
                            ));
                        }
                    }
                },
            }
        }
    };
    crate::job_metrics::record(
        job.request.kind(),
        crate::job_metrics::outcome(&result),
        started.elapsed(),
    );
    if let Err(error) = finish(&pool, &job, result).await {
        tracing::error!(job_id=%job.id, ?error, "media job completion failed");
    }
}

pub fn spawn_worker(pool: Pool<Postgres>, media: MediaArchive) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_cleanup = tokio::time::Instant::now() - Duration::from_secs(60 * 60);
        loop {
            if last_cleanup.elapsed() >= Duration::from_secs(60 * 60) {
                if let Err(error) = cleanup(&pool).await {
                    tracing::warn!(?error, "media job cleanup failed");
                }
                last_cleanup = tokio::time::Instant::now();
            }
            match claim(&pool).await {
                Ok(Some(job)) => {
                    let pool = pool.clone();
                    let media = media.clone();
                    tokio::spawn(execute(pool, media, job));
                }
                Ok(None) => tokio::time::sleep(Duration::from_millis(500)).await,
                Err(error) => {
                    tracing::error!(?error, "media queue claim failed");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    })
}

async fn cleanup(pool: &Pool<Postgres>) -> Result<(), AppError> {
    let managed = PathBuf::from(crate::audio::paths::recording_path()).join(".media-jobs");
    let rows = sqlx::query!(
        "SELECT id,result_path FROM media_jobs WHERE state IN ('ready','failed') AND finished_at < now()-interval '30 days' ORDER BY finished_at LIMIT 100",
    )
    .fetch_all(pool)
    .await?;
    for row in rows {
        let id = row.id;
        if let Some(path) = row.result_path {
            let path = PathBuf::from(path);
            if !path.starts_with(&managed) {
                tracing::error!(job_id=%id, path=%path.display(), "refusing to clean unmanaged media result");
                continue;
            }
            match tokio::fs::remove_file(&path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(job_id=%id, ?error, "could not clean media result");
                    continue;
                }
            }
        }
        sqlx::query!(
            "DELETE FROM media_jobs WHERE id=$1 AND state IN ('ready','failed') AND finished_at < now()-interval '30 days'",
            id
        )
        .execute(pool)
        .await?;
    }
    Ok(())
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
mod tests {
    use super::*;
    use sqlx::PgPool;

    fn request(session_id: i64) -> MediaJobRequest {
        MediaJobRequest::SessionDownload {
            session_id,
            start: None,
            end: None,
            remove_silence: false,
        }
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn admission_is_idempotent_payload_bound_and_shared_by_resource(
        pool: PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let first = enqueue(
            &pool,
            None,
            crate::permissions::Viewer::discord(10),
            "key",
            "resource",
            &request(1),
        )
        .await?;
        assert_eq!(
            enqueue(
                &pool,
                None,
                crate::permissions::Viewer::discord(10),
                "key",
                "resource",
                &request(1)
            )
            .await?
            .id,
            first.id
        );
        assert!(matches!(
            enqueue(
                &pool,
                None,
                crate::permissions::Viewer::discord(10),
                "key",
                "other",
                &request(2)
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        let shared = enqueue(
            &pool,
            None,
            crate::permissions::Viewer::discord(11),
            "another",
            "resource",
            &request(1),
        )
        .await?;
        assert_eq!(shared.id, first.id);
        assert_eq!(load_status(&pool, 11, &first.id).await?.id, first.id);
        assert!(matches!(
            enqueue(
                &pool,
                None,
                crate::permissions::Viewer::discord(12),
                "different",
                "resource",
                &request(2)
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn per_user_admission_is_bounded(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
        for id in 1..=3 {
            enqueue(
                &pool,
                None,
                crate::permissions::Viewer::discord(10),
                &format!("key-{id}"),
                &format!("resource-{id}"),
                &request(id),
            )
            .await?;
        }
        assert!(matches!(
            enqueue(
                &pool,
                None,
                crate::permissions::Viewer::discord(10),
                "key-4",
                "resource-4",
                &request(4)
            )
            .await,
            Err(AppError::UserJobLimitReached)
        ));
        Ok(())
    }

    fn recording_waveform(n: i64) -> MediaJobRequest {
        MediaJobRequest::RecordingWaveform {
            guild_id: 1,
            channel_id: 2,
            year: 2026,
            month: 10,
            file_name: format!("recording-{n}"),
            silence_free: false,
        }
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn recording_waveforms_are_not_metered_per_user(
        pool: PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let viewer = crate::permissions::Viewer::discord(10);
        // A cold channel mix: one waveform per source, more than the limit.
        for n in 1..=PER_USER_ACTIVE_LIMIT + 3 {
            enqueue(
                &pool,
                None,
                viewer,
                &format!("waveform-{n}"),
                &format!("waveform-resource-{n}"),
                &recording_waveform(n),
            )
            .await?;
        }
        // They leave the user's own limit to the jobs the user asked for.
        for id in 1..=PER_USER_ACTIVE_LIMIT {
            enqueue(
                &pool,
                None,
                viewer,
                &format!("key-{id}"),
                &format!("resource-{id}"),
                &request(id),
            )
            .await?;
        }
        assert!(matches!(
            enqueue(&pool, None, viewer, "key-4", "resource-4", &request(4)).await,
            Err(AppError::UserJobLimitReached)
        ));
        // And a user at that limit can still see a mix's waveforms.
        enqueue(
            &pool,
            None,
            viewer,
            "waveform-late",
            "waveform-resource-late",
            &recording_waveform(99),
        )
        .await?;
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn claims_are_globally_bounded_and_expired_attempts_are_fenced(
        pool: PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for id in 1..=5 {
            enqueue(
                &pool,
                None,
                crate::permissions::Viewer::discord(id),
                &format!("key-{id}"),
                &format!("resource-{id}"),
                &request(id),
            )
            .await?;
        }
        let mut claimed = Vec::new();
        for _ in 0..5 {
            if let Some(job) = claim(&pool).await? {
                claimed.push(job);
            }
        }
        assert_eq!(claimed.len(), GLOBAL_RUNNING_LIMIT as usize);
        let old = &claimed[0];
        sqlx::query("UPDATE media_jobs SET lease_expires_at=now()-interval '1 second' WHERE id=$1")
            .bind(&old.id)
            .execute(&pool)
            .await?;
        let recovered = claim(&pool).await?.ok_or("expired job was not recovered")?;
        assert_eq!(recovered.id, old.id);
        assert_ne!(recovered.token, old.token);
        assert!(!report_progress(&pool, &old.id, &old.token, "late", 90).await?);
        assert!(report_progress(&pool, &recovered.id, &recovered.token, "rendering", 50).await?);
        assert!(begin_publication(&pool, &old.id, &old.token).await.is_err());
        let tx = begin_publication(&pool, &recovered.id, &recovered.token).await?;
        complete_publication(tx, &recovered.id, &recovered.token, "/result", None).await?;
        finish(&pool, old, Err(AppError::InternalError)).await?;
        assert_eq!(
            load_status(&pool, old.requester.user_id, &old.id)
                .await?
                .status,
            "ready"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn lease_updates_skip_the_attempts_own_publication(
        pool: PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        enqueue(
            &pool,
            None,
            crate::permissions::Viewer::discord(1),
            "key-1",
            "resource-1",
            &request(1),
        )
        .await?;
        let job = claim(&pool).await?.ok_or("job was not claimed")?;
        sqlx::query(
            "UPDATE media_jobs SET lease_expires_at=now()+interval '5 seconds' WHERE id=$1",
        )
        .bind(&job.id)
        .execute(&pool)
        .await?;
        assert!(renew(&pool, &job).await?);
        let left: f64 = sqlx::query_scalar(
            "SELECT EXTRACT(EPOCH FROM lease_expires_at-now())::float8 FROM media_jobs WHERE id=$1",
        )
        .bind(&job.id)
        .fetch_one(&pool)
        .await?;
        assert!(left > 50.0, "the lease was not extended: {left}s left");

        // The attempt's own loops renew and report while its publication
        // holds the row. Waiting for that lock deadlocked them.
        let tx = begin_publication(&pool, &job.id, &job.token).await?;
        let within = Duration::from_secs(5);
        assert!(tokio::time::timeout(within, renew(&pool, &job)).await??);
        assert!(
            tokio::time::timeout(
                within,
                report_progress(&pool, &job.id, &job.token, "rendering", 50)
            )
            .await??
        );
        complete_publication(tx, &job.id, &job.token, "/result", None).await?;
        assert!(!renew(&pool, &job).await?);
        assert!(!report_progress(&pool, &job.id, &job.token, "late", 90).await?);
        Ok(())
    }

    const INTERNAL: &str =
        "Database Error: relation media_jobs at /srv/sakiot/data/.media-jobs stderr: libopus";

    fn assert_safe(status: &MediaJobStatus) {
        let body = serde_json::to_string(status).unwrap();
        for secret in ["Database", "/srv", "stderr", "libopus"] {
            assert!(!body.contains(secret), "{secret} leaked: {body}");
        }
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn failures_report_a_safe_kind_and_terminal_failures_do_not_promise_retry(
        pool: PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let queued = enqueue(
            &pool,
            None,
            crate::permissions::Viewer::discord(10),
            "key",
            "resource",
            &request(1),
        )
        .await?;
        for attempt in 1..=MAX_ATTEMPTS {
            let job = claim(&pool).await?.ok_or("job was not claimed")?;
            finish(&pool, &job, Err(AppError::FfmpegError(INTERNAL.into()))).await?;
            let status = load_status(&pool, 10, &queued.id).await?;
            assert_safe(&status);
            assert_eq!(status.error_kind, Some(ErrorKind::MediaProcessingFailed));
            assert_eq!(
                status.error.as_deref(),
                Some(ErrorKind::MediaProcessingFailed.default_message())
            );
            let expected = if attempt < MAX_ATTEMPTS {
                "queued"
            } else {
                "failed"
            };
            assert_eq!(status.status, expected);
            assert!(
                !status
                    .error
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains("retry")
            );
            sqlx::query("UPDATE media_jobs SET retry_at=now()")
                .execute(&pool)
                .await?;
        }
        // Old releases read the raw column; it holds the public text too.
        let stored: Option<String> = sqlx::query_scalar("SELECT error FROM media_jobs WHERE id=$1")
            .bind(&queued.id)
            .fetch_one(&pool)
            .await?;
        assert_eq!(
            stored.as_deref(),
            Some(ErrorKind::MediaProcessingFailed.default_message())
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn legacy_error_text_is_never_returned(
        pool: PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let queued = enqueue(
            &pool,
            None,
            crate::permissions::Viewer::discord(10),
            "key",
            "resource",
            &request(1),
        )
        .await?;
        sqlx::query("UPDATE media_jobs SET state='failed', stage='failed', error=$2, error_kind=NULL WHERE id=$1")
            .bind(&queued.id)
            .bind(INTERNAL)
            .execute(&pool)
            .await?;
        let status = load_status(&pool, 10, &queued.id).await?;
        assert_safe(&status);
        assert_eq!(status.error_kind, None);
        assert_eq!(
            status.error.as_deref(),
            Some(crate::errors::UNCLASSIFIED_JOB_ERROR)
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn a_lost_lease_writes_nothing(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
        let queued = enqueue(
            &pool,
            None,
            crate::permissions::Viewer::discord(10),
            "key",
            "resource",
            &request(1),
        )
        .await?;
        let job = claim(&pool).await?.ok_or("job was not claimed")?;
        finish(&pool, &job, Err(AppError::JobLeaseLost)).await?;
        let status = load_status(&pool, 10, &queued.id).await?;
        assert_eq!((status.status.as_str(), status.error), ("running", None));
        Ok(())
    }
}
