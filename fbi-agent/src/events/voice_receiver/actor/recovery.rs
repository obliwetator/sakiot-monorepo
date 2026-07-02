//! Disconnect and rejoin recovery: pausing recordings while a user or the
//! driver is briefly gone, resuming with gap silence, enforcing recovery
//! deadlines, and reaping writers for users who left without a clean event.

use std::sync::atomic::Ordering;

use serenity::model::id::UserId;
use tracing::{error, info, warn};

use super::RecorderActor;
use crate::cast::ToI64;
use crate::events::voice_receiver::{
    disconnect::RECOVERABLE_DISCONNECT_TIMEOUT_MS,
    pause::{USER_REJOIN_RESUME_TIMEOUT_MS, paused_timeout_matches, silence_frames_for_gap_ms},
    state::{PausedRecording, VoiceEventType},
};

impl RecorderActor {
    pub(super) async fn handle_client_disconnect(&mut self, user_id: u64, at_ms: i64) {
        info!("client disconnected id: {}", user_id);

        if let Some(bot_ssrc) = self.recordings.remove_bot_user(user_id) {
            warn!("Removed bot with id: {} and ssrc: {}", user_id, bot_ssrc);
            return;
        }

        let Some(ssrc) = self.recordings.ssrc_for_user(user_id) else {
            warn!("tried to remove bot");
            return;
        };

        self.pause_recording_for_rejoin(user_id, ssrc, at_ms).await;
    }

    async fn pause_recording_for_rejoin(&mut self, user_id: u64, ssrc: u32, at_ms: i64) {
        let Some(recording) = self.recordings.remove_active_by_user(user_id) else {
            warn!(
                user_id,
                ssrc, "ClientDisconnect had no active writer to pause"
            );
            return;
        };

        let paused_at =
            chrono::DateTime::from_timestamp_millis(at_ms).unwrap_or_else(chrono::Utc::now);
        let token = self.paused_token;
        self.paused_token = self.paused_token.saturating_add(1);
        let paused = PausedRecording {
            recording,
            ssrc,
            paused_at,
            token,
            deadline_ms: at_ms.saturating_add(USER_REJOIN_RESUME_TIMEOUT_MS.to_i64()),
        };

        if let Some(previous) = self.recordings.insert_paused(user_id, paused) {
            warn!(
                user_id,
                previous_ssrc = previous.ssrc,
                "Replacing existing paused recording for user"
            );
            self.finalize_recording(
                previous.ssrc,
                previous.recording,
                VoiceEventType::WriterClose,
                previous.paused_at,
            )
            .await;
        }

        crate::events::voice::insert_voice_event(
            &self.pool,
            self.guild_id.to_i64(),
            Some(self.channel_id.to_i64()),
            user_id.to_i64(),
            crate::events::voice::EVT_USER_RECORDING_PAUSE,
        )
        .await;

        info!(
            user_id,
            ssrc,
            timeout_ms = USER_REJOIN_RESUME_TIMEOUT_MS,
            "Paused recording for user rejoin"
        );
    }

    pub(super) async fn resume_paused_recording(&mut self, user_id: u64, ssrc: u32) -> bool {
        let Some(mut paused) = self.recordings.take_paused(user_id) else {
            return false;
        };

        let now = chrono::Utc::now();
        let gap_ms = now
            .signed_duration_since(paused.paused_at)
            .num_milliseconds();
        let frames = silence_frames_for_gap_ms(gap_ms);

        if let Err(err) = paused.recording.writer.write_silence(frames) {
            error!(
                user_id,
                old_ssrc = paused.ssrc,
                new_ssrc = ssrc,
                "Failed to write user rejoin silence: {}",
                err
            );
        }
        paused.recording.ssrc = ssrc;

        self.recordings
            .insert_active(user_id, ssrc, paused.recording);

        crate::events::voice::insert_voice_event(
            &self.pool,
            self.guild_id.to_i64(),
            Some(self.channel_id.to_i64()),
            user_id.to_i64(),
            crate::events::voice::EVT_USER_RECORDING_RESUME,
        )
        .await;

        info!(
            user_id,
            old_ssrc = paused.ssrc,
            new_ssrc = ssrc,
            gap_ms,
            frames,
            "Resumed paused user recording"
        );
        true
    }

    pub(super) async fn handle_driver_disconnect(
        &mut self,
        should_count_disconnect: bool,
        recoverable: bool,
        finalize_empty_channel: bool,
        at_ms: i64,
    ) {
        info!(recoverable, finalize_empty_channel, "driver disconnected");

        if should_count_disconnect {
            self.metrics
                .driver_disconnects
                .fetch_add(1, Ordering::Relaxed);
        }

        if recoverable {
            if self.disconnected_at_ms == 0 {
                self.disconnected_at_ms = at_ms;
                self.recoverable_disconnect_deadline_ms =
                    at_ms.saturating_add(RECOVERABLE_DISCONNECT_TIMEOUT_MS.to_i64());
                info!("Recoverable disconnect recorded at {}", at_ms);
                for user_id in self.recordings.user_ids() {
                    crate::events::voice::insert_voice_event(
                        &self.pool,
                        self.guild_id.to_i64(),
                        Some(self.channel_id.to_i64()),
                        user_id.to_i64(),
                        crate::events::voice::EVT_RECORDING_PAUSE,
                    )
                    .await;
                }
            }
            return;
        }

        if finalize_empty_channel {
            info!(
                "Intentional disconnect with no human users in channel. Closing active recordings."
            );
        }

        self.disconnected_at_ms = 0;
        self.recoverable_disconnect_deadline_ms = 0;
        self.finalize_all_active_recordings(
            VoiceEventType::WriterClose,
            chrono::DateTime::from_timestamp_millis(at_ms).unwrap_or_else(chrono::Utc::now),
        )
        .await;
        self.clear_receiver_state();
    }

    pub(super) async fn handle_driver_connected(&mut self, reconnect: bool, at_ms: i64) {
        if reconnect {
            info!("Reconnected");
            self.metrics
                .driver_reconnects
                .fetch_add(1, Ordering::Relaxed);
        } else {
            info!("Connected");
        }
        self.resume_after_recoverable_disconnect(at_ms).await;
    }

    async fn resume_after_recoverable_disconnect(&mut self, at_ms: i64) {
        let disconnected_at_ms = self.disconnected_at_ms;
        if disconnected_at_ms == 0 {
            return;
        }

        self.disconnected_at_ms = 0;
        self.recoverable_disconnect_deadline_ms = 0;
        let reconnect_time =
            chrono::DateTime::from_timestamp_millis(at_ms).unwrap_or_else(chrono::Utc::now);
        let frames = silence_frames_for_gap_ms(at_ms - disconnected_at_ms);
        info!(
            "Resuming recordings after {}ms disconnect with {} silence frames",
            at_ms - disconnected_at_ms,
            frames
        );

        for user_id in self.recordings.user_ids() {
            crate::events::voice::insert_voice_event(
                &self.pool,
                self.guild_id.to_i64(),
                Some(self.channel_id.to_i64()),
                user_id.to_i64(),
                crate::events::voice::EVT_RECORDING_RESUME,
            )
            .await;
        }

        let active_ssrcs = self.recordings.active_non_bot_ssrcs();
        for ssrc in active_ssrcs {
            let Some(recording) = self.recordings.active_get_mut(ssrc) else {
                continue;
            };
            if let Err(err) = recording.writer.write_silence(frames) {
                error!(
                    "Failed to write reconnect gap silence for ssrc {}: {}",
                    ssrc, err
                );
            }
        }

        for (uid, ssrc) in self.scan_users_no_longer_in_voice_state() {
            warn!(
                "User {} (SSRC {}) is no longer in voice after reconnect. Closing writer.",
                uid, ssrc
            );
            self.finalize_writer_at(ssrc, VoiceEventType::WriterClose, reconnect_time)
                .await;
        }
    }

    pub(super) async fn handle_deadlines(&mut self, now_ms: i64) {
        if recoverable_disconnect_timed_out(
            self.disconnected_at_ms,
            self.recoverable_disconnect_deadline_ms,
            now_ms,
        ) {
            warn!(
                "Recoverable disconnect timed out after {}ms. Closing active recordings.",
                RECOVERABLE_DISCONNECT_TIMEOUT_MS
            );
            self.disconnected_at_ms = 0;
            self.recoverable_disconnect_deadline_ms = 0;
            self.finalize_all_active_recordings(VoiceEventType::WriterClose, chrono::Utc::now())
                .await;
            self.clear_receiver_state();
            let report = crate::events::voice::teardown_voice_session(
                &self.ctx.data,
                &self.pool,
                self.guild_id,
            )
            .await;
            if report.connected_after {
                warn!(
                    guild_id = self.guild_id.get(),
                    remove_error = report.remove_error,
                    "voice call remained connected after recovery timeout teardown"
                );
            }
        }

        let expired = self.recordings.expired_paused_user_ids(now_ms);

        for user_id in expired {
            let Some(paused) = self.recordings.take_paused(user_id) else {
                continue;
            };
            if !paused_timeout_matches(Some(paused.token), paused.token) {
                continue;
            }
            warn!(
                user_id,
                ssrc = paused.ssrc,
                timeout_ms = USER_REJOIN_RESUME_TIMEOUT_MS,
                "User rejoin resume timed out. Closing recording."
            );
            self.finalize_recording(
                paused.ssrc,
                paused.recording,
                VoiceEventType::WriterClose,
                paused.paused_at,
            )
            .await;
        }
    }

    pub(super) async fn reap_stale_users(&mut self) {
        if self.disconnected_at_ms > 0 || !self.recordings.has_users() {
            return;
        }

        for (uid, ssrc) in self.scan_users_no_longer_in_voice_state() {
            warn!(
                "Reaper: User {} (SSRC {}) is no longer in voice state. Closing writer.",
                uid, ssrc
            );
            self.finalize_writer_at(ssrc, VoiceEventType::ZombieReaped, chrono::Utc::now())
                .await;
        }
    }

    fn scan_users_no_longer_in_voice_state(&self) -> Vec<(u64, u32)> {
        let mut users_to_remove = Vec::new();
        if let Some(guild) = self.ctx.cache.guild(self.guild_id) {
            for (uid, ssrc) in self.recordings.user_ssrc_pairs() {
                if !guild.voice_states.contains_key(&UserId::new(uid)) {
                    users_to_remove.push((uid, ssrc));
                }
            }
        }
        users_to_remove
    }
}

fn recoverable_disconnect_timed_out(
    disconnected_at_ms: i64,
    deadline_ms: i64,
    now_ms: i64,
) -> bool {
    disconnected_at_ms > 0 && deadline_ms > 0 && now_ms >= deadline_ms
}

#[cfg(test)]
mod tests {
    use super::recoverable_disconnect_timed_out;

    #[test]
    fn reconnect_before_recovery_deadline_is_preserved() {
        assert!(!recoverable_disconnect_timed_out(1_000, 61_000, 60_999));
        assert!(!recoverable_disconnect_timed_out(0, 0, 61_000));
    }

    #[test]
    fn recovery_deadline_fires_at_most_once_after_state_reset() {
        assert!(recoverable_disconnect_timed_out(1_000, 61_000, 61_000));
        assert!(!recoverable_disconnect_timed_out(0, 0, 61_001));
    }
}
