//! Audited, retryable removal of finalized logical recordings and their media.
//! A tombstone hides the session before any potentially slow archive request.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use actix_web::{HttpRequest, HttpResponse, delete, get, web};
use chrono::{DateTime, Datelike, Utc};
use sakiot_paths::{DataRoots, RecordingKey, SessionKey};
use serde::Serialize;
use sqlx::{Pool, Postgres, Row};

use crate::errors::AppError;
use crate::media_archive::MediaArchive;
use crate::permissions::require_guild_manager;

const MAX_ATTEMPTS: i32 = 10;
pub(crate) const ARCHIVE_LOCK_NAMESPACE: i32 = 0x53414b41;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RecordingDeletionStatus {
    pub id: String,
    pub recording_session_id: String,
    pub status_url: String,
    pub state: String,
    pub stage: String,
    pub attempts: i32,
    pub error: Option<String>,
}

async fn load_status(
    pool: &Pool<Postgres>,
    guild_id: i64,
    id: &str,
) -> Result<RecordingDeletionStatus, AppError> {
    let row = sqlx::query("SELECT recording_session_id,state,stage,attempts,error FROM recording_deletion_jobs WHERE guild_id=$1 AND id=$2")
        .bind(guild_id).bind(id).fetch_optional(pool).await?.ok_or(AppError::FileNotFound)?;
    Ok(RecordingDeletionStatus {
        id: id.to_owned(),
        recording_session_id: row.try_get::<i64, _>("recording_session_id")?.to_string(),
        status_url: format!("/api/admin/guilds/{guild_id}/recording-deletions/{id}"),
        state: row.try_get("state")?,
        stage: row.try_get("stage")?,
        attempts: row.try_get("attempts")?,
        error: row.try_get("error")?,
    })
}

#[utoipa::path(
    delete,
    path = "/api/admin/guilds/{guild_id}/recordings/{recording_session_id}",
    tag = "admin",
    params(("guild_id" = i64, Path), ("recording_session_id" = i64, Path)),
    responses(
        (status = 202, description = "Recording hidden and deletion queued", body = RecordingDeletionStatus),
        (status = 403, description = "Manage Guild required", body = crate::errors::ApiError),
        (status = 404, description = "Recording not found", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[delete("/admin/guilds/{guild_id}/recordings/{recording_session_id}")]
pub async fn delete_recording(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, session_id) = path.into_inner();
    let user_id = require_guild_manager(&req, &pool, guild_id).await?;
    let id = enqueue(&pool, guild_id, session_id, Some(user_id), "manager").await?;
    Ok(HttpResponse::Accepted().json(load_status(&pool, guild_id, &id).await?))
}

#[utoipa::path(
    get,
    path = "/api/admin/guilds/{guild_id}/recording-deletions/{job_id}",
    tag = "admin",
    params(("guild_id" = i64, Path), ("job_id" = String, Path)),
    responses((status = 200, description = "Audited deletion status", body = RecordingDeletionStatus)),
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
) -> Result<String, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(guild_id)
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query("SELECT state,deletion_requested_at FROM recording_sessions WHERE guild_id=$1 AND id=$2 FOR UPDATE")
        .bind(guild_id).bind(session_id).fetch_optional(&mut *tx).await?;
    let Some(row) = row else {
        let previous: Option<String> = sqlx::query_scalar("SELECT id FROM recording_deletion_jobs WHERE guild_id=$1 AND recording_session_id=$2 AND state='ready'")
            .bind(guild_id).bind(session_id).fetch_optional(&mut *tx).await?;
        return previous.ok_or(AppError::FileNotFound);
    };
    let state: String = row.try_get("state")?;
    if state != "finalized" {
        return Err(AppError::Conflict(
            "Only finalized recordings can be deleted".into(),
        ));
    }
    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM recording_deletion_jobs WHERE recording_session_id=$1")
            .bind(session_id)
            .fetch_optional(&mut *tx)
            .await?;
    let id = if let Some(id) = existing {
        // A failed job may be explicitly resubmitted without losing its audit history.
        sqlx::query("UPDATE recording_deletion_jobs SET state='queued',stage='queued',retry_at=now(),attempts=0,error=NULL,updated_at=now() WHERE id=$1 AND state='failed'")
            .bind(&id).execute(&mut *tx).await?;
        id
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        let fragments: Vec<Fragment> = sqlx::query_as("SELECT id,guild_id,channel_id,year,month,file_name FROM audio_files WHERE recording_session_id=$1 ORDER BY id")
            .bind(session_id).fetch_all(&mut *tx).await?;
        let clips = related_clips(&mut tx, guild_id, session_id, &fragments).await?;
        sqlx::query("UPDATE recording_sessions SET deletion_requested_at=now() WHERE id=$1")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE clips SET deleted_at=COALESCE(deleted_at,now()) WHERE guild_id=$1 AND clip_id=ANY($2)")
            .bind(guild_id).bind(clips.iter().map(|clip| clip.id.as_str()).collect::<Vec<_>>())
            .execute(&mut *tx).await?;
        sqlx::query("INSERT INTO recording_deletion_jobs (id,recording_session_id,guild_id,requested_by,reason) VALUES ($1,$2,$3,$4,$5)")
            .bind(&id).bind(session_id).bind(guild_id).bind(actor).bind(reason).execute(&mut *tx).await?;
        id
    };
    tx.commit().await?;
    Ok(id)
}

pub fn spawn_worker(pool: Pool<Postgres>, media: MediaArchive) -> tokio::task::JoinHandle<()> {
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
            if let Err(error) = run_one(&pool, &media).await {
                tracing::error!(?error, "recording deletion worker failed");
            }
        }
    })
}

async fn enqueue_expired(pool: &Pool<Postgres>) -> Result<(), AppError> {
    // One small page per sweep. The queue is durable and subsequent passes
    // pick up the rest without a long transaction or an unbounded delete burst.
    let rows = sqlx::query("SELECT rs.guild_id,rs.id FROM recording_sessions rs JOIN guild_recording_policy p ON p.guild_id=rs.guild_id WHERE p.retention_days IS NOT NULL AND rs.state='finalized' AND rs.ended_at < now() - (p.retention_days * interval '1 day') AND rs.deletion_requested_at IS NULL ORDER BY rs.ended_at,rs.id LIMIT 25")
        .fetch_all(pool).await?;
    for row in rows {
        let guild_id: i64 = row.try_get("guild_id")?;
        let session_id: i64 = row.try_get("id")?;
        if let Err(error) = enqueue(pool, guild_id, session_id, None, "retention").await {
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
    let row = sqlx::query("WITH candidate AS (SELECT id FROM recording_deletion_jobs WHERE attempts < $1 AND ((state='queued' AND retry_at<=now()) OR (state='running' AND lease_expires_at<now())) ORDER BY retry_at,created_at FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE recording_deletion_jobs j SET state='running',stage='checking',attempts=attempts+1,attempt_token=$2,lease_expires_at=now()+interval '5 minutes',updated_at=now() FROM candidate WHERE j.id=candidate.id RETURNING j.id,j.recording_session_id,j.guild_id")
        .bind(MAX_ATTEMPTS).bind(&token).fetch_optional(pool).await?;
    row.map(|row| {
        Ok(Claimed {
            id: row.try_get("id")?,
            session_id: row.try_get("recording_session_id")?,
            guild_id: row.try_get("guild_id")?,
            token,
        })
    })
    .transpose()
}

async fn run_one(pool: &Pool<Postgres>, media: &MediaArchive) -> Result<(), AppError> {
    let Some(job) = claim(pool).await? else {
        return Ok(());
    };
    let result = {
        let mut work = Box::pin(process(pool, media, &job));
        let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
        loop {
            tokio::select! {
                result = &mut work => break result,
                _ = heartbeat.tick() => {
                    let renewed = sqlx::query("UPDATE recording_deletion_jobs SET lease_expires_at=now()+interval '5 minutes',updated_at=now() WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at>now()")
                        .bind(&job.id).bind(&job.token).execute(pool).await?.rows_affected();
                    if renewed != 1 { break Err(AppError::Conflict("Deletion lease lost".into())); }
                }
            }
        }
    };
    if let Err(error) = result {
        tracing::warn!(job_id=%job.id, ?error, "recording deletion attempt failed");
        let public_error = match &error {
            AppError::Conflict(message) => message.as_str(),
            _ => "Deletion could not finish; the worker will retry",
        };
        let waiting = matches!(&error, AppError::Conflict(message) if message.starts_with("Waiting for") || message.starts_with("Guild media work"));
        sqlx::query("UPDATE recording_deletion_jobs SET attempts=CASE WHEN $5 THEN attempts-1 ELSE attempts END,state=CASE WHEN attempts >= $3 AND NOT $5 THEN 'failed' ELSE 'queued' END,stage='retry',error=$4,retry_at=now()+interval '30 seconds',attempt_token=NULL,lease_expires_at=NULL,updated_at=now(),finished_at=CASE WHEN attempts >= $3 AND NOT $5 THEN now() ELSE NULL END WHERE id=$1 AND attempt_token=$2 AND state='running'")
            .bind(&job.id).bind(&job.token).bind(MAX_ATTEMPTS).bind(public_error).bind(waiting).execute(pool).await?;
    }
    Ok(())
}

async fn stage(pool: &Pool<Postgres>, job: &Claimed, value: &str) -> Result<(), AppError> {
    let changed = sqlx::query("UPDATE recording_deletion_jobs SET stage=$3,lease_expires_at=now()+interval '5 minutes',updated_at=now() WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at>now()")
        .bind(&job.id).bind(&job.token).bind(value).execute(pool).await?.rows_affected();
    if changed != 1 {
        return Err(AppError::Conflict("Deletion lease lost".into()));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct Fragment {
    id: i64,
    guild_id: i64,
    channel_id: i64,
    year: i32,
    month: i32,
    file_name: String,
}
#[derive(sqlx::FromRow)]
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

async fn process(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    job: &Claimed,
) -> Result<(), AppError> {
    process_with_roots(pool, media, job, &DataRoots::from_env()).await
}

async fn process_with_roots(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    job: &Claimed,
    roots: &DataRoots,
) -> Result<(), AppError> {
    // Do not race a renderer already admitted before the tombstone. Queued
    // attempts recheck source access and will fail once they start.
    let active: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM media_jobs WHERE guild_id=$1 AND state IN ('queued','running')) + (SELECT count(*) FROM composition_jobs WHERE guild_id=$1 AND state IN ('queued','running'))")
        .bind(job.guild_id).fetch_one(pool).await?;
    if active > 0 {
        return Err(AppError::Conflict(
            "Waiting for in-flight guild media jobs".into(),
        ));
    }
    let session = sqlx::query("SELECT guild_id,starting_channel_id,started_at,ended_at,deletion_requested_at FROM recording_sessions WHERE id=$1 AND guild_id=$2")
        .bind(job.session_id).bind(job.guild_id).fetch_optional(pool).await?.ok_or(AppError::FileNotFound)?;
    let tombstone: Option<DateTime<Utc>> = session.try_get("deletion_requested_at")?;
    if tombstone.is_none() {
        return Err(AppError::Conflict("Recording is not tombstoned".into()));
    }
    let started: DateTime<Utc> = session.try_get("started_at")?;
    let ended: Option<DateTime<Utc>> = session.try_get("ended_at")?;
    let starting_channel: i64 = session.try_get("starting_channel_id")?;
    let fragments: Vec<Fragment> = sqlx::query_as("SELECT id,guild_id,channel_id,year,month,file_name FROM audio_files WHERE recording_session_id=$1 ORDER BY id")
        .bind(job.session_id).fetch_all(pool).await?;
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
            return Err(AppError::Conflict(
                "A clip has an unsafe archive identifier".into(),
            ));
        }
    }
    let media_jobs = related_media_jobs(pool, job).await?;
    let composition_jobs = related_composition_jobs(pool, job, &clips).await?;
    stage(pool, job, "purging_archive").await?;
    // Archive uploads hold this advisory lock for their entire transfer. A
    // claimed upload that starts afterwards sees the tombstone and skips.
    for fragment in &fragments {
        let object_id: Option<i64> =
            sqlx::query_scalar("SELECT id FROM media_objects WHERE audio_file_id=$1")
                .bind(fragment.id)
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
        let object_id: Option<i64> =
            sqlx::query_scalar("SELECT id FROM media_objects WHERE clip_id=$1")
                .bind(&clip.id)
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
                .map_err(|e| AppError::ServiceUnavailable(e.to_string()))?;
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
            AppError::Conflict("A composition job has an invalid identifier".into())
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
    let owned: Option<String> = sqlx::query_scalar("SELECT id FROM recording_deletion_jobs WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at>now() FOR UPDATE")
        .bind(&job.id).bind(&job.token).fetch_optional(&mut *tx).await?;
    if owned.is_none() {
        return Err(AppError::Conflict("Deletion lease lost".into()));
    }
    // A new media request may have entered after the initial check. Leave the
    // tombstone in place and retry rather than publish/erase across it.
    let active: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM media_jobs WHERE guild_id=$1 AND state IN ('queued','running')) + (SELECT count(*) FROM composition_jobs WHERE guild_id=$1 AND state IN ('queued','running'))")
        .bind(job.guild_id).fetch_one(&mut *tx).await?;
    if active > 0 {
        return Err(AppError::Conflict(
            "Guild media work started during deletion".into(),
        ));
    }
    sqlx::query("DELETE FROM media_objects WHERE audio_file_id IN (SELECT id FROM audio_files WHERE recording_session_id=$1) OR clip_id=ANY($2)")
        .bind(job.session_id).bind(clips.iter().map(|c| c.id.as_str()).collect::<Vec<_>>()).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM stamps WHERE recording_session_id=$1 OR audio_file_id IN (SELECT id FROM audio_files WHERE recording_session_id=$1)")
        .bind(job.session_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM clips WHERE clip_id=ANY($1)")
        .bind(clips.iter().map(|c| c.id.as_str()).collect::<Vec<_>>())
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM media_jobs WHERE id=ANY($1)")
        .bind(media_jobs.iter().map(|j| j.id.as_str()).collect::<Vec<_>>())
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM composition_jobs WHERE id=ANY($1)")
        .bind(
            composition_jobs
                .iter()
                .map(|j| j.id.as_str())
                .collect::<Vec<_>>(),
        )
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM clip_source_history WHERE source_clip_id=ANY($1) OR target_clip_id=ANY($1)",
    )
    .bind(clips.iter().map(|c| c.id.as_str()).collect::<Vec<_>>())
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM audio_files WHERE recording_session_id=$1")
        .bind(job.session_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM recording_sessions WHERE id=$1 AND deletion_requested_at IS NOT NULL")
        .bind(job.session_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE recording_deletion_jobs SET state='ready',stage='ready',attempt_token=NULL,lease_expires_at=NULL,error=NULL,finished_at=now(),updated_at=now() WHERE id=$1")
        .bind(&job.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

async fn purge_source(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    object_id: Option<i64>,
    prefix: &str,
    job: &Claimed,
) -> Result<(), AppError> {
    let mut lock = pool.begin().await?;
    if let Some(object_id) = object_id {
        sqlx::query("SELECT pg_advisory_xact_lock($1,$2)")
            .bind(ARCHIVE_LOCK_NAMESPACE)
            .bind(object_id as i32)
            .execute(&mut *lock)
            .await?;
    }
    stage(pool, job, "purging_archive").await?;
    if let Some(archive) = media.archive() {
        archive
            .purge_versions(prefix)
            .await
            .map_err(|error| AppError::ServiceUnavailable(error.to_string()))?;
    }
    lock.commit().await?;
    Ok(())
}

async fn related_clips(
    connection: &mut sqlx::PgConnection,
    guild_id: i64,
    session_id: i64,
    fragments: &[Fragment],
) -> Result<Vec<Clip>, AppError> {
    let rows = sqlx::query("SELECT clip_id,saved_file_name,recording_session_id,original_file_name,channel_id,composition FROM clips WHERE guild_id=$1")
        .bind(guild_id).fetch_all(&mut *connection).await?;
    let mut selected = HashSet::new();
    let mut clips = Vec::new();
    // A composition can depend on any session clip or physical fragment clip.
    // Composed clips cannot themselves be sources, so one pass suffices.
    for row in &rows {
        let id: String = row.try_get("clip_id")?;
        let session: Option<i64> = row.try_get("recording_session_id")?;
        let original: Option<String> = row.try_get("original_file_name")?;
        let channel: Option<i64> = row.try_get("channel_id")?;
        if session == Some(session_id)
            || fragments.iter().any(|f| {
                Some(f.channel_id) == channel && original.as_deref() == Some(f.file_name.as_str())
            })
        {
            selected.insert(id.clone());
            clips.push(Clip {
                id,
                saved: row.try_get("saved_file_name")?,
            });
        }
    }
    for row in &rows {
        let id: String = row.try_get("clip_id")?;
        if selected.contains(&id) {
            continue;
        }
        let composition: Option<serde_json::Value> = row.try_get("composition")?;
        let dependent = composition
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
            selected.insert(id.clone());
            clips.push(Clip {
                id,
                saved: row.try_get("saved_file_name")?,
            });
        }
    }
    // This ledger survives composition-job cleanup and retains dependencies
    // even after a clip was overwritten with an unrelated composition.
    let sources: Vec<&str> = selected.iter().map(String::as_str).collect();
    let historical_targets: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT target_clip_id FROM clip_source_history WHERE source_clip_id=ANY($1)",
    )
    .bind(&sources)
    .fetch_all(&mut *connection)
    .await?;
    for target in historical_targets {
        if selected.contains(&target) {
            continue;
        }
        if let Some(row) = rows
            .iter()
            .find(|row| row.get::<String, _>("clip_id") == target)
        {
            selected.insert(target.clone());
            clips.push(Clip {
                id: target,
                saved: row.try_get("saved_file_name")?,
            });
        }
    }
    Ok(clips)
}

async fn related_media_jobs(
    pool: &Pool<Postgres>,
    job: &Claimed,
) -> Result<Vec<MediaJob>, AppError> {
    let rows =
        sqlx::query("SELECT id FROM media_jobs WHERE guild_id=$1 AND request->>'session_id'=$2")
            .bind(job.guild_id)
            .bind(job.session_id.to_string())
            .fetch_all(pool)
            .await?;
    rows.into_iter()
        .map(|row| {
            Ok(MediaJob {
                id: row.try_get("id")?,
            })
        })
        .collect()
}

async fn related_composition_jobs(
    pool: &Pool<Postgres>,
    job: &Claimed,
    clips: &[Clip],
) -> Result<Vec<CompositionJob>, AppError> {
    let clip_ids: HashSet<&str> = clips.iter().map(|clip| clip.id.as_str()).collect();
    let rows =
        sqlx::query("SELECT id,result_clip_id,snapshot FROM composition_jobs WHERE guild_id=$1")
            .bind(job.guild_id)
            .fetch_all(pool)
            .await?;
    let mut jobs = Vec::new();
    for row in rows {
        let target: String = row.try_get("result_clip_id")?;
        let snapshot: serde_json::Value = row.try_get("snapshot")?;
        let dependent = snapshot
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
        if clip_ids.contains(target.as_str()) || dependent {
            jobs.push(CompositionJob {
                id: row.try_get("id")?,
            });
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

async fn purge_fragment(roots: &DataRoots, fragment: &Fragment) -> Result<(), AppError> {
    if !matches!(
        Path::new(&fragment.file_name)
            .components()
            .collect::<Vec<_>>()
            .as_slice(),
        [std::path::Component::Normal(_)]
    ) {
        return Err(AppError::Conflict(
            "A recording has an unsafe file name".into(),
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
        let job_id = enqueue(&pool, 1, session_id, Some(200), "manager").await?;
        assert_eq!(
            job_id,
            enqueue(&pool, 1, session_id, Some(200), "manager").await?
        );
        let hidden: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM clips WHERE guild_id=1 AND deleted_at IS NOT NULL",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(hidden, 2);
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
        enqueue(&pool, 1, session_id, Some(200), "manager").await?;
        let insert = sqlx::query("INSERT INTO clips (clip_id,guild_id,channel_id,user_id,start_time,original_file_name,recording_session_id) VALUES ('late-clip',1,10,100,0,'session',$1)")
            .bind(session_id).execute(&pool).await;
        assert!(insert.is_err());
        Ok(())
    }
}
