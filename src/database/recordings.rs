use chrono::Datelike;
use sakiot_paths::{DataRoots, RecordingKey};
use sqlx::{Pool, Postgres};

use crate::database::DbResult;
use crate::database::error::expect_rows;

#[cfg(test)]
pub const FINALIZE_REASON_WRITER_CLOSE: i32 = 1;
pub const FINALIZE_REASON_ZOMBIE_REAPED: i32 = 3;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RecordingHandle {
    pub audio_file_id: i64,
    pub file_name: String,
    pub path: String,
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
    let file_name = RecordingKey::stem_for(now.timestamp_millis(), user_id);
    let key = RecordingKey::new(
        guild_id,
        channel_id,
        now.year(),
        now.month(),
        file_name.clone(),
    );

    let recording_root = DataRoots::from_env().recordings_str();
    let dir_path = key.recording_dir(&recording_root);
    let combined_path = key.recording_dir(&recording_root).join(&file_name);
    std::fs::create_dir_all(&dir_path)?;

    let row = sqlx::query!(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month, start_ts, end_ts,
             recording_owner_instance_id, recording_heartbeat_at)
         VALUES
            ($1, $2, $3, $4, $5, $6, $7, NULL, $8, now())
         RETURNING id",
        file_name,
        guild_id,
        channel_id,
        user_id,
        now.year(),
        now.month() as i32,
        now.timestamp_millis(),
        owner_instance_id,
    )
    .fetch_one(pool)
    .await?;

    Ok(RecordingHandle {
        audio_file_id: row.id,
        file_name,
        path: combined_path.to_string_lossy().into_owned(),
    })
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
    let result = sqlx::query!(
        "UPDATE audio_files
            SET end_ts = COALESCE(end_ts, start_ts),
                reaped = TRUE,
                recording_heartbeat_at = NULL,
                finalize_reason_id = $3
          WHERE id = $1
            AND recording_owner_instance_id = $2
            AND end_ts IS NULL",
        audio_file_id,
        owner_instance_id,
        finalize_reason_id,
    )
    .execute(pool)
    .await?;

    expect_rows(result, 1, "mark recording setup failed")?;
    Ok(())
}

pub async fn finalize_recording(
    pool: &Pool<Postgres>,
    audio_file_id: i64,
    owner_instance_id: &str,
    duration_ms: i64,
    finalize_reason_id: i32,
) -> DbResult<()> {
    let result = sqlx::query!(
        "UPDATE audio_files
            SET end_ts = audio_files.start_ts + $1,
                recording_heartbeat_at = NULL,
                finalize_reason_id = $4
          WHERE id = $2
            AND recording_owner_instance_id = $3
            AND end_ts IS NULL",
        duration_ms.max(0),
        audio_file_id,
        owner_instance_id,
        finalize_reason_id,
    )
    .execute(pool)
    .await?;

    expect_rows(result, 1, "finalize recording")?;
    Ok(())
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
            AND NOT EXISTS (
                SELECT 1
                  FROM bot_instances bi
                 WHERE bi.instance_id = audio_files.recording_owner_instance_id
                   AND audio_files.recording_heartbeat_at > now() - interval '120 seconds'
                   AND bi.heartbeat_at > now() - interval '120 seconds'
                   AND bi.state <> 'stopped'
            )"#
    )
    .fetch_all(pool)
    .await?;

    Ok(zombies)
}

pub async fn delete_zombie_recordings(pool: &Pool<Postgres>) -> DbResult<u64> {
    let result = sqlx::query!(
        "DELETE FROM audio_files
          WHERE end_ts IS NULL
            AND NOT EXISTS (
                SELECT 1
                  FROM bot_instances bi
                 WHERE bi.instance_id = audio_files.recording_owner_instance_id
                   AND audio_files.recording_heartbeat_at > now() - interval '120 seconds'
                   AND bi.heartbeat_at > now() - interval '120 seconds'
                   AND bi.state <> 'stopped'
            )"
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
                recording_heartbeat_at = NULL,
                finalize_reason_id = $1
          WHERE end_ts IS NULL
            AND NOT EXISTS (
                SELECT 1
                  FROM bot_instances bi
                 WHERE bi.instance_id = audio_files.recording_owner_instance_id
                   AND audio_files.recording_heartbeat_at > now() - interval '120 seconds'
                   AND bi.heartbeat_at > now() - interval '120 seconds'
                   AND bi.state <> 'stopped'
            )",
        FINALIZE_REASON_ZOMBIE_REAPED,
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
