use sakiot_paths::DataRoots;
use serenity::{
    async_trait,
    client::Context,
    model::id::{ChannelId, GuildId},
    prelude::{RwLock, TypeMap},
};
use songbird::{Event, EventContext, EventHandler as VoiceEventHandler};
use sqlx::{Pool, Postgres};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
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
    ctx: Arc<Context>,
    actor: actor::RecorderHandle,
}

#[derive(Default)]
pub(crate) struct RecordingCoordinatorRegistry {
    actors: Mutex<HashMap<u64, actor::RecorderHandle>>,
}

pub(crate) struct RecordingCoordinatorRegistryKey;

impl serenity::prelude::TypeMapKey for RecordingCoordinatorRegistryKey {
    type Value = Arc<RecordingCoordinatorRegistry>;
}

impl RecordingCoordinatorRegistry {
    async fn get_or_create(
        self: &Arc<Self>,
        pool: Pool<Postgres>,
        ctx: Arc<Context>,
        guild_id: GuildId,
        channel_id: ChannelId,
        metrics: Arc<crate::BotMetrics>,
    ) -> actor::RecorderHandle {
        let mut actors = self.actors.lock().await;
        if let Some(actor) = actors.get(&guild_id.get()) {
            return actor.clone();
        }
        let actor = actor::RecorderHandle::new(
            pool,
            ctx,
            guild_id,
            channel_id,
            metrics,
            Some(Arc::clone(self)),
        )
        .await;
        actors.insert(guild_id.get(), actor.clone());
        actor
    }

    async fn get(&self, guild_id: GuildId) -> Option<actor::RecorderHandle> {
        self.actors.lock().await.get(&guild_id.get()).cloned()
    }

    /// Removes the guild's entry, dropping the actor handle held here. The
    /// actor itself is the only caller; it runs this once its open sessions
    /// are closed so a reconnect creates a fresh actor.
    async fn remove(&self, guild_id: GuildId) {
        self.actors.lock().await.remove(&guild_id.get());
    }
}

impl Receiver {
    pub async fn new(
        pool: Pool<Postgres>,
        ctx: Arc<Context>,
        guild_id: GuildId,
        channel_id: ChannelId,
        metrics: Arc<crate::BotMetrics>,
    ) -> Self {
        let registry = {
            let data = ctx.data.read().await;
            data.get::<RecordingCoordinatorRegistryKey>().cloned()
        };
        let actor = match registry {
            Some(registry) => {
                registry
                    .get_or_create(pool, ctx.clone(), guild_id, channel_id, metrics)
                    .await
            }
            None => {
                warn!(
                    guild_id = guild_id.get(),
                    "recording coordinator registry missing; using unregistered actor"
                );
                actor::RecorderHandle::new(pool, ctx.clone(), guild_id, channel_id, metrics, None)
                    .await
            }
        };
        Self {
            guild_id,
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
                    self.actor.current_channel_id(),
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

async fn coordinator_from_ctx(ctx: &Context, guild_id: GuildId) -> Option<actor::RecorderHandle> {
    let registry = {
        let data = ctx.data.read().await;
        data.get::<RecordingCoordinatorRegistryKey>().cloned()
    }?;
    registry.get(guild_id).await
}

/// Notifies the guild's recorder actor that its voice call was removed so it
/// can pause open sessions and terminate. Best effort: if no actor is
/// registered (or the message cannot be delivered) the actor either already
/// exited or will exit through its handle-drop path.
pub(crate) async fn notify_voice_session_ended(
    data: &Arc<RwLock<TypeMap>>,
    guild_id: GuildId,
    at_ms: i64,
) {
    let registry = {
        let data_read = data.read().await;
        data_read.get::<RecordingCoordinatorRegistryKey>().cloned()
    };
    let Some(registry) = registry else {
        return;
    };
    if let Some(actor) = registry.get(guild_id).await {
        actor
            .send_control(actor::RecorderCommand::VoiceSessionEnded { at_ms })
            .await;
    }
}

pub(crate) async fn begin_handoff(
    ctx: &Context,
    guild_id: GuildId,
    from_channel_id: ChannelId,
    to_channel_id: Option<ChannelId>,
    has_afk_channel: bool,
    pending_cap_seconds: i64,
) {
    if let Some(actor) = coordinator_from_ctx(ctx, guild_id).await {
        actor
            .send_control(actor::RecorderCommand::BeginHandoff {
                from_channel_id,
                to_channel_id,
                has_afk_channel,
                pending_cap_seconds,
            })
            .await;
    }
}

pub(crate) async fn cancel_handoff(ctx: &Context, guild_id: GuildId) {
    if let Some(actor) = coordinator_from_ctx(ctx, guild_id).await {
        actor
            .send_control(actor::RecorderCommand::CancelHandoff)
            .await;
    }
}

pub(crate) async fn complete_handoff(
    ctx: &Context,
    guild_id: GuildId,
    channel_id: ChannelId,
    connected_at_ms: i64,
    has_afk_channel: bool,
    pending_cap_seconds: i64,
) {
    if let Some(actor) = coordinator_from_ctx(ctx, guild_id).await {
        actor
            .send_control(actor::RecorderCommand::CompleteHandoff {
                channel_id,
                connected_at_ms,
                has_afk_channel,
                pending_cap_seconds,
            })
            .await;
    }
}

pub(crate) async fn user_voice_transition(
    ctx: &Context,
    guild_id: GuildId,
    user_id: u64,
    old_channel_id: Option<ChannelId>,
    new_channel_id: Option<ChannelId>,
    afk_channel_id: Option<ChannelId>,
    at_ms: i64,
) -> bool {
    if let Some(actor) = coordinator_from_ctx(ctx, guild_id).await {
        actor
            .send_control(actor::RecorderCommand::UserVoiceTransition {
                user_id,
                old_channel_id,
                new_channel_id,
                afk_channel_id,
                at_ms,
            })
            .await;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::pause::silence_frames_for_gap_ms;

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
