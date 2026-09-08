//! Run-level regression tests for [`RecorderActor`].
//!
//! These drive the real run loop against a [`RecorderEnv`] built from an empty
//! cache and a dummy HTTP client, so they need neither a live Discord gateway
//! nor a database. They exist because the actor used to be untestable, which is
//! why a teardown that awaited its own termination shipped unnoticed.

use std::sync::{Arc, atomic::Ordering};

use serenity::{
    model::id::{ChannelId, GuildId},
    prelude::{RwLock, TypeMap},
};
use sqlx::postgres::{PgPool, PgPoolOptions};

use super::{RecorderCommand, RecorderEnv, RecorderHandle};
use crate::events::voice::coordinator::{VoiceCoordinatorRegistry, VoiceCoordinatorRegistryKey};
use crate::events::voice_receiver::{
    RecordingCoordinatorRegistry, RecordingCoordinatorRegistryKey,
};

const GUILD_BASE: u64 = 9_000_000_000_000_000;
const TERMINATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The manager-missing path never reaches the database, so a lazy pool keeps
/// these tests hermetic.
fn lazy_pool() -> PgPool {
    PgPoolOptions::new()
        .connect_lazy("postgres://postgres:password@127.0.0.1:54320/sakiot_rouvas")
        .expect("lazy pool construction does not connect")
}

/// Spawns a *registered* actor. Registration matters: teardown reaches the
/// actor through the registry, so an unregistered actor cannot reproduce the
/// self-await this suite exists to guard.
async fn spawn(
    data: &Arc<RwLock<TypeMap>>,
    metrics: Arc<crate::BotMetrics>,
    guild: u64,
) -> RecorderHandle {
    let registry = Arc::new(RecordingCoordinatorRegistry::default());
    {
        let mut data_write = data.write().await;
        data_write.insert::<RecordingCoordinatorRegistryKey>(Arc::clone(&registry));
    }

    registry
        .get_or_create(
            lazy_pool(),
            RecorderEnv::for_test(Arc::clone(data)),
            GuildId::new(guild),
            ChannelId::new(1),
            metrics,
        )
        .await
}

/// A recoverable disconnect whose 60 s deadline is already in the past, so the
/// next one-second deadline tick tears the session down.
async fn backdated_recoverable_disconnect(handle: &RecorderHandle) {
    let at_ms = chrono::Utc::now().timestamp_millis() - 600_000;
    handle
        .send_control(RecorderCommand::DriverDisconnected {
            should_count_disconnect: true,
            recoverable: true,
            finalize_empty_channel: false,
            at_ms,
        })
        .await;
}

#[tokio::test]
async fn recoverable_disconnect_timeout_terminates_the_actor() {
    let data = Arc::new(RwLock::new(TypeMap::new()));
    let metrics = Arc::new(crate::BotMetrics::default());
    let handle = spawn(&data, Arc::clone(&metrics), GUILD_BASE + 1).await;

    backdated_recoverable_disconnect(&handle).await;

    tokio::time::timeout(TERMINATION_TIMEOUT, handle.wait_terminated())
        .await
        .expect("the actor must terminate after a recoverable disconnect times out");

    assert!(handle.is_stopping());
    assert_eq!(metrics.recovery_teardowns.load(Ordering::Relaxed), 1);
    assert_eq!(
        metrics
            .recovery_teardown_manager_missing
            .load(Ordering::Relaxed),
        1,
        "an absent Songbird manager must be counted, not silently ignored"
    );
}

#[tokio::test]
async fn teardown_with_an_empty_songbird_manager_also_terminates() {
    let data = Arc::new(RwLock::new(TypeMap::new()));
    {
        let mut data_write = data.write().await;
        data_write.insert::<songbird::SongbirdKey>(songbird::Songbird::serenity());
    }
    let metrics = Arc::new(crate::BotMetrics::default());
    let handle = spawn(&data, Arc::clone(&metrics), GUILD_BASE + 2).await;

    backdated_recoverable_disconnect(&handle).await;

    tokio::time::timeout(TERMINATION_TIMEOUT, handle.wait_terminated())
        .await
        .expect("the actor must terminate when the manager holds no call");

    assert_eq!(metrics.recovery_teardowns.load(Ordering::Relaxed), 1);
    assert_eq!(
        metrics
            .recovery_teardown_manager_missing
            .load(Ordering::Relaxed),
        0
    );
}

#[tokio::test]
async fn reconnect_after_shutdown_gets_a_fresh_actor() {
    let data = Arc::new(RwLock::new(TypeMap::new()));
    let metrics = Arc::new(crate::BotMetrics::default());
    let registry = Arc::new(RecordingCoordinatorRegistry::default());
    let guild = GuildId::new(GUILD_BASE + 3);

    let first = registry
        .get_or_create(
            lazy_pool(),
            RecorderEnv::for_test(Arc::clone(&data)),
            guild,
            ChannelId::new(1),
            Arc::clone(&metrics),
        )
        .await;
    let first_id = first.actor_id();

    first.request_shutdown(0);
    tokio::time::timeout(TERMINATION_TIMEOUT, first.wait_terminated())
        .await
        .expect("the actor must terminate on shutdown");

    let second = registry
        .get_or_create(
            lazy_pool(),
            RecorderEnv::for_test(Arc::clone(&data)),
            guild,
            ChannelId::new(1),
            metrics,
        )
        .await;

    assert!(
        !second.same_actor(&first_id),
        "a reconnect must not be handed the terminated actor"
    );
}

/// The deadlock's worst symptom was not just a stuck actor: teardown holds the
/// guild's operation mutex, so every later voice operation for that guild
/// blocked behind it forever.
#[tokio::test]
async fn actor_exit_releases_the_guild_operation_lock() {
    let data = Arc::new(RwLock::new(TypeMap::new()));
    let coordinators = Arc::new(VoiceCoordinatorRegistry::default());
    {
        let mut data_write = data.write().await;
        data_write.insert::<VoiceCoordinatorRegistryKey>(Arc::clone(&coordinators));
    }
    let metrics = Arc::new(crate::BotMetrics::default());
    let guild = GUILD_BASE + 4;
    let handle = spawn(&data, metrics, guild).await;

    backdated_recoverable_disconnect(&handle).await;

    tokio::time::timeout(TERMINATION_TIMEOUT, handle.wait_terminated())
        .await
        .expect("the actor must terminate after a recoverable disconnect times out");

    let coordinator = coordinators.guild(GuildId::new(guild));
    let _guard = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        coordinator.operation.lock(),
    )
    .await
    .expect("teardown must release the per-guild operation lock when the actor exits");
}
