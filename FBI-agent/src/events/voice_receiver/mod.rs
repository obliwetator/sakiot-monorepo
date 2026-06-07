use sakiot_paths::DataRoots;
use serenity::{
    async_trait,
    client::Context,
    model::id::{ChannelId, GuildId},
};
use songbird::{Event, EventContext, EventHandler as VoiceEventHandler};
use sqlx::{Pool, Postgres};
use std::sync::Arc;
use tracing::warn;

mod actor;
mod disconnect;
mod pause;
mod recordings;
mod state;

pub use state::VoiceEventType;

pub fn recording_file_path() -> std::path::PathBuf {
    DataRoots::from_env().recordings
}

pub fn clips_file_path() -> std::path::PathBuf {
    DataRoots::from_env().clips
}

#[derive(Clone)]
pub struct Receiver {
    guild_id: GuildId,
    channel_id: ChannelId,
    ctx: Arc<Context>,
    actor: actor::RecorderHandle,
}

impl Receiver {
    pub async fn new(
        pool: Pool<Postgres>,
        ctx: Arc<Context>,
        guild_id: GuildId,
        channel_id: ChannelId,
        metrics: Arc<crate::BotMetrics>,
    ) -> Self {
        let actor =
            actor::RecorderHandle::new(pool, ctx.clone(), guild_id, channel_id, metrics).await;
        Self {
            guild_id,
            channel_id,
            ctx,
            actor,
        }
    }

    pub fn last_voice_packet_time(&self) -> i64 {
        self.actor.stats().last_voice_packet_time()
    }
}

#[async_trait]
impl VoiceEventHandler for Receiver {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        use EventContext as Ctx;
        let now_ms = chrono::Utc::now().timestamp_millis();
        match ctx {
            Ctx::SpeakingStateUpdate(speaking) => {
                self.actor
                    .send_control(actor::RecorderCommand::SpeakingState {
                        user_id: speaking.user_id.map(|user_id| user_id.0),
                        ssrc: speaking.ssrc,
                    })
                    .await;
            }
            Ctx::RtpPacket(_packet) => {
                // Raw RTP is unused; Opus payload is read from VoiceTick.
            }
            Ctx::VoiceTick(tick) => {
                let packets = tick
                    .speaking
                    .iter()
                    .filter_map(|(ssrc, data)| {
                        actor::extract_opus_payload(data).and_then(|opus| {
                            (!opus.is_empty()).then_some(actor::VoicePacket { ssrc: *ssrc, opus })
                        })
                    })
                    .collect();
                self.actor.try_send_tick(now_ms, packets);
            }
            Ctx::RtcpPacket(_data) => {}
            Ctx::DriverDisconnect(data) => {
                let command = actor::disconnect_command(
                    &self.ctx,
                    self.guild_id,
                    self.channel_id,
                    data,
                    now_ms,
                );
                self.actor.send_control(command).await;
            }
            Ctx::DriverConnect(_) => {
                self.actor
                    .send_control(actor::RecorderCommand::DriverConnected {
                        reconnect: false,
                        at_ms: now_ms,
                    })
                    .await;
            }
            Ctx::DriverReconnect(_) => {
                self.actor
                    .send_control(actor::RecorderCommand::DriverConnected {
                        reconnect: true,
                        at_ms: now_ms,
                    })
                    .await;
            }
            Ctx::ClientDisconnect(client_disconnect) => {
                self.actor
                    .send_control(actor::RecorderCommand::ClientDisconnect {
                        user_id: client_disconnect.user_id.0,
                        at_ms: now_ms,
                    })
                    .await;
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
    use crate::cast::ToI64;

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
            silence_frames_for_gap_ms(USER_REJOIN_RESUME_TIMEOUT_MS.to_i64()),
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
