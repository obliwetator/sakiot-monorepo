use std::sync::Arc;
use std::sync::atomic::Ordering;

use serenity::client::Cache;
use serenity::prelude::{RwLock, TypeMap};
use songbird::SongbirdKey;

use crate::{BotMetrics, BotMetricsKey};

#[derive(Default)]
pub(super) struct GlobalMetricsSnapshot {
    pub total_guilds: i32,
    pub commands_executed: i32,
    pub active_voice_connections: i32,
    pub uptime_seconds: i64,
    pub active_recordings: i32,
    pub writer_setup_failures: i32,
    pub audio_packets_received: i64,
    pub audio_packets_dropped: i64,
    pub gateway_reconnects: i32,
    pub driver_reconnects: i32,
    pub voice_state_updates_received: i64,
    pub db_query_errors: i32,
    pub db_insert_failures: i32,
    pub grpc_active_streams: i32,
    pub process_rss_bytes: i64,
    pub process_open_fds: i32,
    pub tokio_active_tasks: i32,
    pub messages_received: i32,
    pub last_voice_packet_time: i64,
    pub active_recording_users: i64,
    pub voice_users: i64,
}

impl GlobalMetricsSnapshot {
    pub(super) fn capture(data: &TypeMap, cache: &Cache) -> Self {
        let mut snap = Self::default();

        if let Some(metrics) = data.get::<BotMetricsKey>() {
            snap.commands_executed = metrics.commands_executed.load(Ordering::Relaxed) as i32;
            snap.active_voice_connections =
                metrics.active_voice_connections.load(Ordering::Relaxed) as i32;
            snap.uptime_seconds = metrics.start_time.elapsed().as_secs() as i64;
            snap.active_recordings = metrics.active_recordings.load(Ordering::Relaxed) as i32;
            snap.writer_setup_failures =
                metrics.writer_setup_failures.load(Ordering::Relaxed) as i32;
            snap.audio_packets_received =
                metrics.audio_packets_received.load(Ordering::Relaxed) as i64;
            snap.audio_packets_dropped =
                metrics.audio_packets_dropped.load(Ordering::Relaxed) as i64;
            snap.gateway_reconnects = metrics.gateway_reconnects.load(Ordering::Relaxed) as i32;
            snap.driver_reconnects = metrics.driver_reconnects.load(Ordering::Relaxed) as i32;
            snap.voice_state_updates_received =
                metrics.voice_state_updates_received.load(Ordering::Relaxed) as i64;
            snap.db_query_errors = metrics.db_query_errors.load(Ordering::Relaxed) as i32;
            snap.db_insert_failures = metrics.db_insert_failures.load(Ordering::Relaxed) as i32;
            snap.grpc_active_streams = metrics.grpc_active_streams.load(Ordering::Relaxed) as i32;
            snap.process_rss_bytes = metrics.process_rss_bytes.load(Ordering::Relaxed) as i64;
            snap.process_open_fds = metrics.process_open_fds.load(Ordering::Relaxed) as i32;
            snap.tokio_active_tasks = metrics.tokio_active_tasks.load(Ordering::Relaxed) as i32;
            snap.messages_received = metrics.messages_received.load(Ordering::Relaxed) as i32;
            snap.last_voice_packet_time = metrics.last_voice_packet_time.load(Ordering::Relaxed);
            snap.active_recording_users = metrics.active_recording_users.len() as i64;
            snap.voice_users = metrics.voice_users.len() as i64;
        }

        if data.get::<SongbirdKey>().is_some() {
            snap.total_guilds = cache.guilds().len() as i32;
        }

        snap
    }
}

/// RAII guard: increments `grpc_active_streams` on construction, decrements on drop.
/// Survives task cancellation, unlike the previous manual fetch_sub at loop break.
pub(super) struct StreamLifetime(Option<Arc<BotMetrics>>);

impl StreamLifetime {
    pub(super) async fn acquire(data: &Arc<RwLock<TypeMap>>) -> Self {
        let m = data.read().await.get::<BotMetricsKey>().cloned();
        if let Some(m) = &m {
            m.grpc_active_streams.fetch_add(1, Ordering::Relaxed);
        }
        Self(m)
    }
}

impl Drop for StreamLifetime {
    fn drop(&mut self) {
        if let Some(m) = &self.0 {
            m.grpc_active_streams.fetch_sub(1, Ordering::Relaxed);
        }
    }
}
