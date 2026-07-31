use sqlx::{Pool, Postgres};

use crate::database::DbResult;

#[derive(Debug, Clone, Copy, Eq, PartialEq, sqlx::FromRow)]
pub struct ActiveStampRecording {
    pub audio_file_id: i64,
    pub recording_session_id: Option<i64>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "stamps insert mirrors command payload"
)]
pub async fn create_stamp(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_id: i64,
    target_user_id: i64,
    stamper_user_id: i64,
    stamp_ts: i64,
    offset_ms: i32,
    audio_file_id: Option<i64>,
    recording_session_id: Option<i64>,
    note: Option<&str>,
) -> DbResult<i64> {
    let stamp_id = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO stamps
             (guild_id, channel_id, target_user_id, stamper_user_id,
              stamp_ts, offset_ms, audio_file_id, recording_session_id, note)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           RETURNING id"#,
    )
    .bind(guild_id)
    .bind(channel_id)
    .bind(target_user_id)
    .bind(stamper_user_id)
    .bind(stamp_ts)
    .bind(offset_ms)
    .bind(audio_file_id)
    .bind(recording_session_id)
    .bind(note)
    .fetch_one(pool)
    .await?;

    Ok(stamp_id)
}

pub async fn latest_stamp_ts(
    pool: &Pool<Postgres>,
    guild_id: i64,
    target_user_id: i64,
) -> DbResult<Option<i64>> {
    let last_ts = sqlx::query_scalar!(
        r#"SELECT MAX(stamp_ts)
             FROM stamps
            WHERE guild_id = $1
              AND target_user_id = $2"#,
        guild_id,
        target_user_id,
    )
    .fetch_one(pool)
    .await?;

    Ok(last_ts)
}

pub async fn active_recording_for_stamp(
    pool: &Pool<Postgres>,
    target_user_id: i64,
    guild_id: i64,
    channel_id: i64,
    stamp_ts: i64,
) -> DbResult<Option<ActiveStampRecording>> {
    let stale_after_seconds = crate::heartbeat::STALE_AFTER_SECONDS as f64;
    let active_recording = sqlx::query_as::<_, ActiveStampRecording>(
        r#"SELECT id AS audio_file_id,
                  recording_session_id
             FROM audio_files
            WHERE user_id = $1
              AND guild_id = $2
              AND channel_id = $3
              AND start_ts <= $4
              AND end_ts IS NULL
              AND EXISTS (
                  SELECT 1
                    FROM bot_instances bi
                   WHERE bi.instance_id = audio_files.recording_owner_instance_id
                     AND audio_files.recording_heartbeat_at > now() - ($5::double precision * interval '1 second')
                     AND bi.heartbeat_at > now() - ($5::double precision * interval '1 second')
                     AND bi.state <> 'stopped'
              )
            ORDER BY start_ts DESC
            LIMIT 1"#,
    )
    .bind(target_user_id)
    .bind(guild_id)
    .bind(channel_id)
    .bind(stamp_ts)
    .bind(stale_after_seconds)
    .fetch_optional(pool)
    .await?;

    Ok(active_recording)
}
