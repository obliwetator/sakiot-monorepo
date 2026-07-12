use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use dashmap::DashMap;
use serenity::model::id::GuildId;
use tokio::sync::Mutex;

#[derive(Default)]
pub(crate) struct VoiceCoordinatorRegistry {
    guilds: DashMap<u64, Arc<GuildVoiceCoordinator>>,
}

impl VoiceCoordinatorRegistry {
    pub(crate) fn guild(&self, guild_id: GuildId) -> Arc<GuildVoiceCoordinator> {
        self.guilds
            .entry(guild_id.get())
            .or_insert_with(|| Arc::new(GuildVoiceCoordinator::default()))
            .clone()
    }
}

pub(crate) struct VoiceCoordinatorRegistryKey;

impl serenity::prelude::TypeMapKey for VoiceCoordinatorRegistryKey {
    type Value = Arc<VoiceCoordinatorRegistry>;
}

#[derive(Default)]
pub(crate) struct GuildVoiceCoordinator {
    pub(crate) operation: Mutex<()>,
    route_generation: AtomicU64,
    empty_generation: AtomicU64,
    retry_running: AtomicBool,
    operation_counter: AtomicU64,
}

impl GuildVoiceCoordinator {
    pub(crate) fn signal(&self) -> u64 {
        self.route_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub(crate) fn generation(&self) -> u64 {
        self.route_generation.load(Ordering::SeqCst)
    }

    pub(crate) fn invalidate_empty_timer(&self) {
        self.empty_generation.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn next_empty_timer(&self) -> u64 {
        self.empty_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub(crate) fn empty_timer_matches(&self, token: u64) -> bool {
        self.empty_generation.load(Ordering::SeqCst) == token
    }

    pub(crate) fn begin_retry(&self) -> bool {
        self.retry_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub(crate) fn finish_retry(&self) {
        self.retry_running.store(false, Ordering::SeqCst);
    }

    pub(crate) fn operation_id(
        &self,
        instance_id: &str,
        guild_id: GuildId,
        started_at_ms: i64,
    ) -> String {
        let counter = self.operation_counter.fetch_add(1, Ordering::SeqCst) + 1;
        format!(
            "{}-{}-{}-{}",
            instance_id,
            guild_id.get(),
            started_at_ms,
            counter
        )
    }
}

#[cfg(test)]
mod tests {
    use super::GuildVoiceCoordinator;

    #[test]
    fn empty_timer_tokens_coalesce_stale_signals() {
        let coordinator = GuildVoiceCoordinator::default();
        let first = coordinator.next_empty_timer();
        let second = coordinator.next_empty_timer();
        assert!(!coordinator.empty_timer_matches(first));
        assert!(coordinator.empty_timer_matches(second));
    }

    #[test]
    fn only_one_retry_worker_runs() {
        let coordinator = GuildVoiceCoordinator::default();
        assert!(coordinator.begin_retry());
        assert!(!coordinator.begin_retry());
        coordinator.finish_retry();
        assert!(coordinator.begin_retry());
    }
}
