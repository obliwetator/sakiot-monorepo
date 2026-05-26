use opentelemetry::KeyValue;
use opentelemetry::metrics::Histogram;
use serenity::prelude::TypeMapKey;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64};
use std::time::Instant;

use super::presence::{VoiceUserKey, VoiceUserPresence};
use super::recording::GuildRecordingMetrics;

pub struct BotMetrics {
    pub start_time: Instant,
    pub commands_executed: AtomicU32,
    pub active_voice_connections: AtomicU32,
    pub update_tx: tokio::sync::watch::Sender<()>,
    pub voice_update_tx: tokio::sync::watch::Sender<()>,
    pub user_start_times: dashmap::DashMap<u64, i64>,
    // Voice recording pipeline — global aggregates
    pub active_recordings: AtomicU32,
    pub recordings_started: AtomicU64,
    pub recordings_finished: AtomicU64,
    pub recording_finalize_errors: AtomicU64,
    pub writer_setup_failures: AtomicU32,
    pub audio_packets_received: AtomicU64,
    pub audio_packets_dropped: AtomicU64,
    pub last_voice_packet_time: AtomicI64,
    pub(super) recording_duration_seconds: Histogram<f64>,
    // Voice recording pipeline — per-guild breakdown
    pub guild_recording_metrics: dashmap::DashMap<u64, Arc<GuildRecordingMetrics>>,
    // Voice recording pipeline — per-channel breakdown
    pub channel_recording_metrics: dashmap::DashMap<(u64, u64), Arc<GuildRecordingMetrics>>,
    // Current voice channel presence keyed by guild/user.
    pub voice_users: dashmap::DashMap<VoiceUserKey, VoiceUserPresence>,
    pub active_recording_users: dashmap::DashMap<VoiceUserKey, u64>,
    // Discord gateway health
    pub gateway_reconnects: AtomicU32,
    pub gateway_disconnects: AtomicU32,
    pub driver_reconnects: AtomicU32,
    pub driver_disconnects: AtomicU32,
    pub voice_state_updates_received: AtomicU64,
    // Database health
    pub db_query_errors: AtomicU32,
    pub db_insert_failures: AtomicU32,
    // gRPC server health
    pub grpc_active_streams: AtomicU32,
    // Bot activity
    pub messages_received: AtomicU32,
    // Process health (sampled every 15s)
    pub process_rss_bytes: AtomicU64,
    pub process_open_fds: AtomicU32,
    pub tokio_active_tasks: AtomicU32,
}

impl BotMetrics {
    pub fn record_gateway_resume(&self) {
        self.gateway_reconnects
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _ = self.update_tx.send(());
    }

    pub fn record_voice_state_update(&self) {
        self.voice_state_updates_received
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_command_executed(&self) {
        self.commands_executed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _ = self.update_tx.send(());
    }

    /// Returns the metrics entry for `guild_id`, creating it on first access.
    pub fn guild_metrics(&self, guild_id: u64) -> Arc<GuildRecordingMetrics> {
        self.guild_recording_metrics
            .entry(guild_id)
            .or_insert_with(|| Arc::new(GuildRecordingMetrics::new()))
            .clone()
    }

    pub fn channel_metrics(&self, guild_id: u64, channel_id: u64) -> Arc<GuildRecordingMetrics> {
        self.channel_recording_metrics
            .entry((guild_id, channel_id))
            .or_insert_with(|| Arc::new(GuildRecordingMetrics::new()))
            .clone()
    }

    pub fn track_voice_presence(
        &self,
        guild_id: u64,
        user_id: u64,
        presence: Option<VoiceUserPresence>,
    ) {
        let key = VoiceUserKey { guild_id, user_id };
        if let Some(presence) = presence {
            self.voice_users.insert(key, presence);
        } else {
            self.voice_users.remove(&key);
            self.active_recording_users.remove(&key);
        }
    }

    pub fn track_recording_started(
        &self,
        guild_metrics: &GuildRecordingMetrics,
        channel_metrics: &GuildRecordingMetrics,
        guild_id: u64,
        channel_id: u64,
        user_id: u64,
    ) {
        self.active_recordings
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        guild_metrics
            .active_recordings
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        channel_metrics
            .active_recordings
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.recordings_started
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.active_recording_users
            .insert(VoiceUserKey { guild_id, user_id }, channel_id);
    }

    pub fn track_recording_finished(
        &self,
        guild_metrics: &GuildRecordingMetrics,
        channel_metrics: &GuildRecordingMetrics,
        guild_id: u64,
        channel_id: u64,
        user_id: u64,
        duration_seconds: f64,
    ) {
        self.active_recordings
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        guild_metrics
            .active_recordings
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        channel_metrics
            .active_recordings
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        self.recordings_finished
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.recording_duration_seconds.record(
            duration_seconds.max(0.0),
            &[
                KeyValue::new("guild_id", guild_id.to_string()),
                KeyValue::new("channel_id", channel_id.to_string()),
            ],
        );
        self.active_recording_users
            .remove(&VoiceUserKey { guild_id, user_id });
    }

    pub fn track_recording_finalize_error(&self) {
        self.recording_finalize_errors
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn track_writer_setup_failure(
        &self,
        guild_metrics: &GuildRecordingMetrics,
        channel_metrics: &GuildRecordingMetrics,
    ) {
        self.writer_setup_failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        guild_metrics
            .writer_setup_failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        channel_metrics
            .writer_setup_failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn track_audio_packet_received(
        &self,
        guild_metrics: &GuildRecordingMetrics,
        channel_metrics: &GuildRecordingMetrics,
    ) {
        self.audio_packets_received
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        guild_metrics
            .audio_packets_received
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        channel_metrics
            .audio_packets_received
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn track_last_voice_packet(
        &self,
        guild_metrics: &GuildRecordingMetrics,
        channel_metrics: &GuildRecordingMetrics,
        timestamp_ms: i64,
    ) {
        self.last_voice_packet_time
            .store(timestamp_ms, std::sync::atomic::Ordering::Relaxed);
        guild_metrics
            .last_voice_packet_time
            .store(timestamp_ms, std::sync::atomic::Ordering::Relaxed);
        channel_metrics
            .last_voice_packet_time
            .store(timestamp_ms, std::sync::atomic::Ordering::Relaxed);
    }

    fn recording_duration_histogram() -> Histogram<f64> {
        opentelemetry::global::meter(crate::config::SERVICE_NAME)
            .f64_histogram("recording_duration_seconds")
            .with_description("Finalized recording duration in seconds")
            .with_unit("s")
            .build()
    }
}

impl Default for BotMetrics {
    fn default() -> Self {
        let (tx, _) = tokio::sync::watch::channel(());
        let (voice_tx, _) = tokio::sync::watch::channel(());
        Self {
            start_time: Instant::now(),
            commands_executed: AtomicU32::new(0),
            active_voice_connections: AtomicU32::new(0),
            update_tx: tx,
            voice_update_tx: voice_tx,
            user_start_times: dashmap::DashMap::new(),
            active_recordings: AtomicU32::new(0),
            recordings_started: AtomicU64::new(0),
            recordings_finished: AtomicU64::new(0),
            recording_finalize_errors: AtomicU64::new(0),
            writer_setup_failures: AtomicU32::new(0),
            audio_packets_received: AtomicU64::new(0),
            audio_packets_dropped: AtomicU64::new(0),
            last_voice_packet_time: AtomicI64::new(0),
            recording_duration_seconds: Self::recording_duration_histogram(),
            guild_recording_metrics: dashmap::DashMap::new(),
            channel_recording_metrics: dashmap::DashMap::new(),
            voice_users: dashmap::DashMap::new(),
            active_recording_users: dashmap::DashMap::new(),
            gateway_reconnects: AtomicU32::new(0),
            gateway_disconnects: AtomicU32::new(0),
            driver_reconnects: AtomicU32::new(0),
            driver_disconnects: AtomicU32::new(0),
            voice_state_updates_received: AtomicU64::new(0),
            db_query_errors: AtomicU32::new(0),
            db_insert_failures: AtomicU32::new(0),
            grpc_active_streams: AtomicU32::new(0),
            messages_received: AtomicU32::new(0),
            process_rss_bytes: AtomicU64::new(0),
            process_open_fds: AtomicU32::new(0),
            tokio_active_tasks: AtomicU32::new(0),
        }
    }
}

pub struct BotMetricsKey;
impl TypeMapKey for BotMetricsKey {
    type Value = Arc<BotMetrics>;
}
