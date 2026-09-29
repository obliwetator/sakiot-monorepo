//! Run-level regression tests for [`RecorderActor`].
//!
//! These drive the real run loop against a [`RecorderEnv`] built from an empty
//! cache and a dummy HTTP client, so they need neither a live Discord gateway
//! nor a database: queries go to an already-closed pool and fail immediately. They exist because the actor used to be untestable, which is
//! why a teardown that awaited its own termination shipped unnoticed. Where a
//! test needs state the handle cannot reach, such as an open writer, it seeds a
//! [`detached_actor`] and calls the handler directly.

use std::io::BufWriter;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use serenity::{
    model::id::{ChannelId, GuildId},
    prelude::{RwLock, TypeMap},
};
use sqlx::postgres::{PgPool, PgPoolOptions};

use super::{RecorderActor, RecorderCommand, RecorderEnv, RecorderHandle};
use crate::events::ogg_opus_writer::OggOpusWriter;
use crate::events::voice::coordinator::{VoiceCoordinatorRegistry, VoiceCoordinatorRegistryKey};
use crate::events::voice_receiver::{
    RecordingCoordinatorRegistry, RecordingCoordinatorRegistryKey,
    recordings::{RecorderStats, Recordings},
    state::UserRecording,
};

const GUILD_BASE: u64 = 9_000_000_000_000_000;
const TERMINATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Never connected; only [`closed_pool`] hands it out.
fn lazy_pool() -> PgPool {
    PgPoolOptions::new()
        .connect_lazy("postgres://postgres:password@127.0.0.1:54320/sakiot_rouvas")
        .expect("lazy pool construction does not connect")
}

/// A pool that is already closed, so every query fails immediately and
/// deterministically. Departure bookkeeping (pausing sessions, recording voice
/// events) and the policy check both query the database; an open lazy pool
/// would instead retry an unreachable server past every test deadline, and
/// against a reachable one the tests would depend on its state.
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
    spawn_with_pool(data, metrics, guild, 1, closed_pool().await).await
}

/// An actor for channel 1 that is never spawned, so a test can seed its state
/// and call its handlers directly.
fn detached_actor(guild: u64, metrics: Arc<crate::BotMetrics>, pool: PgPool) -> RecorderActor {
    let stats = Arc::new(RecorderStats::default());
    let guild_metrics = metrics.guild_metrics(guild);
    let channel_metrics = metrics.channel_metrics(guild, 1);
    RecorderActor {
        pool,
        env: RecorderEnv::for_test(Arc::new(RwLock::new(TypeMap::new()))),
        guild_id: GuildId::new(guild),
        channel_id: ChannelId::new(1),
        metrics,
        guild_metrics,
        channel_metrics,
        recording_owner_instance_id: "test-instance".to_string(),
        stats: Arc::clone(&stats),
        recordings: Recordings::new(Arc::clone(&stats)),
        link: super::Link::default(),
        current_channel_id: Arc::new(AtomicU64::new(1)),
        stopping: Arc::new(AtomicBool::new(false)),
        has_afk_channel: false,
        pending_cap_seconds: crate::database::logical_recordings::DEFAULT_PENDING_CAP_SECONDS,
        policy: super::RecordingPolicy::new(stats),
        opted_out: super::OptedOutSpeakers::default(),
        registry: None,
        actor_id: Arc::new(()),
    }
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
            closed_pool().await,
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
            closed_pool().await,
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

/// A writer whose write fails is closed once instead of being retried on every
/// tick. Retrying logged an error per speaker every 20 ms while the heartbeat
/// kept the recording's row looking healthy.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_failed_write_closes_the_writer_instead_of_retrying_every_tick() {
    const GUILD: u64 = GUILD_BASE + 9;
    const USER: u64 = 42;
    const SSRC: u32 = 7;
    let metrics = Arc::new(crate::BotMetrics::default());
    let mut actor = detached_actor(GUILD, Arc::clone(&metrics), closed_pool().await);

    // /dev/full accepts the open and fails every write with ENOSPC. The
    // BufWriter absorbs the Ogg headers, so the failure surfaces mid-tick.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    let writer = OggOpusWriter::new(BufWriter::new(file), SSRC, 0).unwrap();
    actor.recordings.insert_active(
        USER,
        SSRC,
        UserRecording {
            writer,
            audio_file_id: 1,
            recording_session_id: 1,
            file_name: "dev-full".to_string(),
            start_time: chrono::Utc::now(),
            user_id: USER,
            ssrc: SSRC,
        },
    );
    metrics.track_recording_started(&actor.guild_metrics, &actor.channel_metrics, GUILD, 1, USER);

    // Claim this second's policy check: against the closed pool it would fail
    // closed and suspend recording before any write happened.
    let at_ms = chrono::Utc::now().timestamp_millis();
    assert!(actor.policy.check_due(at_ms));

    // 200 s of silence debt overflows the BufWriter within this one tick.
    actor.handle_voice_tick(at_ms, Vec::new(), 10_000).await;

    assert!(
        !actor.recordings.has_active_ssrc(SSRC),
        "the failed writer must be closed"
    );
    assert_eq!(metrics.recordings_finished.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.active_recordings.load(Ordering::Relaxed), 0);

    actor.handle_voice_tick(at_ms + 20, Vec::new(), 0).await;
    assert_eq!(
        metrics.recordings_finished.load(Ordering::Relaxed),
        1,
        "the writer must be finalized exactly once"
    );
}

/// An opted-out speaker gets no writer, but is remembered with their SSRC so a
/// writer can reopen the moment they opt back in.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn an_opted_out_speaker_gets_no_writer(pool: PgPool) {
    const GUILD: u64 = GUILD_BASE + 10;
    let mut actor = detached_actor(GUILD, Arc::new(crate::BotMetrics::default()), pool.clone());
    crate::database::opt_outs::set_opted_out(&pool, GUILD as i64, 42, true)
        .await
        .unwrap();

    actor
        .open_user_recording(42, 7, &serenity::model::guild::Member::default())
        .await;

    assert!(!actor.recordings.has_active_ssrc(7));
    assert_eq!(actor.opted_out.user_ids(), vec![42]);
}

/// Opting out mid-session closes the open writer and pauses the user's
/// logical session within one policy check; opting back in forgets them so a
/// writer reopens (here the user is not in the cached channel, so none does).
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn opting_out_mid_session_closes_the_writer(pool: PgPool) {
    const GUILD: u64 = GUILD_BASE + 11;
    const USER: u64 = 42;
    const SSRC: u32 = 7;
    sqlx::query(
        "INSERT INTO bot_instances (instance_id, role, state, heartbeat_at, started_at)
         VALUES ('test-instance', 'active', 'active', now(), now())",
    )
    .execute(&pool)
    .await
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut actor = detached_actor(GUILD, Arc::new(crate::BotMetrics::default()), pool.clone());
    let handle = crate::database::recordings::create_recording_for_test(
        &pool,
        GUILD as i64,
        1,
        USER as i64,
        chrono::Utc::now(),
        "test-instance",
        root.path(),
    )
    .await
    .unwrap();
    let file = std::fs::File::create(format!("{}.ogg", handle.path)).unwrap();
    actor.recordings.insert_active(
        USER,
        SSRC,
        UserRecording {
            writer: OggOpusWriter::new(BufWriter::new(file), SSRC, 0).unwrap(),
            audio_file_id: handle.audio_file_id,
            recording_session_id: handle.recording_session_id,
            file_name: handle.file_name,
            start_time: handle.start_time,
            user_id: USER,
            ssrc: SSRC,
        },
    );

    crate::database::opt_outs::set_opted_out(&pool, GUILD as i64, USER as i64, true)
        .await
        .unwrap();
    let at_ms = chrono::Utc::now().timestamp_millis();
    actor.refresh_recording_policy(at_ms).await;

    assert!(!actor.recordings.has_active_ssrc(SSRC));
    assert_eq!(actor.opted_out.user_ids(), vec![USER]);
    let closed: bool =
        sqlx::query_scalar("SELECT end_ts IS NOT NULL FROM audio_files WHERE id = $1")
            .bind(handle.audio_file_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(closed, "the fragment must be finalized");
    let pending_reason: Option<String> =
        sqlx::query_scalar("SELECT pending_reason FROM recording_sessions WHERE id = $1")
            .bind(handle.recording_session_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(pending_reason.as_deref(), Some("opted_out"));

    crate::database::opt_outs::set_opted_out(&pool, GUILD as i64, USER as i64, false)
        .await
        .unwrap();
    actor.refresh_recording_policy(at_ms + 1_000).await;
    assert!(actor.opted_out.user_ids().is_empty());
}
