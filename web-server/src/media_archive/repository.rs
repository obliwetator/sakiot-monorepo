use std::path::PathBuf;

use chrono::{DateTime, Utc};
use sakiot_paths::{DataRoots, RecordingKey};
use sqlx::{Pool, Postgres, Row};

use crate::errors::AppError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SourceId {
    Recording(i64),
    Clip(String),
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

pub(crate) async fn reconcile(pool: &Pool<Postgres>) -> Result<u64, sqlx::Error> {
    let recordings = sqlx::query(
        "INSERT INTO media_objects (audio_file_id)
         SELECT af.id
           FROM audio_files af
          WHERE af.end_ts IS NOT NULL
         ON CONFLICT (audio_file_id) WHERE audio_file_id IS NOT NULL DO NOTHING",
    )
    .execute(pool)
    .await?
    .rows_affected();
    let clips = sqlx::query(
        "INSERT INTO media_objects (clip_id)
         SELECT c.clip_id
           FROM clips c
          WHERE c.saved_file_name IS NOT NULL
            AND btrim(c.saved_file_name) <> ''
         ON CONFLICT (clip_id) WHERE clip_id IS NOT NULL DO NOTHING",
    )
    .execute(pool)
    .await?
    .rows_affected();
    Ok(recordings + clips)
}

pub(crate) async fn claim_batch(
    pool: &Pool<Postgres>,
    owner: &str,
    limit: i64,
) -> Result<Vec<WorkItem>, AppError> {
    let rows = sqlx::query(
        "WITH candidates AS (
             SELECT id
               FROM media_objects
              WHERE (
                        (state = 'pending' AND retry_at <= now())
                     OR (state = 'uploading' AND lease_expires_at < now())
                    )
                AND (lease_expires_at IS NULL OR lease_expires_at < now())
              ORDER BY retry_at, created_at, id
              FOR UPDATE SKIP LOCKED
              LIMIT $2
         )
         UPDATE media_objects object
            SET state = 'uploading',
                attempts = object.attempts + 1,
                lease_owner = $1,
                lease_expires_at = now() + interval '5 minutes',
                last_error = NULL,
                updated_at = now()
           FROM candidates
          WHERE object.id = candidates.id
         RETURNING object.id,
                   object.audio_file_id,
                   object.clip_id,
                   object.object_key,
                   object.bytes,
                   object.sha256,
                   object.attempts",
    )
    .bind(owner)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let mut work = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row.try_get("id")?;
        let source = match (
            row.try_get::<Option<i64>, _>("audio_file_id")?,
            row.try_get::<Option<String>, _>("clip_id")?,
        ) {
            (Some(audio_file_id), None) => SourceId::Recording(audio_file_id),
            (None, Some(clip_id)) => SourceId::Clip(clip_id),
            _ => {
                mark_conflict(pool, id, owner, "media row has invalid source identity").await?;
                continue;
            }
        };
        let Some(path) = source_path(pool, &source).await? else {
            mark_missing(
                pool,
                id,
                owner,
                "source database row or saved path is missing",
            )
            .await?;
            continue;
        };
        work.push(WorkItem {
            id,
            source,
            path,
            object_key: row.try_get("object_key")?,
            bytes: optional_bytes(&row, "bytes")?,
            sha256: row.try_get("sha256")?,
            attempts: row.try_get("attempts")?,
        });
    }
    Ok(work)
}

pub(crate) async fn renew_lease(
    pool: &Pool<Postgres>,
    id: i64,
    owner: &str,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE media_objects
            SET lease_expires_at = now() + interval '5 minutes',
                updated_at = now()
          WHERE id = $1 AND state = 'uploading' AND lease_owner = $2",
    )
    .bind(id)
    .bind(owner)
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

pub(crate) async fn record_prepared(
    pool: &Pool<Postgres>,
    item: &WorkItem,
    owner: &str,
    object_key: &str,
    bytes: u64,
    sha256: &str,
) -> Result<bool, AppError> {
    let bytes = i64::try_from(bytes).map_err(|_| AppError::InternalError)?;
    let result = sqlx::query(
        "UPDATE media_objects
            SET object_key = $3,
                bytes = $4,
                sha256 = $5,
                lease_expires_at = now() + interval '5 minutes',
                updated_at = now()
          WHERE id = $1
            AND state = 'uploading'
            AND lease_owner = $2
            AND (object_key IS NULL OR object_key = $3)
            AND (bytes IS NULL OR bytes = $4)
            AND (sha256 IS NULL OR sha256 = $5)",
    )
    .bind(item.id)
    .bind(owner)
    .bind(object_key)
    .bind(bytes)
    .bind(sha256)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub(crate) async fn mark_uploaded(
    pool: &Pool<Postgres>,
    id: i64,
    owner: &str,
    etag: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE media_objects
            SET etag = COALESCE($3, etag),
                uploaded_at = COALESCE(uploaded_at, now()),
                lease_expires_at = now() + interval '5 minutes',
                updated_at = now()
          WHERE id = $1 AND state = 'uploading' AND lease_owner = $2",
    )
    .bind(id)
    .bind(owner)
    .bind(etag)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn mark_available(
    pool: &Pool<Postgres>,
    id: i64,
    owner: &str,
    etag: Option<&str>,
    retention_days: u64,
) -> Result<bool, AppError> {
    let retention_days = i64::try_from(retention_days).map_err(|_| AppError::InternalError)?;
    let result = sqlx::query(
        "UPDATE media_objects
            SET state = 'available',
                etag = COALESCE($3, etag),
                uploaded_at = COALESCE(uploaded_at, now()),
                verified_at = now(),
                local_delete_after = now() + ($4::bigint * interval '1 day'),
                lease_owner = NULL,
                lease_expires_at = NULL,
                retry_at = now(),
                last_error = NULL,
                updated_at = now()
          WHERE id = $1 AND state = 'uploading' AND lease_owner = $2",
    )
    .bind(id)
    .bind(owner)
    .bind(etag)
    .bind(retention_days)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub(crate) async fn mark_pending(
    pool: &Pool<Postgres>,
    item: &WorkItem,
    owner: &str,
    error: &str,
) -> Result<(), sqlx::Error> {
    let exponent = u32::try_from(item.attempts.saturating_sub(1).clamp(0, 8)).unwrap_or(8);
    let base_seconds = 15u64
        .saturating_mul(2u64.saturating_pow(exponent))
        .min(3_600);
    let jitter_seconds = fastrand::u64(0..=(base_seconds / 2).max(1));
    let delay_seconds = i64::try_from(base_seconds + jitter_seconds).unwrap_or(5_400);
    sqlx::query(
        "UPDATE media_objects
            SET state = 'pending',
                retry_at = now() + ($4::bigint * interval '1 second'),
                lease_owner = NULL,
                lease_expires_at = NULL,
                last_error = left($3, 4000),
                updated_at = now()
          WHERE id = $1 AND state = 'uploading' AND lease_owner = $2",
    )
    .bind(item.id)
    .bind(owner)
    .bind(error)
    .bind(delay_seconds)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn mark_missing(
    pool: &Pool<Postgres>,
    id: i64,
    owner: &str,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE media_objects
            SET state = 'missing',
                lease_owner = NULL,
                lease_expires_at = NULL,
                last_error = left($3, 4000),
                updated_at = now()
          WHERE id = $1 AND state = 'uploading' AND lease_owner = $2",
    )
    .bind(id)
    .bind(owner)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn mark_conflict(
    pool: &Pool<Postgres>,
    id: i64,
    owner: &str,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE media_objects
            SET state = 'conflict',
                lease_owner = NULL,
                lease_expires_at = NULL,
                last_error = left($3, 4000),
                updated_at = now()
          WHERE id = $1 AND state = 'uploading' AND lease_owner = $2",
    )
    .bind(id)
    .bind(owner)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn available_object(
    pool: &Pool<Postgres>,
    source: &SourceId,
) -> Result<Option<AvailableObject>, AppError> {
    let row =
        match source {
            SourceId::Recording(audio_file_id) => sqlx::query(
                "SELECT id, audio_file_id, clip_id, object_key, bytes, sha256, local_delete_after
               FROM media_objects
              WHERE audio_file_id = $1
                AND state = 'available'
                AND verified_at IS NOT NULL",
            )
            .bind(audio_file_id)
            .fetch_optional(pool)
            .await?,
            SourceId::Clip(clip_id) => sqlx::query(
                "SELECT id, audio_file_id, clip_id, object_key, bytes, sha256, local_delete_after
               FROM media_objects
              WHERE clip_id = $1
                AND state = 'available'
                AND verified_at IS NOT NULL",
            )
            .bind(clip_id)
            .fetch_optional(pool)
            .await?,
        };
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(available_from_row(pool, row).await?))
}

pub(crate) async fn recording_id(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_id: i64,
    year: i32,
    month: i32,
    file_name: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let stem = file_name.strip_suffix(".ogg").unwrap_or(file_name);
    sqlx::query_scalar(
        "SELECT id
           FROM audio_files
          WHERE guild_id = $1
            AND channel_id = $2
            AND year = $3
            AND month = $4
            AND (file_name = $5 OR file_name = $6)
          ORDER BY id DESC
          LIMIT 1",
    )
    .bind(guild_id)
    .bind(channel_id)
    .bind(year)
    .bind(month)
    .bind(file_name)
    .bind(stem)
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

async fn available_from_row(
    pool: &Pool<Postgres>,
    row: sqlx::postgres::PgRow,
) -> Result<AvailableObject, AppError> {
    let source = match (
        row.try_get::<Option<i64>, _>("audio_file_id")?,
        row.try_get::<Option<String>, _>("clip_id")?,
    ) {
        (Some(id), None) => SourceId::Recording(id),
        (None, Some(id)) => SourceId::Clip(id),
        _ => return Err(AppError::InternalError),
    };
    let path = source_path(pool, &source)
        .await?
        .ok_or(AppError::FileNotFound)?;
    Ok(AvailableObject {
        id: row.try_get("id")?,
        path,
        object_key: row.try_get("object_key")?,
        bytes: required_bytes(&row, "bytes")?,
        sha256: row.try_get("sha256")?,
        local_delete_after: row.try_get("local_delete_after")?,
    })
}

pub(crate) async fn list_available(
    pool: &Pool<Postgres>,
) -> Result<Vec<AvailableObject>, AppError> {
    let rows = sqlx::query(
        "SELECT id, audio_file_id, clip_id, object_key, bytes, sha256, local_delete_after
           FROM media_objects
          WHERE state = 'available' AND verified_at IS NOT NULL
          ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    let mut objects = Vec::with_capacity(rows.len());
    for row in rows {
        objects.push(available_from_row(pool, row).await?);
    }
    Ok(objects)
}

pub(crate) async fn list_available_recordings(
    pool: &Pool<Postgres>,
    audio_file_ids: &[i64],
) -> Result<Vec<AvailableObject>, AppError> {
    if audio_file_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT id, audio_file_id, clip_id, object_key, bytes, sha256, local_delete_after
           FROM media_objects
          WHERE state = 'available'
            AND verified_at IS NOT NULL
            AND audio_file_id = ANY($1)
          ORDER BY id",
    )
    .bind(audio_file_ids)
    .fetch_all(pool)
    .await?;
    let mut objects = Vec::with_capacity(rows.len());
    for row in rows {
        objects.push(available_from_row(pool, row).await?);
    }
    Ok(objects)
}

pub(crate) async fn list_available_clips(
    pool: &Pool<Postgres>,
    clip_ids: &[String],
) -> Result<Vec<AvailableObject>, AppError> {
    if clip_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT id, audio_file_id, clip_id, object_key, bytes, sha256, local_delete_after
           FROM media_objects
          WHERE state = 'available'
            AND verified_at IS NOT NULL
            AND clip_id = ANY($1)
          ORDER BY id",
    )
    .bind(clip_ids)
    .fetch_all(pool)
    .await?;
    let mut objects = Vec::with_capacity(rows.len());
    for row in rows {
        objects.push(available_from_row(pool, row).await?);
    }
    Ok(objects)
}

pub(crate) async fn reset_local_retention(
    pool: &Pool<Postgres>,
    id: i64,
    retention_days: u64,
) -> Result<(), AppError> {
    let retention_days = i64::try_from(retention_days).map_err(|_| AppError::InternalError)?;
    sqlx::query(
        "UPDATE media_objects
            SET local_delete_after = now() + ($2::bigint * interval '1 day'),
                updated_at = now()
          WHERE id = $1 AND state = 'available' AND verified_at IS NOT NULL",
    )
    .bind(id)
    .bind(retention_days)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn mark_remote_missing(
    pool: &Pool<Postgres>,
    id: i64,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE media_objects
            SET state = 'missing',
                last_error = left($2, 4000),
                updated_at = now()
          WHERE id = $1",
    )
    .bind(id)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn refresh_available_verification(
    pool: &Pool<Postgres>,
    id: i64,
    etag: Option<&str>,
    retention_days: u64,
) -> Result<bool, AppError> {
    let retention_days = i64::try_from(retention_days).map_err(|_| AppError::InternalError)?;
    Ok(sqlx::query(
        "UPDATE media_objects
            SET etag = COALESCE($2, etag),
                verified_at = now(),
                local_delete_after = GREATEST(
                    local_delete_after,
                    now() + ($3::bigint * interval '1 day')
                ),
                last_error = NULL,
                updated_at = now()
          WHERE id = $1 AND state = 'available'",
    )
    .bind(id)
    .bind(etag)
    .bind(retention_days)
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

pub(crate) async fn mark_verification_conflict(
    pool: &Pool<Postgres>,
    id: i64,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE media_objects
            SET state = 'conflict',
                lease_owner = NULL,
                lease_expires_at = NULL,
                last_error = left($2, 4000),
                updated_at = now()
          WHERE id = $1 AND state = 'available'",
    )
    .bind(id)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn status(pool: &Pool<Postgres>) -> Result<ArchiveStatus, sqlx::Error> {
    let row = sqlx::query(
        "SELECT
             count(*) FILTER (WHERE state = 'pending')::bigint AS pending_objects,
             COALESCE(sum(bytes) FILTER (WHERE state IN ('pending', 'uploading')), 0)::bigint AS pending_bytes,
             count(*) FILTER (WHERE state = 'uploading')::bigint AS uploading_objects,
             count(*) FILTER (WHERE state = 'available')::bigint AS available_objects,
             COALESCE(sum(bytes) FILTER (WHERE state = 'available'), 0)::bigint AS available_bytes,
             count(*) FILTER (WHERE state = 'missing')::bigint AS missing_objects,
             count(*) FILTER (WHERE state = 'conflict')::bigint AS conflict_objects,
             COALESCE(
                 EXTRACT(EPOCH FROM (now() - min(created_at) FILTER (
                     WHERE state IN ('pending', 'uploading')
                 )))::bigint,
                 0
             ) AS oldest_backlog_seconds
           FROM media_objects",
    )
    .fetch_one(pool)
    .await?;
    Ok(ArchiveStatus {
        pending_objects: row.try_get("pending_objects")?,
        pending_bytes: row.try_get("pending_bytes")?,
        uploading_objects: row.try_get("uploading_objects")?,
        available_objects: row.try_get("available_objects")?,
        available_bytes: row.try_get("available_bytes")?,
        missing_objects: row.try_get("missing_objects")?,
        conflict_objects: row.try_get("conflict_objects")?,
        oldest_backlog_seconds: row.try_get("oldest_backlog_seconds")?,
    })
}

pub(crate) async fn eligible_status(pool: &Pool<Postgres>) -> Result<EligibleStatus, sqlx::Error> {
    let row = sqlx::query(
        "SELECT
             (SELECT count(*)::bigint FROM audio_files WHERE end_ts IS NOT NULL) AS recordings,
             (SELECT count(*)::bigint FROM clips
               WHERE saved_file_name IS NOT NULL AND btrim(saved_file_name) <> '') AS clips,
             (SELECT count(*)::bigint FROM media_objects) AS tracked",
    )
    .fetch_one(pool)
    .await?;
    Ok(EligibleStatus {
        recordings: row.try_get("recordings")?,
        clips: row.try_get("clips")?,
        tracked: row.try_get("tracked")?,
    })
}

pub(crate) async fn next_retry_at(
    pool: &Pool<Postgres>,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT min(retry_at)
           FROM media_objects
          WHERE state = 'pending'",
    )
    .fetch_one(pool)
    .await
}

pub(crate) async fn requeue_missing(pool: &Pool<Postgres>) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
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
            let row = sqlx::query(
                "SELECT guild_id, channel_id, year, month, file_name
                   FROM audio_files
                  WHERE id = $1 AND end_ts IS NOT NULL",
            )
            .bind(audio_file_id)
            .fetch_optional(pool)
            .await?;
            row.map(|row| {
                let month: i32 = row.try_get("month")?;
                let month = u32::try_from(month).map_err(|_| AppError::InternalError)?;
                Ok(RecordingKey::new(
                    row.try_get("guild_id")?,
                    row.try_get("channel_id")?,
                    row.try_get("year")?,
                    month,
                    row.try_get::<String, _>("file_name")?,
                )
                .recording_path(&roots.recordings_str()))
            })
            .transpose()
        }
        SourceId::Clip(clip_id) => {
            let saved = sqlx::query_scalar::<_, Option<String>>(
                "SELECT saved_file_name FROM clips WHERE clip_id = $1",
            )
            .bind(clip_id)
            .fetch_optional(pool)
            .await?
            .flatten();
            saved
                .map(|saved| super::clip_local_path(&saved))
                .transpose()
        }
    }
}

fn optional_bytes(row: &sqlx::postgres::PgRow, column: &str) -> Result<Option<u64>, AppError> {
    row.try_get::<Option<i64>, _>(column)?
        .map(|bytes| u64::try_from(bytes).map_err(|_| AppError::InternalError))
        .transpose()
}

fn required_bytes(row: &sqlx::postgres::PgRow, column: &str) -> Result<u64, AppError> {
    optional_bytes(row, column)?.ok_or(AppError::InternalError)
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;

    async fn seed_sources(pool: &PgPool) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO audio_files
                (file_name, guild_id, channel_id, user_id, year, month, start_ts, end_ts)
             VALUES
                ('media-finalized', 1, 10, 100, 2026, 7, 1000, 2000),
                ('media-active', 1, 10, 100, 2026, 7, 3000, NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO clips (clip_id, start_time, saved_file_name)
             VALUES
                ('media-saved-clip', 0, '2026/07/media-saved-clip.ogg'),
                ('media-unsaved-clip', 0, NULL)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn reconciliation_queues_only_finalized_or_saved_sources(
        pool: PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        seed_sources(&pool).await?;
        assert_eq!(reconcile(&pool).await?, 2);
        assert_eq!(reconcile(&pool).await?, 0);
        let sources: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT af.file_name, mo.clip_id
               FROM media_objects mo
               LEFT JOIN audio_files af ON af.id = mo.audio_file_id
              ORDER BY COALESCE(af.file_name, mo.clip_id)",
        )
        .fetch_all(&pool)
        .await?;
        assert_eq!(
            sources,
            vec![
                (Some("media-finalized".to_owned()), None),
                (None, Some("media-saved-clip".to_owned())),
            ]
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn claims_are_exclusive_and_expired_leases_recover(
        pool: PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        seed_sources(&pool).await?;
        reconcile(&pool).await?;
        let first = claim_batch(&pool, "worker-a", 1).await?;
        let second = claim_batch(&pool, "worker-b", 1).await?;
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_ne!(first[0].id, second[0].id);
        assert!(claim_batch(&pool, "worker-c", 1).await?.is_empty());

        sqlx::query(
            "UPDATE media_objects
                SET lease_expires_at = now() - interval '1 second'
              WHERE id = $1",
        )
        .bind(first[0].id)
        .execute(&pool)
        .await?;
        let recovered = claim_batch(&pool, "worker-c", 1).await?;
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].id, first[0].id);
        assert_eq!(recovered[0].attempts, 2);
        mark_pending(&pool, &recovered[0], "worker-c", "temporary B2 outage").await?;
        let retry: (
            String,
            Option<String>,
            Option<DateTime<Utc>>,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT state, lease_owner, retry_at, last_error
                   FROM media_objects
                  WHERE id = $1",
        )
        .bind(recovered[0].id)
        .fetch_one(&pool)
        .await?;
        assert_eq!(retry.0, "pending");
        assert_eq!(retry.1, None);
        assert!(retry.2.is_some_and(|retry_at| retry_at > Utc::now()));
        assert_eq!(retry.3.as_deref(), Some("temporary B2 outage"));
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn full_reverification_keeps_media_available(
        pool: PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        seed_sources(&pool).await?;
        reconcile(&pool).await?;
        let id: i64 = sqlx::query_scalar(
            "UPDATE media_objects
                SET state = 'available',
                    object_key = 'media/v1/recordings/1/test.ogg',
                    bytes = 4,
                    sha256 = repeat('a', 64),
                    uploaded_at = now() - interval '1 day',
                    verified_at = now() - interval '1 day',
                    local_delete_after = now()
              WHERE audio_file_id IS NOT NULL
              RETURNING id",
        )
        .fetch_one(&pool)
        .await?;

        assert!(refresh_available_verification(&pool, id, Some("\"new-etag\""), 7).await?);
        let (state, etag, delete_after): (String, Option<String>, DateTime<Utc>) = sqlx::query_as(
            "SELECT state, etag, local_delete_after
                   FROM media_objects
                  WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await?;
        assert_eq!(state, "available");
        assert_eq!(etag.as_deref(), Some("\"new-etag\""));
        assert!(delete_after > Utc::now() + chrono::Duration::days(6));
        Ok(())
    }
}
