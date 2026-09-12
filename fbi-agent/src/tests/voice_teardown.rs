//! Departure-notification regression tests.
//!
//! `RecorderActor::run` calls teardown from inside its own task. If that
//! teardown awaited the actor's termination it would wait for a signal that is
//! only sent after the loop it is running inside returns — a permanent hang
//! that also pins the guild's per-guild operation mutex and poisons
//! `get_or_create` for that guild.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use serenity::async_trait;
use serenity::model::id::GuildId;
use serenity::prelude::{RwLock, TypeMap};
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::events::voice_receiver::{DepartureNotify, Terminable, signal_departure};

#[derive(Default)]
struct FakeActor {
    signaled: AtomicBool,
    awaited: AtomicBool,
}

#[async_trait]
impl Terminable for FakeActor {
    fn termination_id(&self) -> Arc<()> {
        Arc::new(())
    }

    fn signal_exit(&self, _at_ms: i64) {
        self.signaled.store(true, Ordering::Release);
    }

    async fn await_termination(&self) {
        self.awaited.store(true, Ordering::Release);
    }
}

#[tokio::test]
async fn caller_mode_signals_without_awaiting_its_own_termination() {
    let actor = FakeActor::default();

    let evict = signal_departure(&actor, 1, DepartureNotify::Caller).await;

    assert!(actor.signaled.load(Ordering::Acquire));
    assert!(
        !actor.awaited.load(Ordering::Acquire),
        "an actor must never await its own termination"
    );
    assert!(
        !evict,
        "an actor must not evict itself; run() removes its own registry entry"
    );
}

#[tokio::test]
async fn registry_mode_awaits_termination_and_reports_eviction() {
    let actor = FakeActor::default();

    let evict = signal_departure(&actor, 1, DepartureNotify::Registry).await;

    assert!(actor.signaled.load(Ordering::Acquire));
    assert!(actor.awaited.load(Ordering::Acquire));
    assert!(evict);
}

/// The manager-missing branch notifies the actor too, so it needs the same
/// caller-mode suppression as the normal departure branch.
///
/// This asserts the branch's shape and that it returns without blocking; the
/// "never await your own termination" invariant itself is guarded by the
/// `signal_departure` tests above, which fail if caller mode ever awaits.
#[tokio::test]
async fn caller_mode_teardown_without_songbird_manager_returns_promptly() {
    let pool = lazy_pool();

    // No SongbirdKey: teardown takes the manager-missing branch.
    let data = Arc::new(RwLock::new(TypeMap::new()));
    let guild_id = GuildId::new(9_000_000_000_000_001);

    let report = tokio::time::timeout(
        Duration::from_secs(5),
        crate::events::voice::try_teardown_voice_session(&data, &pool, guild_id),
    )
    .await
    .expect("caller-mode teardown must not await the actor's own termination")
    .expect("an uncontended teardown must produce a report");

    assert!(report.manager_missing);
    assert!(!report.had_call);
    assert!(!report.connected_after);
}

/// `release_disconnected_lease` and the voice-connection gauge both no-op
/// without their `TypeMap` keys, so the manager-missing path never reaches the
/// database. A lazy pool keeps this test hermetic.
fn lazy_pool() -> PgPool {
    PgPoolOptions::new()
        .connect_lazy("postgres://postgres:password@127.0.0.1:54320/sakiot_rouvas")
        .expect("lazy pool construction does not connect")
}
