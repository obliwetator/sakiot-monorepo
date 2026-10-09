//! Objects with a verified remote copy: looking them up, listing them, and
//! recording what local restores and verification find.

use super::*;

pub(crate) async fn available_object(
    pool: &Pool<Postgres>,
    source: &SourceId,
) -> Result<Option<AvailableObject>, AppError> {
    let row = match source {
        SourceId::Recording(audio_file_id) => {
            sqlx::query_as!(
                AvailableMediaRow,
                r#"SELECT id,
                      audio_file_id,
                      clip_id,
                      clip_saved_file_name,
                      object_key AS "object_key!",
                      bytes AS "bytes!",
                      sha256 AS "sha256!",
                      local_delete_after AS "local_delete_after!"
                 FROM media_objects
                WHERE audio_file_id = $1
                  AND state = 'available'
                  AND verified_at IS NOT NULL"#,
                audio_file_id
            )
            .fetch_optional(pool)
            .await?
        }
        SourceId::Clip(clip_id) => {
            sqlx::query_as!(
                AvailableMediaRow,
                r#"SELECT id,
                      audio_file_id,
                      clip_id,
                      clip_saved_file_name,
                      object_key AS "object_key!",
                      bytes AS "bytes!",
                      sha256 AS "sha256!",
                      local_delete_after AS "local_delete_after!"
                 FROM media_objects
                WHERE clip_id = $1
                  AND state = 'available'
                  AND verified_at IS NOT NULL"#,
                clip_id
            )
            .fetch_optional(pool)
            .await?
        }
    };
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(available_from_row(pool, row).await?))
}

pub(crate) async fn list_available(
    pool: &Pool<Postgres>,
) -> Result<Vec<AvailableObject>, AppError> {
    let rows = sqlx::query_as!(
        AvailableMediaRow,
        r#"SELECT id,
                  audio_file_id,
                  clip_id,
                  clip_saved_file_name,
                  object_key AS "object_key!",
                  bytes AS "bytes!",
                  sha256 AS "sha256!",
                  local_delete_after AS "local_delete_after!"
             FROM media_objects
            WHERE state = 'available' AND verified_at IS NOT NULL
            ORDER BY id"#,
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
    let rows = sqlx::query_as!(
        AvailableMediaRow,
        r#"SELECT id,
                  audio_file_id,
                  clip_id,
                  clip_saved_file_name,
                  object_key AS "object_key!",
                  bytes AS "bytes!",
                  sha256 AS "sha256!",
                  local_delete_after AS "local_delete_after!"
             FROM media_objects
            WHERE state = 'available'
              AND verified_at IS NOT NULL
              AND audio_file_id = ANY($1)
            ORDER BY id"#,
        audio_file_ids,
    )
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
    let rows = sqlx::query_as!(
        AvailableMediaRow,
        r#"SELECT id,
                  audio_file_id,
                  clip_id,
                  clip_saved_file_name,
                  object_key AS "object_key!",
                  bytes AS "bytes!",
                  sha256 AS "sha256!",
                  local_delete_after AS "local_delete_after!"
             FROM media_objects
            WHERE state = 'available'
              AND verified_at IS NOT NULL
              AND clip_id = ANY($1)
            ORDER BY id"#,
        clip_ids,
    )
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
    sqlx::query!(
        "UPDATE media_objects
            SET local_delete_after = now() + ($2::bigint * interval '1 day'),
                updated_at = now()
          WHERE id = $1 AND state = 'available' AND verified_at IS NOT NULL",
        id,
        retention_days,
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn mark_remote_missing(
    pool: &Pool<Postgres>,
    id: i64,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE media_objects
            SET state = 'missing',
                last_error = left($2, 4000),
                updated_at = now()
          WHERE id = $1",
        id,
        error,
    )
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
    Ok(sqlx::query!(
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
        id,
        etag,
        retention_days,
    )
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
    sqlx::query!(
        "UPDATE media_objects
            SET state = 'conflict',
                lease_owner = NULL,
                lease_expires_at = NULL,
                last_error = left($2, 4000),
                updated_at = now()
          WHERE id = $1 AND state = 'available'",
        id,
        error,
    )
    .execute(pool)
    .await?;
    Ok(())
}
