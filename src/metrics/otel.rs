use opentelemetry::KeyValue;
use std::sync::Arc;

use super::bot::BotMetrics;

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

        register_voice_presence_gauges(&meter, &metrics);
        register_deployment_gauges(&meter, &metrics, &runtime, &release_id);

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
}

fn register_voice_presence_gauges(
    meter: &opentelemetry::metrics::Meter,
    metrics: &Arc<BotMetrics>,
) {
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
}

fn register_deployment_gauges(
    meter: &opentelemetry::metrics::Meter,
    metrics: &Arc<BotMetrics>,
    runtime: &Arc<crate::runtime::RuntimeState>,
    release_id: &str,
) {
    meter
        .u64_observable_gauge("bot_up")
        .with_description("Bot process health: 1 while the process exports metrics")
        .with_callback(|observer| observer.observe(1, &[]))
        .build();

    {
        let r = runtime.clone();
        let release_id = release_id.to_string();
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
        let release_id = release_id.to_string();
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
        let release_id = release_id.to_string();
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
        let release_id = release_id.to_string();
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
        let release_id = release_id.to_string();
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
        let release_id = release_id.to_string();
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
        let release_id = release_id.to_string();
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
}
