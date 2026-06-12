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

use std::{sync::Arc, time::Duration};

use serenity::{
    client::Context,
    model::id::{ChannelId, GuildId},
};
use sqlx::{Pool, Postgres};
use tokio::sync::mpsc;

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
    paused_token: u64,
    disconnected_at_ms: i64,
    recoverable_disconnect_deadline_ms: i64,
}

impl RecorderActor {
    async fn run(mut self, mut rx: mpsc::Receiver<RecorderCommand>) {
        let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
        let mut reaper = tokio::time::interval(REAPER_INTERVAL);
        let mut deadlines = tokio::time::interval(DEADLINE_INTERVAL);

        loop {
            tokio::select! {
                command = rx.recv() => {
                    let Some(command) = command else {
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
        }
    }

    fn clear_receiver_state(&mut self) {
        self.recordings.clear();
        self.disconnected_at_ms = 0;
        self.recoverable_disconnect_deadline_ms = 0;
    }
}
