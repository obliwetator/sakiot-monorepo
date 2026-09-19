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
use sqlx::{Pool, Postgres, Row};

use crate::audio::WaveformProgressContainer;
use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::media_archive::MediaArchive;

/// The composition queue uses the same transaction lock for cross-queue limits.
pub const QUEUE_LOCK: i64 = 0x53414b4d45444941;
const MAX_ATTEMPTS: i32 = 3;
const GLOBAL_RUNNING_LIMIT: i64 = 4;
const PER_USER_ACTIVE_LIMIT: i64 = 3;

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
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ClaimedMediaJob {
    pub id: String,
    pub user_id: i64,
    pub request: MediaJobRequest,
    pub token: String,
}

pub async fn enqueue(
    pool: &Pool<Postgres>,
    guild_id: Option<i64>,
    user_id: i64,
    idempotency_key: &str,
    resource_key: &str,
    request: &MediaJobRequest,
) -> Result<MediaJobStatus, AppError> {
    if idempotency_key.is_empty() || idempotency_key.len() > 128 {
        return Err(AppError::BadRequest("Invalid media job request key".into()));
    }
    let request_json = serde_json::to_value(request).map_err(|_| AppError::InternalError)?;
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(QUEUE_LOCK)
        .execute(&mut *tx)
        .await?;

    if let Some(row) = sqlx::query(
        "SELECT id, request FROM media_jobs WHERE user_id = $1 AND kind = $2 AND idempotency_key = $3",
    )
    .bind(user_id)
    .bind(request.kind())
    .bind(idempotency_key)
    .fetch_optional(&mut *tx)
    .await?
    {
        let previous: serde_json::Value = row.try_get("request")?;
        if previous != request_json {
            return Err(AppError::Conflict(
                "This media request key was already used for different work".into(),
            ));
        }
        let id: String = row.try_get("id")?;
        sqlx::query("INSERT INTO media_job_viewers (job_id,user_id) VALUES ($1,$2) ON CONFLICT DO NOTHING")
            .bind(&id).bind(user_id).execute(&mut *tx).await?;
        tx.commit().await?;
        return load_status(pool, user_id, &id).await;
    }

    if let Some(row) = sqlx::query(
        "SELECT id, request FROM media_jobs WHERE kind = $1 AND resource_key = $2 AND state IN ('queued', 'running') ORDER BY created_at LIMIT 1",
    )
    .bind(request.kind())
    .bind(resource_key)
    .fetch_optional(&mut *tx)
    .await?
    {
        let previous: serde_json::Value = row.try_get("request")?;
        if previous != request_json {
            return Err(AppError::Conflict(
                "This media resource is already being processed with different settings".into(),
            ));
        }
        let id: String = row.try_get("id")?;
        sqlx::query("INSERT INTO media_job_viewers (job_id,user_id) VALUES ($1,$2) ON CONFLICT DO NOTHING")
            .bind(&id).bind(user_id).execute(&mut *tx).await?;
        tx.commit().await?;
        return load_status(pool, user_id, &id).await;
    }

    let active: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM media_jobs WHERE user_id = $1 AND state IN ('queued','running')) + (SELECT count(*) FROM composition_jobs WHERE user_id = $1 AND state IN ('queued','running'))",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if active >= PER_USER_ACTIVE_LIMIT {
        return Err(AppError::ServiceUnavailable(
            "You already have the maximum number of active media jobs".into(),
        ));
    }

    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO media_jobs (id, kind, guild_id, user_id, idempotency_key, resource_key, request) VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(&id)
    .bind(request.kind())
    .bind(guild_id)
    .bind(user_id)
    .bind(idempotency_key)
    .bind(resource_key)
    .bind(request_json)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO media_job_viewers (job_id,user_id) VALUES ($1,$2)")
        .bind(&id)
        .bind(user_id)
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
    let row = sqlx::query(
        "SELECT j.id, j.kind, j.state, j.stage, j.progress, j.result_url, j.error FROM media_jobs j JOIN media_job_viewers v ON v.job_id=j.id WHERE j.id = $1 AND v.user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::FileNotFound)?;
    Ok(MediaJobStatus {
        id: row.try_get("id")?,
        kind: row.try_get("kind")?,
        status: row.try_get("state")?,
        stage: row.try_get("stage")?,
        progress: row.try_get("progress")?,
        result_url: row.try_get("result_url")?,
        error: row.try_get("error")?,
    })
}

pub async fn active_for_resource(
    pool: &Pool<Postgres>,
    user_id: i64,
    kind: &str,
    resource_key: &str,
) -> Result<Option<MediaJobStatus>, AppError> {
    let id: Option<String> = sqlx::query_scalar(
        "SELECT j.id FROM media_jobs j JOIN media_job_viewers v ON v.job_id=j.id WHERE v.user_id=$1 AND j.kind=$2 AND j.resource_key=$3 AND j.state IN ('queued','running') ORDER BY j.created_at DESC LIMIT 1",
    )
    .bind(user_id).bind(kind).bind(resource_key).fetch_optional(pool).await?;
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
    let id: Option<String> = sqlx::query_scalar(
        "SELECT j.id FROM media_jobs j JOIN media_job_viewers v ON v.job_id=j.id WHERE v.user_id=$1 AND j.kind=$2 AND j.resource_key=$3 AND j.state='failed' ORDER BY j.created_at DESC LIMIT 1",
    )
    .bind(user_id).bind(kind).bind(resource_key).fetch_optional(pool).await?;
    match id {
        Some(id) => load_status(pool, user_id, &id).await.map(Some),
        None => Ok(None),
    }
}

async fn claim(pool: &Pool<Postgres>) -> Result<Option<ClaimedMediaJob>, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(QUEUE_LOCK)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE media_jobs SET state='failed', stage='failed', error='Media worker stopped repeatedly. Submit the request again.', attempt_token=NULL, lease_expires_at=NULL, finished_at=now(), updated_at=now() WHERE state='running' AND lease_expires_at < now() AND attempts >= $1",
    )
    .bind(MAX_ATTEMPTS)
    .execute(&mut *tx)
    .await?;
    let running: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM media_jobs WHERE state='running' AND lease_expires_at >= now()) + (SELECT count(*) FROM composition_jobs WHERE state='running' AND lease_expires_at >= now())",
    )
    .fetch_one(&mut *tx)
    .await?;
    if running >= GLOBAL_RUNNING_LIMIT {
        tx.commit().await?;
        return Ok(None);
    }
    let token = uuid::Uuid::new_v4().to_string();
    let row = sqlx::query(
        "WITH candidate AS (SELECT id FROM media_jobs WHERE attempts < $2 AND ((state='queued' AND retry_at <= now()) OR (state='running' AND lease_expires_at < now())) ORDER BY created_at,id FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE media_jobs j SET state='running',stage='preparing',progress=0,attempts=attempts+1,attempt_token=$1,lease_expires_at=now()+interval '60 seconds',error=NULL,updated_at=now() FROM candidate WHERE j.id=candidate.id RETURNING j.id,j.user_id,j.request",
    )
    .bind(&token)
    .bind(MAX_ATTEMPTS)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    row.map(|row| {
        let request: serde_json::Value = row.try_get("request")?;
        Ok(ClaimedMediaJob {
            id: row.try_get("id")?,
            user_id: row.try_get("user_id")?,
            request: serde_json::from_value(request).map_err(|_| AppError::InternalError)?,
            token,
        })
    })
    .transpose()
}

async fn renew(pool: &Pool<Postgres>, job: &ClaimedMediaJob) -> Result<bool, AppError> {
    Ok(sqlx::query("UPDATE media_jobs SET lease_expires_at=now()+interval '60 seconds',updated_at=now() WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at > now()")
        .bind(&job.id).bind(&job.token).execute(pool).await?.rows_affected() == 1)
}

pub async fn report_progress(
    pool: &Pool<Postgres>,
    id: &str,
    token: &str,
    stage: &str,
    progress: i16,
) -> Result<bool, AppError> {
    Ok(sqlx::query("UPDATE media_jobs SET stage=$3,progress=$4,updated_at=now() WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at > now()")
        .bind(id).bind(token).bind(stage).bind(progress.clamp(0, 99)).execute(pool).await?.rows_affected() == 1)
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
                    return Err(AppError::Conflict("Media job lease lost".into()));
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
    let owned: Option<String> = sqlx::query_scalar(
        "SELECT id FROM media_jobs WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at > now() FOR UPDATE",
    )
    .bind(id)
    .bind(token)
    .fetch_optional(&mut *tx)
    .await?;
    if owned.is_none() {
        return Err(AppError::Conflict("Media job lease lost".into()));
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
    let updated = sqlx::query(
        "UPDATE media_jobs SET state='ready',stage='ready',progress=100,result_url=$3,result_path=$4,error=NULL,attempt_token=NULL,lease_expires_at=NULL,finished_at=now(),updated_at=now() WHERE id=$1 AND attempt_token=$2 AND state='running'",
    )
    .bind(id)
    .bind(token)
    .bind(result_url)
    .bind(result_path)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(AppError::Conflict("Media job lease lost".into()));
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
        Err(error) if job_attempts(pool, &job.id).await? < MAX_ATTEMPTS => (
            "queued",
            "queued",
            0,
            None,
            None,
            Some(error.to_string()),
            true,
        ),
        Err(error) => (
            "failed",
            "failed",
            0,
            None,
            None,
            Some(error.to_string()),
            false,
        ),
    };
    let query = if retry {
        "UPDATE media_jobs SET state=$3,stage=$4,progress=$5,result_url=$6,result_path=$7,error=$8,attempt_token=NULL,lease_expires_at=NULL,retry_at=now()+interval '5 seconds',finished_at=CASE WHEN $3 IN ('ready','failed') THEN now() ELSE NULL END,updated_at=now() WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at > now()"
    } else {
        "UPDATE media_jobs SET state=$3,stage=$4,progress=$5,result_url=$6,result_path=$7,error=$8,attempt_token=NULL,lease_expires_at=NULL,retry_at=now(),finished_at=CASE WHEN $3 IN ('ready','failed') THEN now() ELSE NULL END,updated_at=now() WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at > now()"
    };
    sqlx::query(query)
        .bind(&job.id)
        .bind(&job.token)
        .bind(state)
        .bind(stage)
        .bind(progress)
        .bind(result_url)
        .bind(result_path)
        .bind(error)
        .execute(pool)
        .await?;
    Ok(())
}

async fn job_attempts(pool: &Pool<Postgres>, id: &str) -> Result<i32, AppError> {
    Ok(
        sqlx::query_scalar("SELECT attempts FROM media_jobs WHERE id=$1")
            .bind(id)
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
    match &job.request {
        MediaJobRequest::RecordingSilence {
            guild_id,
            channel_id,
            year,
            month,
            file_name,
        } => {
            crate::audio::silence::run_recording_silence_job(
                pool,
                media,
                *guild_id,
                *channel_id,
                *year,
                *month,
                file_name,
                &job.id,
                &job.token,
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
                pool,
                media,
                *guild_id,
                *channel_id,
                *year,
                *month,
                file_name,
                *silence_free,
                &job.id,
                &job.token,
            )
            .await
        }
        MediaJobRequest::ClipWaveform { guild_id, clip_id } => {
            crate::audio::peaks::run_clip_waveform_job(
                pool, media, *guild_id, clip_id, &job.id, &job.token,
            )
            .await
        }
        MediaJobRequest::SessionWaveform {
            session_id,
            silence_free,
        } => {
            crate::audio::sessions::run_session_waveform_job(
                pool,
                media,
                job.user_id,
                *session_id,
                *silence_free,
                &job.id,
                &job.token,
            )
            .await
        }
        MediaJobRequest::SessionSilence { session_id } => {
            crate::audio::sessions::run_session_silence_job(
                pool,
                media,
                job.user_id,
                *session_id,
                &job.id,
                &job.token,
            )
            .await
        }
        MediaJobRequest::SessionMix {
            session_id,
            scope,
            participants,
        } => {
            crate::audio::sessions::run_session_mix_job(
                pool,
                media,
                job.user_id,
                *session_id,
                scope,
                participants.clone(),
                &job.id,
                &job.token,
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
                pool,
                media,
                job.user_id,
                *session_id,
                *start,
                *end,
                *remove_silence,
                &job.id,
                &job.token,
            )
            .await
        }
    }
}

async fn execute(pool: Pool<Postgres>, media: MediaArchive, job: ClaimedMediaJob) {
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
                _ = &mut deadline => break Err(AppError::ServiceUnavailable(
                    "Media job exceeded the 30-minute attempt limit".into(),
                )),
                _ = interval.tick() => match renew(&pool, &job).await {
                    Ok(true) => lease_confirmed_at = tokio::time::Instant::now(),
                    Ok(false) => break Err(AppError::Conflict("Media job lease lost".into())),
                    Err(error) => {
                        tracing::warn!(job_id=%job.id, ?error, "media lease renewal failed");
                        if lease_confirmed_at.elapsed() >= Duration::from_secs(40) {
                            break Err(AppError::ServiceUnavailable(
                                "Media job lease could not be renewed".into(),
                            ));
                        }
                    }
                },
            }
        }
    };
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
    let rows = sqlx::query(
        "SELECT id,result_path FROM media_jobs WHERE state IN ('ready','failed') AND finished_at < now()-interval '30 days' ORDER BY finished_at LIMIT 100",
    )
    .fetch_all(pool)
    .await?;
    for row in rows {
        let id: String = row.try_get("id")?;
        let path: Option<String> = row.try_get("result_path")?;
        if let Some(path) = path {
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
        sqlx::query("DELETE FROM media_jobs WHERE id=$1 AND state IN ('ready','failed') AND finished_at < now()-interval '30 days'")
            .bind(&id).execute(pool).await?;
    }
    Ok(())
}

#[utoipa::path(get, path = "/api/media-jobs/{job_id}", tag = "media", responses((status = 200, body = MediaJobStatus)))]
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
    let row = sqlx::query("SELECT j.state,j.result_path,j.request FROM media_jobs j JOIN media_job_viewers v ON v.job_id=j.id WHERE j.id=$1 AND v.user_id=$2")
        .bind(path.as_str()).bind(token.user_id).fetch_optional(pool.get_ref()).await?
        .ok_or(AppError::FileNotFound)?;
    let state: String = row.try_get("state")?;
    let result_path: Option<String> = row.try_get("result_path")?;
    let job_request: serde_json::Value = row.try_get("request")?;
    if state != "ready" {
        return Err(AppError::Conflict("Media result is not ready".into()));
    }
    let job_request: MediaJobRequest =
        serde_json::from_value(job_request).map_err(|_| AppError::InternalError)?;
    if let MediaJobRequest::SessionDownload { session_id, .. } = job_request {
        crate::audio::sessions::require_session_access(&pool, session_id, token.user_id).await?;
    } else {
        return Err(AppError::FileNotFound);
    }
    let result_path = result_path.ok_or(AppError::FileNotFound)?;
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
        let first = enqueue(&pool, None, 10, "key", "resource", &request(1)).await?;
        assert_eq!(
            enqueue(&pool, None, 10, "key", "resource", &request(1))
                .await?
                .id,
            first.id
        );
        assert!(matches!(
            enqueue(&pool, None, 10, "key", "other", &request(2)).await,
            Err(AppError::Conflict(_))
        ));
        let shared = enqueue(&pool, None, 11, "another", "resource", &request(1)).await?;
        assert_eq!(shared.id, first.id);
        assert_eq!(load_status(&pool, 11, &first.id).await?.id, first.id);
        assert!(matches!(
            enqueue(&pool, None, 12, "different", "resource", &request(2)).await,
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
                10,
                &format!("key-{id}"),
                &format!("resource-{id}"),
                &request(id),
            )
            .await?;
        }
        assert!(matches!(
            enqueue(&pool, None, 10, "key-4", "resource-4", &request(4)).await,
            Err(AppError::ServiceUnavailable(_))
        ));
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
                id,
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
            load_status(&pool, old.user_id, &old.id).await?.status,
            "ready"
        );
        Ok(())
    }
}
