use crate::cast::ToI64;
use std::{
    env,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serenity::prelude::TypeMapKey;
use tokio::sync::Notify;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BotRole {
    Active,
    Drain,
}

impl BotRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Drain => "drain",
        }
    }

    fn from_env(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "drain" | "draining" => Self::Drain,
            _ => Self::Active,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub instance_id: String,
    pub initial_role: BotRole,
    pub drain_timeout: Duration,
}

impl RuntimeConfig {
    pub fn from_env() -> Self {
        let instance_id = env::var("BOT_INSTANCE_ID")
            .unwrap_or_else(|_| format!("{}-{}", crate::config::SERVICE_NAME, std::process::id()));
        let initial_role = env::var("BOT_ROLE")
            .map(|role| BotRole::from_env(&role))
            .unwrap_or(BotRole::Active);
        let drain_timeout = env::var("DRAIN_TIMEOUT_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(30 * 60));

        Self {
            instance_id,
            initial_role,
            drain_timeout,
        }
    }
}

#[derive(Debug)]
pub struct RuntimeState {
    config: RuntimeConfig,
    draining: AtomicBool,
    shutdown_when_empty: AtomicBool,
    force_shutdown: AtomicBool,
    drain_started_at: AtomicI64,
    notify: Notify,
}

impl RuntimeState {
    pub fn new(config: RuntimeConfig) -> Arc<Self> {
        let initially_draining = config.initial_role == BotRole::Drain;
        Arc::new(Self {
            draining: AtomicBool::new(initially_draining),
            shutdown_when_empty: AtomicBool::new(initially_draining),
            force_shutdown: AtomicBool::new(false),
            drain_started_at: AtomicI64::new(if initially_draining { now_unix() } else { 0 }),
            config,
            notify: Notify::new(),
        })
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn role(&self) -> BotRole {
        if self.is_draining() {
            BotRole::Drain
        } else {
            BotRole::Active
        }
    }

    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::SeqCst)
    }

    pub fn shutdown_when_empty(&self) -> bool {
        self.shutdown_when_empty.load(Ordering::SeqCst)
    }

    pub fn force_shutdown_requested(&self) -> bool {
        self.force_shutdown.load(Ordering::SeqCst)
    }

    pub fn drain_age_seconds(&self) -> u64 {
        let started = self.drain_started_at.load(Ordering::SeqCst);
        if started <= 0 {
            return 0;
        }
        now_unix().saturating_sub(started) as u64
    }

    pub fn start_drain(&self, shutdown_when_empty: bool) {
        if !self.draining.swap(true, Ordering::SeqCst) {
            self.drain_started_at.store(now_unix(), Ordering::SeqCst);
        }
        if shutdown_when_empty {
            self.shutdown_when_empty.store(true, Ordering::SeqCst);
        }
        self.notify.notify_waiters();
    }

    pub fn cancel_drain(&self) -> bool {
        if self.force_shutdown_requested() {
            return false;
        }

        self.shutdown_when_empty.store(false, Ordering::SeqCst);
        self.draining.store(false, Ordering::SeqCst);
        self.drain_started_at.store(0, Ordering::SeqCst);
        self.notify.notify_waiters();
        true
    }

    pub fn force_shutdown(&self) {
        self.start_drain(true);
        self.force_shutdown.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub async fn changed(&self) {
        self.notify.notified().await;
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_i64())
        .unwrap_or(0)
}

pub struct RuntimeStateKey;
impl TypeMapKey for RuntimeStateKey {
    type Value = Arc<RuntimeState>;
}

pub async fn state_from_ctx(ctx: &serenity::client::Context) -> Option<Arc<RuntimeState>> {
    let data = ctx.data.read().await;
    data.get::<RuntimeStateKey>().cloned()
}

pub async fn is_draining_ctx(ctx: &serenity::client::Context) -> bool {
    state_from_ctx(ctx)
        .await
        .map(|state| state.is_draining())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{BotRole, RuntimeConfig, RuntimeState};
    use std::time::Duration;

    fn runtime() -> std::sync::Arc<RuntimeState> {
        RuntimeState::new(RuntimeConfig {
            instance_id: "test".to_string(),
            initial_role: BotRole::Active,
            drain_timeout: Duration::from_secs(30),
        })
    }

    #[test]
    fn cancel_drain_restores_active_role() {
        let runtime = runtime();
        runtime.start_drain(true);

        assert!(runtime.cancel_drain());
        assert_eq!(runtime.role(), BotRole::Active);
        assert!(!runtime.shutdown_when_empty());
        assert_eq!(runtime.drain_age_seconds(), 0);
    }

    #[test]
    fn force_shutdown_cannot_be_cancelled() {
        let runtime = runtime();
        runtime.force_shutdown();

        assert!(!runtime.cancel_drain());
        assert!(runtime.is_draining());
        assert!(runtime.shutdown_when_empty());
    }
}
