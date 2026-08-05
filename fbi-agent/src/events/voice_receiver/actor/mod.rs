//! Recorder actor: owns per-channel recording state and processes commands
//! sent from the voice receiver event handlers.
//!
//! Split by concern:
//! - [`handle`]: the cloneable handle, command types, and queue accounting
//! - [`lifecycle`]: opening, writing, heartbeating, and finalizing recordings
//! - [`recovery`]: pause/resume, disconnect recovery, deadlines, stale reaping
//! - [`packets`]: RTP payload extraction and disconnect command mapping

mod handle;
mod lifecycle;
mod packets;
mod recovery;

pub(super) use handle::{RecorderCommand, RecorderHandle, VoicePacket};
pub(super) use packets::{disconnect_command, extract_opus_payload};

use std::{
    sync::{Arc, atomic::AtomicU64},
    time::Duration,
};

use serenity::{
    client::Context,
    model::id::{ChannelId, GuildId},
};
use sqlx::{Pool, Postgres};
use tokio::sync::{mpsc, watch};

use super::{
    recordings::{RecorderStats, Recordings},
    state::VoiceEventType,
};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const REAPER_INTERVAL: Duration = Duration::from_secs(60);
const DEADLINE_INTERVAL: Duration = Duration::from_secs(1);

struct RecorderActor {
    pool: Pool<Postgres>,
    ctx: Arc<Context>,
    guild_id: GuildId,
    channel_id: ChannelId,
    metrics: Arc<crate::BotMetrics>,
    guild_metrics: Arc<crate::GuildRecordingMetrics>,
    channel_metrics: Arc<crate::GuildRecordingMetrics>,
    recording_owner_instance_id: String,
    stats: Arc<RecorderStats>,
    recordings: Recordings,
    disconnected_at_ms: i64,
    recoverable_disconnect_deadline_ms: i64,
    current_channel_id: Arc<AtomicU64>,
    planned_handoff: Option<PlannedHandoff>,
    has_afk_channel: bool,
    pending_cap_seconds: i64,
    /// Set when the guild's voice call has been removed; the run loop exits
    /// after the current command is handled.
    voice_session_ended: bool,
    /// Registry entry for this guild; the actor removes itself on exit so a
    /// later reconnect starts with fresh state. `None` for unregistered actors.
    registry: Option<Arc<super::RecordingCoordinatorRegistry>>,
    /// Stable identity prevents an exiting actor from removing a newer
    /// registry generation after a panic/recovery race.
    actor_id: Arc<()>,
}

#[derive(Clone, Copy, Debug)]
struct PlannedHandoff {
    from_channel_id: ChannelId,
    to_channel_id: Option<ChannelId>,
}

impl RecorderActor {
    async fn run(
        mut self,
        mut rx: mpsc::Receiver<RecorderCommand>,
        mut shutdown_rx: watch::Receiver<Option<i64>>,
        terminated_tx: watch::Sender<bool>,
    ) {
        let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
        let mut reaper = tokio::time::interval(REAPER_INTERVAL);
        let mut deadlines = tokio::time::interval(DEADLINE_INTERVAL);

        loop {
            tokio::select! {
                biased;
                changed = shutdown_rx.changed() => {
                    let at_ms = if changed.is_ok() {
                        (*shutdown_rx.borrow()).unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
                    } else {
                        chrono::Utc::now().timestamp_millis()
                    };
                    self.handle_voice_session_ended(at_ms).await;
                    break;
                }
                command = rx.recv() => {
                    let Some(command) = command else {
                        self.handle_voice_session_ended(chrono::Utc::now().timestamp_millis())
                            .await;
                        break;
                    };
                    self.handle_command(command).await;
                }
                _ = heartbeat.tick() => {
                    self.heartbeat_active_recordings().await;
                }
                _ = reaper.tick() => {
                    self.reap_stale_users().await;
                }
                _ = deadlines.tick() => {
                    self.handle_deadlines(chrono::Utc::now().timestamp_millis()).await;
                }
            }
        }

        self.finalize_all_active_recordings(VoiceEventType::WriterClose, chrono::Utc::now())
            .await;
        self.clear_receiver_state();
        self.remove_from_registry().await;
        terminated_tx.send_replace(true);
    }

    async fn handle_command(&mut self, command: RecorderCommand) {
        match command {
            RecorderCommand::SpeakingState { user_id, ssrc } => {
                self.handle_speaking_state_update(user_id, ssrc).await;
            }
            RecorderCommand::VoiceTick { at_ms, packets } => {
                self.handle_voice_tick(at_ms, packets).await;
            }
            RecorderCommand::ClientDisconnect { user_id, at_ms } => {
                self.handle_client_disconnect(user_id, at_ms).await;
            }
            RecorderCommand::DriverDisconnected {
                should_count_disconnect,
                recoverable,
                finalize_empty_channel,
                at_ms,
            } => {
                self.handle_driver_disconnect(
                    should_count_disconnect,
                    recoverable,
                    finalize_empty_channel,
                    at_ms,
                )
                .await;
            }
            RecorderCommand::DriverConnected { reconnect, at_ms } => {
                self.handle_driver_connected(reconnect, at_ms).await;
            }
            RecorderCommand::BeginHandoff {
                from_channel_id,
                to_channel_id,
                has_afk_channel,
                pending_cap_seconds,
            } => {
                self.has_afk_channel = has_afk_channel;
                self.pending_cap_seconds = pending_cap_seconds;
                self.planned_handoff = Some(PlannedHandoff {
                    from_channel_id,
                    to_channel_id,
                });
            }
            RecorderCommand::CancelHandoff => {
                self.planned_handoff = None;
            }
            RecorderCommand::CompleteHandoff {
                channel_id,
                connected_at_ms,
                has_afk_channel,
                pending_cap_seconds,
            } => {
                self.has_afk_channel = has_afk_channel;
                self.pending_cap_seconds = pending_cap_seconds;
                self.complete_handoff(channel_id, connected_at_ms).await;
            }
            RecorderCommand::UserVoiceTransition {
                user_id,
                old_channel_id,
                new_channel_id,
                afk_channel_id,
                at_ms,
            } => {
                self.handle_user_voice_transition(
                    user_id,
                    old_channel_id,
                    new_channel_id,
                    afk_channel_id,
                    at_ms,
                )
                .await;
            }
        }
    }

    fn clear_receiver_state(&mut self) {
        self.recordings.clear();
        self.disconnected_at_ms = 0;
        self.recoverable_disconnect_deadline_ms = 0;
    }
}
