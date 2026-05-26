use sakiot_paths::{CLIPS_ROOT, RECORDING_ROOT};
use serenity::{
    async_trait,
    client::Context,
    model::id::{ChannelId, GuildId},
};
use songbird::{Event, EventContext, EventHandler as VoiceEventHandler};
use sqlx::{Pool, Postgres};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use tokio::sync::{Mutex, RwLock};
use tracing::warn;

mod background;
mod disconnect;
mod finalize;
mod pause;
mod persistence;
mod speaking;
mod state;
mod tick;

pub use state::VoiceEventType;

pub const RECORDING_FILE_PATH: &str = RECORDING_ROOT;
pub const CLIPS_FILE_PATH: &str = CLIPS_ROOT;

#[derive(Clone)]
pub struct Receiver {
    inner: Arc<InnerReceiver>,
}

pub(super) struct InnerReceiver {
    pub(super) pool: Pool<Postgres>,
    pub(super) channel_id: ChannelId,
    pub(super) ctx_main: Arc<Context>,
    pub(super) guild_id: GuildId,
    /// Active per-user recordings keyed by SSRC.
    pub(super) ssrc_writer_hashmap:
        Arc<RwLock<HashMap<u32, Arc<Mutex<state::UserRecording>>>>>,
    pub(super) user_id_hashmap: Arc<RwLock<HashMap<u64, u32>>>,
    pub(super) paused_recordings: Arc<RwLock<HashMap<u64, state::PausedRecording>>>,
    pub(super) paused_recording_token: AtomicU64,
    pub(super) bot_ssrcs: Arc<RwLock<HashSet<u32>>>,
    pub(super) bot_user_id_hashmap: Arc<RwLock<HashMap<u64, u32>>>,
    pub(super) metrics: Arc<crate::BotMetrics>,
    pub(super) guild_metrics: Arc<crate::GuildRecordingMetrics>,
    pub(super) channel_metrics: Arc<crate::GuildRecordingMetrics>,
    pub(super) recording_owner_instance_id: String,
    pub last_voice_packet_time: AtomicI64,
    /// Wallclock millisecond when the first non-bot user joined this session.
    /// 0 = inactive. Used to pad new joiners' files with leading silence so
    /// every per-user .ogg shares granule-zero = session-start.
    pub(super) session_start_ms: AtomicI64,
    /// Wallclock millisecond when a recoverable driver disconnect began.
    /// 0 = active/no pending resume.
    pub(super) disconnected_at_ms: AtomicI64,
}

impl Receiver {
    pub async fn new(
        pool: Pool<Postgres>,
        ctx: Arc<Context>,
        guild_id: GuildId,
        channel_id: ChannelId,
        metrics: Arc<crate::BotMetrics>,
    ) -> Self {
        let guild_metrics = metrics.guild_metrics(guild_id.get());
        let channel_metrics = metrics.channel_metrics(guild_id.get(), channel_id.get());
        let recording_owner_instance_id = {
            let data = ctx.data.read().await;
            data.get::<crate::runtime::RuntimeStateKey>()
                .map(|runtime| runtime.config().instance_id.clone())
                .unwrap_or_else(|| {
                    format!("{}-{}", crate::config::SERVICE_NAME, std::process::id())
                })
        };
        let inner = Arc::new(InnerReceiver {
            pool,
            ctx_main: ctx,
            user_id_hashmap: Arc::new(RwLock::new(HashMap::new())),
            ssrc_writer_hashmap: Arc::new(RwLock::new(HashMap::new())),
            paused_recordings: Arc::new(RwLock::new(HashMap::new())),
            paused_recording_token: AtomicU64::new(1),
            bot_ssrcs: Arc::new(RwLock::new(HashSet::new())),
            bot_user_id_hashmap: Arc::new(RwLock::new(HashMap::new())),
            guild_id,
            channel_id,
            metrics,
            guild_metrics,
            channel_metrics,
            recording_owner_instance_id,
            last_voice_packet_time: AtomicI64::new(chrono::Utc::now().timestamp_millis()),
            session_start_ms: AtomicI64::new(0),
            disconnected_at_ms: AtomicI64::new(0),
        });

        background::spawn_heartbeat(&inner);
        background::spawn_reaper(&inner);

        Self { inner }
    }

    pub fn last_voice_packet_time(&self) -> i64 {
        self.inner.last_voice_packet_time.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl VoiceEventHandler for Receiver {
    #[tracing::instrument(level = "trace", skip_all, name = "receiver_act", fields(guild_id = %self.inner.guild_id))]
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        use EventContext as Ctx;
        match ctx {
            Ctx::SpeakingStateUpdate(speaking) => {
                speaking::handle_speaking_state_update(&self.inner, speaking).await;
            }
            Ctx::RtpPacket(_packet) => {
                // Raw RTP — unused; we read Opus payload from VoiceTick instead.
            }
            Ctx::VoiceTick(tick) => {
                tick::handle_voice_tick(&self.inner, tick).await;
            }
            Ctx::RtcpPacket(_data) => {}
            Ctx::DriverDisconnect(data) => {
                disconnect::handle_driver_disconnect(&self.inner, data).await;
            }
            Ctx::DriverConnect(_) => {
                disconnect::handle_driver_connect(&self.inner).await;
            }
            Ctx::DriverReconnect(_) => {
                disconnect::handle_driver_reconnect(&self.inner).await;
            }
            Ctx::ClientDisconnect(client_disconnect) => {
                pause::handle_client_disconnect(&self.inner, client_disconnect).await;
            }
            _ => {
                warn!("Unhandled voice event context");
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::pause::{
        USER_REJOIN_RESUME_TIMEOUT_MS, paused_timeout_matches, silence_frames_for_gap_ms,
    };

    #[test]
    fn gap_ms_rounds_up_to_20ms_silence_frames() {
        assert_eq!(silence_frames_for_gap_ms(-1), 0);
        assert_eq!(silence_frames_for_gap_ms(0), 0);
        assert_eq!(silence_frames_for_gap_ms(1), 1);
        assert_eq!(silence_frames_for_gap_ms(20), 1);
        assert_eq!(silence_frames_for_gap_ms(21), 2);
        assert_eq!(silence_frames_for_gap_ms(40), 2);
        assert_eq!(silence_frames_for_gap_ms(41), 3);
    }

    #[test]
    fn ten_minute_user_rejoin_gap_maps_to_silence_frames() {
        assert_eq!(
            silence_frames_for_gap_ms(USER_REJOIN_RESUME_TIMEOUT_MS as i64),
            30_000
        );
    }

    #[test]
    fn stale_user_rejoin_timeout_does_not_match_new_pause_token() {
        assert!(paused_timeout_matches(Some(7), 7));
        assert!(!paused_timeout_matches(Some(8), 7));
        assert!(!paused_timeout_matches(None, 7));
    }

    #[test]
    fn user_recording_resume_events_are_distinct_from_bot_resume_events() {
        assert_ne!(
            crate::events::voice::EVT_USER_RECORDING_PAUSE,
            crate::events::voice::EVT_RECORDING_PAUSE
        );
        assert_ne!(
            crate::events::voice::EVT_USER_RECORDING_RESUME,
            crate::events::voice::EVT_RECORDING_RESUME
        );
        assert_eq!(crate::events::voice::EVT_USER_RECORDING_PAUSE, 20);
        assert_eq!(crate::events::voice::EVT_USER_RECORDING_RESUME, 21);
    }
}
