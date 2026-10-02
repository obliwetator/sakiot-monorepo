use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64};

/// Per-guild recording health, mirroring the global counters on BotMetrics.
pub struct GuildRecordingMetrics {
    pub active_recordings: AtomicU32,
    pub writer_setup_failures: AtomicU32,
    pub audio_packets_received: AtomicU64,
    pub audio_packets_dropped: AtomicU64,
    pub last_voice_packet_time: AtomicI64,
    /// Commands waiting in the recorder's queue at the last delivered tick.
    /// Rising toward the queue's capacity means the recorder is falling
    /// behind and ticks will soon be dropped.
    pub recorder_queue_depth: AtomicU32,
}

impl GuildRecordingMetrics {
    pub fn new() -> Self {
        Self {
            active_recordings: AtomicU32::new(0),
            writer_setup_failures: AtomicU32::new(0),
            audio_packets_received: AtomicU64::new(0),
            audio_packets_dropped: AtomicU64::new(0),
            last_voice_packet_time: AtomicI64::new(0),
            recorder_queue_depth: AtomicU32::new(0),
        }
    }
}

impl Default for GuildRecordingMetrics {
    fn default() -> Self {
        Self::new()
    }
}
