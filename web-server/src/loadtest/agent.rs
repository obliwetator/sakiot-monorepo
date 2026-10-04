//! The synthetic bot instance behind the fixture guilds.
//!
//! Liveness and presence come from bot heartbeats: a recording is live only
//! while its owner instance heartbeats, and presence is "available" only
//! while a running instance owns the guild's projection. This task heartbeats
//! as `loadtest-agent-<port>` the way the agent does (every 10 s), owns every
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
        let closed = super::live::close_orphans(&state.pool, &state.agent_id, &live).await?;
        if closed > 0 {
            tracing::info!(closed, "closed orphaned load-test recordings");
        }
    }

    let live = state.sims.audio_file_ids();
    if !live.is_empty() {
        sqlx::query(
            "UPDATE audio_files SET recording_heartbeat_at = now()
              WHERE id = ANY($1) AND recording_owner_instance_id = $2 AND end_ts IS NULL",
        )
        .bind(&live)
        .bind(&state.agent_id)
        .execute(&state.pool)
        .await?;
    }
    Ok(())
}
