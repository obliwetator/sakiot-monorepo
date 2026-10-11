//! The synthetic bot instance behind the fixture guilds.
//!
//! Liveness and presence come from bot heartbeats: a recording is live only
//! while its owner instance and its guild's recorder heartbeat, and presence
//! is "available" only while a running instance owns the guild's projection.
//! This task heartbeats as `loadtest-agent-<port>` the way the agent does
//! (every 10 s, one recorder row per guild in voice), owns every
//! fixture guild's projection, and re-adds the fixtures to `guilds_present`,
//! which a real agent resets to its own guild list when it starts.
//!
//! The instance has no gRPC address, so the web server never routes a clip
//! playback to it. If this process stops, the heartbeats stop and a real
//! agent's reaper closes its recordings, as it would after a crash.

use std::sync::Arc;
use std::time::Duration;

use sqlx::{Postgres, Transaction};

use super::LoadtestState;

const HEARTBEAT: Duration = Duration::from_secs(10);

pub(super) async fn register(
    tx: &mut Transaction<'_, Postgres>,
    agent_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO bot_instances (instance_id, role, state, heartbeat_at, started_at)
         VALUES ($1, 'active', 'active', now(), now())
         ON CONFLICT (instance_id) DO UPDATE
            SET state = 'active', heartbeat_at = now(), updated_at = now()",
    )
    .bind(agent_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) async fn run(state: Arc<LoadtestState>) {
    let mut interval = tokio::time::interval(HEARTBEAT);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut recovered = false;
    loop {
        interval.tick().await;
        match beat(&state, !recovered).await {
            Ok(()) => recovered = true,
            Err(error) => tracing::warn!(?error, "load-test agent heartbeat failed"),
        }
    }
}

async fn beat(state: &LoadtestState, recover: bool) -> Result<(), sqlx::Error> {
    let guilds = super::fixtures::fixture_guilds(&state.pool).await?;
    if guilds.is_empty() && state.sims.is_empty() {
        return Ok(());
    }
    let mut tx = state.pool.begin().await?;
    register(&mut tx, &state.agent_id).await?;
    sqlx::query(
        "INSERT INTO guilds_present (guild_id) SELECT * FROM UNNEST($1::bigint[])
         ON CONFLICT DO NOTHING",
    )
    .bind(&guilds)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE guild_projection_state
            SET owner_instance_id = $2, presence_synced_at = now(), updated_at = now()
          WHERE guild_id = ANY($1) AND owner_instance_id IS DISTINCT FROM $2",
    )
    .bind(&guilds)
    .bind(&state.agent_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    if recover {
        // A previous process's simulations died with it; close what they
        // left open, ending each recording at its last heartbeat.
        let live = state.sims.audio_file_ids();
        let closed =
            super::live::close_orphans(&state.pool, &state.agent_id, &state.recorder_prefix, &live)
                .await?;
        if closed > 0 {
            tracing::info!(closed, "closed orphaned load-test recordings");
        }
    }

    // One recorder per guild, as the agent heartbeats: the cost follows the
    // guilds in voice, not the recordings in them.
    let recorders: Vec<(String, i64)> = state
        .sims
        .live_guilds()
        .into_iter()
        .map(|guild_id| (state.recorder_id(guild_id), guild_id))
        .collect();
    if !recorders.is_empty() {
        let mut tx = state.pool.begin().await?;
        heartbeat_recorders(&mut tx, &state.agent_id, &recorders).await?;
        tx.commit().await?;
    }
    Ok(())
}

pub(super) async fn heartbeat_recorders(
    tx: &mut Transaction<'_, Postgres>,
    agent_id: &str,
    recorders: &[(String, i64)],
) -> Result<(), sqlx::Error> {
    let (ids, guilds): (Vec<String>, Vec<i64>) = recorders.iter().cloned().unzip();
    sqlx::query(
        "INSERT INTO recorder_heartbeats (recorder_id, owner_instance_id, guild_id)
         SELECT recorder_id, $2, guild_id FROM UNNEST($1::text[], $3::bigint[])
                AS r (recorder_id, guild_id)
         ON CONFLICT (recorder_id) DO UPDATE SET heartbeat_at = now()",
    )
    .bind(&ids)
    .bind(agent_id)
    .bind(&guilds)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
