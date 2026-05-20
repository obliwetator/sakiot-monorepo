use opentelemetry::KeyValue;
use opentelemetry::metrics::Histogram;
use serenity::prelude::TypeMapKey;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64};
use std::time::Instant;

/// Per-guild recording health, mirroring the global counters on BotMetrics.
pub struct GuildRecordingMetrics {
    pub active_recordings: AtomicU32,
    pub writer_setup_failures: AtomicU32,
    pub audio_packets_received: AtomicU64,
    pub audio_packets_dropped: AtomicU64,
    pub last_voice_packet_time: AtomicI64,
}

impl GuildRecordingMetrics {
    pub fn new() -> Self {
        Self {
            active_recordings: AtomicU32::new(0),
            writer_setup_failures: AtomicU32::new(0),
            audio_packets_received: AtomicU64::new(0),
            audio_packets_dropped: AtomicU64::new(0),
            last_voice_packet_time: AtomicI64::new(0),
        }
    }
}

impl Default for GuildRecordingMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct VoiceUserKey {
    pub guild_id: u64,
    pub user_id: u64,
}

#[derive(Clone, Copy)]
pub struct VoiceUserPresence {
    pub channel_id: u64,
    pub is_bot: bool,
    pub server_mute: bool,
    pub server_deaf: bool,
    pub self_mute: bool,
    pub self_deaf: bool,
    pub suppress: bool,
    pub streaming: bool,
    pub video: bool,
}

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
    recording_duration_seconds: Histogram<f64>,
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

fn deployment_release_id(instance_id: &str) -> String {
    std::env::var("RELEASE_ID")
        .ok()
        .filter(|release_id| !release_id.is_empty())
        .or_else(|| {
            instance_id
                .rsplit_once('-')
                .map(|(_, release_id)| release_id.to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn deployment_labels(runtime: &crate::runtime::RuntimeState, release_id: &str) -> Vec<KeyValue> {
    vec![
        KeyValue::new("instance_id", runtime.config().instance_id.clone()),
        KeyValue::new("release_id", release_id.to_string()),
        KeyValue::new("role", runtime.role().as_str()),
    ]
}

impl BotMetrics {
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

    /// Registers standard BotMetrics instruments to OpenTelemetry.
    pub fn register_otel_metrics(metrics: Arc<Self>, runtime: Arc<crate::runtime::RuntimeState>) {
        let meter = opentelemetry::global::meter(crate::config::SERVICE_NAME);
        let release_id = deployment_release_id(runtime.config().instance_id.as_str());

        macro_rules! u32_counter {
            ($name:literal, $desc:literal, $field:ident) => {
                let m = metrics.clone();
                meter
                    .u64_observable_counter($name)
                    .with_description($desc)
                    .with_callback(move |observer| {
                        observer.observe(
                            m.$field.load(std::sync::atomic::Ordering::Relaxed) as u64,
                            &[],
                        );
                    })
                    .build();
            };
        }

        macro_rules! u64_counter {
            ($name:literal, $desc:literal, $field:ident) => {
                let m = metrics.clone();
                meter
                    .u64_observable_counter($name)
                    .with_description($desc)
                    .with_callback(move |observer| {
                        observer.observe(m.$field.load(std::sync::atomic::Ordering::Relaxed), &[]);
                    })
                    .build();
            };
        }

        macro_rules! u32_gauge {
            ($name:literal, $desc:literal, $field:ident) => {
                let m = metrics.clone();
                meter
                    .u64_observable_gauge($name)
                    .with_description($desc)
                    .with_callback(move |observer| {
                        observer.observe(
                            m.$field.load(std::sync::atomic::Ordering::Relaxed) as u64,
                            &[],
                        );
                    })
                    .build();
            };
        }

        macro_rules! u64_gauge {
            ($name:literal, $desc:literal, $field:ident) => {
                let m = metrics.clone();
                meter
                    .u64_observable_gauge($name)
                    .with_description($desc)
                    .with_callback(move |observer| {
                        observer.observe(m.$field.load(std::sync::atomic::Ordering::Relaxed), &[]);
                    })
                    .build();
            };
        }

        macro_rules! guild_counter {
            ($name:literal, $desc:literal, $field:ident, $map:expr) => {{
                let m = metrics.clone();
                meter
                    .u64_observable_counter($name)
                    .with_description($desc)
                    .with_callback(move |observer| {
                        for entry in m.guild_recording_metrics.iter() {
                            let guild_id = entry.key();
                            let guild_metrics = entry.value();
                            observer.observe(
                                $map(
                                    guild_metrics
                                        .$field
                                        .load(std::sync::atomic::Ordering::Relaxed),
                                ),
                                &[opentelemetry::KeyValue::new(
                                    "guild_id",
                                    guild_id.to_string(),
                                )],
                            );
                        }
                    })
                    .build();
            }};
        }

        macro_rules! guild_gauge {
            ($name:literal, $desc:literal, $field:ident) => {
                let m = metrics.clone();
                meter
                    .u64_observable_gauge($name)
                    .with_description($desc)
                    .with_callback(move |observer| {
                        for entry in m.guild_recording_metrics.iter() {
                            let guild_id = entry.key();
                            let guild_metrics = entry.value();
                            observer.observe(
                                guild_metrics
                                    .$field
                                    .load(std::sync::atomic::Ordering::Relaxed)
                                    as u64,
                                &[opentelemetry::KeyValue::new(
                                    "guild_id",
                                    guild_id.to_string(),
                                )],
                            );
                        }
                    })
                    .build();
            };
        }

        macro_rules! channel_counter {
            ($name:literal, $desc:literal, $field:ident, $map:expr) => {{
                let m = metrics.clone();
                meter
                    .u64_observable_counter($name)
                    .with_description($desc)
                    .with_callback(move |observer| {
                        for entry in m.channel_recording_metrics.iter() {
                            let (guild_id, channel_id) = entry.key();
                            let channel_metrics = entry.value();
                            observer.observe(
                                $map(
                                    channel_metrics
                                        .$field
                                        .load(std::sync::atomic::Ordering::Relaxed),
                                ),
                                &[
                                    opentelemetry::KeyValue::new("guild_id", guild_id.to_string()),
                                    opentelemetry::KeyValue::new(
                                        "channel_id",
                                        channel_id.to_string(),
                                    ),
                                ],
                            );
                        }
                    })
                    .build();
            }};
        }

        macro_rules! channel_gauge {
            ($name:literal, $desc:literal, $field:ident) => {
                let m = metrics.clone();
                meter
                    .u64_observable_gauge($name)
                    .with_description($desc)
                    .with_callback(move |observer| {
                        for entry in m.channel_recording_metrics.iter() {
                            let (guild_id, channel_id) = entry.key();
                            let channel_metrics = entry.value();
                            observer.observe(
                                channel_metrics
                                    .$field
                                    .load(std::sync::atomic::Ordering::Relaxed)
                                    as u64,
                                &[
                                    opentelemetry::KeyValue::new("guild_id", guild_id.to_string()),
                                    opentelemetry::KeyValue::new(
                                        "channel_id",
                                        channel_id.to_string(),
                                    ),
                                ],
                            );
                        }
                    })
                    .build();
            };
        }

        {
            let m = metrics.clone();
            meter
                .u64_observable_gauge("voice_user_present")
                .with_description("Current voice channel presence per user")
                .with_callback(move |observer| {
                    for entry in m.voice_users.iter() {
                        let key = entry.key();
                        let presence = entry.value();
                        observer.observe(
                            1,
                            &[
                                KeyValue::new("guild_id", key.guild_id.to_string()),
                                KeyValue::new("channel_id", presence.channel_id.to_string()),
                                KeyValue::new("user_id", key.user_id.to_string()),
                                KeyValue::new("is_bot", presence.is_bot.to_string()),
                                KeyValue::new("server_mute", presence.server_mute.to_string()),
                                KeyValue::new("server_deaf", presence.server_deaf.to_string()),
                                KeyValue::new("self_mute", presence.self_mute.to_string()),
                                KeyValue::new("self_deaf", presence.self_deaf.to_string()),
                                KeyValue::new("suppress", presence.suppress.to_string()),
                                KeyValue::new("streaming", presence.streaming.to_string()),
                                KeyValue::new("video", presence.video.to_string()),
                            ],
                        );
                    }
                })
                .build();
        }

        {
            let m = metrics.clone();
            meter
                .u64_observable_gauge("voice_channel_users")
                .with_description("Current voice users per guild/channel")
                .with_callback(move |observer| {
                    let mut counts: std::collections::HashMap<(u64, u64, bool), u64> =
                        std::collections::HashMap::new();
                    for entry in m.voice_users.iter() {
                        let key = entry.key();
                        let presence = entry.value();
                        *counts
                            .entry((key.guild_id, presence.channel_id, presence.is_bot))
                            .or_default() += 1;
                    }
                    for ((guild_id, channel_id, is_bot), count) in counts {
                        observer.observe(
                            count,
                            &[
                                KeyValue::new("guild_id", guild_id.to_string()),
                                KeyValue::new("channel_id", channel_id.to_string()),
                                KeyValue::new("is_bot", is_bot.to_string()),
                            ],
                        );
                    }
                })
                .build();
        }

        {
            let m = metrics.clone();
            meter
                .u64_observable_gauge("voice_channel_state_users")
                .with_description("Current voice users per guild/channel/state")
                .with_callback(move |observer| {
                    let mut counts: std::collections::HashMap<(u64, u64, &'static str), u64> =
                        std::collections::HashMap::new();
                    for entry in m.voice_users.iter() {
                        let key = entry.key();
                        let presence = entry.value();
                        let states = [
                            ("server_mute", presence.server_mute),
                            ("server_deaf", presence.server_deaf),
                            ("self_mute", presence.self_mute),
                            ("self_deaf", presence.self_deaf),
                            ("suppress", presence.suppress),
                            ("streaming", presence.streaming),
                            ("video", presence.video),
                        ];
                        for (state, enabled) in states {
                            if enabled {
                                *counts
                                    .entry((key.guild_id, presence.channel_id, state))
                                    .or_default() += 1;
                            }
                        }
                    }
                    for ((guild_id, channel_id, state), count) in counts {
                        observer.observe(
                            count,
                            &[
                                KeyValue::new("guild_id", guild_id.to_string()),
                                KeyValue::new("channel_id", channel_id.to_string()),
                                KeyValue::new("state", state),
                            ],
                        );
                    }
                })
                .build();
        }

        {
            let m = metrics.clone();
            meter
                .u64_observable_gauge("recording_user_active")
                .with_description("Current active recording users")
                .with_callback(move |observer| {
                    for entry in m.active_recording_users.iter() {
                        let key = entry.key();
                        let channel_id = entry.value();
                        observer.observe(
                            1,
                            &[
                                KeyValue::new("guild_id", key.guild_id.to_string()),
                                KeyValue::new("channel_id", channel_id.to_string()),
                                KeyValue::new("user_id", key.user_id.to_string()),
                            ],
                        );
                    }
                })
                .build();
        }

        meter
            .u64_observable_gauge("bot_up")
            .with_description("Bot process health: 1 while the process exports metrics")
            .with_callback(|observer| observer.observe(1, &[]))
            .build();

        {
            let r = runtime.clone();
            let release_id = release_id.clone();
            meter
                .u64_observable_gauge("bot_instance_info")
                .with_description("Bot deployment instance metadata")
                .with_callback(move |observer| {
                    let labels = deployment_labels(&r, release_id.as_str());
                    observer.observe(1, &labels);
                })
                .build();
        }

        {
            let m = metrics.clone();
            let r = runtime.clone();
            let release_id = release_id.clone();
            meter
                .u64_observable_gauge("bot_instance_uptime_seconds")
                .with_description("Bot deployment instance uptime in seconds")
                .with_unit("s")
                .with_callback(move |observer| {
                    let labels = deployment_labels(&r, release_id.as_str());
                    observer.observe(m.start_time.elapsed().as_secs(), &labels);
                })
                .build();
        }

        {
            let m = metrics.clone();
            let r = runtime.clone();
            let release_id = release_id.clone();
            meter
                .u64_observable_gauge("bot_instance_voice_connections")
                .with_description("Current active voice connections for this deployment instance")
                .with_callback(move |observer| {
                    let labels = deployment_labels(&r, release_id.as_str());
                    observer.observe(
                        m.active_voice_connections
                            .load(std::sync::atomic::Ordering::Relaxed)
                            as u64,
                        &labels,
                    );
                })
                .build();
        }

        {
            let m = metrics.clone();
            let r = runtime.clone();
            let release_id = release_id.clone();
            meter
                .u64_observable_gauge("bot_instance_active_recordings")
                .with_description("Current active recordings for this deployment instance")
                .with_callback(move |observer| {
                    let labels = deployment_labels(&r, release_id.as_str());
                    observer.observe(
                        m.active_recordings
                            .load(std::sync::atomic::Ordering::Relaxed)
                            as u64,
                        &labels,
                    );
                })
                .build();
        }

        {
            let r = runtime.clone();
            let release_id = release_id.clone();
            meter
                .u64_observable_gauge("bot_instance_draining")
                .with_description("Bot deployment instance drain state: 1 while draining")
                .with_callback(move |observer| {
                    let labels = deployment_labels(&r, release_id.as_str());
                    observer.observe(if r.is_draining() { 1 } else { 0 }, &labels);
                })
                .build();
        }

        {
            let r = runtime.clone();
            let release_id = release_id.clone();
            meter
                .u64_observable_gauge("bot_instance_shutdown_when_empty")
                .with_description("Bot deployment instance exits when voice is empty: 1 when armed")
                .with_callback(move |observer| {
                    let labels = deployment_labels(&r, release_id.as_str());
                    observer.observe(if r.shutdown_when_empty() { 1 } else { 0 }, &labels);
                })
                .build();
        }

        {
            let r = runtime.clone();
            meter
                .u64_observable_gauge("bot_instance_force_shutdown_requested")
                .with_description("Bot deployment force shutdown state: 1 after force requested")
                .with_callback(move |observer| {
                    let labels = deployment_labels(&r, release_id.as_str());
                    observer.observe(if r.force_shutdown_requested() { 1 } else { 0 }, &labels);
                })
                .build();
        }

        {
            let m = metrics.clone();
            meter
                .u64_observable_gauge("uptime_seconds")
                .with_description("Bot process uptime in seconds")
                .with_unit("s")
                .with_callback(move |observer| {
                    observer.observe(m.start_time.elapsed().as_secs(), &[])
                })
                .build();
        }

        {
            let m = metrics.clone();
            meter
                .i64_observable_gauge("last_voice_packet_timestamp_seconds")
                .with_description("Unix timestamp in seconds of the last observed voice packet")
                .with_unit("s")
                .with_callback(move |observer| {
                    let ms = m
                        .last_voice_packet_time
                        .load(std::sync::atomic::Ordering::Relaxed);
                    observer.observe(ms / 1000, &[]);
                })
                .build();
        }

        {
            let m = metrics.clone();
            meter
                .u64_observable_gauge("last_voice_packet_age_seconds")
                .with_description("Seconds since the last observed voice packet")
                .with_unit("s")
                .with_callback(move |observer| {
                    let last_ms = m
                        .last_voice_packet_time
                        .load(std::sync::atomic::Ordering::Relaxed);
                    if last_ms > 0 {
                        let now_ms = chrono::Utc::now().timestamp_millis();
                        observer.observe(((now_ms - last_ms).max(0) / 1000) as u64, &[]);
                    }
                })
                .build();
        }

        u64_gauge!(
            "process_rss_bytes",
            "RSS memory usage in bytes",
            process_rss_bytes
        );
        u32_gauge!(
            "process_open_fds",
            "Open file descriptors",
            process_open_fds
        );
        u32_gauge!(
            "tokio_active_tasks",
            "Tokio runtime active tasks",
            tokio_active_tasks
        );
        u32_gauge!(
            "grpc_active_streams",
            "Current active gRPC dashboard streams",
            grpc_active_streams
        );
        u32_gauge!(
            "active_voice_connections",
            "Current active voice connections",
            active_voice_connections
        );
        u32_gauge!(
            "active_recordings",
            "Current active recordings",
            active_recordings
        );
        guild_gauge!(
            "guild_active_recordings",
            "Number of active recordings per guild",
            active_recordings
        );
        channel_gauge!(
            "channel_active_recordings",
            "Number of active recordings per channel",
            active_recordings
        );

        u32_counter!(
            "commands_executed",
            "Total commands executed",
            commands_executed
        );
        u32_counter!(
            "messages_received",
            "Total regular messages received",
            messages_received
        );
        u64_counter!(
            "voice_state_updates_received",
            "Total Discord voice state updates received",
            voice_state_updates_received
        );
        u64_counter!(
            "recordings_started",
            "Total recording writers opened",
            recordings_started
        );
        u64_counter!(
            "recordings_finished",
            "Total recording writers closed",
            recordings_finished
        );
        u64_counter!(
            "recording_finalize_errors",
            "Total recording writer finalization failures",
            recording_finalize_errors
        );
        u64_counter!(
            "audio_packets_received",
            "Total audio packets received",
            audio_packets_received
        );
        u64_counter!(
            "audio_packets_dropped",
            "Total audio packets dropped globally",
            audio_packets_dropped
        );
        u32_counter!(
            "writer_setup_failures",
            "Total file writer setup failures",
            writer_setup_failures
        );
        u32_counter!(
            "gateway_reconnects",
            "Total Discord gateway reconnects",
            gateway_reconnects
        );
        u32_counter!(
            "gateway_disconnects",
            "Total Discord gateway disconnects",
            gateway_disconnects
        );
        u32_counter!(
            "driver_reconnects",
            "Total Songbird driver reconnects",
            driver_reconnects
        );
        u32_counter!(
            "driver_disconnects",
            "Total Songbird driver disconnects",
            driver_disconnects
        );
        u32_counter!(
            "db_query_errors",
            "Total database query errors",
            db_query_errors
        );
        u32_counter!(
            "db_insert_failures",
            "Total database insert failures",
            db_insert_failures
        );

        guild_counter!(
            "guild_audio_packets_received",
            "Total audio packets received per guild",
            audio_packets_received,
            std::convert::identity
        );
        channel_counter!(
            "channel_audio_packets_received",
            "Total audio packets received per channel",
            audio_packets_received,
            std::convert::identity
        );
        guild_counter!(
            "guild_audio_packets_dropped",
            "Total audio packets dropped per guild",
            audio_packets_dropped,
            std::convert::identity
        );
        channel_counter!(
            "channel_audio_packets_dropped",
            "Total audio packets dropped per channel",
            audio_packets_dropped,
            std::convert::identity
        );
        guild_counter!(
            "guild_writer_setup_failures",
            "Total file writer setup failures per guild",
            writer_setup_failures,
            |v| v as u64
        );
        channel_counter!(
            "channel_writer_setup_failures",
            "Total file writer setup failures per channel",
            writer_setup_failures,
            |v| v as u64
        );
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

    /// Starts a background task to monitor process health (memory, FDs, active tasks).
    pub fn start_sysinfo_monitoring(metrics: Arc<Self>) {
        tokio::spawn(async move {
            let mut sys = sysinfo::System::new();
            let pid = sysinfo::get_current_pid().unwrap_or(sysinfo::Pid::from_u32(0));

            loop {
                // Memory (RSS) from sysinfo
                sys.refresh_processes_specifics(
                    sysinfo::ProcessesToUpdate::Some(&[pid]),
                    true,
                    sysinfo::ProcessRefreshKind::nothing().with_memory(),
                );
                if let Some(process) = sys.process(pid) {
                    metrics
                        .process_rss_bytes
                        .store(process.memory(), std::sync::atomic::Ordering::Relaxed);
                }

                // Open file descriptors: count entries in /proc/self/fd on linux
                #[cfg(target_os = "linux")]
                if let Ok(mut dir) = tokio::fs::read_dir("/proc/self/fd").await {
                    let mut count: u32 = 0;
                    while dir.next_entry().await.ok().flatten().is_some() {
                        count += 1;
                    }
                    metrics
                        .process_open_fds
                        .store(count, std::sync::atomic::Ordering::Relaxed);
                }

                // Tokio runtime task count (requires tokio_unstable)
                let task_count = tokio::runtime::Handle::current()
                    .metrics()
                    .num_alive_tasks() as u32;
                metrics
                    .tokio_active_tasks
                    .store(task_count, std::sync::atomic::Ordering::Relaxed);

                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            }
        });
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
