//! Audited recording removal. Soft deletion is immediate and retains all data;
//! permanent purging needs an explicit request and an enabled server policy.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use actix_web::{HttpRequest, HttpResponse, delete, get, web};
use chrono::{DateTime, Datelike, Utc};
use sakiot_paths::{DataRoots, RecordingKey, SessionKey};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres};

use crate::errors::{AppError, ErrorKind, JobError};
use crate::media_archive::MediaArchive;
use crate::permissions::require_guild_manager;

const MAX_ATTEMPTS: i32 = 10;
pub(crate) const ARCHIVE_LOCK_NAMESPACE: i32 = 0x53414b41;

#[derive(Clone, Copy, Debug, Default)]
pub struct DeletionPolicy {
    pub allow_permanent: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DeletionMode {
    #[default]
    Soft,
    Permanent,
}

impl DeletionMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Soft => "soft",
            Self::Permanent => "permanent",
        }
    }
}

#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
struct DeleteRecordingQuery {
    /// Defaults to soft. Permanent requires the server feature flag.
    mode: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RecordingDeletionStatus {
    pub id: String,
    pub recording_session_id: String,
    pub status_url: String,
    pub state: String,
    pub stage: String,
    pub mode: String,
    pub attempts: i32,
    /// Safe explanation of the last attempt's outcome; never internal detail.
    pub error: Option<String>,
    /// Stable classification of `error`. Absent for records that predate it.
    pub error_kind: Option<ErrorKind>,
}

async fn load_status(
    pool: &Pool<Postgres>,
    guild_id: i64,
    id: &str,
) -> Result<RecordingDeletionStatus, AppError> {
    let row = sqlx::query!(
        "SELECT recording_session_id,state,stage,mode,attempts,error,error_kind FROM recording_deletion_jobs WHERE guild_id=$1 AND id=$2",
        guild_id,
        id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let error = JobError::from_columns(row.error_kind.as_deref(), row.error.as_deref());
    Ok(RecordingDeletionStatus {
        id: id.to_owned(),
        recording_session_id: row.recording_session_id.to_string(),
        status_url: format!("/api/admin/guilds/{guild_id}/recording-deletions/{id}"),
        state: row.state,
        stage: row.stage,
        mode: row.mode,
        attempts: row.attempts,
        error_kind: error.as_ref().and_then(|error| error.kind),
        error: error.map(|error| error.message),
    })
}

#[utoipa::path(
    delete,
    path = "/api/admin/guilds/{guild_id}/recordings/{recording_session_id}",
    tag = "admin",
    params(("guild_id" = i64, Path), ("recording_session_id" = i64, Path), DeleteRecordingQuery),
    responses(
        (status = 202, description = "Recording hidden; soft deletion is complete, permanent deletion may be queued", body = RecordingDeletionStatus),
        (status = 403, description = "Manage Guild required or permanent deletion disabled", body = crate::errors::ApiError),
        (status = 404, description = "Recording not found", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[delete("/admin/guilds/{guild_id}/recordings/{recording_session_id}")]
pub async fn delete_recording(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, i64)>,
    query: web::Query<DeleteRecordingQuery>,
    policy: web::Data<DeletionPolicy>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, session_id) = path.into_inner();
    let user_id = require_guild_manager(&req, &pool, guild_id).await?;
    let mode = match query.mode.as_deref() {
        None | Some("soft") => DeletionMode::Soft,
        Some("permanent") => DeletionMode::Permanent,
        Some(_) => {
            return Err(AppError::BadRequest(
                "Unknown recording deletion mode".into(),
            ));
        }
    };
    if mode == DeletionMode::Permanent && !policy.allow_permanent {
        return Err(AppError::Forbidden);
    }
    let id = enqueue(&pool, guild_id, session_id, Some(user_id), "manager", mode).await?;
    Ok(HttpResponse::Accepted().json(load_status(&pool, guild_id, &id).await?))
}

#[utoipa::path(
    get,
    path = "/api/admin/guilds/{guild_id}/recording-deletions/{job_id}",
    tag = "admin",
    params(("guild_id" = i64, Path), ("job_id" = String, Path)),
    responses(
        (status = 200, description = "Audited deletion status", body = RecordingDeletionStatus),
        (status = 403, description = "Manage Guild required", body = crate::errors::ApiError),
        (status = 404, description = "Deletion record not found", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/admin/guilds/{guild_id}/recording-deletions/{job_id}")]
pub async fn get_recording_deletion(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, String)>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, id) = path.into_inner();
    require_guild_manager(&req, &pool, guild_id).await?;
    Ok(HttpResponse::Ok().json(load_status(&pool, guild_id, &id).await?))
}

async fn enqueue(
    pool: &Pool<Postgres>,
    guild_id: i64,
    session_id: i64,
    actor: Option<i64>,
    reason: &str,
    mode: DeletionMode,
) -> Result<String, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query!("SELECT pg_advisory_xact_lock($1)", guild_id)
        .execute(&mut *tx)
        .await?;
    let state = sqlx::query_scalar!(
        "SELECT state FROM recording_sessions WHERE guild_id=$1 AND id=$2 FOR UPDATE",
        guild_id,
        session_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some(state) = state else {
        let previous = sqlx::query_scalar!(
            "SELECT id FROM recording_deletion_jobs WHERE guild_id=$1 AND recording_session_id=$2 AND state='ready'",
            guild_id,
            session_id
        )
        .fetch_optional(&mut *tx)
        .await?;
        return previous.ok_or(AppError::FileNotFound);
    };
    if state != "finalized" {
        return Err(AppError::Conflict(
            "Only finalized recordings can be deleted".into(),
        ));
    }
    let existing = sqlx::query!(
        "SELECT id,mode,state FROM recording_deletion_jobs WHERE recording_session_id=$1 FOR UPDATE",
        session_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    let id = if let Some(existing) = existing {
        // Merely repeating the default soft request never advances a job to
        // irreversible deletion, even if a feature flag later changes.
        if mode == DeletionMode::Permanent
            && (existing.mode == "soft" || matches!(existing.state.as_str(), "failed" | "paused"))
        {
            sqlx::query!(
                "UPDATE recording_deletion_jobs SET mode='permanent',state='queued',stage='queued',retry_at=now(),attempts=0,error=NULL,error_kind=NULL,finished_at=NULL,permanent_requested_by=COALESCE(permanent_requested_by,$2),permanent_requested_at=CASE WHEN permanent_requested_by IS NULL AND $2::bigint IS NOT NULL THEN now() ELSE COALESCE(permanent_requested_at,now()) END,updated_at=now() WHERE id=$1",
                existing.id,
                actor
            )
            .execute(&mut *tx)
            .await?;
        }
        existing.id
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        let fragments = session_fragments(&mut *tx, session_id).await?;
        let clips = related_clips(&mut tx, guild_id, session_id, &fragments).await?;
        sqlx::query!(
            "UPDATE recording_sessions SET deletion_requested_at=now() WHERE id=$1",
            session_id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE clips SET deleted_at=COALESCE(deleted_at,now()) WHERE guild_id=$1 AND clip_id=ANY($2)",
            guild_id,
            &clip_ids(&clips) as &[String]
        )
        .execute(&mut *tx)
        .await?;
        match mode {
            DeletionMode::Soft => {
                sqlx::query!(
                    "INSERT INTO recording_deletion_jobs (id,recording_session_id,guild_id,requested_by,reason,mode,state,stage,finished_at) VALUES ($1,$2,$3,$4,$5,'soft','soft_deleted','soft_deleted',now())",
                    id,
                    session_id,
                    guild_id,
                    actor,
                    reason
                )
                .execute(&mut *tx)
                .await?;
            }
            DeletionMode::Permanent => {
                sqlx::query!(
                    "INSERT INTO recording_deletion_jobs (id,recording_session_id,guild_id,requested_by,reason,mode,permanent_requested_by,permanent_requested_at) VALUES ($1,$2,$3,$4,$5,$6,$4,now())",
                    id,
                    session_id,
                    guild_id,
                    actor,
                    reason,
                    mode.as_str()
                )
                .execute(&mut *tx)
                .await?;
            }
        }
        id
    };
    tx.commit().await?;
    Ok(id)
}

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

async fn enqueue_expired(pool: &Pool<Postgres>) -> Result<(), AppError> {
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

struct Claimed {
    id: String,
    session_id: i64,
    guild_id: i64,
    token: String,
}

async fn claim(pool: &Pool<Postgres>) -> Result<Option<Claimed>, AppError> {
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
enum AttemptError {
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
    fn failed(kind: ErrorKind, detail: impl Into<String>) -> Self {
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

async fn run_one(
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
async fn record_outcome(
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

async fn stage(pool: &Pool<Postgres>, job: &Claimed, value: &str) -> Result<(), AttemptError> {
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

struct Fragment {
    id: i64,
    guild_id: i64,
    channel_id: i64,
    year: i32,
    month: i32,
    file_name: String,
}
struct Clip {
    id: String,
    saved: Option<String>,
}
struct MediaJob {
    id: String,
}
struct CompositionJob {
    id: String,
}

async fn process_with_roots(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    job: &Claimed,
    roots: &DataRoots,
) -> Result<(), AttemptError> {
    // Do not race a renderer already admitted before the tombstone. Queued
    // attempts recheck source access and will fail once they start.
    if active_guild_media_jobs(pool, job.guild_id).await? > 0 {
        return Err(AttemptError::Waiting);
    }
    let session = sqlx::query!(
        "SELECT starting_channel_id,started_at,ended_at,deletion_requested_at FROM recording_sessions WHERE id=$1 AND guild_id=$2",
        job.session_id,
        job.guild_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::FileNotFound)?;
    if session.deletion_requested_at.is_none() {
        return Err(AttemptError::failed(
            ErrorKind::InternalError,
            "recording is not tombstoned",
        ));
    }
    let (started, ended, starting_channel) = (
        session.started_at,
        session.ended_at,
        session.starting_channel_id,
    );
    let fragments = session_fragments(pool, job.session_id).await?;
    let clips = {
        let mut connection = pool.acquire().await?;
        related_clips(&mut connection, job.guild_id, job.session_id, &fragments).await?
    };
    for clip in &clips {
        if clip.id.is_empty()
            || !clip
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(AttemptError::failed(
                ErrorKind::UnsafeMediaReference,
                format!("clip {:?} has an unsafe archive identifier", clip.id),
            ));
        }
    }
    let media_jobs = related_media_jobs(pool, job).await?;
    let composition_jobs = related_composition_jobs(pool, job, &clips).await?;
    stage(pool, job, "purging_archive").await?;
    // Archive uploads hold this advisory lock for their entire transfer. A
    // claimed upload that starts afterwards sees the tombstone and skips.
    for fragment in &fragments {
        let object_id = sqlx::query_scalar!(
            "SELECT id FROM media_objects WHERE audio_file_id=$1",
            fragment.id
        )
        .fetch_optional(pool)
        .await?;
        purge_source(
            pool,
            media,
            object_id,
            &format!("media/v1/recordings/{}/", fragment.id),
            job,
        )
        .await?;
    }
    for clip in &clips {
        let object_id =
            sqlx::query_scalar!("SELECT id FROM media_objects WHERE clip_id=$1", clip.id)
                .fetch_optional(pool)
                .await?;
        if let Some(object_id) = object_id {
            purge_source(
                pool,
                media,
                Some(object_id),
                &format!("media/v1/clips/{}/", clip.id),
                job,
            )
            .await?;
        } else if let Some(archive) = media.archive() {
            // Reconciliation may not yet have tracked a short-lived clip.
            archive
                .purge_versions(&format!("media/v1/clips/{}/", clip.id))
                .await
                .map_err(|error| AppError::MediaArchiveUnavailable(error.to_string()))?;
        }
    }
    stage(pool, job, "purging_local").await?;
    for fragment in &fragments {
        purge_fragment(roots, fragment).await?;
    }
    purge_session_cache(
        roots,
        job.guild_id,
        job.session_id,
        starting_channel,
        started,
        ended,
        &fragments,
    )
    .await?;
    for clip in &clips {
        if let Some(saved) = &clip.saved {
            remove_file_if_exists(&crate::media_archive::clip_local_path_in(
                &roots.clips,
                saved,
            )?)
            .await?;
        }
        purge_clip_waveforms(&roots.waveforms, &clip.id).await?;
    }
    for media_job in &media_jobs {
        purge_job_outputs(&roots.recordings.join(".media-jobs"), &media_job.id).await?;
    }
    for composition_job in &composition_jobs {
        uuid::Uuid::parse_str(&composition_job.id).map_err(|_| {
            AttemptError::failed(
                ErrorKind::UnsafeMediaReference,
                format!(
                    "composition job {:?} has an invalid identifier",
                    composition_job.id
                ),
            )
        })?;
        remove_dir_if_exists(
            &roots
                .clips
                .join(".composition-jobs")
                .join(&composition_job.id),
        )
        .await?;
        purge_job_outputs(&roots.clips.join("compositions"), &composition_job.id).await?;
    }
    stage(pool, job, "removing_records").await?;
    let mut tx = pool.begin().await?;
    let owned = sqlx::query_scalar!(
        "SELECT id FROM recording_deletion_jobs WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at>now() FOR UPDATE",
        job.id,
        job.token
    )
    .fetch_optional(&mut *tx)
    .await?;
    if owned.is_none() {
        return Err(AttemptError::LeaseLost);
    }
    // A new media request may have entered after the initial check. Leave the
    // tombstone in place and retry rather than publish/erase across it.
    if active_guild_media_jobs(&mut *tx, job.guild_id).await? > 0 {
        return Err(AttemptError::Waiting);
    }
    let clip_ids = clip_ids(&clips);
    sqlx::query!(
        "DELETE FROM media_objects WHERE audio_file_id IN (SELECT id FROM audio_files WHERE recording_session_id=$1) OR clip_id=ANY($2)",
        job.session_id,
        &clip_ids
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM stamps WHERE recording_session_id=$1 OR audio_file_id IN (SELECT id FROM audio_files WHERE recording_session_id=$1)",
        job.session_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM clips WHERE clip_id=ANY($1)", &clip_ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query!(
        "DELETE FROM media_jobs WHERE id=ANY($1)",
        &media_jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>()
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM composition_jobs WHERE id=ANY($1)",
        &composition_jobs
            .iter()
            .map(|j| j.id.clone())
            .collect::<Vec<_>>()
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM clip_source_history WHERE source_clip_id=ANY($1) OR target_clip_id=ANY($1)",
        &clip_ids
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM audio_files WHERE recording_session_id=$1",
        job.session_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM recording_sessions WHERE id=$1 AND deletion_requested_at IS NOT NULL",
        job.session_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE recording_deletion_jobs SET state='ready',stage='ready',attempt_token=NULL,lease_expires_at=NULL,error=NULL,error_kind=NULL,finished_at=now(),updated_at=now() WHERE id=$1 AND attempt_token=$2",
        job.id,
        job.token
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn purge_source(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    object_id: Option<i64>,
    prefix: &str,
    job: &Claimed,
) -> Result<(), AttemptError> {
    let mut lock = pool.begin().await?;
    if let Some(object_id) = object_id {
        sqlx::query!(
            "SELECT pg_advisory_xact_lock($1,$2)",
            ARCHIVE_LOCK_NAMESPACE,
            object_id as i32
        )
        .execute(&mut *lock)
        .await?;
    }
    stage(pool, job, "purging_archive").await?;
    if let Some(archive) = media.archive() {
        archive
            .purge_versions(prefix)
            .await
            .map_err(|error| AppError::MediaArchiveUnavailable(error.to_string()))?;
    }
    lock.commit().await?;
    Ok(())
}

/// The session's physical fragments, in upload order.
async fn session_fragments(
    executor: impl sqlx::PgExecutor<'_>,
    session_id: i64,
) -> Result<Vec<Fragment>, sqlx::Error> {
    sqlx::query_as!(
        Fragment,
        "SELECT id,guild_id,channel_id,year,month,file_name FROM audio_files WHERE recording_session_id=$1 ORDER BY id",
        session_id
    )
    .fetch_all(executor)
    .await
}

/// Queued or running media and composition jobs in the guild.
async fn active_guild_media_jobs(
    executor: impl sqlx::PgExecutor<'_>,
    guild_id: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT (SELECT count(*) FROM media_jobs WHERE guild_id=$1 AND state IN ('queued','running')) + (SELECT count(*) FROM composition_jobs WHERE guild_id=$1 AND state IN ('queued','running')) AS "active!""#,
        guild_id
    )
    .fetch_one(executor)
    .await
}

fn clip_ids(clips: &[Clip]) -> Vec<String> {
    clips.iter().map(|clip| clip.id.clone()).collect()
}

/// A guild clip as the dependency scan needs it.
struct GuildClip {
    clip_id: String,
    saved_file_name: Option<String>,
    recording_session_id: Option<i64>,
    original_file_name: Option<String>,
    channel_id: Option<i64>,
    composition: Option<serde_json::Value>,
}

async fn related_clips(
    connection: &mut sqlx::PgConnection,
    guild_id: i64,
    session_id: i64,
    fragments: &[Fragment],
) -> Result<Vec<Clip>, AppError> {
    let rows = sqlx::query_as!(
        GuildClip,
        "SELECT clip_id,saved_file_name,recording_session_id,original_file_name,channel_id,composition FROM clips WHERE guild_id=$1",
        guild_id
    )
    .fetch_all(&mut *connection)
    .await?;
    let mut selected = HashSet::new();
    let mut clips = Vec::new();
    // A composition can depend on any session clip or physical fragment clip.
    // Composed clips cannot themselves be sources, so one pass suffices.
    for row in &rows {
        if row.recording_session_id == Some(session_id)
            || fragments.iter().any(|f| {
                Some(f.channel_id) == row.channel_id
                    && row.original_file_name.as_deref() == Some(f.file_name.as_str())
            })
        {
            selected.insert(row.clip_id.clone());
            clips.push(Clip {
                id: row.clip_id.clone(),
                saved: row.saved_file_name.clone(),
            });
        }
    }
    for row in &rows {
        if selected.contains(&row.clip_id) {
            continue;
        }
        let dependent = row
            .composition
            .as_ref()
            .and_then(|c| c.get("segments"))
            .and_then(|s| s.as_array())
            .is_some_and(|segments| {
                segments.iter().any(|segment| {
                    segment
                        .get("source_id")
                        .and_then(|id| id.as_str())
                        .is_some_and(|id| selected.contains(id))
                })
            });
        if dependent {
            selected.insert(row.clip_id.clone());
            clips.push(Clip {
                id: row.clip_id.clone(),
                saved: row.saved_file_name.clone(),
            });
        }
    }
    // This ledger survives composition-job cleanup and retains dependencies
    // even after a clip was overwritten with an unrelated composition.
    let sources: Vec<String> = selected.iter().cloned().collect();
    let historical_targets = sqlx::query_scalar!(
        r#"SELECT DISTINCT target_clip_id AS "target_clip_id!" FROM clip_source_history WHERE source_clip_id=ANY($1)"#,
        &sources
    )
    .fetch_all(&mut *connection)
    .await?;
    for target in historical_targets {
        if selected.contains(&target) {
            continue;
        }
        if let Some(row) = rows.iter().find(|row| row.clip_id == target) {
            selected.insert(target.clone());
            clips.push(Clip {
                id: target,
                saved: row.saved_file_name.clone(),
            });
        }
    }
    Ok(clips)
}

async fn related_media_jobs(
    pool: &Pool<Postgres>,
    job: &Claimed,
) -> Result<Vec<MediaJob>, AppError> {
    Ok(sqlx::query_scalar!(
        "SELECT id FROM media_jobs WHERE guild_id=$1 AND request->>'session_id'=$2",
        job.guild_id,
        job.session_id.to_string()
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|id| MediaJob { id })
    .collect())
}

async fn related_composition_jobs(
    pool: &Pool<Postgres>,
    job: &Claimed,
    clips: &[Clip],
) -> Result<Vec<CompositionJob>, AppError> {
    let clip_ids: HashSet<&str> = clips.iter().map(|clip| clip.id.as_str()).collect();
    let rows = sqlx::query!(
        "SELECT id,result_clip_id,snapshot FROM composition_jobs WHERE guild_id=$1",
        job.guild_id
    )
    .fetch_all(pool)
    .await?;
    let mut jobs = Vec::new();
    for row in rows {
        let dependent = row
            .snapshot
            .get("body")
            .and_then(|body| body.get("segments"))
            .and_then(|segments| segments.as_array())
            .is_some_and(|segments| {
                segments.iter().any(|segment| {
                    segment
                        .get("source_id")
                        .and_then(|id| id.as_str())
                        .is_some_and(|id| clip_ids.contains(id))
                })
            });
        if clip_ids.contains(row.result_clip_id.as_str()) || dependent {
            jobs.push(CompositionJob { id: row.id });
        }
    }
    Ok(jobs)
}

async fn purge_job_outputs(root: &Path, id: &str) -> Result<(), AppError> {
    let mut entries = match tokio::fs::read_dir(root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let prefix = format!("{id}-");
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".ogg") {
            remove_file_if_exists(&entry.path()).await?;
        }
    }
    Ok(())
}

async fn remove_file_if_exists(path: &Path) -> Result<(), AppError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn remove_dir_if_exists(path: &Path) -> Result<(), AppError> {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn purge_fragment(roots: &DataRoots, fragment: &Fragment) -> Result<(), AttemptError> {
    if !matches!(
        Path::new(&fragment.file_name)
            .components()
            .collect::<Vec<_>>()
            .as_slice(),
        [std::path::Component::Normal(_)]
    ) {
        return Err(AttemptError::failed(
            ErrorKind::UnsafeMediaReference,
            format!("recording fragment {} has an unsafe file name", fragment.id),
        ));
    }
    let key = RecordingKey::new(
        fragment.guild_id,
        fragment.channel_id,
        fragment.year,
        fragment.month as u32,
        fragment.file_name.clone(),
    );
    remove_file_if_exists(&key.recording_path(&roots.recordings_str())).await?;
    let no_silence = key.no_silence_path(&roots.no_silence_str());
    remove_file_if_exists(&no_silence).await?;
    if let Some(parent) = no_silence.parent() {
        purge_cached_variants(
            parent,
            &format!("{}{}.", sakiot_paths::NO_SILENCE_PREFIX, fragment.file_name),
            &[".tmp.ogg"],
        )
        .await?;
    }
    remove_file_if_exists(&key.waveform_path(&roots.waveforms_str())).await?;
    remove_file_if_exists(&roots.waveforms.join(format!(
        "{}{}.dat",
        sakiot_paths::NO_SILENCE_PREFIX,
        fragment.file_name
    )))
    .await?;
    for prefix in [
        format!("{}.", fragment.file_name),
        format!("{}{}.", sakiot_paths::NO_SILENCE_PREFIX, fragment.file_name),
    ] {
        purge_cached_variants(&roots.waveforms, &prefix, &[".tmp.dat"]).await?;
    }
    remove_dir_if_exists(&key.live_dir(&roots.recordings_str())).await?;
    Ok(())
}

async fn purge_session_cache(
    roots: &DataRoots,
    guild_id: i64,
    id: i64,
    channel: i64,
    started: DateTime<Utc>,
    ended: Option<DateTime<Utc>>,
    fragments: &[Fragment],
) -> Result<(), AppError> {
    let started_ms = started.timestamp_millis();
    if let Some(ended) = ended {
        let session_dir = roots.no_silence.join("logical_sessions");
        remove_file_if_exists(&session_dir.join(format!(
            "{id}-{started_ms}-{}.ogg",
            ended.timestamp_millis()
        )))
        .await?;
        purge_cached_variants(
            &session_dir,
            &format!("{id}-{started_ms}-{}.", ended.timestamp_millis()),
            &[".tmp.ogg"],
        )
        .await?;
    }
    for suffix in ["", "-silence-free"] {
        remove_file_if_exists(
            &roots
                .waveforms
                .join(format!("logical-session-{id}{suffix}.dat")),
        )
        .await?;
        purge_cached_variants(
            &roots.waveforms,
            &format!("logical-session-{id}{suffix}."),
            &[".tmp.dat"],
        )
        .await?;
    }
    purge_cached_variants(
        &roots.waveforms,
        &format!("logical-session-{id}-"),
        &[".ogg"],
    )
    .await?;
    let mut directories = HashSet::new();
    directories.insert((channel, started.year(), started.month()));
    for f in fragments {
        directories.insert((f.channel_id, f.year, f.month as u32));
    }
    for (ch, year, month) in directories {
        let key = SessionKey::new(guild_id, ch, year, month, started_ms);
        // The all-recordings mix is a cache and can contain these bytes too.
        remove_dir_if_exists(&key.mix_dir(&roots.recordings_str())).await?;
    }
    Ok(())
}

async fn purge_clip_waveforms(root: &Path, clip_id: &str) -> Result<(), AppError> {
    purge_cached_variants(root, &format!("clip-{clip_id}-"), &[".dat"]).await
}

async fn purge_cached_variants(
    root: &Path,
    prefix: &str,
    endings: &[&str],
) -> Result<(), AppError> {
    let mut entries = match tokio::fs::read_dir(root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(prefix) && endings.iter().any(|ending| name.ends_with(ending)) {
            remove_file_if_exists(&entry.path()).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    async fn seed_session(pool: &PgPool, ended: &str) -> Result<(i64, i64), sqlx::Error> {
        let session_id: i64 = sqlx::query_scalar("INSERT INTO recording_sessions (guild_id,user_id,starting_channel_id,state,started_at,ended_at) VALUES (1,100,10,'finalized',now()-interval '40 days',now()-interval '39 days') RETURNING id")
            .fetch_one(pool).await?;
        let audio_id: i64 = sqlx::query_scalar("INSERT INTO audio_files (file_name,guild_id,channel_id,user_id,year,month,start_ts,end_ts,recording_session_id,segment_index) VALUES ($1,1,10,100,2026,9,1000,2000,$2,0) RETURNING id")
            .bind(ended).bind(session_id).fetch_one(pool).await?;
        Ok((session_id, audio_id))
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn deletion_tombstones_then_removes_sources_derivatives_and_audit_survives(
        pool: PgPool,
    ) -> TestResult {
        let temp = tempfile::tempdir()?;
        let roots = DataRoots::new(temp.path());
        let (session_id, audio_id) = seed_session(&pool, "lifecycle-recording").await?;
        let (_, unrelated_id) = seed_session(&pool, "unrelated-recording").await?;
        sqlx::query("INSERT INTO clips (clip_id,guild_id,channel_id,user_id,start_time,original_file_name,saved_file_name,recording_session_id) VALUES ('lifecycle-source',1,10,100,0,'session:1','2026/09/lifecycle-source.ogg',$1)")
            .bind(session_id).execute(&pool).await?;
        sqlx::query("INSERT INTO clips (clip_id,guild_id,channel_id,user_id,start_time,original_file_name,saved_file_name,composition) VALUES ('lifecycle-compose',1,10,100,0,'compose','compositions/lifecycle-compose.ogg',$1)")
            .bind(serde_json::json!({"segments":[{"source_id":"lifecycle-source"}]})).execute(&pool).await?;
        sqlx::query("INSERT INTO media_objects (audio_file_id) VALUES ($1),($2)")
            .bind(audio_id)
            .bind(unrelated_id)
            .execute(&pool)
            .await?;
        sqlx::query(
            "INSERT INTO media_objects (clip_id) VALUES ('lifecycle-source'),('lifecycle-compose')",
        )
        .execute(&pool)
        .await?;
        sqlx::query("INSERT INTO stamps (guild_id,channel_id,target_user_id,stamper_user_id,stamp_ts,audio_file_id,recording_session_id) VALUES (1,10,100,200,1500,$1,$2)")
            .bind(audio_id).bind(session_id).execute(&pool).await?;
        let media_id = uuid::Uuid::new_v4().to_string();
        let download = roots
            .recordings
            .join(".media-jobs")
            .join(format!("{media_id}-attempt.ogg"));
        sqlx::query("INSERT INTO media_jobs (id,kind,guild_id,user_id,idempotency_key,resource_key,request,state,result_path) VALUES ($1,'session_download',1,100,$1,$1,$2,'ready',$3)")
            .bind(&media_id).bind(serde_json::json!({"kind":"session_download","session_id":session_id})).bind(download.to_string_lossy().as_ref()).execute(&pool).await?;
        let composition_id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO composition_jobs (id,guild_id,user_id,idempotency_key,request,snapshot,result_clip_id,state) VALUES ($1,1,100,$1,'{}',$2,'lifecycle-compose','ready')")
            .bind(&composition_id).bind(serde_json::json!({"body":{"segments":[{"source_id":"lifecycle-source"}]}})).execute(&pool).await?;
        // The current editor state no longer references this recording, but
        // its previous archived revisions still do.
        sqlx::query("UPDATE clips SET composition=$1 WHERE clip_id='lifecycle-compose'")
            .bind(serde_json::json!({"segments":[{"source_id":"other-source"}]}))
            .execute(&pool)
            .await?;

        let key = RecordingKey::new(1, 10, 2026, 9, "lifecycle-recording");
        let original = key.recording_path(&roots.recordings_str());
        let no_silence = key.no_silence_path(&roots.no_silence_str());
        let waveform = key.waveform_path(&roots.waveforms_str());
        let silence_attempt = no_silence.with_extension("job.token.tmp.ogg");
        let waveform_attempt = roots
            .waveforms
            .join("lifecycle-recording.job.token.tmp.dat");
        let session_waveform_attempt = roots
            .waveforms
            .join(format!("logical-session-{session_id}.job.token.tmp.dat"));
        let session_composite_attempt = roots
            .waveforms
            .join(format!("logical-session-{session_id}-token.ogg"));
        let clip = roots.clips.join("2026/09/lifecycle-source.ogg");
        let composed = roots.clips.join("compositions/lifecycle-compose.ogg");
        let render = roots
            .clips
            .join("compositions")
            .join(format!("{composition_id}-old.ogg"));
        let paths = [
            &original,
            &no_silence,
            &waveform,
            &silence_attempt,
            &waveform_attempt,
            &session_waveform_attempt,
            &session_composite_attempt,
            &clip,
            &composed,
            &download,
            &render,
        ];
        for path in paths {
            tokio::fs::create_dir_all(path.parent().unwrap()).await?;
            tokio::fs::write(path, b"test-media").await?;
        }
        let job_id = enqueue(
            &pool,
            1,
            session_id,
            Some(200),
            "manager",
            DeletionMode::Soft,
        )
        .await?;
        assert_eq!(
            job_id,
            enqueue(
                &pool,
                1,
                session_id,
                Some(200),
                "manager",
                DeletionMode::Soft
            )
            .await?
        );
        let hidden: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM clips WHERE guild_id=1 AND deleted_at IS NOT NULL",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(hidden, 2);
        let soft_status = load_status(&pool, 1, &job_id).await?;
        assert_eq!(soft_status.state, "soft_deleted");
        assert_eq!(soft_status.mode, "soft");
        assert!(claim(&pool).await?.is_none());
        for path in paths {
            assert!(path.exists(), "soft deletion removed {}", path.display());
        }
        let retained_files: i64 =
            sqlx::query_scalar("SELECT count(*) FROM audio_files WHERE id=$1")
                .bind(audio_id)
                .fetch_one(&pool)
                .await?;
        assert_eq!(retained_files, 1);
        let retained_archive_rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM media_objects WHERE audio_file_id=$1 OR clip_id IN ('lifecycle-source','lifecycle-compose')",
        )
        .bind(audio_id)
        .fetch_one(&pool)
        .await?;
        assert_eq!(retained_archive_rows, 3);
        assert_eq!(
            job_id,
            enqueue(
                &pool,
                1,
                session_id,
                Some(200),
                "manager",
                DeletionMode::Permanent
            )
            .await?
        );
        let claim = claim(&pool).await?.unwrap();
        assert_eq!(claim.id, job_id);
        process_with_roots(&pool, &MediaArchive::disabled(), &claim, &roots).await?;
        for path in paths {
            assert!(!path.exists(), "{} survived", path.display());
        }
        let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM audio_files WHERE id=$1")
            .bind(audio_id)
            .fetch_one(&pool)
            .await?;
        assert_eq!(remaining, 0);
        let unrelated: i64 = sqlx::query_scalar("SELECT count(*) FROM audio_files WHERE id=$1")
            .bind(unrelated_id)
            .fetch_one(&pool)
            .await?;
        assert_eq!(unrelated, 1);
        let remaining_clips: i64 =
            sqlx::query_scalar("SELECT count(*) FROM clips WHERE guild_id=1")
                .fetch_one(&pool)
                .await?;
        assert_eq!(remaining_clips, 0);
        let stamps: i64 =
            sqlx::query_scalar("SELECT count(*) FROM stamps WHERE recording_session_id=$1")
                .bind(session_id)
                .fetch_one(&pool)
                .await?;
        assert_eq!(stamps, 0);
        let status = load_status(&pool, 1, &job_id).await?;
        assert_eq!(status.state, "ready");
        assert_eq!(status.mode, "permanent");
        assert_eq!(status.recording_session_id, session_id.to_string());
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn retention_and_fencing_prevent_late_publication(pool: PgPool) -> TestResult {
        let (session_id, _) = seed_session(&pool, "retention-recording").await?;
        sqlx::query("INSERT INTO guild_recording_policy (guild_id,retention_days) VALUES (1,30)")
            .execute(&pool)
            .await?;
        enqueue_expired(&pool).await?;
        let retention: (String, String) = sqlx::query_as(
            "SELECT mode,state FROM recording_deletion_jobs WHERE recording_session_id=$1",
        )
        .bind(session_id)
        .fetch_one(&pool)
        .await?;
        assert_eq!(retention, ("soft".into(), "soft_deleted".into()));
        assert!(claim(&pool).await?.is_none());
        enqueue(
            &pool,
            1,
            session_id,
            Some(200),
            "manager",
            DeletionMode::Permanent,
        )
        .await?;
        let authorization: (String, Option<i64>, Option<i64>, bool) = sqlx::query_as(
            "SELECT reason,requested_by,permanent_requested_by,permanent_requested_at IS NOT NULL FROM recording_deletion_jobs WHERE recording_session_id=$1",
        )
        .bind(session_id)
        .fetch_one(&pool)
        .await?;
        assert_eq!(authorization, ("retention".into(), None, Some(200), true));
        let first = claim(&pool).await?.unwrap();
        assert_eq!(first.session_id, session_id);
        sqlx::query("UPDATE recording_deletion_jobs SET lease_expires_at=now()-interval '1 second' WHERE id=$1")
            .bind(&first.id).execute(&pool).await?;
        let second = claim(&pool).await?.unwrap();
        assert_ne!(first.token, second.token);
        assert!(stage(&pool, &first, "stale").await.is_err());
        let temp = tempfile::tempdir()?;
        process_with_roots(
            &pool,
            &MediaArchive::disabled(),
            &second,
            &DataRoots::new(temp.path()),
        )
        .await?;
        assert_eq!(load_status(&pool, 1, &second.id).await?.state, "ready");
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn tombstone_rejects_late_clip_publication(pool: PgPool) -> TestResult {
        let (session_id, _) = seed_session(&pool, "late-clip-recording").await?;
        enqueue(
            &pool,
            1,
            session_id,
            Some(200),
            "manager",
            DeletionMode::Soft,
        )
        .await?;
        let insert = sqlx::query("INSERT INTO clips (clip_id,guild_id,channel_id,user_id,start_time,original_file_name,recording_session_id) VALUES ('late-clip',1,10,100,0,'session',$1)")
            .bind(session_id).execute(&pool).await;
        assert!(insert.is_err());
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn database_rejects_a_soft_job_in_the_purge_queue(pool: PgPool) -> TestResult {
        let unsafe_job = sqlx::query("INSERT INTO recording_deletion_jobs (id,recording_session_id,guild_id,reason) VALUES ('unsafe-soft-job',1,1,'manager')")
            .execute(&pool).await;
        assert!(unsafe_job.is_err());
        assert!(claim(&pool).await?.is_none());
        Ok(())
    }

    async fn permanent_job(pool: &PgPool, file_name: &str) -> Result<String, AppError> {
        let (session_id, _) = seed_session(pool, file_name).await?;
        enqueue(
            pool,
            1,
            session_id,
            Some(200),
            "manager",
            DeletionMode::Permanent,
        )
        .await
    }

    async fn make_due(pool: &PgPool, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE recording_deletion_jobs SET retry_at=now() WHERE id=$1")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn waiting_for_media_work_never_spends_the_failure_budget(pool: PgPool) -> TestResult {
        let temp = tempfile::tempdir()?;
        let roots = DataRoots::new(temp.path());
        let job_id = permanent_job(&pool, "waiting-recording").await?;
        sqlx::query("INSERT INTO media_jobs (id,kind,guild_id,user_id,idempotency_key,resource_key,request) VALUES ('busy','session_download',1,100,'busy','busy','{}')")
            .execute(&pool)
            .await?;
        for _ in 0..MAX_ATTEMPTS + 3 {
            make_due(&pool, &job_id).await?;
            run_one(&pool, &MediaArchive::disabled(), &roots).await?;
        }
        let waiting = load_status(&pool, 1, &job_id).await?;
        assert_eq!(
            (
                waiting.state.as_str(),
                waiting.stage.as_str(),
                waiting.attempts
            ),
            ("queued", "waiting", 0)
        );
        assert_eq!(waiting.error_kind, Some(ErrorKind::WaitingForMediaWork));
        assert_eq!(
            waiting.error.as_deref(),
            Some(ErrorKind::WaitingForMediaWork.default_message())
        );

        // Once the guild's media work drains, the same job completes.
        sqlx::query("UPDATE media_jobs SET state='ready' WHERE id='busy'")
            .execute(&pool)
            .await?;
        make_due(&pool, &job_id).await?;
        run_one(&pool, &MediaArchive::disabled(), &roots).await?;
        let done = load_status(&pool, 1, &job_id).await?;
        assert_eq!((done.state.as_str(), done.attempts), ("ready", 1));
        assert_eq!((done.error, done.error_kind), (None, None));
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn real_failures_spend_attempts_until_the_limit(pool: PgPool) -> TestResult {
        let temp = tempfile::tempdir()?;
        let roots = DataRoots::new(temp.path());
        let job_id = permanent_job(&pool, "failing-recording").await?;
        // A stored name that would escape the recordings root must stop the
        // purge, however often it is retried.
        sqlx::query(
            "UPDATE audio_files SET file_name='../escape' WHERE file_name='failing-recording'",
        )
        .execute(&pool)
        .await?;
        for attempt in 1..=MAX_ATTEMPTS {
            make_due(&pool, &job_id).await?;
            run_one(&pool, &MediaArchive::disabled(), &roots).await?;
            let status = load_status(&pool, 1, &job_id).await?;
            assert_eq!(status.attempts, attempt);
            let expected = if attempt < MAX_ATTEMPTS {
                ("queued", "retry")
            } else {
                ("failed", "failed")
            };
            assert_eq!((status.state.as_str(), status.stage.as_str()), expected);
            assert_eq!(status.error_kind, Some(ErrorKind::UnsafeMediaReference));
            let message = status.error.unwrap_or_default();
            assert_eq!(message, ErrorKind::UnsafeMediaReference.default_message());
            assert!(!message.contains("escape"));
            assert!(!message.to_lowercase().contains("retry"));
        }
        make_due(&pool, &job_id).await?;
        assert!(claim(&pool).await?.is_none());
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn a_stale_attempt_cannot_touch_a_reclaimed_job(pool: PgPool) -> TestResult {
        let temp = tempfile::tempdir()?;
        let roots = DataRoots::new(temp.path());
        let job_id = permanent_job(&pool, "reclaimed-recording").await?;
        let recording = RecordingKey::new(1, 10, 2026, 9, "reclaimed-recording")
            .recording_path(&roots.recordings_str());
        tokio::fs::create_dir_all(recording.parent().unwrap()).await?;
        tokio::fs::write(&recording, b"test-media").await?;

        let stale = claim(&pool).await?.unwrap();
        sqlx::query("UPDATE recording_deletion_jobs SET lease_expires_at=now()-interval '1 second' WHERE id=$1")
            .bind(&job_id)
            .execute(&pool)
            .await?;
        let current = claim(&pool).await?.unwrap();
        stage(&pool, &current, "purging_archive").await?;
        let snapshot = || {
            sqlx::query_as::<_, (String, String, i32, Option<String>, Option<String>, Option<String>)>(
                "SELECT state,stage,attempts,attempt_token,error,error_kind FROM recording_deletion_jobs WHERE id=$1",
            )
            .bind(&job_id)
            .fetch_one(&pool)
        };
        let before = snapshot().await?;

        assert!(matches!(
            stage(&pool, &stale, "late").await,
            Err(AttemptError::LeaseLost)
        ));
        assert!(matches!(
            process_with_roots(&pool, &MediaArchive::disabled(), &stale, &roots).await,
            Err(AttemptError::LeaseLost)
        ));
        assert!(recording.exists(), "a stale attempt purged media");
        for outcome in [
            Err(AttemptError::Waiting),
            Err(AttemptError::failed(
                ErrorKind::InternalError,
                "late failure",
            )),
            Err(AttemptError::LeaseLost),
            Ok(()),
        ] {
            record_outcome(&pool, &stale, outcome).await?;
        }
        assert_eq!(snapshot().await?, before);

        process_with_roots(&pool, &MediaArchive::disabled(), &current, &roots).await?;
        assert_eq!(load_status(&pool, 1, &job_id).await?.state, "ready");
        assert!(!recording.exists());
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn legacy_deletion_errors_are_not_echoed(pool: PgPool) -> TestResult {
        let job_id = permanent_job(&pool, "legacy-recording").await?;
        sqlx::query("UPDATE recording_deletion_jobs SET error='IO Error: /srv/sakiot/data/recordings permission denied', error_kind=NULL WHERE id=$1")
            .bind(&job_id)
            .execute(&pool)
            .await?;
        let status = load_status(&pool, 1, &job_id).await?;
        let body = serde_json::to_string(&status)?;
        assert!(!body.contains("/srv"), "{body}");
        assert!(!body.contains("IO Error"), "{body}");
        assert_eq!(status.error_kind, None);
        assert_eq!(
            status.error.as_deref(),
            Some(crate::errors::UNCLASSIFIED_JOB_ERROR)
        );
        Ok(())
    }
}
