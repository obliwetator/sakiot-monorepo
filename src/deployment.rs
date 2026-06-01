use std::{sync::Arc, time::Duration};

use serenity::model::id::{ChannelId, GuildId};
use sqlx::{Pool, Postgres};
use tracing::{debug, error, warn};

use crate::runtime::RuntimeState;

pub const LEASE_STALE_AFTER_SECONDS: i64 = 120;

pub async fn upsert_instance(pool: &Pool<Postgres>, runtime: &RuntimeState) {
    if let Err(err) = sqlx::query!(
        "INSERT INTO bot_instances (instance_id, role, state, heartbeat_at, started_at)
         VALUES ($1, $2, $3, now(), now())
         ON CONFLICT (instance_id) DO UPDATE
            SET role = EXCLUDED.role,
                state = EXCLUDED.state,
                heartbeat_at = now()",
        runtime.config().instance_id,
        runtime.role().as_str(),
        if runtime.is_draining() {
            "draining"
        } else {
            "active"
        }
    )
    .execute(pool)
    .await
    {
        error!("instance upsert failed: {}", err);
    }
}

pub fn start_heartbeat(pool: Pool<Postgres>, runtime: Arc<RuntimeState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            heartbeat_instance_and_leases(&pool, &runtime).await;
        }
    });
}

pub async fn heartbeat_instance_and_leases(pool: &Pool<Postgres>, runtime: &RuntimeState) {
    upsert_instance(pool, runtime).await;

    if let Err(err) = sqlx::query!(
        "UPDATE voice_session_leases
            SET state = $2, heartbeat_at = now()
          WHERE owner_instance_id = $1",
        runtime.config().instance_id,
        if runtime.is_draining() {
            "draining"
        } else {
            "active"
        }
    )
    .execute(pool)
    .await
    {
        warn!("voice lease heartbeat failed: {}", err);
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum VoiceLeaseClaim {
    Claimed,
    OwnedByOther(String),
}

pub async fn claim_voice_session(
    pool: &Pool<Postgres>,
    runtime: &RuntimeState,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> Result<VoiceLeaseClaim, sqlx::Error> {
    let result = sqlx::query!(
        "INSERT INTO voice_session_leases
            (guild_id, channel_id, owner_instance_id, state, heartbeat_at, started_at)
         VALUES ($1, $2, $3, $4, now(), now())
         ON CONFLICT (guild_id) DO UPDATE
            SET channel_id = EXCLUDED.channel_id,
                owner_instance_id = EXCLUDED.owner_instance_id,
                state = EXCLUDED.state,
                heartbeat_at = now()
          WHERE voice_session_leases.owner_instance_id = EXCLUDED.owner_instance_id
             OR voice_session_leases.heartbeat_at <= now() - interval '120 seconds'
             OR NOT EXISTS (
                SELECT 1
                  FROM bot_instances b
                 WHERE b.instance_id = voice_session_leases.owner_instance_id
                   AND b.heartbeat_at > now() - interval '120 seconds'
                   AND b.state <> 'stopped'
             )",
        guild_id.get() as i64,
        channel_id.get() as i64,
        runtime.config().instance_id,
        if runtime.is_draining() {
            "draining"
        } else {
            "active"
        }
    )
    .execute(pool)
    .await?;

    if result.rows_affected() > 0 {
        return Ok(VoiceLeaseClaim::Claimed);
    }

    Ok(VoiceLeaseClaim::OwnedByOther(
        active_lease_owner(pool, guild_id)
            .await?
            .unwrap_or_else(|| "unknown".to_string()),
    ))
}

pub async fn release_voice_session(
    pool: &Pool<Postgres>,
    runtime: &RuntimeState,
    guild_id: GuildId,
) {
    match sqlx::query!(
        "DELETE FROM voice_session_leases
          WHERE guild_id = $1 AND owner_instance_id = $2",
        guild_id.get() as i64,
        runtime.config().instance_id
    )
    .execute(pool)
    .await
    {
        Ok(result) => {
            debug!(
                guild_id = guild_id.get(),
                rows = result.rows_affected(),
                "voice lease released"
            );
        }
        Err(err) => {
            warn!(
                guild_id = guild_id.get(),
                "voice lease release failed: {}", err
            );
        }
    }
}

pub async fn active_lease_owner(
    pool: &Pool<Postgres>,
    guild_id: GuildId,
) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query_scalar!(
        "SELECT v.owner_instance_id
           FROM voice_session_leases v
           JOIN bot_instances b ON b.instance_id = v.owner_instance_id
          WHERE v.guild_id = $1
            AND v.heartbeat_at > now() - interval '120 seconds'
            AND b.heartbeat_at > now() - interval '120 seconds'
            AND b.state <> 'stopped'
          LIMIT 1",
        guild_id.get() as i64
    )
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

pub async fn mark_instance_stopped(pool: &Pool<Postgres>, runtime: &RuntimeState) {
    if let Err(err) = sqlx::query!(
        "DELETE FROM voice_session_leases
          WHERE owner_instance_id = $1",
        runtime.config().instance_id
    )
    .execute(pool)
    .await
    {
        warn!("stopped instance voice lease cleanup failed: {}", err);
    }

    if let Err(err) = sqlx::query!(
        "UPDATE audio_files
            SET end_ts = COALESCE(end_ts, start_ts),
                reaped = CASE WHEN end_ts IS NULL THEN TRUE ELSE reaped END,
                recording_heartbeat_at = NULL
          WHERE recording_owner_instance_id = $1
            AND end_ts IS NULL",
        runtime.config().instance_id
    )
    .execute(pool)
    .await
    {
        warn!("stopped instance recording cleanup failed: {}", err);
    }

    if let Err(err) = sqlx::query!(
        "UPDATE bot_instances
            SET state = 'stopped', heartbeat_at = now()
          WHERE instance_id = $1",
        runtime.config().instance_id
    )
    .execute(pool)
    .await
    {
        warn!("instance stopped mark failed: {}", err);
    }
}
