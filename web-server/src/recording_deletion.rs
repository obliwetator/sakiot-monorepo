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

mod files;
mod process;
mod worker;

pub use worker::spawn_worker;

use files::*;
use process::*;
use worker::*;

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

#[cfg(test)]
mod tests;
