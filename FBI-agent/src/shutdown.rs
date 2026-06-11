use std::{sync::Arc, time::Duration};

use serenity::prelude::{RwLock, TypeMap};
use sqlx::{Pool, Postgres};
use tokio::sync::watch;
use tracing::{error, info, warn};

pub(crate) const SHARD_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GracefulShutdownTrigger {
    Signal,
    DrainComplete,
    DrainTimeout,
    ForceShutdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShutdownMonitorExit {
    Triggered(GracefulShutdownTrigger),
    Cancelled,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ShutdownMonitorError {
    #[error("failed to register SIGTERM handler")]
    Sigterm(#[source] std::io::Error),
    #[error("failed to wait for shutdown signal")]
    Signal(#[source] std::io::Error),
    #[error("SIGTERM stream closed unexpectedly")]
    SigtermStreamClosed,
    #[error("runtime shutdown channel closed unexpectedly")]
    ShutdownChannelClosed,
}

pub(crate) fn spawn_shutdown_monitor(
    runtime: Arc<crate::runtime::RuntimeState>,
    pool: Pool<Postgres>,
    data: Arc<RwLock<TypeMap>>,
    shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<Result<ShutdownMonitorExit, ShutdownMonitorError>> {
    tokio::spawn(run_shutdown_monitor(runtime, pool, data, shutdown_rx))
}

async fn run_shutdown_monitor(
    runtime: Arc<crate::runtime::RuntimeState>,
    pool: Pool<Postgres>,
    data: Arc<RwLock<TypeMap>>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<ShutdownMonitorExit, ShutdownMonitorError> {
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(ShutdownMonitorError::Sigterm)?;

    loop {
        let triggered_by_signal = if runtime.shutdown_when_empty() {
            false
        } else {
            #[cfg(unix)]
            let wake =
                wait_for_shutdown_signal_or_drain(runtime.clone(), &mut shutdown_rx, &mut sigterm)
                    .await?;
            #[cfg(not(unix))]
            let wake = wait_for_shutdown_signal_or_drain(runtime.clone(), &mut shutdown_rx).await?;

            match wake {
                MonitorWake::Signal => true,
                MonitorWake::DrainRequested => false,
                MonitorWake::Cancelled => return Ok(ShutdownMonitorExit::Cancelled),
            }
        };

        if triggered_by_signal {
            runtime.start_drain(true);
        }
        clear_voice_presence_metrics(&data).await;
        if let Err(err) = crate::deployment::heartbeat_instance_and_leases(&pool, &runtime).await {
            error!("shutdown heartbeat failed: {}", err);
        }

        match wait_for_drain(&runtime, &data, &mut shutdown_rx).await? {
            DrainWait::Cancelled => return Ok(ShutdownMonitorExit::Cancelled),
            DrainWait::DrainCancelled => info!("drain cancelled; resuming active runtime"),
            DrainWait::Complete => {
                return Ok(ShutdownMonitorExit::Triggered(if triggered_by_signal {
                    GracefulShutdownTrigger::Signal
                } else {
                    GracefulShutdownTrigger::DrainComplete
                }));
            }
            DrainWait::TimedOut => {
                return Ok(ShutdownMonitorExit::Triggered(
                    GracefulShutdownTrigger::DrainTimeout,
                ));
            }
            DrainWait::Forced => {
                return Ok(ShutdownMonitorExit::Triggered(
                    GracefulShutdownTrigger::ForceShutdown,
                ));
            }
        }
    }
}

enum MonitorWake {
    Signal,
    DrainRequested,
    Cancelled,
}

enum DrainWait {
    Complete,
    TimedOut,
    Forced,
    DrainCancelled,
    Cancelled,
}

async fn wait_for_drain(
    runtime: &crate::runtime::RuntimeState,
    data: &Arc<RwLock<TypeMap>>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<DrainWait, ShutdownMonitorError> {
    let deadline = if runtime.config().drain_timeout.is_zero() {
        None
    } else {
        Some(tokio::time::Instant::now() + runtime.config().drain_timeout)
    };

    loop {
        if !runtime.is_draining() || !runtime.shutdown_when_empty() {
            return Ok(DrainWait::DrainCancelled);
        }
        if runtime.force_shutdown_requested() {
            warn!("force shutdown requested; bypassing drain wait");
            return Ok(DrainWait::Forced);
        }

        let active = active_voice_connection_count(data).await;
        if active == 0 {
            info!("drain complete; requesting runtime shutdown");
            return Ok(DrainWait::Complete);
        }
        if let Some(deadline) = deadline
            && tokio::time::Instant::now() >= deadline
        {
            warn!(active_voice_connections = active, "drain timeout reached");
            return Ok(DrainWait::TimedOut);
        }

        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(2)) => {}
            () = runtime.changed() => {}
            changed = shutdown_rx.changed() => {
                if changed.is_err() {
                    return Err(ShutdownMonitorError::ShutdownChannelClosed);
                }
                if *shutdown_rx.borrow() {
                    return Ok(DrainWait::Cancelled);
                }
            }
        }
    }
}

pub(crate) async fn active_voice_connection_count(data: &Arc<RwLock<TypeMap>>) -> u32 {
    let (manager, metrics) = {
        let data_read = data.read().await;
        (
            data_read.get::<songbird::SongbirdKey>().cloned(),
            data_read.get::<crate::BotMetricsKey>().cloned(),
        )
    };

    match manager {
        Some(manager) => crate::events::voice::connected_voice_connection_count(&manager).await,
        None => metrics
            .map(|metrics| {
                metrics
                    .active_voice_connections
                    .load(std::sync::atomic::Ordering::Relaxed)
            })
            .unwrap_or(0),
    }
}

pub(crate) async fn clear_voice_presence_metrics(data: &Arc<RwLock<TypeMap>>) {
    let data_read = data.read().await;
    if let Some(metrics) = data_read.get::<crate::BotMetricsKey>() {
        metrics.clear_voice_presence();
    }
}

#[cfg(unix)]
async fn wait_for_shutdown_signal_or_drain(
    runtime: Arc<crate::runtime::RuntimeState>,
    shutdown_rx: &mut watch::Receiver<bool>,
    sigterm: &mut tokio::signal::unix::Signal,
) -> Result<MonitorWake, ShutdownMonitorError> {
    loop {
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.map_err(ShutdownMonitorError::Signal)?;
                return Ok(MonitorWake::Signal);
            }
            signal = sigterm.recv() => {
                return signal
                    .map(|()| MonitorWake::Signal)
                    .ok_or(ShutdownMonitorError::SigtermStreamClosed);
            }
            () = runtime.changed() => {
                if runtime.shutdown_when_empty() {
                    return Ok(MonitorWake::DrainRequested);
                }
            }
            changed = shutdown_rx.changed() => {
                if changed.is_err() {
                    return Err(ShutdownMonitorError::ShutdownChannelClosed);
                }
                if *shutdown_rx.borrow() {
                    return Ok(MonitorWake::Cancelled);
                }
            }
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal_or_drain(
    runtime: Arc<crate::runtime::RuntimeState>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<MonitorWake, ShutdownMonitorError> {
    loop {
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.map_err(ShutdownMonitorError::Signal)?;
                return Ok(MonitorWake::Signal);
            }
            () = runtime.changed() => {
                if runtime.shutdown_when_empty() {
                    return Ok(MonitorWake::DrainRequested);
                }
            }
            changed = shutdown_rx.changed() => {
                if changed.is_err() {
                    return Err(ShutdownMonitorError::ShutdownChannelClosed);
                }
                if *shutdown_rx.borrow() {
                    return Ok(MonitorWake::Cancelled);
                }
            }
        }
    }
}

#[cfg(test)]
fn count_connected_channels<T>(channels: impl IntoIterator<Item = Option<T>>) -> u32 {
    channels
        .into_iter()
        .filter(Option::is_some)
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    #[test]
    fn disconnected_calls_do_not_count_toward_drain() {
        let channels = [Some(10_u64), None, Some(20_u64), None];
        assert_eq!(super::count_connected_channels(channels), 2);
        assert_eq!(super::count_connected_channels([None::<u64>]), 0);
    }
}
