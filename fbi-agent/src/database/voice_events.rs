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
    previous_channel_id: Option<i64>,
) -> DbResult<()> {
    sqlx::query(
        "INSERT INTO voice_state_events
            (guild_id, channel_id, previous_channel_id, user_id, event_type_id)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(guild_id)
    .bind(channel_id)
    .bind(previous_channel_id)
    .bind(user_id)
    .bind(event_type_id)
    .execute(pool)
    .await?;

    Ok(())
}

#[derive(Debug)]
pub struct VoiceConnectionEvent<'a> {
    pub operation_id: &'a str,
    pub guild_id: i64,
    pub owner_instance_id: Option<&'a str>,
    pub release_id: Option<&'a str>,
    pub trigger: &'a str,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub from_channel_id: Option<i64>,
    pub to_channel_id: Option<i64>,
    pub population_snapshot: &'a serde_json::Value,
    pub outcome: &'a str,
    pub error: Option<&'a str>,
    pub fallback_outcome: Option<&'a str>,
    pub fallback_error: Option<&'a str>,
}

pub async fn insert_voice_connection_event(
    pool: &Pool<Postgres>,
    event: VoiceConnectionEvent<'_>,
) -> DbResult<()> {
    sqlx::query(
        "INSERT INTO voice_connection_events
            (operation_id, guild_id, owner_instance_id, release_id, trigger,
             started_at, completed_at, from_channel_id, to_channel_id,
             population_snapshot, outcome, error, fallback_outcome, fallback_error)
         VALUES
            ($1, $2, $3, $4, $5,
             to_timestamp($6::double precision / 1000.0),
             to_timestamp($7::double precision / 1000.0),
             $8, $9, $10::jsonb, $11, $12, $13, $14)",
    )
    .bind(event.operation_id)
    .bind(event.guild_id)
    .bind(event.owner_instance_id)
    .bind(event.release_id)
    .bind(event.trigger)
    .bind(event.started_at_ms)
    .bind(event.completed_at_ms.max(event.started_at_ms))
    .bind(event.from_channel_id)
    .bind(event.to_channel_id)
    .bind(event.population_snapshot.to_string())
    .bind(event.outcome)
    .bind(event.error)
    .bind(event.fallback_outcome)
    .bind(event.fallback_error)
    .execute(pool)
    .await?;
    Ok(())
}
