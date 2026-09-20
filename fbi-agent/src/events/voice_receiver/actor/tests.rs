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

/// A pool that is already closed, so every query fails immediately and
/// deterministically. The policy path must fail closed without reaching a real
/// database (and without depending on one being reachable).
async fn closed_pool() -> PgPool {
    let pool = lazy_pool();
    pool.close().await;
    pool
}

/// Polls until `predicate` holds so tests do not race the actor's command loop.
async fn wait_until(mut predicate: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + TERMINATION_TIMEOUT;
    while !predicate() {
        assert!(
            std::time::Instant::now() < deadline,
            "condition was not reached before the deadline"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Spawns a *registered* actor. Registration matters: teardown reaches the
/// actor through the registry, so an unregistered actor cannot reproduce the
/// self-await this suite exists to guard.
async fn spawn_with_pool(
    data: &Arc<RwLock<TypeMap>>,
    metrics: Arc<crate::BotMetrics>,
    guild: u64,
    channel: u64,
    pool: PgPool,
) -> RecorderHandle {
    let registry = Arc::new(RecordingCoordinatorRegistry::default());
    {
        let mut data_write = data.write().await;
        data_write.insert::<RecordingCoordinatorRegistryKey>(Arc::clone(&registry));
    }

    registry
        .get_or_create(
            pool,
            RecorderEnv::for_test(Arc::clone(data)),
            GuildId::new(guild),
            ChannelId::new(channel),
            metrics,
        )
        .await
}

async fn spawn(
    data: &Arc<RwLock<TypeMap>>,
    metrics: Arc<crate::BotMetrics>,
    guild: u64,
) -> RecorderHandle {
    spawn_with_pool(data, metrics, guild, 1, lazy_pool()).await
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

/// A policy check that cannot reach the database must fail closed for privacy
/// *without* killing the recorder actor: terminating it left the guild's
/// Songbird receiver attached to a dead handle, so recording could not resume
/// until the call was torn down and rejoined from scratch.
#[tokio::test]
async fn policy_check_failure_suspends_recording_without_killing_the_actor() {
    let data = Arc::new(RwLock::new(TypeMap::new()));
    let metrics = Arc::new(crate::BotMetrics::default());
    let handle = spawn_with_pool(
        &data,
        Arc::clone(&metrics),
        GUILD_BASE + 5,
        1,
        closed_pool().await,
    )
    .await;

    handle.try_send_tick(chrono::Utc::now().timestamp_millis(), vec![]);

    wait_until(|| handle.stats().policy_suspended()).await;
    assert_eq!(
        metrics.recording_policy_suspensions.load(Ordering::Relaxed),
        1
    );
    assert!(
        !handle.is_stopping(),
        "a policy violation must not stop the actor"
    );
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            handle.wait_terminated()
        )
        .await
        .is_err(),
        "the actor must stay alive after a failed policy check"
    );
}

#[tokio::test]
async fn policy_suspension_is_idempotent_and_the_actor_still_terminates() {
    let data = Arc::new(RwLock::new(TypeMap::new()));
    let metrics = Arc::new(crate::BotMetrics::default());
    let handle = spawn_with_pool(
        &data,
        Arc::clone(&metrics),
        GUILD_BASE + 6,
        1,
        closed_pool().await,
    )
    .await;

    let now = chrono::Utc::now().timestamp_millis();
    handle.try_send_tick(now, vec![]);
    wait_until(|| handle.stats().policy_suspended()).await;

    handle.try_send_tick(now + 2_000, vec![]);
    handle.try_send_tick(now + 4_000, vec![]);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        metrics.recording_policy_suspensions.load(Ordering::Relaxed),
        1,
        "an already-suspended actor must not count a second suspension"
    );

    handle.request_shutdown(now + 5_000);
    tokio::time::timeout(TERMINATION_TIMEOUT, handle.wait_terminated())
        .await
        .expect("a suspended actor must still terminate on shutdown");
}

/// An externally moved bot produces no planned handoff. The driver's own
/// connect event is the only authoritative source for the channel it is in,
/// and the recorder's policy checks and fragment metadata depend on it.
#[tokio::test]
async fn driver_connect_repoints_the_recorder_to_the_actual_channel() {
    let data = Arc::new(RwLock::new(TypeMap::new()));
    let metrics = Arc::new(crate::BotMetrics::default());
    let handle = spawn_with_pool(&data, metrics, GUILD_BASE + 7, 1, closed_pool().await).await;

    assert_eq!(handle.current_channel_id(), ChannelId::new(1));
    handle
        .send_control(RecorderCommand::DriverConnected {
            reconnect: false,
            channel_id: ChannelId::new(99),
            at_ms: chrono::Utc::now().timestamp_millis(),
        })
        .await;

    wait_until(|| handle.current_channel_id() == ChannelId::new(99)).await;
}

/// End-to-end policy round trip against a real database: an excluded channel
/// suspends recording while the actor stays alive, and removing the exclusion
/// resumes it in the same call.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn excluded_channel_suspends_and_reenabling_resumes_without_terminating(pool: PgPool) {
    let guild = GUILD_BASE + 8;
    let channel = 4_242_u64;
    sqlx::query(
        "INSERT INTO guild_recording_policy (guild_id, excluded_channel_ids) VALUES ($1, ARRAY[$2::bigint])",
    )
    .bind(guild as i64)
    .bind(channel as i64)
    .execute(&pool)
    .await
    .expect("policy seed");

    let data = Arc::new(RwLock::new(TypeMap::new()));
    let metrics = Arc::new(crate::BotMetrics::default());
    let handle = spawn_with_pool(&data, metrics, guild, channel, pool.clone()).await;

    let now = chrono::Utc::now().timestamp_millis();
    handle.try_send_tick(now, vec![]);
    wait_until(|| handle.stats().policy_suspended()).await;
    assert!(!handle.is_stopping());

    sqlx::query("DELETE FROM guild_recording_policy WHERE guild_id = $1")
        .bind(guild as i64)
        .execute(&pool)
        .await
        .expect("policy removal");

    handle.try_send_tick(now + 2_000, vec![]);
    wait_until(|| !handle.stats().policy_suspended()).await;
    assert!(!handle.is_stopping());

    handle.request_shutdown(now + 3_000);
    tokio::time::timeout(TERMINATION_TIMEOUT, handle.wait_terminated())
        .await
        .expect("the actor must terminate after the policy round trip");
}
