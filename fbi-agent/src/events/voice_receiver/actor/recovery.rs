//! Guild-scoped logical recording recovery.
//!
//! Physical writers close at real user/bot departure timestamps. Logical
//! sessions remain pending in PostgreSQL and resume as a new channel-bound
//! fragment with an explicit silence gap.

use std::sync::atomic::Ordering;

use serenity::model::id::{ChannelId, UserId};
use tracing::{info, warn};

use super::RecorderActor;
use crate::cast::ToI64;
use crate::events::voice_receiver::{
    disconnect::RECOVERABLE_DISCONNECT_TIMEOUT_MS, state::VoiceEventType,
};

impl RecorderActor {
    pub(super) async fn handle_client_disconnect(&mut self, user_id: u64, at_ms: i64) {
        info!(user_id, "voice client disconnected");
        if let Some(bot_ssrc) = self.recordings.remove_bot_user(user_id) {
            info!(user_id, bot_ssrc, "removed bot SSRC mapping");
            return;
        }

        self.pause_user_recording(user_id, None, "disconnect", true, at_ms)
            .await;
    }

    pub(super) async fn handle_user_voice_transition(
        &mut self,
        user_id: u64,
        old_channel_id: Option<ChannelId>,
        new_channel_id: Option<ChannelId>,
        afk_channel_id: Option<ChannelId>,
        at_ms: i64,
    ) {
        if self.recordings.remove_bot_user(user_id).is_some() {
            return;
        }

        let current = self.channel_id;
        let left_current = old_channel_id == Some(current) && new_channel_id != Some(current);
        let reached_current = new_channel_id == Some(current) && old_channel_id != Some(current);
        let entered_afk = new_channel_id.is_some() && new_channel_id == afk_channel_id;
        let disconnected = new_channel_id.is_none();

        if left_current {
            let (reason, starts_grace) = if disconnected {
                ("disconnect", true)
            } else if entered_afk {
                ("afk", true)
            } else {
                ("user_moved", false)
            };
            self.pause_user_recording(user_id, new_channel_id, reason, starts_grace, at_ms)
                .await;
            return;
        }

        if reached_current {
            self.resume_pending_user(user_id, current, at_ms).await;
            return;
        }

        if disconnected || entered_afk {
            let reason = if entered_afk { "afk" } else { "disconnect" };
            if let Err(err) = crate::database::logical_recordings::mark_pending_user_unavailable(
                &self.pool,
                crate::database::logical_recordings::PendingUserUnavailableRequest {
                    guild_id: self.guild_id.to_i64(),
                    user_id: user_id.to_i64(),
                    at_ms,
                    reason,
                    channel_id: new_channel_id.map(ToI64::to_i64),
                    has_afk_channel: self.has_afk_channel,
                    pending_cap_seconds: self.pending_cap_seconds,
                    owner_instance_id: &self.recording_owner_instance_id,
                },
            )
            .await
            {
                warn!(user_id, "failed to start pending grace: {}", err);
            }
        }
    }

    async fn pause_user_recording(
        &mut self,
        user_id: u64,
        to_channel_id: Option<ChannelId>,
        reason: &str,
        starts_grace: bool,
        at_ms: i64,
    ) {
        let current_channel = self.channel_id;
        let Some(recording) = self.recordings.remove_active_by_user(user_id) else {
            let paused = match crate::database::logical_recordings::pause_active_user(
                &self.pool,
                crate::database::logical_recordings::PauseActiveUserRequest {
                    guild_id: self.guild_id.to_i64(),
                    user_id: user_id.to_i64(),
                    at_ms,
                    reason,
                    from_channel_id: Some(current_channel.to_i64()),
                    to_channel_id: to_channel_id.map(ToI64::to_i64),
                    has_afk_channel: self.has_afk_channel,
                    starts_grace,
                    pending_cap_seconds: self.pending_cap_seconds,
                    owner_instance_id: &self.recording_owner_instance_id,
                },
            )
            .await
            {
                Ok(paused) => paused,
                Err(err) => {
                    warn!(user_id, "failed to pause silent logical session: {}", err);
                    false
                }
            };
            if !paused
                && starts_grace
                && let Err(err) =
                    crate::database::logical_recordings::mark_pending_user_unavailable(
                        &self.pool,
                        crate::database::logical_recordings::PendingUserUnavailableRequest {
                            guild_id: self.guild_id.to_i64(),
                            user_id: user_id.to_i64(),
                            at_ms,
                            reason,
                            channel_id: to_channel_id.map(ToI64::to_i64),
                            has_afk_channel: self.has_afk_channel,
                            pending_cap_seconds: self.pending_cap_seconds,
                            owner_instance_id: &self.recording_owner_instance_id,
                        },
                    )
                    .await
            {
                warn!(user_id, "failed to update pending user state: {}", err);
            }
            if paused {
                crate::events::voice::insert_voice_event(
                    &self.pool,
                    self.guild_id.to_i64(),
                    Some(current_channel.to_i64()),
                    user_id.to_i64(),
                    crate::events::voice::EVT_USER_RECORDING_PAUSE,
                )
                .await;
            }
            return;
        };

        let ssrc = recording.ssrc;
        let recording_session_id = recording.recording_session_id;
        self.finalize_recording(
            ssrc,
            recording,
            VoiceEventType::WriterClose,
            timestamp(at_ms),
        )
        .await;

        if let Err(err) = crate::database::logical_recordings::pause_session(
            &self.pool,
            crate::database::logical_recordings::PauseRequest {
                recording_session_id,
                at_ms,
                reason,
                from_channel_id: Some(current_channel.to_i64()),
                to_channel_id: to_channel_id.map(ToI64::to_i64),
                has_afk_channel: self.has_afk_channel,
                starts_grace,
                pending_cap_seconds: self.pending_cap_seconds,
                owner_instance_id: &self.recording_owner_instance_id,
            },
        )
        .await
        {
            warn!(
                user_id,
                recording_session_id, "failed to pause logical recording: {}", err
            );
        }

        crate::events::voice::insert_voice_event(
            &self.pool,
            self.guild_id.to_i64(),
            Some(current_channel.to_i64()),
            user_id.to_i64(),
            crate::events::voice::EVT_USER_RECORDING_PAUSE,
        )
        .await;
    }

    async fn pause_all_for_departure(
        &mut self,
        from_channel_id: ChannelId,
        to_channel_id: Option<ChannelId>,
        reason: &str,
        starts_grace: bool,
        at_ms: i64,
    ) {
        let recordings = self.recordings.take_all_active();
        for (ssrc, recording) in recordings {
            let user_id = recording.user_id;
            let recording_session_id = recording.recording_session_id;
            self.finalize_recording(
                ssrc,
                recording,
                VoiceEventType::WriterClose,
                timestamp(at_ms),
            )
            .await;

            if let Err(err) = crate::database::logical_recordings::pause_session(
                &self.pool,
                crate::database::logical_recordings::PauseRequest {
                    recording_session_id,
                    at_ms,
                    reason,
                    from_channel_id: Some(from_channel_id.to_i64()),
                    to_channel_id: to_channel_id.map(ToI64::to_i64),
                    has_afk_channel: self.has_afk_channel,
                    starts_grace,
                    pending_cap_seconds: self.pending_cap_seconds,
                    owner_instance_id: &self.recording_owner_instance_id,
                },
            )
            .await
            {
                warn!(
                    user_id,
                    recording_session_id,
                    "failed to pause logical recording at bot departure: {}",
                    err
                );
            }

            crate::events::voice::insert_voice_event(
                &self.pool,
                self.guild_id.to_i64(),
                Some(from_channel_id.to_i64()),
                user_id.to_i64(),
                crate::events::voice::EVT_RECORDING_PAUSE,
            )
            .await;
        }
        match crate::database::logical_recordings::owned_active_sessions(
            &self.pool,
            self.guild_id.to_i64(),
            &self.recording_owner_instance_id,
        )
        .await
        {
            Ok(sessions) => {
                for (recording_session_id, user_id) in sessions {
                    if let Err(err) = crate::database::logical_recordings::pause_session(
                        &self.pool,
                        crate::database::logical_recordings::PauseRequest {
                            recording_session_id,
                            at_ms,
                            reason,
                            from_channel_id: Some(from_channel_id.to_i64()),
                            to_channel_id: to_channel_id.map(ToI64::to_i64),
                            has_afk_channel: self.has_afk_channel,
                            starts_grace,
                            pending_cap_seconds: self.pending_cap_seconds,
                            owner_instance_id: &self.recording_owner_instance_id,
                        },
                    )
                    .await
                    {
                        warn!(
                            user_id,
                            recording_session_id,
                            "failed to pause silent logical session at bot departure: {}",
                            err
                        );
                        continue;
                    }
                    crate::events::voice::insert_voice_event(
                        &self.pool,
                        self.guild_id.to_i64(),
                        Some(from_channel_id.to_i64()),
                        user_id,
                        crate::events::voice::EVT_RECORDING_PAUSE,
                    )
                    .await;
                }
            }
            Err(err) => warn!(
                guild_id = self.guild_id.get(),
                "failed to find silent logical sessions at bot departure: {}", err
            ),
        }
        self.recordings.clear();
    }

    async fn resume_pending_user(&self, user_id: u64, channel_id: ChannelId, at_ms: i64) {
        match crate::database::logical_recordings::resume_pending_user(
            &self.pool,
            self.guild_id.to_i64(),
            user_id.to_i64(),
            channel_id.to_i64(),
            at_ms,
            &self.recording_owner_instance_id,
        )
        .await
        {
            Ok(Some(recording_session_id)) => {
                crate::events::voice::insert_voice_event(
                    &self.pool,
                    self.guild_id.to_i64(),
                    Some(channel_id.to_i64()),
                    user_id.to_i64(),
                    crate::events::voice::EVT_USER_RECORDING_RESUME,
                )
                .await;
                crate::events::voice::insert_voice_event(
                    &self.pool,
                    self.guild_id.to_i64(),
                    Some(channel_id.to_i64()),
                    user_id.to_i64(),
                    crate::events::voice::EVT_RECORDING_RESUME,
                )
                .await;
                info!(
                    user_id,
                    recording_session_id,
                    channel_id = channel_id.get(),
                    "logical recording resumed"
                );
            }
            Ok(None) => {}
            Err(err) => warn!(user_id, "failed to resume logical recording: {}", err),
        }
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

        if let Some(planned) = self.planned_handoff {
            self.pause_all_for_departure(
                planned.from_channel_id,
                planned.to_channel_id,
                if planned.to_channel_id.is_some() {
                    "handoff"
                } else {
                    "bot_departure"
                },
                false,
                at_ms,
            )
            .await;
            if planned.to_channel_id.is_some() {
                self.disconnected_at_ms = at_ms;
                self.recoverable_disconnect_deadline_ms =
                    at_ms.saturating_add(RECOVERABLE_DISCONNECT_TIMEOUT_MS as i64);
            }
            return;
        }

        if recoverable {
            if self.disconnected_at_ms == 0 {
                self.disconnected_at_ms = at_ms;
                self.recoverable_disconnect_deadline_ms =
                    at_ms.saturating_add(RECOVERABLE_DISCONNECT_TIMEOUT_MS as i64);
                self.pause_all_for_departure(
                    self.channel_id,
                    Some(self.channel_id),
                    "network",
                    true,
                    at_ms,
                )
                .await;
            }
            return;
        }

        self.pause_all_for_departure(
            self.channel_id,
            None,
            if finalize_empty_channel {
                "empty_channel"
            } else {
                "bot_departure"
            },
            false,
            at_ms,
        )
        .await;
        self.disconnected_at_ms = 0;
        self.recoverable_disconnect_deadline_ms = 0;
    }

    pub(super) async fn handle_driver_connected(&mut self, reconnect: bool, at_ms: i64) {
        if reconnect {
            self.metrics
                .driver_reconnects
                .fetch_add(1, Ordering::Relaxed);
        }
        let channel_id = self
            .planned_handoff
            .and_then(|planned| planned.to_channel_id)
            .unwrap_or(self.channel_id);
        self.complete_handoff(channel_id, at_ms).await;
    }

    pub(super) async fn complete_handoff(&mut self, channel_id: ChannelId, connected_at_ms: i64) {
        self.channel_id = channel_id;
        self.current_channel_id
            .store(channel_id.get(), Ordering::Relaxed);
        self.channel_metrics = self
            .metrics
            .channel_metrics(self.guild_id.get(), channel_id.get());
        self.planned_handoff = None;
        self.disconnected_at_ms = 0;
        self.recoverable_disconnect_deadline_ms = 0;

        let users = self.human_users_in_channel(channel_id);
        for user_id in users {
            self.resume_pending_user(user_id, channel_id, connected_at_ms)
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
                guild_id = self.guild_id.get(),
                "voice recovery timed out; tearing down stale call"
            );
            self.disconnected_at_ms = 0;
            self.recoverable_disconnect_deadline_ms = 0;
            self.planned_handoff = None;
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
    }

    /// The guild's voice call was removed (or the actor lost every handle).
    /// Pause all open logical sessions as a bot departure, then let the run
    /// loop terminate. Pending-session expiry is handled by the global expiry
    /// task, so nothing needs this actor alive after the departure is recorded.
    pub(super) async fn handle_voice_session_ended(&mut self, at_ms: i64) {
        if self.voice_session_ended {
            return;
        }
        self.voice_session_ended = true;
        if let Some(planned) = self.planned_handoff.take() {
            self.pause_all_for_departure(
                planned.from_channel_id,
                planned.to_channel_id,
                if planned.to_channel_id.is_some() {
                    "handoff"
                } else {
                    "bot_departure"
                },
                false,
                at_ms,
            )
            .await;
        } else {
            self.pause_all_for_departure(self.channel_id, None, "bot_departure", false, at_ms)
                .await;
        }
        self.disconnected_at_ms = 0;
        self.recoverable_disconnect_deadline_ms = 0;
    }

    /// Drops this guild's entry from the recorder registry so the sender is
    /// released and a future reconnect starts with a fresh actor.
    pub(super) async fn remove_from_registry(&self) {
        if let Some(registry) = &self.registry {
            registry.remove(self.guild_id).await;
        }
    }

    pub(super) async fn reap_stale_users(&mut self) {
        if self.disconnected_at_ms > 0 || !self.recordings.has_users() {
            return;
        }

        for (uid, ssrc) in self.scan_users_no_longer_in_recorded_channel() {
            warn!(uid, ssrc, "recording user no longer in recorded channel");
            self.pause_user_recording(
                uid,
                None,
                "disconnect",
                true,
                chrono::Utc::now().timestamp_millis(),
            )
            .await;
        }
    }

    fn scan_users_no_longer_in_recorded_channel(&self) -> Vec<(u64, u32)> {
        let mut users_to_remove = Vec::new();
        if let Some(guild) = self.ctx.cache.guild(self.guild_id) {
            for (uid, ssrc) in self.recordings.user_ssrc_pairs() {
                let still_here = guild
                    .voice_states
                    .get(&UserId::new(uid))
                    .is_some_and(|voice| voice.channel_id == Some(self.channel_id));
                if !still_here {
                    users_to_remove.push((uid, ssrc));
                }
            }
        }
        users_to_remove
    }

    fn human_users_in_channel(&self, channel_id: ChannelId) -> Vec<u64> {
        let Some(guild) = self.ctx.cache.guild(self.guild_id) else {
            return Vec::new();
        };
        guild
            .voice_states
            .iter()
            .filter_map(|(user_id, voice)| {
                if voice.channel_id != Some(channel_id) {
                    return None;
                }
                let member = guild.members.get(user_id)?;
                (!member.user.bot).then_some(user_id.get())
            })
            .collect()
    }
}

fn timestamp(at_ms: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp_millis(at_ms).unwrap_or_else(chrono::Utc::now)
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
