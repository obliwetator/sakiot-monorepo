use std::path::PathBuf;

use chrono::{DateTime, Utc};
use sakiot_paths::{DataRoots, RecordingKey};
use sqlx::{Pool, Postgres};

use crate::errors::AppError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SourceId {
    Recording(i64),
    Clip(String),
}

/// Row shape shared by every `media_objects` query that claims work items.
#[derive(Clone, Debug, sqlx::FromRow)]
struct ClaimedMediaRow {
    id: i64,
    audio_file_id: Option<i64>,
    clip_id: Option<String>,
    clip_saved_file_name: Option<String>,
    object_key: Option<String>,
    bytes: Option<i64>,
    sha256: Option<String>,
    attempts: i32,
}

/// Row shape shared by every `media_objects` query that reads available
/// objects. The `!`-annotated columns are nullable in the schema but
/// guaranteed non-null for `state = 'available'` rows by the
/// `media_objects_available_metadata_check` constraint, so the queries below
/// override their inferred nullability.
#[derive(Clone, Debug, sqlx::FromRow)]
struct AvailableMediaRow {
    id: i64,
    audio_file_id: Option<i64>,
    clip_id: Option<String>,
    clip_saved_file_name: Option<String>,
    object_key: String,
    bytes: i64,
    sha256: String,
    local_delete_after: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkItem {
    pub id: i64,
    pub source: SourceId,
    pub path: PathBuf,
    pub object_key: Option<String>,
    pub bytes: Option<u64>,
    pub sha256: Option<String>,
    pub attempts: i32,
}

#[derive(Clone, Debug)]
pub(crate) struct AvailableObject {
    pub id: i64,
    pub path: PathBuf,
    pub object_key: String,
    pub bytes: u64,
    pub sha256: String,
    pub local_delete_after: DateTime<Utc>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ArchiveStatus {
    pub pending_objects: i64,
    pub pending_bytes: i64,
    pub uploading_objects: i64,
    pub available_objects: i64,
    pub available_bytes: i64,
    pub missing_objects: i64,
    pub conflict_objects: i64,
    pub oldest_backlog_seconds: i64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct EligibleStatus {
    pub recordings: i64,
    pub clips: i64,
    pub tracked: i64,
}

mod available;
mod upload;

pub(crate) use available::*;
pub(crate) use upload::*;

pub(crate) async fn recording_id(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_id: i64,
    year: i32,
    month: i32,
    file_name: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let stem = file_name.strip_suffix(".ogg").unwrap_or(file_name);
    sqlx::query_scalar!(
        "SELECT id
           FROM audio_files
          WHERE guild_id = $1
            AND channel_id = $2
            AND year = $3
            AND month = $4
            AND (file_name = $5 OR file_name = $6)
          ORDER BY id DESC
          LIMIT 1",
        guild_id,
        channel_id,
        year,
        month,
        file_name,
        stem,
    )
    .fetch_optional(pool)
    .await
}

pub(crate) async fn recording_source_path(
    pool: &Pool<Postgres>,
    audio_file_id: i64,
) -> Result<Option<PathBuf>, AppError> {
    source_path(pool, &SourceId::Recording(audio_file_id)).await
}

pub(crate) async fn clip_source_path(
    pool: &Pool<Postgres>,
    clip_id: &str,
) -> Result<Option<PathBuf>, AppError> {
    source_path(pool, &SourceId::Clip(clip_id.to_owned())).await
}

async fn revision_path(
    pool: &Pool<Postgres>,
    source: &SourceId,
    clip_saved_file_name: Option<String>,
) -> Result<Option<PathBuf>, AppError> {
    match source {
        SourceId::Recording(_) => source_path(pool, source).await,
        SourceId::Clip(_) => clip_saved_file_name
            .map(|saved| super::clip_local_path(&saved))
            .transpose(),
    }
}

async fn available_from_row(
    pool: &Pool<Postgres>,
    row: AvailableMediaRow,
) -> Result<AvailableObject, AppError> {
    let source = match (row.audio_file_id, row.clip_id) {
        (Some(id), None) => SourceId::Recording(id),
        (None, Some(id)) => SourceId::Clip(id),
        _ => return Err(AppError::InternalError),
    };
    let clip_saved_file_name = row.clip_saved_file_name.clone();
    let path = revision_path(pool, &source, clip_saved_file_name)
        .await?
        .ok_or(AppError::FileNotFound)?;
    Ok(AvailableObject {
        id: row.id,
        path,
        object_key: row.object_key,
        bytes: required_bytes(row.bytes)?,
        sha256: row.sha256,
        local_delete_after: row.local_delete_after,
    })
}

pub(crate) async fn status(pool: &Pool<Postgres>) -> Result<ArchiveStatus, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT
              count(*) FILTER (WHERE state = 'pending')::bigint AS "pending_objects!",
              COALESCE(sum(bytes) FILTER (WHERE state IN ('pending', 'uploading')), 0)::bigint AS "pending_bytes!",
              count(*) FILTER (WHERE state = 'uploading')::bigint AS "uploading_objects!",
              count(*) FILTER (WHERE state = 'available')::bigint AS "available_objects!",
              COALESCE(sum(bytes) FILTER (WHERE state = 'available'), 0)::bigint AS "available_bytes!",
              count(*) FILTER (WHERE state = 'missing')::bigint AS "missing_objects!",
              count(*) FILTER (WHERE state = 'conflict')::bigint AS "conflict_objects!",
              COALESCE(
                  EXTRACT(EPOCH FROM (now() - min(created_at) FILTER (
                      WHERE state IN ('pending', 'uploading')
                  )))::bigint,
                  0
              ) AS "oldest_backlog_seconds!"
            FROM media_objects"#,
    )
    .fetch_one(pool)
    .await?;
    Ok(ArchiveStatus {
        pending_objects: row.pending_objects,
        pending_bytes: row.pending_bytes,
        uploading_objects: row.uploading_objects,
        available_objects: row.available_objects,
        available_bytes: row.available_bytes,
        missing_objects: row.missing_objects,
        conflict_objects: row.conflict_objects,
        oldest_backlog_seconds: row.oldest_backlog_seconds,
    })
}

pub(crate) async fn eligible_status(pool: &Pool<Postgres>) -> Result<EligibleStatus, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT
              (SELECT count(*)::bigint FROM audio_files WHERE end_ts IS NOT NULL) AS "recordings!",
              (SELECT count(*)::bigint FROM clips
                WHERE saved_file_name IS NOT NULL AND btrim(saved_file_name) <> '') AS "clips!",
              (SELECT count(*)::bigint FROM media_objects) AS "tracked!""#,
    )
    .fetch_one(pool)
    .await?;
    Ok(EligibleStatus {
        recordings: row.recordings,
        clips: row.clips,
        tracked: row.tracked,
    })
}

pub(crate) async fn next_retry_at(
    pool: &Pool<Postgres>,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    sqlx::query_scalar!("SELECT min(retry_at) FROM media_objects WHERE state = 'pending'")
        .fetch_one(pool)
        .await
}

pub(crate) async fn requeue_missing(pool: &Pool<Postgres>) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query!(
        "UPDATE media_objects
            SET state = 'pending',
                retry_at = now(),
                lease_owner = NULL,
                lease_expires_at = NULL,
                updated_at = now()
          WHERE state = 'missing'",
    )
    .execute(pool)
    .await?
    .rows_affected())
}

async fn source_path(
    pool: &Pool<Postgres>,
    source: &SourceId,
) -> Result<Option<PathBuf>, AppError> {
    let roots = DataRoots::from_env();
    match source {
        SourceId::Recording(audio_file_id) => {
            let row = sqlx::query!(
                "SELECT guild_id, channel_id, year, month, file_name
                   FROM audio_files
                  WHERE id = $1 AND end_ts IS NOT NULL",
                audio_file_id
            )
            .fetch_optional(pool)
            .await?;
            row.map(|row| {
                let month = u32::try_from(row.month).map_err(|_| AppError::InternalError)?;
                Ok(
                    RecordingKey::new(row.guild_id, row.channel_id, row.year, month, row.file_name)
                        .recording_path(&roots.recordings_str()),
                )
            })
            .transpose()
        }
        SourceId::Clip(clip_id) => {
            let saved = sqlx::query_scalar!(
                "SELECT saved_file_name FROM clips WHERE clip_id = $1",
                clip_id
            )
            .fetch_optional(pool)
            .await?
            .flatten();
            saved
                .map(|saved| super::clip_local_path(&saved))
                .transpose()
        }
    }
}

fn optional_bytes(bytes: Option<i64>) -> Result<Option<u64>, AppError> {
    bytes
        .map(|bytes| u64::try_from(bytes).map_err(|_| AppError::InternalError))
        .transpose()
}

fn required_bytes(bytes: i64) -> Result<u64, AppError> {
    u64::try_from(bytes).map_err(|_| AppError::InternalError)
}

#[cfg(test)]
mod tests;
