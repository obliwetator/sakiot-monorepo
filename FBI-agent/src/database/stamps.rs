use sqlx::{Pool, Postgres};

use crate::database::DbResult;

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
    note: Option<&str>,
) -> DbResult<i64> {
    let row = sqlx::query!(
        r#"INSERT INTO stamps
             (guild_id, channel_id, target_user_id, stamper_user_id,
              stamp_ts, offset_ms, audio_file_id, note)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING id"#,
        guild_id,
        channel_id,
        target_user_id,
        stamper_user_id,
        stamp_ts,
        offset_ms,
        audio_file_id,
        note,
    )
    .fetch_one(pool)
    .await?;

    Ok(row.id)
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

pub async fn active_audio_file_id_for_stamp(
    pool: &Pool<Postgres>,
    target_user_id: i64,
    guild_id: i64,
    channel_id: i64,
    stamp_ts: i64,
) -> DbResult<Option<i64>> {
    let stale_after_seconds = crate::heartbeat::STALE_AFTER_SECONDS as f64;
    let active_file_id = sqlx::query_scalar!(
        r#"SELECT id
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
        target_user_id,
        guild_id,
        channel_id,
        stamp_ts,
        stale_after_seconds,
    )
    .fetch_optional(pool)
    .await?;

    Ok(active_file_id)
}
