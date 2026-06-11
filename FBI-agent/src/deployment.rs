use std::{sync::Arc, time::Duration};

use serenity::model::id::{ChannelId, GuildId};
use sqlx::{Pool, Postgres};
use tokio::sync::watch;
use tracing::{debug, warn};

use crate::database::{self, DbResult};
use crate::runtime::RuntimeState;

pub const LEASE_STALE_AFTER_SECONDS: i64 = crate::heartbeat::STALE_AFTER_SECONDS;

pub use database::runtime::VoiceLeaseClaim;

pub async fn upsert_instance(pool: &Pool<Postgres>, runtime: &RuntimeState) -> DbResult<()> {
    database::runtime::upsert_instance(pool, runtime).await
}

pub fn start_heartbeat(
    pool: Pool<Postgres>,
    runtime: Arc<RuntimeState>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(err) = heartbeat_instance_and_leases(&pool, &runtime).await {
                        warn!("instance/lease heartbeat failed: {}", err);
                    }
                }
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        return;
                    }
                }
            }
        }
    })
}

pub async fn heartbeat_instance_and_leases(
    pool: &Pool<Postgres>,
    runtime: &RuntimeState,
) -> DbResult<()> {
    database::runtime::heartbeat_instance_and_leases(pool, runtime).await
}

pub async fn claim_voice_session(
    pool: &Pool<Postgres>,
    runtime: &RuntimeState,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> DbResult<VoiceLeaseClaim> {
    database::runtime::claim_voice_session(pool, runtime, guild_id, channel_id).await
}

pub async fn release_voice_session(
    pool: &Pool<Postgres>,
    runtime: &RuntimeState,
    guild_id: GuildId,
) -> DbResult<()> {
    let rows = database::runtime::release_voice_session(pool, runtime, guild_id).await?;
    debug!(guild_id = guild_id.get(), rows, "voice lease released");
    Ok(())
}

pub async fn active_lease_owner(
    pool: &Pool<Postgres>,
    guild_id: GuildId,
) -> DbResult<Option<String>> {
    database::runtime::active_lease_owner(pool, guild_id).await
}

pub async fn mark_instance_stopped(pool: &Pool<Postgres>, runtime: &RuntimeState) -> DbResult<()> {
    let cleanup = database::runtime::mark_instance_stopped(pool, runtime).await?;
    debug!(
        leases_deleted = cleanup.leases_deleted,
        recordings_closed = cleanup.recordings_closed,
        instances_updated = cleanup.instances_updated,
        "instance stopped cleanup complete"
    );
    Ok(())
}
