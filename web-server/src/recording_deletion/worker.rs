//! The deletion queue: claiming jobs, running one attempt, and recording
//! its outcome under the attempt's lease.

use super::*;

pub fn spawn_worker(
    pool: Pool<Postgres>,
    media: MediaArchive,
    policy: DeletionPolicy,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut retention = tokio::time::interval(Duration::from_secs(60));
        loop {
            tokio::select! {
                _ = retention.tick() => {
                    if let Err(error) = enqueue_expired(&pool).await {
                        tracing::error!(?error, "retention sweep failed");
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
            if policy.allow_permanent
                && let Err(error) = run_one(&pool, &media, &DataRoots::from_env()).await
            {
                tracing::error!(?error, "recording deletion worker failed");
            }
        }
    })
}

pub(super) async fn enqueue_expired(pool: &Pool<Postgres>) -> Result<(), AppError> {
    // One small page per sweep. Retention only hides data; it never authorizes
    // permanent destruction, regardless of the permanent-delete feature flag.
    let rows = sqlx::query!(
        "SELECT rs.guild_id,rs.id FROM recording_sessions rs JOIN guild_recording_policy p ON p.guild_id=rs.guild_id WHERE p.retention_days IS NOT NULL AND rs.state='finalized' AND rs.ended_at < now() - (p.retention_days * interval '1 day') AND rs.deletion_requested_at IS NULL ORDER BY rs.ended_at,rs.id LIMIT 25"
    )
    .fetch_all(pool)
    .await?;
    for row in rows {
        let (guild_id, session_id) = (row.guild_id, row.id);
        if let Err(error) = enqueue(
            pool,
            guild_id,
            session_id,
            None,
            "retention",
            DeletionMode::Soft,
        )
        .await
        {
            tracing::warn!(guild_id, session_id, ?error, "retention enqueue skipped");
        }
    }
    Ok(())
}

pub(super) struct Claimed {
    pub(super) id: String,
    pub(super) session_id: i64,
    pub(super) guild_id: i64,
    pub(super) token: String,
}

pub(super) async fn claim(pool: &Pool<Postgres>) -> Result<Option<Claimed>, AppError> {
    let token = uuid::Uuid::new_v4().to_string();
    let row = sqlx::query!(
        "WITH candidate AS (SELECT id FROM recording_deletion_jobs WHERE mode='permanent' AND attempts < $1 AND ((state='queued' AND retry_at<=now()) OR (state='running' AND lease_expires_at<now())) ORDER BY retry_at,created_at FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE recording_deletion_jobs j SET state='running',stage='checking',attempts=attempts+1,attempt_token=$2,lease_expires_at=now()+interval '5 minutes',updated_at=now() FROM candidate WHERE j.id=candidate.id RETURNING j.id,j.recording_session_id,j.guild_id",
        MAX_ATTEMPTS,
        token
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| Claimed {
        id: row.id,
        session_id: row.recording_session_id,
        guild_id: row.guild_id,
        token,
    }))
}

/// Why a permanent-deletion attempt stopped before finishing. Retry policy
/// is decided by the variant alone, never by the wording shown to managers.
#[derive(Debug, thiserror::Error)]
pub(super) enum AttemptError {
    /// Guild media work is queued or running. Requeue without spending an
    /// attempt: waiting is expected and must not exhaust the failure budget.
    #[error("waiting for guild media work")]
    Waiting,
    /// Another attempt now owns the job. Stop without writing any status.
    #[error("deletion lease lost")]
    LeaseLost,
    /// A real failure. Spends an attempt and backs off until the limit.
    #[error("{}: {detail}", kind.as_str())]
    Failed { kind: ErrorKind, detail: String },
}

impl AttemptError {
    pub(super) fn failed(kind: ErrorKind, detail: impl Into<String>) -> Self {
        Self::Failed {
            kind,
            detail: detail.into(),
        }
    }
}

impl From<AppError> for AttemptError {
    fn from(error: AppError) -> Self {
        match error {
            AppError::JobLeaseLost => Self::LeaseLost,
            error => Self::failed(error.kind(), format!("{error:?}")),
        }
    }
}

impl From<sqlx::Error> for AttemptError {
    fn from(error: sqlx::Error) -> Self {
        AppError::from(error).into()
    }
}

pub(super) async fn run_one(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    roots: &DataRoots,
) -> Result<(), AppError> {
    let Some(job) = claim(pool).await? else {
        return Ok(());
    };
    let result = {
        let mut work = Box::pin(process_with_roots(pool, media, &job, roots));
        let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
        loop {
            tokio::select! {
                result = &mut work => break result,
                _ = heartbeat.tick() => {
                    let renewed = sqlx::query!(
                        "UPDATE recording_deletion_jobs SET lease_expires_at=now()+interval '5 minutes',updated_at=now() WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at>now()",
                        job.id,
                        job.token
                    )
                    .execute(pool)
                    .await?
                    .rows_affected();
                    if renewed != 1 { break Err(AttemptError::LeaseLost); }
                }
            }
        }
    };
    record_outcome(pool, &job, result).await
}

/// Persist an attempt's outcome. Every write is fenced by the attempt token,
/// so an attempt that lost its lease cannot touch a reclaimed job.
pub(super) async fn record_outcome(
    pool: &Pool<Postgres>,
    job: &Claimed,
    result: Result<(), AttemptError>,
) -> Result<(), AppError> {
    match result {
        Ok(()) => {}
        Err(AttemptError::LeaseLost) => {
            tracing::warn!(job_id = %job.id, "recording deletion lease lost; leaving the job to its new owner");
        }
        Err(AttemptError::Waiting) => {
            tracing::info!(job_id = %job.id, "recording deletion waiting for guild media work");
            let kind = ErrorKind::WaitingForMediaWork;
            sqlx::query!(
                "UPDATE recording_deletion_jobs SET attempts=attempts-1,state='queued',stage='waiting',error=$3,error_kind=$4,retry_at=now()+interval '30 seconds',attempt_token=NULL,lease_expires_at=NULL,updated_at=now() WHERE id=$1 AND attempt_token=$2 AND state='running'",
                job.id,
                job.token,
                kind.default_message(),
                kind.as_str()
            )
            .execute(pool)
            .await?;
        }
        Err(AttemptError::Failed { kind, detail }) => {
            tracing::warn!(job_id = %job.id, kind = kind.as_str(), %detail, "recording deletion attempt failed");
            sqlx::query!(
                "UPDATE recording_deletion_jobs SET state=CASE WHEN attempts >= $3 THEN 'failed' ELSE 'queued' END,stage=CASE WHEN attempts >= $3 THEN 'failed' ELSE 'retry' END,error=$4,error_kind=$5,retry_at=now()+interval '30 seconds',attempt_token=NULL,lease_expires_at=NULL,updated_at=now(),finished_at=CASE WHEN attempts >= $3 THEN now() ELSE NULL END WHERE id=$1 AND attempt_token=$2 AND state='running'",
                job.id,
                job.token,
                MAX_ATTEMPTS,
                kind.default_message(),
                kind.as_str()
            )
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

pub(super) async fn stage(
    pool: &Pool<Postgres>,
    job: &Claimed,
    value: &str,
) -> Result<(), AttemptError> {
    let changed = sqlx::query!(
        "UPDATE recording_deletion_jobs SET stage=$3,lease_expires_at=now()+interval '5 minutes',updated_at=now() WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at>now()",
        job.id,
        job.token,
        value
    )
    .execute(pool)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(AttemptError::LeaseLost);
    }
    Ok(())
}
