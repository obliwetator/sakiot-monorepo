use sqlx::{Pool, Postgres};

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

/// Who writes a recording: the agent instance, and the recorder actor in it
/// whose heartbeat keeps the file live.
#[derive(Debug, Clone, Copy)]
pub struct RecordingOwner<'a> {
    pub instance_id: &'a str,
    pub recorder_id: &'a str,
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
    owner: RecordingOwner<'_>,
) -> DbResult<RecordingHandle> {
    crate::database::logical_recordings::create_fragment(
        pool, guild_id, channel_id, user_id, now, owner,
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
    owner: RecordingOwner<'_>,
    recording_root: &std::path::Path,
) -> DbResult<RecordingHandle> {
    crate::database::logical_recordings::create_fragment_in(
        pool,
        guild_id,
        channel_id,
        user_id,
        now,
        owner,
        recording_root,
    )
    .await
}

/// Marks a recorder actor alive. Every file it creates carries its id, so one
/// row per actor keeps all of its files live (`live_audio_files`).
pub async fn heartbeat_recorder(
    pool: &Pool<Postgres>,
    recorder_id: &str,
    owner_instance_id: &str,
    guild_id: i64,
) -> DbResult<()> {
    sqlx::query!(
        "INSERT INTO recorder_heartbeats (recorder_id, owner_instance_id, guild_id)
         VALUES ($1, $2, $3)
         ON CONFLICT (recorder_id) DO UPDATE SET heartbeat_at = now()",
        recorder_id,
        owner_instance_id,
        guild_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// A recorder actor exited: its files stop counting as live at once instead
/// of after `STALE_AFTER_SECONDS`.
pub async fn end_recorder(pool: &Pool<Postgres>, recorder_id: &str) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM recorder_heartbeats WHERE recorder_id = $1",
        recorder_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Recorder rows that stopped heartbeating no longer keep anything live.
pub async fn delete_stale_recorders(pool: &Pool<Postgres>) -> DbResult<u64> {
    let stale_after_seconds = crate::heartbeat::STALE_AFTER_SECONDS as f64;
    Ok(sqlx::query!(
        "DELETE FROM recorder_heartbeats
          WHERE heartbeat_at <= now() - ($1::double precision * interval '1 second')",
        stale_after_seconds
    )
    .execute(pool)
    .await?
    .rows_affected())
}

pub async fn mark_recording_setup_failed(
    pool: &Pool<Postgres>,
    audio_file_id: i64,
    owner_instance_id: &str,
    finalize_reason_id: i32,
) -> DbResult<()> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query!(
        "UPDATE audio_files
            SET end_ts = COALESCE(end_ts, start_ts),
                reaped = TRUE,
                finalize_reason_id = $3
          WHERE id = $1
            AND recording_owner_instance_id = $2
            AND end_ts IS NULL
         RETURNING recording_session_id, segment_index, channel_id, end_ts",
        audio_file_id,
        owner_instance_id,
        finalize_reason_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    let row = row.ok_or(DbError::UnexpectedRows {
        operation: "mark recording setup failed",
        expected: 1,
        actual: 0,
    })?;
    if let Some(recording_session_id) = row.recording_session_id {
        crate::database::logical_recordings::insert_fragment_close_event(
            &mut tx,
            recording_session_id,
            row.end_ts.unwrap_or(0),
            row.channel_id,
            audio_file_id,
            row.segment_index,
            "setup_failed",
        )
        .await?;
        crate::database::logical_recordings::finalize_setup_failed_session(
            &mut tx,
            recording_session_id,
            row.end_ts.unwrap_or(0),
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
    let row = sqlx::query!(
        "UPDATE audio_files
            SET end_ts = audio_files.start_ts + $1,
                finalize_reason_id = $4
          WHERE id = $2
            AND recording_owner_instance_id = $3
            AND end_ts IS NULL
         RETURNING recording_session_id, segment_index, channel_id, end_ts",
        duration_ms,
        audio_file_id,
        owner_instance_id,
        finalize_reason_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    let row = row.ok_or(DbError::UnexpectedRows {
        operation: "finalize recording",
        expected: 1,
        actual: 0,
    })?;
    if let Some(recording_session_id) = row.recording_session_id {
        crate::database::logical_recordings::insert_fragment_close_event(
            &mut tx,
            recording_session_id,
            row.end_ts.unwrap_or(0),
            row.channel_id,
            audio_file_id,
            row.segment_index,
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
    let zombies = sqlx::query_as!(
        ZombieRecording,
        r#"SELECT id AS "audio_file_id!", file_name, guild_id, channel_id, start_ts
           FROM audio_files
          WHERE end_ts IS NULL
            AND NOT EXISTS (SELECT 1 FROM live_audio_files live WHERE live.id = audio_files.id)"#
    )
    .fetch_all(pool)
    .await?;

    Ok(zombies)
}

pub async fn delete_zombie_recordings(pool: &Pool<Postgres>) -> DbResult<u64> {
    let result = sqlx::query!(
        "DELETE FROM audio_files
          WHERE end_ts IS NULL
            AND NOT EXISTS (SELECT 1 FROM live_audio_files live WHERE live.id = audio_files.id)"
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

pub async fn mark_zombie_recordings_reaped(pool: &Pool<Postgres>) -> DbResult<u64> {
    let result = sqlx::query!(
        "UPDATE audio_files
            SET end_ts = start_ts,
                reaped = TRUE,
                finalize_reason_id = $1
          WHERE end_ts IS NULL
            AND NOT EXISTS (SELECT 1 FROM live_audio_files live WHERE live.id = audio_files.id)",
        FINALIZE_REASON_ZOMBIE_REAPED
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
