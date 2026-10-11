use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

use opentelemetry::KeyValue;
use opentelemetry::metrics::Meter;

use super::bot::BotMetrics;
use super::recording::GuildRecordingMetrics;
use crate::runtime::RuntimeState;

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

fn deployment_labels(runtime: &RuntimeState, release_id: &str) -> Vec<KeyValue> {
    vec![
        KeyValue::new("instance_id", runtime.config().instance_id.clone()),
        KeyValue::new("release_id", release_id.to_string()),
        KeyValue::new("role", runtime.role().as_str()),
    ]
}

/// Shared handles that every instrument callback captures.
struct Instruments {
    meter: Meter,
    metrics: Arc<BotMetrics>,
    runtime: Arc<RuntimeState>,
    release_id: String,
}

impl Instruments {
    fn context(&self) -> (Arc<BotMetrics>, Arc<RuntimeState>, String) {
        (
            self.metrics.clone(),
            self.runtime.clone(),
            self.release_id.clone(),
        )
    }

    fn counter(
        &self,
        name: &'static str,
        description: &'static str,
        read: impl Fn(&BotMetrics) -> u64 + Send + Sync + 'static,
    ) {
        let (metrics, runtime, release_id) = self.context();
        self.meter
            .u64_observable_counter(name)
            .with_description(description)
            .with_callback(move |observer| {
                let labels = deployment_labels(&runtime, release_id.as_str());
                observer.observe(read(&metrics), &labels);
            })
            .build();
    }

    fn gauge(
        &self,
        name: &'static str,
        description: &'static str,
        read: impl Fn(&BotMetrics) -> u64 + Send + Sync + 'static,
    ) {
        let (metrics, runtime, release_id) = self.context();
        self.meter
            .u64_observable_gauge(name)
            .with_description(description)
            .with_callback(move |observer| {
                let labels = deployment_labels(&runtime, release_id.as_str());
                observer.observe(read(&metrics), &labels);
            })
            .build();
    }

    fn guild_counter(
        &self,
        name: &'static str,
        description: &'static str,
        read: impl Fn(&GuildRecordingMetrics) -> u64 + Send + Sync + 'static,
    ) {
        let (metrics, runtime, release_id) = self.context();
        self.meter
            .u64_observable_counter(name)
            .with_description(description)
            .with_callback(move |observer| {
                for entry in metrics.guild_recording_metrics.iter() {
                    let mut labels = deployment_labels(&runtime, release_id.as_str());
                    labels.push(KeyValue::new("guild_id", entry.key().to_string()));
                    observer.observe(read(entry.value()), &labels);
                }
            })
            .build();
    }

    fn guild_gauge(
        &self,
        name: &'static str,
        description: &'static str,
        read: impl Fn(&GuildRecordingMetrics) -> u64 + Send + Sync + 'static,
    ) {
        let (metrics, runtime, release_id) = self.context();
        self.meter
            .u64_observable_gauge(name)
            .with_description(description)
            .with_callback(move |observer| {
                for entry in metrics.guild_recording_metrics.iter() {
                    let mut labels = deployment_labels(&runtime, release_id.as_str());
                    labels.push(KeyValue::new("guild_id", entry.key().to_string()));
                    observer.observe(read(entry.value()), &labels);
                }
            })
            .build();
    }

    fn channel_counter(
        &self,
        name: &'static str,
        description: &'static str,
        read: impl Fn(&GuildRecordingMetrics) -> u64 + Send + Sync + 'static,
    ) {
        let (metrics, runtime, release_id) = self.context();
        self.meter
            .u64_observable_counter(name)
            .with_description(description)
            .with_callback(move |observer| {
                for entry in metrics.channel_recording_metrics.iter() {
                    let (guild_id, channel_id) = entry.key();
                    let mut labels = deployment_labels(&runtime, release_id.as_str());
                    labels.push(KeyValue::new("guild_id", guild_id.to_string()));
                    labels.push(KeyValue::new("channel_id", channel_id.to_string()));
                    observer.observe(read(entry.value()), &labels);
                }
            })
            .build();
    }

    fn channel_gauge(
        &self,
        name: &'static str,
        description: &'static str,
        read: impl Fn(&GuildRecordingMetrics) -> u64 + Send + Sync + 'static,
    ) {
        let (metrics, runtime, release_id) = self.context();
        self.meter
            .u64_observable_gauge(name)
            .with_description(description)
            .with_callback(move |observer| {
                for entry in metrics.channel_recording_metrics.iter() {
                    let (guild_id, channel_id) = entry.key();
                    let mut labels = deployment_labels(&runtime, release_id.as_str());
                    labels.push(KeyValue::new("guild_id", guild_id.to_string()));
                    labels.push(KeyValue::new("channel_id", channel_id.to_string()));
                    observer.observe(read(entry.value()), &labels);
                }
            })
            .build();
    }
}

impl BotMetrics {
    /// Registers standard BotMetrics instruments to OpenTelemetry.
    pub fn register_otel_metrics(metrics: Arc<Self>, runtime: Arc<RuntimeState>) {
        let meter = opentelemetry::global::meter(crate::config::SERVICE_NAME);
        let release_id = deployment_release_id(runtime.config().instance_id.as_str());
        let instruments = Instruments {
            meter,
            metrics,
            runtime,
            release_id,
        };

        register_voice_presence_gauges(&instruments);
        register_deployment_gauges(&instruments);
        register_process_gauges(&instruments);
        register_process_usage(&instruments);
        register_event_counters(&instruments);
        register_recording_counters(&instruments);
    }
}

fn register_process_gauges(instruments: &Instruments) {
    instruments.gauge("process_rss_bytes", "RSS memory usage in bytes", |m| {
        m.process_rss_bytes.load(Relaxed)
    });
    instruments.gauge("process_open_fds", "Open file descriptors", |m| {
        u64::from(m.process_open_fds.load(Relaxed))
    });
    instruments.gauge("tokio_active_tasks", "Tokio runtime active tasks", |m| {
        u64::from(m.tokio_active_tasks.load(Relaxed))
    });
    instruments.gauge(
        "active_voice_connections",
        "Current active voice connections",
        |m| u64::from(m.active_voice_connections.load(Relaxed)),
    );
    instruments.gauge("active_recordings", "Current active recordings", |m| {
        u64::from(m.active_recordings.load(Relaxed))
    });
    instruments.guild_gauge(
        "guild_recorder_queue_depth",
        "Commands waiting in the guild recorder's queue at its last tick",
        |g| u64::from(g.recorder_queue_depth.load(Relaxed)),
    );
    instruments.guild_gauge(
        "guild_active_recordings",
        "Number of active recordings per guild",
        |g| u64::from(g.active_recordings.load(Relaxed)),
    );
    instruments.channel_gauge(
        "channel_active_recordings",
        "Number of active recordings per channel",
        |c| u64::from(c.active_recordings.load(Relaxed)),
    );
}

/// This process's CPU time, disk I/O and threads, read from `/proc/self` at
/// export. With the voice counters they give what a recording costs: their
/// rates over the same window, divided by the speakers (`audio_packets_received`
/// / 50 a second) or by `active_recordings`.
fn register_process_usage(instruments: &Instruments) {
    let (_, runtime, release_id) = instruments.context();
    instruments
        .meter
        .f64_observable_counter("fbi_agent_process_cpu_seconds")
        .with_description("CPU time of this process, by mode")
        .with_unit("s")
        .with_callback(move |observer| {
            let Some(cpu) = sakiot_process::cpu_seconds() else {
                return;
            };
            for (mode, seconds) in [("user", cpu.user), ("system", cpu.system)] {
                let mut labels = deployment_labels(&runtime, release_id.as_str());
                labels.push(KeyValue::new("mode", mode));
                observer.observe(seconds, &labels);
            }
        })
        .build();
    let (_, runtime, release_id) = instruments.context();
    instruments
        .meter
        .u64_observable_counter("fbi_agent_process_io_bytes")
        .with_description("Bytes this process read from and wrote to storage")
        .with_unit("By")
        .with_callback(move |observer| {
            let Some(io) = sakiot_process::io_counters() else {
                return;
            };
            for (op, bytes) in [("read", io.read_bytes), ("write", io.write_bytes)] {
                let mut labels = deployment_labels(&runtime, release_id.as_str());
                labels.push(KeyValue::new("op", op));
                observer.observe(bytes, &labels);
            }
        })
        .build();
    let (_, runtime, release_id) = instruments.context();
    instruments
        .meter
        .u64_observable_counter("fbi_agent_process_io_syscalls")
        .with_description("Read and write system calls this process made")
        .with_callback(move |observer| {
            let Some(io) = sakiot_process::io_counters() else {
                return;
            };
            for (op, calls) in [("read", io.read_syscalls), ("write", io.write_syscalls)] {
                let mut labels = deployment_labels(&runtime, release_id.as_str());
                labels.push(KeyValue::new("op", op));
                observer.observe(calls, &labels);
            }
        })
        .build();
    instruments.gauge(
        "fbi_agent_process_threads",
        "Threads of this process",
        |_| sakiot_process::status_value("Threads:").unwrap_or_default(),
    );
}

fn register_event_counters(instruments: &Instruments) {
    instruments.counter("commands_executed", "Total commands executed", |m| {
        u64::from(m.commands_executed.load(Relaxed))
    });
    instruments.counter(
        "messages_received",
        "Total regular messages received",
        |m| u64::from(m.messages_received.load(Relaxed)),
    );
    instruments.counter(
        "voice_state_updates_received",
        "Total Discord voice state updates received",
        |m| m.voice_state_updates_received.load(Relaxed),
    );
    instruments.counter(
        "gateway_reconnects",
        "Total Discord gateway reconnects",
        |m| u64::from(m.gateway_reconnects.load(Relaxed)),
    );
    instruments.counter(
        "gateway_disconnects",
        "Total Discord gateway disconnects",
        |m| u64::from(m.gateway_disconnects.load(Relaxed)),
    );
    instruments.counter(
        "driver_reconnects",
        "Total Songbird driver reconnects",
        |m| u64::from(m.driver_reconnects.load(Relaxed)),
    );
    instruments.counter(
        "driver_disconnects",
        "Total Songbird driver disconnects",
        |m| u64::from(m.driver_disconnects.load(Relaxed)),
    );
    instruments.counter("db_query_errors", "Total database query errors", |m| {
        u64::from(m.db_query_errors.load(Relaxed))
    });
    instruments.counter(
        "db_insert_failures",
        "Total database insert failures",
        |m| u64::from(m.db_insert_failures.load(Relaxed)),
    );
}

fn register_recording_counters(instruments: &Instruments) {
    instruments.counter(
        "recordings_started",
        "Total recording writers opened",
        |m| m.recordings_started.load(Relaxed),
    );
    instruments.counter(
        "recordings_finished",
        "Total recording writers closed",
        |m| m.recordings_finished.load(Relaxed),
    );
    instruments.counter(
        "recording_finalize_errors",
        "Total recording writer finalization failures",
        |m| m.recording_finalize_errors.load(Relaxed),
    );
    instruments.counter(
        "recovery_teardowns",
        "Total actor teardowns after a recoverable disconnect timed out",
        |m| m.recovery_teardowns.load(Relaxed),
    );
    instruments.counter(
        "recovery_teardown_manager_missing",
        "Total recovery teardowns that found no Songbird manager",
        |m| m.recovery_teardown_manager_missing.load(Relaxed),
    );
    instruments.counter(
        "recording_policy_suspensions",
        "Total times recording was suspended because the channel is excluded",
        |m| m.recording_policy_suspensions.load(Relaxed),
    );
    instruments.counter(
        "audio_packets_received",
        "Total audio packets received",
        |m| m.audio_packets_received.load(Relaxed),
    );
    instruments.counter(
        "voice_rtp_bytes_received",
        "RTP bytes of received voice packets, without IP and UDP headers",
        |m| m.voice_rtp_bytes_received.load(Relaxed),
    );
    instruments.counter(
        "audio_packets_dropped",
        "Total audio packets dropped globally",
        |m| m.audio_packets_dropped.load(Relaxed),
    );
    instruments.counter(
        "writer_setup_failures",
        "Total file writer setup failures",
        |m| u64::from(m.writer_setup_failures.load(Relaxed)),
    );
    instruments.counter(
        "waveform_peaks_failures",
        "Total recordings that lost their waveform peaks file",
        |m| m.waveform_peaks_failures.load(Relaxed),
    );
    instruments.guild_counter(
        "guild_audio_packets_received",
        "Total audio packets received per guild",
        |g| g.audio_packets_received.load(Relaxed),
    );
    instruments.channel_counter(
        "channel_audio_packets_received",
        "Total audio packets received per channel",
        |c| c.audio_packets_received.load(Relaxed),
    );
    instruments.guild_counter(
        "guild_audio_packets_dropped",
        "Total audio packets dropped per guild",
        |g| g.audio_packets_dropped.load(Relaxed),
    );
    instruments.channel_counter(
        "channel_audio_packets_dropped",
        "Total audio packets dropped per channel",
        |c| c.audio_packets_dropped.load(Relaxed),
    );
    instruments.guild_counter(
        "guild_writer_setup_failures",
        "Total file writer setup failures per guild",
        |g| u64::from(g.writer_setup_failures.load(Relaxed)),
    );
    instruments.channel_counter(
        "channel_writer_setup_failures",
        "Total file writer setup failures per channel",
        |c| u64::from(c.writer_setup_failures.load(Relaxed)),
    );
}

fn register_voice_presence_gauges(instruments: &Instruments) {
    let Instruments {
        meter,
        metrics,
        runtime,
        release_id,
    } = instruments;

    {
        let m = metrics.clone();
        let r = runtime.clone();
        let release_id = release_id.to_string();
        meter
            .u64_observable_gauge("voice_user_present")
            .with_description("Current voice channel presence per user")
            .with_callback(move |observer| {
                for entry in m.voice_users.iter() {
                    let key = entry.key();
                    let presence = entry.value();
                    let mut labels = deployment_labels(&r, release_id.as_str());
                    labels.extend([
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
                    ]);
                    observer.observe(1, &labels);
                }
            })
            .build();
    }

    {
        let m = metrics.clone();
        let r = runtime.clone();
        let release_id = release_id.to_string();
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
                    let mut labels = deployment_labels(&r, release_id.as_str());
                    labels.extend([
                        KeyValue::new("guild_id", guild_id.to_string()),
                        KeyValue::new("channel_id", channel_id.to_string()),
                        KeyValue::new("is_bot", is_bot.to_string()),
                    ]);
                    observer.observe(count, &labels);
                }
            })
            .build();
    }

    {
        let m = metrics.clone();
        let r = runtime.clone();
        let release_id = release_id.to_string();
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
                    let mut labels = deployment_labels(&r, release_id.as_str());
                    labels.extend([
                        KeyValue::new("guild_id", guild_id.to_string()),
                        KeyValue::new("channel_id", channel_id.to_string()),
                        KeyValue::new("state", state),
                    ]);
                    observer.observe(count, &labels);
                }
            })
            .build();
    }

    {
        let m = metrics.clone();
        let r = runtime.clone();
        let release_id = release_id.to_string();
        meter
            .u64_observable_gauge("recording_user_active")
            .with_description("Current active recording users")
            .with_callback(move |observer| {
                for entry in m.active_recording_users.iter() {
                    let key = entry.key();
                    let channel_id = entry.value();
                    let mut labels = deployment_labels(&r, release_id.as_str());
                    labels.extend([
                        KeyValue::new("guild_id", key.guild_id.to_string()),
                        KeyValue::new("channel_id", channel_id.to_string()),
                        KeyValue::new("user_id", key.user_id.to_string()),
                    ]);
                    observer.observe(1, &labels);
                }
            })
            .build();
    }
}

fn register_deployment_gauges(instruments: &Instruments) {
    let Instruments {
        meter,
        metrics,
        runtime,
        release_id,
    } = instruments;

    meter
        .u64_observable_gauge("bot_up")
        .with_description("Bot process health: 1 while the process exports metrics")
        .with_callback({
            let r = runtime.clone();
            let release_id = release_id.to_string();
            move |observer| {
                let labels = deployment_labels(&r, release_id.as_str());
                observer.observe(1, &labels);
            }
        })
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
                observer.observe(u64::from(m.active_voice_connections.load(Relaxed)), &labels);
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
                observer.observe(u64::from(m.active_recordings.load(Relaxed)), &labels);
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
        let r = runtime.clone();
        let release_id = release_id.to_string();
        meter
            .u64_observable_gauge("uptime_seconds")
            .with_description("Bot process uptime in seconds")
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
            .i64_observable_gauge("last_voice_packet_timestamp_seconds")
            .with_description("Unix timestamp in seconds of the last observed voice packet")
            .with_unit("s")
            .with_callback(move |observer| {
                let labels = deployment_labels(&r, release_id.as_str());
                let ms = m.last_voice_packet_time.load(Relaxed);
                observer.observe(ms / 1000, &labels);
            })
            .build();
    }

    {
        let m = metrics.clone();
        let r = runtime.clone();
        let release_id = release_id.to_string();
        meter
            .u64_observable_gauge("last_voice_packet_age_seconds")
            .with_description("Seconds since the last observed voice packet")
            .with_unit("s")
            .with_callback(move |observer| {
                let labels = deployment_labels(&r, release_id.as_str());
                let last_ms = m.last_voice_packet_time.load(Relaxed);
                if last_ms > 0 {
                    let now_ms = chrono::Utc::now().timestamp_millis();
                    observer.observe(((now_ms - last_ms).max(0) / 1000) as u64, &labels);
                }
            })
            .build();
    }
}
