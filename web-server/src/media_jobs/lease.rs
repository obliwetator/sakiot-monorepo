//! An attempt's lease: claiming and renewing it, reporting progress under it,
//! and the fenced publication that only a live lease may complete.

use super::*;

pub(super) async fn claim(pool: &Pool<Postgres>) -> Result<Option<ClaimedMediaJob>, AppError> {
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

pub(super) async fn renew(pool: &Pool<Postgres>, job: &ClaimedMediaJob) -> Result<bool, AppError> {
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

pub(super) async fn finish(
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
