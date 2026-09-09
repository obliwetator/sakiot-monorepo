use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};

use serenity::model::id::{ChannelId, GuildId};
use sqlx::{Pool, Postgres};
use tokio::sync::{mpsc, watch};
use tracing::warn;

use super::{RecorderActor, RecorderEnv};
use crate::events::voice_receiver::recordings::{RecorderStats, Recordings};

const COMMAND_CAPACITY: usize = 256;
const CONTROL_SEND_TIMEOUT: Duration = Duration::from_millis(250);
/// Upper bound on the silence ticks one delivery replays after a stall. The
/// recorder writes 20 ms frames, so this caps the compensating burst at five
/// seconds instead of letting a long stall write a huge silent gap.
const MAX_COMPENSATED_TICKS: u32 = 250;

#[derive(Clone)]
pub(in crate::events::voice_receiver) struct RecorderHandle {
    tx: mpsc::Sender<RecorderCommand>,
    stats: Arc<RecorderStats>,
    metrics: Arc<crate::BotMetrics>,
    guild_metrics: Arc<crate::GuildRecordingMetrics>,
    channel_metrics: Arc<crate::GuildRecordingMetrics>,
    current_channel_id: Arc<AtomicU64>,
    actor_id: Arc<()>,
    stopping: Arc<AtomicBool>,
    /// Ticks the bounded queue refused since the last delivery; the next
    /// delivered tick replays them as silence to keep the recording aligned.
    dropped_ticks: Arc<AtomicU32>,
    shutdown_tx: watch::Sender<Option<i64>>,
    terminated_rx: watch::Receiver<bool>,
}

impl RecorderHandle {
    pub(in crate::events::voice_receiver) async fn new(
        pool: Pool<Postgres>,
        env: RecorderEnv,
        guild_id: GuildId,
        channel_id: ChannelId,
        metrics: Arc<crate::BotMetrics>,
        registry: Option<Arc<super::super::RecordingCoordinatorRegistry>>,
    ) -> Self {
        let guild_metrics = metrics.guild_metrics(guild_id.get());
        let channel_metrics = metrics.channel_metrics(guild_id.get(), channel_id.get());
        let recording_owner_instance_id = {
            let data = env.data.read().await;
            data.get::<crate::runtime::RuntimeStateKey>()
                .map(|runtime| runtime.config().instance_id.clone())
                .unwrap_or_else(|| {
                    format!("{}-{}", crate::config::SERVICE_NAME, std::process::id())
                })
        };
        let stats = Arc::new(RecorderStats::default());
        let current_channel_id = Arc::new(AtomicU64::new(channel_id.get()));
        let actor_id = Arc::new(());
        let stopping = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel(COMMAND_CAPACITY);
        let (shutdown_tx, shutdown_rx) = watch::channel(None);
        let (terminated_tx, terminated_rx) = watch::channel(false);
        let actor = RecorderActor {
            pool,
            env,
            guild_id,
            channel_id,
            metrics: metrics.clone(),
            guild_metrics: guild_metrics.clone(),
            channel_metrics: channel_metrics.clone(),
            recording_owner_instance_id,
            stats: stats.clone(),
            recordings: Recordings::new(stats.clone()),
            disconnected_at_ms: 0,
            recoverable_disconnect_deadline_ms: 0,
            current_channel_id: current_channel_id.clone(),
            stopping: Arc::clone(&stopping),
            planned_handoff: None,
            has_afk_channel: false,
            pending_cap_seconds: crate::database::logical_recordings::DEFAULT_PENDING_CAP_SECONDS,
            voice_session_ended: false,
            registry,
            actor_id: Arc::clone(&actor_id),
        };
        tokio::spawn(actor.run(rx, shutdown_rx, terminated_tx));

        Self {
            tx,
            stats,
            metrics,
            guild_metrics,
            channel_metrics,
            current_channel_id,
            actor_id,
            stopping,
            dropped_ticks: Arc::new(AtomicU32::new(0)),
            shutdown_tx,
            terminated_rx,
        }
    }

    pub(in crate::events::voice_receiver) fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    pub(in crate::events::voice_receiver) fn same_actor(&self, actor_id: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.actor_id, actor_id)
    }

    pub(in crate::events::voice_receiver) fn actor_id(&self) -> Arc<()> {
        Arc::clone(&self.actor_id)
    }

    /// Requests actor shutdown through an unbounded watch signal. Terminal
    /// delivery must not compete with the bounded packet/control queue.
    pub(in crate::events::voice_receiver) fn request_shutdown(&self, at_ms: i64) {
        self.stopping.store(true, Ordering::Release);
        self.shutdown_tx.send_replace(Some(at_ms));
    }

    /// Waits until cleanup completed, or until the actor task disappeared.
    /// A dropped sender means the task panicked; registry cleanup can still
    /// remove its dead handle and permit a fresh actor.
    pub(in crate::events::voice_receiver) async fn wait_terminated(&self) {
        let mut terminated_rx = self.terminated_rx.clone();
        if *terminated_rx.borrow() {
            return;
        }
        let _ = terminated_rx.wait_for(|terminated| *terminated).await;
    }

    pub(in crate::events::voice_receiver) fn stats(&self) -> &RecorderStats {
        &self.stats
    }

    pub(in crate::events::voice_receiver) fn current_channel_id(&self) -> ChannelId {
        ChannelId::new(self.current_channel_id.load(Ordering::Relaxed).max(1))
    }

    pub(in crate::events::voice_receiver) async fn send_control(&self, command: RecorderCommand) {
        match tokio::time::timeout(CONTROL_SEND_TIMEOUT, self.tx.send(command)).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => warn!("recorder actor closed before control event was delivered"),
            Err(_) => {
                warn!(
                    timeout_ms = CONTROL_SEND_TIMEOUT.as_millis() as u64,
                    "recorder control event timed out"
                );
            }
        }
    }

    pub(in crate::events::voice_receiver) fn try_send_tick(
        &self,
        at_ms: i64,
        packets: Vec<VoicePacket>,
    ) {
        let packet_count = packets.len();
        // Ticks dropped since the last delivery. The recorder advances every
        // active user by exactly one frame per tick, so a dropped tick has to
        // be replayed as silence or every later frame shifts earlier than its
        // wall-clock timestamp.
        let owed = self.dropped_ticks.swap(0, Ordering::Relaxed);
        // One delivery replays at most MAX_COMPENSATED_TICKS frames; the rest
        // stays owed, so even a stall longer than the cap ends with an aligned
        // timeline instead of losing the excess time for good.
        let (silence_ticks, remaining) = split_compensation(owed);
        match self.tx.try_send(RecorderCommand::VoiceTick {
            at_ms,
            packets,
            silence_ticks,
        }) {
            Ok(()) => {
                if remaining > 0 {
                    // fetch_add, not store: a concurrent sender may have added
                    // debt since the swap and a store would drop it.
                    self.dropped_ticks.fetch_add(remaining, Ordering::Relaxed);
                }
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Put the whole debt back, plus this tick, for the next
                // delivery.
                self.dropped_ticks
                    .fetch_add(owed.saturating_add(1), Ordering::Relaxed);
                let drop_count =
                    voice_tick_drop_count(self.stats.active_user_count(), packet_count);
                self.metrics.track_audio_packets_dropped(
                    &self.guild_metrics,
                    &self.channel_metrics,
                    drop_count,
                );
                warn!(
                    drop_count,
                    compensated_ticks = silence_ticks,
                    outstanding_ticks = remaining,
                    "recorder voice tick dropped because actor queue is full"
                );
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                warn!("recorder actor closed before voice tick was delivered");
            }
        }
    }
}

#[derive(Debug)]
pub(in crate::events::voice_receiver) struct VoicePacket {
    pub(in crate::events::voice_receiver) ssrc: u32,
    pub(in crate::events::voice_receiver) opus: Vec<u8>,
}

#[derive(Debug)]
pub(in crate::events::voice_receiver) enum RecorderCommand {
    SpeakingState {
        user_id: Option<u64>,
        ssrc: u32,
    },
    VoiceTick {
        at_ms: i64,
        packets: Vec<VoicePacket>,
        /// Dropped ticks this delivery compensates for, written as silence.
        silence_ticks: u32,
    },
    ClientDisconnect {
        user_id: u64,
        at_ms: i64,
    },
    DriverDisconnected {
        should_count_disconnect: bool,
        recoverable: bool,
        finalize_empty_channel: bool,
        at_ms: i64,
    },
    DriverConnected {
        reconnect: bool,
        at_ms: i64,
    },
    BeginHandoff {
        from_channel_id: ChannelId,
        to_channel_id: Option<ChannelId>,
        has_afk_channel: bool,
        pending_cap_seconds: i64,
    },
    CancelHandoff,
    CompleteHandoff {
        channel_id: ChannelId,
        connected_at_ms: i64,
        has_afk_channel: bool,
        pending_cap_seconds: i64,
    },
    UserVoiceTransition {
        user_id: u64,
        old_channel_id: Option<ChannelId>,
        new_channel_id: Option<ChannelId>,
        afk_channel_id: Option<ChannelId>,
        at_ms: i64,
    },
}

/// Splits the outstanding tick debt into the frames this delivery replays and
/// the debt carried to later deliveries. The cap bounds one delivery's silent
/// burst; the remainder is never discarded, so the timeline realigns even
/// after a stall longer than the cap.
fn split_compensation(owed: u32) -> (u32, u32) {
    let delivered = owed.min(MAX_COMPENSATED_TICKS);
    (delivered, owed - delivered)
}

fn voice_tick_drop_count(active_user_count: usize, packet_count: usize) -> u64 {
    active_user_count.max(packet_count).max(1) as u64
}

#[cfg(test)]
mod tests {
    use super::{split_compensation, voice_tick_drop_count};

    #[test]
    fn compensation_is_capped_per_delivery_but_never_discarded() {
        assert_eq!(split_compensation(0), (0, 0));
        assert_eq!(split_compensation(4), (4, 0));
        assert_eq!(split_compensation(250), (250, 0));
        // A stall longer than the cap keeps the excess owed for later
        // deliveries instead of losing the timeline offset permanently.
        assert_eq!(split_compensation(300), (250, 50));
        assert_eq!(split_compensation(u32::MAX), (250, u32::MAX - 250));
    }

    #[test]
    fn voice_tick_drop_count_uses_largest_available_signal() {
        assert_eq!(voice_tick_drop_count(0, 0), 1);
        assert_eq!(voice_tick_drop_count(3, 0), 3);
        assert_eq!(voice_tick_drop_count(1, 4), 4);
    }
}
