use opentelemetry::KeyValue;
use opentelemetry::metrics::Histogram;
use serenity::prelude::TypeMapKey;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64};
use std::time::Instant;

use super::presence::{VoiceUserKey, VoiceUserPresence};
use super::recording::GuildRecordingMetrics;

pub struct BotMetrics {
    pub start_time: Instant,
    pub commands_executed: AtomicU32,
    pub active_voice_connections: AtomicU32,
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
    }

    pub fn record_voice_state_update(&self) {
        self.voice_state_updates_received
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_command_executed(&self) {
        self.commands_executed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

    pub fn clear_voice_presence(&self) {
        self.voice_users.clear();
    }

    /// Reconciles event-driven presence metrics against Serenity's current
    /// cache snapshot. This repairs handler events skipped during draining or
    /// transient routing gaps without an O(users²) scan after every event.
    pub fn prune_stale_voice_metrics(&self, current_voice_users: &HashSet<VoiceUserKey>) {
        self.voice_users
            .retain(|key, _| current_voice_users.contains(key));
        self.active_recording_users
            .retain(|key, _| current_voice_users.contains(key));
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

    pub fn track_audio_packets_dropped(
        &self,
        guild_metrics: &GuildRecordingMetrics,
        channel_metrics: &GuildRecordingMetrics,
        count: u64,
    ) {
        self.audio_packets_dropped
            .fetch_add(count, std::sync::atomic::Ordering::Relaxed);
        guild_metrics
            .audio_packets_dropped
            .fetch_add(count, std::sync::atomic::Ordering::Relaxed);
        channel_metrics
            .audio_packets_dropped
            .fetch_add(count, std::sync::atomic::Ordering::Relaxed);
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
        Self {
            start_time: Instant::now(),
            commands_executed: AtomicU32::new(0),
            active_voice_connections: AtomicU32::new(0),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_reconciles_presence_metrics_with_cache_snapshot() {
        let metrics = BotMetrics::default();
        metrics.track_voice_presence(
            10,
            1,
            Some(VoiceUserPresence {
                channel_id: 100,
                is_bot: false,
                server_mute: false,
                server_deaf: false,
                self_mute: false,
                self_deaf: false,
                suppress: false,
                streaming: false,
                video: false,
            }),
        );

        metrics.track_voice_presence(
            10,
            2,
            Some(VoiceUserPresence {
                channel_id: 200,
                is_bot: false,
                server_mute: false,
                server_deaf: false,
                self_mute: false,
                self_deaf: false,
                suppress: false,
                streaming: false,
                video: false,
            }),
        );
        metrics.active_recording_users.insert(
            VoiceUserKey {
                guild_id: 10,
                user_id: 2,
            },
            200,
        );

        let current = HashSet::from([VoiceUserKey {
            guild_id: 10,
            user_id: 1,
        }]);
        metrics.prune_stale_voice_metrics(&current);

        assert_eq!(metrics.voice_users.len(), 1);
        assert!(metrics.active_recording_users.is_empty());
    }
}
