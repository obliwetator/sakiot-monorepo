//! PostgreSQL owns job state. A lease token fences every attempt, including
//! final publication; polling is read-only and terminal results are retained.
use super::*;
use crate::errors::{ErrorKind, JobError};

pub const RENDERER_VERSION: i32 = 2;
pub(super) const MAX_ATTEMPTS: i32 = 3;
const QUEUE_LOCK: i64 = crate::media_jobs::QUEUE_LOCK;
/// Queued or running media and composition jobs admitted across all users.
const GLOBAL_ACTIVE_LIMIT: i64 = 100;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Snapshot {
    pub body: ComposeClipBody,
    pub sources: Vec<String>,
    pub overwrite: Option<ComposeOverwrite>,
    pub channel_id: i64,
    pub name: String,
}

pub(super) struct Job {
    pub id: String,
    pub guild_id: i64,
    pub user_id: i64,
    pub result_clip_id: String,
    pub snapshot: Snapshot,
    pub token: String,
}

pub(super) async fn existing(
    pool: &Pool<Postgres>,
    guild_id: i64,
    user_id: i64,
    key: &str,
    request: &serde_json::Value,
) -> Result<Option<String>, AppError> {
    let row = sqlx::query!(
        "SELECT id, request FROM composition_jobs WHERE guild_id = $1 AND user_id = $2 AND idempotency_key = $3",
        guild_id,
        user_id,
        key
    )
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        if row.request != *request {
            return Err(AppError::Conflict(
                "This export request key was already used for a different edit".into(),
            ));
        }
        Ok(row.id)
    })
    .transpose()
}

pub(super) async fn enqueue(
    pool: &Pool<Postgres>,
    guild_id: i64,
    user_id: i64,
    key: &str,
    request: &serde_json::Value,
    snapshot: &Snapshot,
) -> Result<String, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query!("SELECT pg_advisory_xact_lock($1)", QUEUE_LOCK)
        .execute(&mut *tx)
        .await?;
    let previous = sqlx::query!(
        "SELECT id, request FROM composition_jobs WHERE guild_id = $1 AND user_id = $2 AND idempotency_key = $3",
        guild_id,
        user_id,
        key
    )
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row) = previous {
        if row.request != *request {
            return Err(AppError::Conflict(
                "This export request key was already used for a different edit".into(),
            ));
        }
        return Ok(row.id);
    }
    // Personal and shared capacity are reported separately: only the first is
    // something the caller can resolve by waiting for their own exports.
    let (total, owned): (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM composition_jobs WHERE state IN ('queued','running'))
              + (SELECT count(*) FROM media_jobs WHERE state IN ('queued','running')),
            (SELECT count(*) FROM composition_jobs WHERE user_id=$1 AND state IN ('queued','running'))
              + (SELECT count(*) FROM media_jobs WHERE user_id=$1 AND state IN ('queued','running'))",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if owned >= crate::media_jobs::PER_USER_ACTIVE_LIMIT {
        return Err(AppError::UserJobLimitReached);
    }
    if total >= GLOBAL_ACTIVE_LIMIT {
        return Err(AppError::ExportQueueFull);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let result_id = snapshot
        .overwrite
        .as_ref()
        .map(|target| target.clip_id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    sqlx::query!(
        "INSERT INTO composition_jobs (id, guild_id, user_id, idempotency_key, request, snapshot, result_clip_id, renderer_version) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        id,
        guild_id,
        user_id,
        key,
        request,
        serde_json::to_value(snapshot).map_err(|_| AppError::InternalError)?,
        result_id,
        RENDERER_VERSION
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!(job_id = %id, guild_id, user_id, "composition queued");
    Ok(id)
}

pub(super) async fn claim(pool: &Pool<Postgres>) -> Result<Option<(String, String)>, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query!("SELECT pg_advisory_xact_lock($1)", QUEUE_LOCK)
        .execute(&mut *tx)
        .await?;
    sqlx::query!(
        "UPDATE composition_jobs SET state = 'failed', stage = 'failed', error = $2, error_kind = $3, attempt_token = NULL, lease_expires_at = NULL, finished_at = now(), updated_at = now() WHERE state = 'running' AND lease_expires_at < now() AND attempts >= $1",
        MAX_ATTEMPTS,
        ErrorKind::WorkerInterrupted.default_message(),
        ErrorKind::WorkerInterrupted.as_str()
    )
    .execute(&mut *tx)
    .await?;
    // One active composition per database, including overlapping web releases.
    let (running_compositions, running_total): (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM composition_jobs WHERE state='running' AND lease_expires_at >= now()),
            (SELECT count(*) FROM composition_jobs WHERE state='running' AND lease_expires_at >= now())
              + (SELECT count(*) FROM media_jobs WHERE state='running' AND lease_expires_at >= now())",
    )
    .fetch_one(&mut *tx)
    .await?;
    if running_compositions >= 1 || running_total >= 4 {
        tx.commit().await?;
        return Ok(None);
    }
    // Version 2 renders every segment in shared stereo DSP. Migrate only
    // unleased v1 work under the queue lock; active old workers can finish.
    // Expired attempts receive a new fencing token when claimed below.
    sqlx::query!(
        "UPDATE composition_jobs SET renderer_version = $1, updated_at = now() WHERE renderer_version = 1 AND (state = 'queued' OR (state = 'running' AND lease_expires_at < now()))",
        RENDERER_VERSION
    )
    .execute(&mut *tx)
    .await?;
    let token = uuid::Uuid::new_v4().to_string();
    let id = sqlx::query_scalar!(
        "WITH candidate AS (
        SELECT id FROM composition_jobs WHERE renderer_version = $2 AND attempts < $3
        AND ((state = 'queued' AND retry_at <= now()) OR (state = 'running' AND lease_expires_at < now()))
        ORDER BY created_at, id FOR UPDATE SKIP LOCKED LIMIT 1
        ) UPDATE composition_jobs j SET state = 'running', stage = 'preparing', progress = 0,
        attempts = attempts + 1, attempt_token = $1, lease_expires_at = now() + interval '60 seconds',
        error = NULL, error_kind = NULL, updated_at = now() FROM candidate WHERE j.id = candidate.id RETURNING j.id",
        token,
        RENDERER_VERSION,
        MAX_ATTEMPTS
    )
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id.map(|id| (id, token)))
}

pub(super) async fn load(pool: &Pool<Postgres>, id: &str, token: &str) -> Result<Job, AppError> {
    let row = sqlx::query!(
        "SELECT id, guild_id, user_id, result_clip_id, snapshot FROM composition_jobs WHERE id = $1 AND attempt_token = $2 AND state = 'running' AND lease_expires_at > now() AND renderer_version = $3",
        id,
        token,
        RENDERER_VERSION
    )
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::JobLeaseLost)?;
    Ok(Job {
        id: row.id,
        guild_id: row.guild_id,
        user_id: row.user_id,
        result_clip_id: row.result_clip_id,
        token: token.to_owned(),
        snapshot: serde_json::from_value(row.snapshot).map_err(|_| AppError::InternalError)?,
    })
}

pub(super) async fn renew(
    pool: &Pool<Postgres>,
    id: &str,
    token: &str,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query!(
        "UPDATE composition_jobs SET lease_expires_at = now() + interval '60 seconds', updated_at = now() WHERE id = $1 AND attempt_token = $2 AND state = 'running' AND lease_expires_at > now()",
        id,
        token
    )
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

pub(super) async fn report(
    pool: &Pool<Postgres>,
    id: &str,
    token: &str,
    stage: &str,
    progress: i16,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query!(
        "UPDATE composition_jobs SET stage = $3, progress = GREATEST(progress, $4), updated_at = now() WHERE id = $1 AND attempt_token = $2 AND state = 'running' AND lease_expires_at > now()",
        id,
        token,
        stage,
        progress.clamp(0, 99)
    )
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

/// Record a failed attempt. `retryable` alone decides whether it is retried;
/// the stored message is the kind's public text and never promises a retry.
pub(super) async fn fail(
    pool: &Pool<Postgres>,
    id: &str,
    token: &str,
    kind: ErrorKind,
    retryable: bool,
) -> Result<(), sqlx::Error> {
    let message = kind.default_message();
    let kind = kind.as_str();
    sqlx::query!(
        "UPDATE composition_jobs SET state = CASE WHEN $4 AND attempts < $5 THEN 'queued' ELSE 'failed' END,
        stage = CASE WHEN $4 AND attempts < $5 THEN 'retrying' ELSE 'failed' END,
        error = $3, error_kind = $6, attempt_token = NULL, lease_expires_at = NULL,
        retry_at = now() + attempts * interval '10 seconds',
        finished_at = CASE WHEN $4 AND attempts < $5 THEN NULL ELSE now() END, updated_at = now()
        WHERE id = $1 AND attempt_token = $2 AND state = 'running'",
        id,
        token,
        message,
        retryable,
        MAX_ATTEMPTS,
        kind
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn status(
    pool: &Pool<Postgres>,
    guild_id: i64,
    user_id: i64,
    id: &str,
) -> Result<ComposeClipStatus, AppError> {
    let row = sqlx::query!(
        "SELECT state, stage, progress, error, error_kind, result_clip_id FROM composition_jobs WHERE id = $1 AND guild_id = $2 AND user_id = $3",
        id,
        guild_id,
        user_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::ClipNotFound)?;
    let error = JobError::from_columns(row.error_kind.as_deref(), row.error.as_deref());
    Ok(ComposeClipStatus {
        result_clip_id: (row.state == "ready").then_some(row.result_clip_id),
        status: row.state,
        stage: row.stage,
        progress: row.progress,
        error_kind: error.as_ref().and_then(|error| error.kind),
        error: error.map(|error| error.message),
    })
}
