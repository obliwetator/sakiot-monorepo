//! Moving an object through the upload lifecycle: reconciling sources,
//! claiming work under a lease, and recording each state it reaches.

use super::*;

/// Track every finished recording and saved clip for archiving. Synthetic
/// load-test guilds (`crate::synthetic`) are skipped: their audio is
/// generated noise, and uploading it would only cost storage and bandwidth.
pub(crate) async fn reconcile(pool: &Pool<Postgres>) -> Result<u64, sqlx::Error> {
    let recordings = sqlx::query!(
        "INSERT INTO media_objects (audio_file_id)
         SELECT af.id
           FROM audio_files af
          WHERE af.end_ts IS NOT NULL
            AND af.guild_id < $1
            AND NOT EXISTS (SELECT 1 FROM recording_sessions rs WHERE rs.id=af.recording_session_id AND rs.deletion_requested_at IS NOT NULL)
          ON CONFLICT (audio_file_id) WHERE audio_file_id IS NOT NULL DO NOTHING",
        crate::synthetic::SYNTHETIC_ID_FLOOR,
    )
    .execute(pool)
    .await?
    .rows_affected();
    let clips = sqlx::query!(
        "INSERT INTO media_objects (clip_id)
         SELECT c.clip_id
           FROM clips c
          WHERE c.saved_file_name IS NOT NULL
            AND btrim(c.saved_file_name) <> ''
            AND c.deleted_at IS NULL
            AND (c.guild_id IS NULL OR c.guild_id < $1)
            AND NOT EXISTS (SELECT 1 FROM recording_sessions rs WHERE rs.id=c.recording_session_id AND rs.deletion_requested_at IS NOT NULL)
          ON CONFLICT (clip_id) WHERE clip_id IS NOT NULL DO NOTHING",
        crate::synthetic::SYNTHETIC_ID_FLOOR,
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
    let rows: Vec<ClaimedMediaRow> = sqlx::query_as!(
        ClaimedMediaRow,
        "WITH candidates AS (
             SELECT id
               FROM media_objects
              WHERE (
                        (state = 'pending' AND retry_at <= now())
                     OR (state = 'uploading' AND lease_expires_at < now())
                    )
                AND (lease_expires_at IS NULL OR lease_expires_at < now())
                AND NOT EXISTS (SELECT 1 FROM audio_files af JOIN recording_sessions rs ON rs.id=af.recording_session_id WHERE af.id=media_objects.audio_file_id AND rs.deletion_requested_at IS NOT NULL)
                AND NOT EXISTS (SELECT 1 FROM clips c WHERE c.clip_id=media_objects.clip_id AND (c.deleted_at IS NOT NULL OR EXISTS (SELECT 1 FROM recording_sessions rs WHERE rs.id=c.recording_session_id AND rs.deletion_requested_at IS NOT NULL)))
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
                   object.clip_saved_file_name,
                   object.object_key,
                   object.bytes,
                   object.sha256,
                   object.attempts",
        owner,
        limit,
    )
    .fetch_all(pool)
    .await?;

    let mut work = Vec::with_capacity(rows.len());
    for row in rows {
        let source = match (row.audio_file_id, row.clip_id) {
            (Some(audio_file_id), None) => SourceId::Recording(audio_file_id),
            (None, Some(clip_id)) => SourceId::Clip(clip_id),
            _ => {
                mark_conflict(pool, row.id, owner, "media row has invalid source identity").await?;
                continue;
            }
        };
        let Some(path) = revision_path(pool, &source, row.clip_saved_file_name).await? else {
            mark_missing(
                pool,
                row.id,
                owner,
                "source database row or saved path is missing",
            )
            .await?;
            continue;
        };
        work.push(WorkItem {
            id: row.id,
            source,
            path,
            object_key: row.object_key,
            bytes: optional_bytes(row.bytes)?,
            sha256: row.sha256,
            attempts: row.attempts,
        });
    }
    Ok(work)
}

pub(crate) async fn renew_lease(
    pool: &Pool<Postgres>,
    id: i64,
    owner: &str,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query!(
        "UPDATE media_objects
            SET lease_expires_at = now() + interval '5 minutes',
                updated_at = now()
          WHERE id = $1 AND state = 'uploading' AND lease_owner = $2",
        id,
        owner,
    )
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
    let result = sqlx::query!(
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
        item.id,
        owner,
        object_key,
        bytes,
        sha256,
    )
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
    sqlx::query!(
        "UPDATE media_objects
            SET etag = COALESCE($3, etag),
                uploaded_at = COALESCE(uploaded_at, now()),
                lease_expires_at = now() + interval '5 minutes',
                updated_at = now()
          WHERE id = $1 AND state = 'uploading' AND lease_owner = $2",
        id,
        owner,
        etag,
    )
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
    let result = sqlx::query!(
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
        id,
        owner,
        etag,
        retention_days,
    )
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
    sqlx::query!(
        "UPDATE media_objects
            SET state = 'pending',
                retry_at = now() + ($4::bigint * interval '1 second'),
                lease_owner = NULL,
                lease_expires_at = NULL,
                last_error = left($3, 4000),
                updated_at = now()
          WHERE id = $1 AND state = 'uploading' AND lease_owner = $2",
        item.id,
        owner,
        error,
        delay_seconds,
    )
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
    sqlx::query!(
        "UPDATE media_objects
            SET state = 'missing',
                lease_owner = NULL,
                lease_expires_at = NULL,
                last_error = left($3, 4000),
                updated_at = now()
          WHERE id = $1 AND state = 'uploading' AND lease_owner = $2",
        id,
        owner,
        error,
    )
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
    sqlx::query!(
        "UPDATE media_objects
            SET state = 'conflict',
                lease_owner = NULL,
                lease_expires_at = NULL,
                last_error = left($3, 4000),
                updated_at = now()
          WHERE id = $1 AND state = 'uploading' AND lease_owner = $2",
        id,
        owner,
        error,
    )
    .execute(pool)
    .await?;
    Ok(())
}
