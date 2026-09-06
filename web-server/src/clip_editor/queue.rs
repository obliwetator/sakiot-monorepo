//! PostgreSQL owns job state. A lease token fences every attempt, including
//! final publication; polling is read-only and terminal results are retained.
use super::*;

pub(super) const RENDERER_VERSION: i32 = 1;
pub(super) const MAX_ATTEMPTS: i32 = 3;
const QUEUE_LOCK: i64 = 0x53414b434f4d50;

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
    let row = sqlx::query("SELECT id, request FROM composition_jobs WHERE guild_id = $1 AND user_id = $2 AND idempotency_key = $3")
        .bind(guild_id).bind(user_id).bind(key).fetch_optional(pool).await?;
    row.map(|row| {
        if row.try_get::<serde_json::Value, _>("request")? != *request {
            return Err(AppError::Conflict(
                "This export request key was already used for a different edit".into(),
            ));
        }
        Ok(row.try_get("id")?)
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
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(QUEUE_LOCK)
        .execute(&mut *tx)
        .await?;
    let previous = sqlx::query("SELECT id, request FROM composition_jobs WHERE guild_id = $1 AND user_id = $2 AND idempotency_key = $3")
        .bind(guild_id).bind(user_id).bind(key).fetch_optional(&mut *tx).await?;
    if let Some(row) = previous {
        if row.try_get::<serde_json::Value, _>("request")? != *request {
            return Err(AppError::Conflict(
                "This export request key was already used for a different edit".into(),
            ));
        }
        return Ok(row.try_get("id")?);
    }
    let row = sqlx::query("SELECT count(*) AS total, count(*) FILTER (WHERE user_id = $1) AS owned FROM composition_jobs WHERE state IN ('queued', 'running')")
        .bind(user_id).fetch_one(&mut *tx).await?;
    if row.try_get::<i64, _>("total")? >= 100 || row.try_get::<i64, _>("owned")? >= 3 {
        return Err(AppError::ServiceUnavailable(
            "Export queue is full; try again after an export finishes".into(),
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let result_id = snapshot
        .overwrite
        .as_ref()
        .map(|target| target.clip_id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    sqlx::query("INSERT INTO composition_jobs (id, guild_id, user_id, idempotency_key, request, snapshot, result_clip_id, renderer_version) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(&id).bind(guild_id).bind(user_id).bind(key).bind(request)
        .bind(serde_json::to_value(snapshot).map_err(|_| AppError::InternalError)?)
        .bind(result_id).bind(RENDERER_VERSION).execute(&mut *tx).await?;
    tx.commit().await?;
    tracing::info!(job_id = %id, guild_id, user_id, "composition queued");
    Ok(id)
}

pub(super) async fn claim(pool: &Pool<Postgres>) -> Result<Option<(String, String)>, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(QUEUE_LOCK)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE composition_jobs SET state = 'failed', stage = 'failed', error = 'Export worker stopped repeatedly. Please submit the export again.', attempt_token = NULL, lease_expires_at = NULL, finished_at = now(), updated_at = now() WHERE state = 'running' AND lease_expires_at < now() AND attempts >= $1")
        .bind(MAX_ATTEMPTS).execute(&mut *tx).await?;
    // One active composition per database, including overlapping web releases.
    let running: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM composition_jobs WHERE state = 'running' AND lease_expires_at >= now())")
        .fetch_one(&mut *tx).await?;
    if running {
        tx.commit().await?;
        return Ok(None);
    }
    let token = uuid::Uuid::new_v4().to_string();
    let id: Option<String> = sqlx::query_scalar("WITH candidate AS (
        SELECT id FROM composition_jobs WHERE renderer_version = $2 AND attempts < $3
        AND ((state = 'queued' AND retry_at <= now()) OR (state = 'running' AND lease_expires_at < now()))
        ORDER BY created_at, id FOR UPDATE SKIP LOCKED LIMIT 1
        ) UPDATE composition_jobs j SET state = 'running', stage = 'preparing', progress = 0,
        attempts = attempts + 1, attempt_token = $1, lease_expires_at = now() + interval '60 seconds',
        error = NULL, updated_at = now() FROM candidate WHERE j.id = candidate.id RETURNING j.id")
        .bind(&token).bind(RENDERER_VERSION).bind(MAX_ATTEMPTS).fetch_optional(&mut *tx).await?;
    tx.commit().await?;
    Ok(id.map(|id| (id, token)))
}

pub(super) async fn load(pool: &Pool<Postgres>, id: &str, token: &str) -> Result<Job, AppError> {
    let row = sqlx::query("SELECT id, guild_id, user_id, result_clip_id, snapshot FROM composition_jobs WHERE id = $1 AND attempt_token = $2 AND state = 'running' AND lease_expires_at > now() AND renderer_version = $3")
        .bind(id).bind(token).bind(RENDERER_VERSION).fetch_optional(pool).await?.ok_or(AppError::Conflict("Export lease lost".into()))?;
    Ok(Job {
        id: row.try_get("id")?,
        guild_id: row.try_get("guild_id")?,
        user_id: row.try_get("user_id")?,
        result_clip_id: row.try_get("result_clip_id")?,
        token: token.to_owned(),
        snapshot: serde_json::from_value(row.try_get("snapshot")?)
            .map_err(|_| AppError::InternalError)?,
    })
}

pub(super) async fn renew(
    pool: &Pool<Postgres>,
    id: &str,
    token: &str,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query("UPDATE composition_jobs SET lease_expires_at = now() + interval '60 seconds', updated_at = now() WHERE id = $1 AND attempt_token = $2 AND state = 'running' AND lease_expires_at > now()")
        .bind(id).bind(token).execute(pool).await?.rows_affected() == 1)
}

pub(super) async fn report(
    pool: &Pool<Postgres>,
    id: &str,
    token: &str,
    stage: &str,
    progress: i16,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query("UPDATE composition_jobs SET stage = $3, progress = GREATEST(progress, $4), updated_at = now() WHERE id = $1 AND attempt_token = $2 AND state = 'running' AND lease_expires_at > now()")
        .bind(id).bind(token).bind(stage).bind(progress.clamp(0,99)).execute(pool).await?.rows_affected() == 1)
}

pub(super) async fn fail(
    pool: &Pool<Postgres>,
    id: &str,
    token: &str,
    message: &str,
    retryable: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE composition_jobs SET state = CASE WHEN $4 AND attempts < $5 THEN 'queued' ELSE 'failed' END,
        stage = CASE WHEN $4 AND attempts < $5 THEN 'retrying' ELSE 'failed' END,
        error = $3, attempt_token = NULL, lease_expires_at = NULL,
        retry_at = now() + attempts * interval '10 seconds',
        finished_at = CASE WHEN $4 AND attempts < $5 THEN NULL ELSE now() END, updated_at = now()
        WHERE id = $1 AND attempt_token = $2 AND state = 'running'")
        .bind(id).bind(token).bind(message).bind(retryable).bind(MAX_ATTEMPTS).execute(pool).await?;
    Ok(())
}

pub(super) async fn status(
    pool: &Pool<Postgres>,
    guild_id: i64,
    user_id: i64,
    id: &str,
) -> Result<ComposeClipStatus, AppError> {
    let row = sqlx::query("SELECT state, stage, progress, error, result_clip_id FROM composition_jobs WHERE id = $1 AND guild_id = $2 AND user_id = $3")
        .bind(id).bind(guild_id).bind(user_id).fetch_optional(pool).await?.ok_or(AppError::ClipNotFound)?;
    let state: String = row.try_get("state")?;
    Ok(ComposeClipStatus {
        result_clip_id: (state == "ready")
            .then(|| row.try_get("result_clip_id"))
            .transpose()?,
        status: state,
        stage: row.try_get("stage")?,
        progress: row.try_get("progress")?,
        error: row.try_get("error")?,
    })
}
