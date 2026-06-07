use std::{sync::Arc, time::Duration};

use serenity::{
    gateway::ShardManager,
    prelude::{RwLock, TypeMap},
};
use sqlx::{Pool, Postgres};
use tokio::{sync::watch, task::AbortHandle};
use tracing::{error, info, warn};

const SHARD_SHUTDOWN_TIMEOUT_SECONDS: u64 = 15;

pub(crate) fn spawn_shutdown_task(
    runtime: Arc<crate::runtime::RuntimeState>,
    pool: Pool<Postgres>,
    data: Arc<RwLock<TypeMap>>,
    shard_manager: Arc<ShardManager>,
    bot_abort: AbortHandle,
    shutdown_tx: watch::Sender<bool>,
) -> tokio::task::JoinHandle<bool> {
    tokio::spawn(async move {
        loop {
            let triggered_by_signal = if runtime.shutdown_when_empty() {
                false
            } else {
                wait_for_shutdown_signal_or_drain(runtime.clone()).await
            };
            if triggered_by_signal {
                runtime.start_drain(true);
            }
            clear_voice_presence_metrics(&data).await;
            if let Err(err) =
                crate::deployment::heartbeat_instance_and_leases(&pool, &runtime).await
            {
                error!("shutdown heartbeat failed: {}", err);
            }

            if wait_for_drain(&runtime, &data).await {
                break;
            }
            info!("drain cancelled; resuming active runtime");
        }

        let shutdown_result = tokio::time::timeout(
            Duration::from_secs(SHARD_SHUTDOWN_TIMEOUT_SECONDS),
            shard_manager.shutdown_all(),
        )
        .await;
        let bot_aborted_for_shutdown = if shutdown_result.is_err() {
            warn!(
                timeout_seconds = SHARD_SHUTDOWN_TIMEOUT_SECONDS,
                "shard shutdown timed out; aborting Discord client task"
            );
            bot_abort.abort();
            true
        } else {
            false
        };
        let _ = shutdown_tx.send(true);
        bot_aborted_for_shutdown
    })
}

async fn wait_for_drain(
    runtime: &crate::runtime::RuntimeState,
    data: &Arc<RwLock<TypeMap>>,
) -> bool {
    let deadline = if runtime.config().drain_timeout.is_zero() {
        None
    } else {
        Some(tokio::time::Instant::now() + runtime.config().drain_timeout)
    };
    loop {
        if !runtime.is_draining() || !runtime.shutdown_when_empty() {
            return false;
        }
        if runtime.force_shutdown_requested() {
            warn!("force shutdown requested; bypassing drain wait");
            return true;
        }

        let active = active_voice_connection_count(data).await;
        if active == 0 {
            info!("drain complete; shutting down shards");
            return true;
        }
        if let Some(deadline) = deadline
            && tokio::time::Instant::now() >= deadline
        {
            warn!(active_voice_connections = active, "drain timeout reached");
            return true;
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(2)) => {}
            () = runtime.changed() => {}
        }
    }
}

async fn active_voice_connection_count(data: &Arc<RwLock<TypeMap>>) -> u32 {
    let data_read = data.read().await;
    data_read
        .get::<songbird::SongbirdKey>()
        .map(|manager| manager.iter().count() as u32)
        .or_else(|| {
            data_read.get::<crate::BotMetricsKey>().map(|metrics| {
                metrics
                    .active_voice_connections
                    .load(std::sync::atomic::Ordering::Relaxed)
            })
        })
        .unwrap_or(0)
}

async fn clear_voice_presence_metrics(data: &Arc<RwLock<TypeMap>>) {
    let data_read = data.read().await;
    if let Some(metrics) = data_read.get::<crate::BotMetricsKey>() {
        metrics.clear_voice_presence();
    }
}

async fn wait_for_shutdown_signal_or_drain(runtime: Arc<crate::runtime::RuntimeState>) -> bool {
    #[cfg(unix)]
    {
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(err) => {
                    error!("failed to register SIGTERM handler: {}", err);
                    runtime.changed().await;
                    return false;
                }
            };

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => return true,
                _ = sigterm.recv() => return true,
                _ = runtime.changed() => {
                    if runtime.shutdown_when_empty() {
                        return false;
                    }
                }
            }
        }
    }

    #[cfg(not(unix))]
    {
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => return true,
                _ = runtime.changed() => {
                    if runtime.shutdown_when_empty() {
                        return false;
                    }
                }
            }
        }
    }
}
