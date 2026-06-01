use sqlx::{Pool, Postgres};

use crate::database::DbResult;

pub async fn insert_receiver_voice_event(
    pool: &Pool<Postgres>,
    guild_id: i64,
    user_id: i64,
    ssrc: i64,
    event_type_id: i32,
    details: &str,
) -> DbResult<()> {
    sqlx::query(
        "INSERT INTO voice_events (guild_id, user_id, ssrc, event_type_id, details)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(ssrc)
    .bind(event_type_id)
    .bind(details)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_voice_state_event(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_id: Option<i64>,
    user_id: i64,
    event_type_id: i32,
) -> DbResult<()> {
    sqlx::query!(
        "INSERT INTO voice_state_events (guild_id, channel_id, user_id, event_type_id)
         VALUES ($1, $2, $3, $4)",
        guild_id,
        channel_id,
        user_id,
        event_type_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_voice_connection_event(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_id: Option<i64>,
    owner_instance_id: Option<&str>,
    event_type: &str,
    reason: Option<&str>,
    details: Option<&str>,
) -> DbResult<()> {
    sqlx::query(
        "INSERT INTO voice_connection_events
            (guild_id, channel_id, owner_instance_id, event_type, reason, details)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(guild_id)
    .bind(channel_id)
    .bind(owner_instance_id)
    .bind(event_type)
    .bind(reason)
    .bind(details)
    .execute(pool)
    .await?;

    Ok(())
}
