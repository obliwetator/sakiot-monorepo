use sqlx::{Pool, Postgres, Row};

use crate::database::error::expect_rows;
use crate::database::{DbError, DbResult};

#[cfg(test)]
pub const FINALIZE_REASON_WRITER_CLOSE: i32 = 1;
pub const FINALIZE_REASON_ZOMBIE_REAPED: i32 = 3;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RecordingHandle {
    pub audio_file_id: i64,
    pub recording_session_id: i64,
    pub segment_index: i32,
    pub file_name: String,
    pub path: String,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub initial_silence_ms: i64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ZombieRecording {
    pub audio_file_id: i64,
    pub file_name: String,
    pub guild_id: i64,
    pub channel_id: i64,
    pub start_ts: Option<i64>,
}

pub async fn create_recording(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_id: i64,
    user_id: i64,
    now: chrono::DateTime<chrono::Utc>,
    owner_instance_id: &str,
) -> DbResult<RecordingHandle> {
    crate::database::logical_recordings::create_fragment(
        pool,
        guild_id,
        channel_id,
        user_id,
        now,
        owner_instance_id,
    )
    .await
}

#[cfg(test)]
pub async fn create_recording_for_test(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_id: i64,
    user_id: i64,
    now: chrono::DateTime<chrono::Utc>,
    owner_instance_id: &str,
    recording_root: &std::path::Path,
) -> DbResult<RecordingHandle> {
    crate::database::logical_recordings::create_fragment_in(
        pool,
        guild_id,
        channel_id,
        user_id,
        now,
        owner_instance_id,
        recording_root,
    )
    .await
}

pub async fn heartbeat_active_recordings(
    pool: &Pool<Postgres>,
    audio_file_ids: &[i64],
    owner_instance_id: &str,
) -> DbResult<u64> {
    if audio_file_ids.is_empty() {
        return Ok(0);
    }

    let result = sqlx::query(
        "UPDATE audio_files
            SET recording_heartbeat_at = now()
          WHERE id = ANY($1)
            AND recording_owner_instance_id = $2
            AND end_ts IS NULL",
    )
    .bind(audio_file_ids)
    .bind(owner_instance_id)
    .execute(pool)
    .await?;

    expect_rows(
        result,
        audio_file_ids.len() as u64,
        "heartbeat active recordings",
    )
}

pub async fn mark_recording_setup_failed(
    pool: &Pool<Postgres>,
    audio_file_id: i64,
    owner_instance_id: &str,
    finalize_reason_id: i32,
) -> DbResult<()> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "UPDATE audio_files
            SET end_ts = COALESCE(end_ts, start_ts),
                reaped = TRUE,
                recording_heartbeat_at = NULL,
                finalize_reason_id = $3
          WHERE id = $1
            AND recording_owner_instance_id = $2
            AND end_ts IS NULL
         RETURNING recording_session_id, segment_index, channel_id, end_ts",
    )
    .bind(audio_file_id)
    .bind(owner_instance_id)
    .bind(finalize_reason_id)
    .fetch_optional(&mut *tx)
    .await?;
    let row = row.ok_or(DbError::UnexpectedRows {
        operation: "mark recording setup failed",
        expected: 1,
        actual: 0,
    })?;
    if let Some(recording_session_id) = row.try_get::<Option<i64>, _>("recording_session_id")? {
        let end_ts = row.try_get::<Option<i64>, _>("end_ts")?.unwrap_or(0);
        crate::database::logical_recordings::insert_fragment_close_event(
            &mut tx,
            recording_session_id,
            end_ts,
            row.try_get("channel_id")?,
            audio_file_id,
            row.try_get("segment_index")?,
            "setup_failed",
        )
        .await?;
        crate::database::logical_recordings::finalize_setup_failed_session(
            &mut tx,
            recording_session_id,
            end_ts,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn finalize_recording(
    pool: &Pool<Postgres>,
    audio_file_id: i64,
    owner_instance_id: &str,
    duration_ms: i64,
    finalize_reason_id: i32,
) -> DbResult<()> {
    let duration_ms = duration_ms.max(0);
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "UPDATE audio_files
            SET end_ts = audio_files.start_ts + $1,
                recording_heartbeat_at = NULL,
                finalize_reason_id = $4
          WHERE id = $2
            AND recording_owner_instance_id = $3
            AND end_ts IS NULL
         RETURNING recording_session_id, segment_index, channel_id, end_ts",
    )
    .bind(duration_ms)
    .bind(audio_file_id)
    .bind(owner_instance_id)
    .bind(finalize_reason_id)
    .fetch_optional(&mut *tx)
    .await?;
    let row = row.ok_or(DbError::UnexpectedRows {
        operation: "finalize recording",
        expected: 1,
        actual: 0,
    })?;
    if let Some(recording_session_id) = row.try_get::<Option<i64>, _>("recording_session_id")? {
        crate::database::logical_recordings::insert_fragment_close_event(
            &mut tx,
            recording_session_id,
            row.try_get::<Option<i64>, _>("end_ts")?.unwrap_or(0),
            row.try_get("channel_id")?,
            audio_file_id,
            row.try_get("segment_index")?,
            finalize_reason_name(finalize_reason_id),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

fn finalize_reason_name(finalize_reason_id: i32) -> &'static str {
    match finalize_reason_id {
        1 => "writer_close",
        2 => "writer_error",
        3 => "zombie_reaped",
        4 => "file_create",
        5 => "writer_init",
        _ => "unknown",
    }
}

pub async fn last_reap_ts(pool: &Pool<Postgres>) -> DbResult<i64> {
    let last_reap_ts =
        sqlx::query_scalar!("SELECT last_reap_ts FROM bot_reaper_state WHERE id = 1")
            .fetch_optional(pool)
            .await?
            .unwrap_or(0);

    Ok(last_reap_ts)
}

pub async fn zombie_recordings(pool: &Pool<Postgres>) -> DbResult<Vec<ZombieRecording>> {
    let stale_after_seconds = crate::heartbeat::STALE_AFTER_SECONDS as f64;
    let zombies = sqlx::query_as!(
        ZombieRecording,
        r#"SELECT id AS "audio_file_id!", file_name, guild_id, channel_id, start_ts
           FROM audio_files
          WHERE end_ts IS NULL
            AND NOT EXISTS (
                SELECT 1
                  FROM bot_instances bi
                 WHERE bi.instance_id = audio_files.recording_owner_instance_id
                   AND audio_files.recording_heartbeat_at > now() - ($1::double precision * interval '1 second')
                   AND bi.heartbeat_at > now() - ($1::double precision * interval '1 second')
                   AND bi.state <> 'stopped'
            )"#,
        stale_after_seconds
    )
    .fetch_all(pool)
    .await?;

    Ok(zombies)
}

pub async fn delete_zombie_recordings(pool: &Pool<Postgres>) -> DbResult<u64> {
    let stale_after_seconds = crate::heartbeat::STALE_AFTER_SECONDS as f64;
    let result = sqlx::query!(
        "DELETE FROM audio_files
          WHERE end_ts IS NULL
            AND NOT EXISTS (
                SELECT 1
                  FROM bot_instances bi
                 WHERE bi.instance_id = audio_files.recording_owner_instance_id
                   AND audio_files.recording_heartbeat_at > now() - ($1::double precision * interval '1 second')
                   AND bi.heartbeat_at > now() - ($1::double precision * interval '1 second')
                   AND bi.state <> 'stopped'
            )",
        stale_after_seconds
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

pub async fn mark_zombie_recordings_reaped(pool: &Pool<Postgres>) -> DbResult<u64> {
    let stale_after_seconds = crate::heartbeat::STALE_AFTER_SECONDS as f64;
    let result = sqlx::query!(
        "UPDATE audio_files
            SET end_ts = start_ts,
                reaped = TRUE,
                recording_heartbeat_at = NULL,
                finalize_reason_id = $1
          WHERE end_ts IS NULL
            AND NOT EXISTS (
                SELECT 1
                  FROM bot_instances bi
                 WHERE bi.instance_id = audio_files.recording_owner_instance_id
                   AND audio_files.recording_heartbeat_at > now() - ($2::double precision * interval '1 second')
                   AND bi.heartbeat_at > now() - ($2::double precision * interval '1 second')
                   AND bi.state <> 'stopped'
            )",
        FINALIZE_REASON_ZOMBIE_REAPED,
        stale_after_seconds
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

pub async fn bump_last_reap_ts(pool: &Pool<Postgres>, now_ms: i64) -> DbResult<()> {
    sqlx::query!(
        "INSERT INTO bot_reaper_state (id, last_reap_ts)
         VALUES (1, $1)
         ON CONFLICT (id) DO UPDATE SET last_reap_ts = EXCLUDED.last_reap_ts",
        now_ms
    )
    .execute(pool)
    .await?;

    Ok(())
}
