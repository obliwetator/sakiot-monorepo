//! Guild-scoped logical recording recovery.
//!
//! Physical writers close at real user/bot departure timestamps. Logical
//! sessions remain pending in PostgreSQL and resume as a new channel-bound
//! fragment with an explicit silence gap.

use std::collections::HashSet;
use std::sync::atomic::Ordering;

use serenity::model::id::{ChannelId, UserId};
use tracing::{info, warn};

use super::{RecorderActor, link::Departure, timestamp};
use crate::cast::ToI64;
use crate::events::voice_receiver::state::VoiceEventType;

impl RecorderActor {
    pub(super) async fn handle_client_disconnect(&mut self, user_id: u64, at_ms: i64) {
        info!(user_id, "voice client disconnected");
        if let Some(bot_ssrc) = self.recordings.remove_bot_user(user_id) {
            info!(user_id, bot_ssrc, "removed bot SSRC mapping");
            return;
        }

        self.opted_out.forget(user_id);
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
            self.opted_out.forget(user_id);
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

    /// Closes every open writer and pauses its logical session.
    ///
    /// Used both for real departures (bot leaves, network drop, handoff) and for
    /// policy suspension, where the bot stays connected but the channel must not
    /// be recorded. Pausing rather than finalizing keeps the session resumable
    /// with an explicit gap when recording is allowed again.
    async fn pause_all_for_departure(&mut self, departure: Departure, at_ms: i64) {
        let Departure {
            from_channel_id,
            to_channel_id,
            reason,
            starts_grace,
        } = departure;
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

    /// Resumes the user's pending logical session in `channel_id`, returning
    /// whether one was actually reopened (a user with no pending session, or an
    /// excluded channel, resumes nothing).
    async fn resume_pending_user(&self, user_id: u64, channel_id: ChannelId, at_ms: i64) -> bool {
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
                true
            }
            Ok(None) => false,
            Err(err) => {
                warn!(user_id, "failed to resume logical recording: {}", err);
                false
            }
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

        if let Some(departure) = self.link.driver_disconnected(
            self.channel_id,
            recoverable,
            finalize_empty_channel,
            at_ms,
        ) {
            self.pause_all_for_departure(departure, at_ms).await;
        }
    }

    pub(super) async fn handle_driver_connected(
        &mut self,
        reconnect: bool,
        channel_id: ChannelId,
        at_ms: i64,
    ) {
        if reconnect {
            self.metrics
                .driver_reconnects
                .fetch_add(1, Ordering::Relaxed);
        }
        // The driver reports the channel it actually connected to. An
        // externally moved bot (a drag or force-move we did not initiate)
        // produces no planned handoff, so trusting `self.channel_id` here left
        // the recorder — and with it the exclusion check and every fragment's
        // `channel_id` — pinned to the previous channel for the whole call.
        self.complete_handoff(channel_id, at_ms).await;
    }

    pub(super) async fn complete_handoff(&mut self, channel_id: ChannelId, connected_at_ms: i64) {
        self.channel_id = channel_id;
        self.current_channel_id
            .store(channel_id.get(), Ordering::Relaxed);
        self.channel_metrics = self
            .metrics
            .channel_metrics(self.guild_id.get(), channel_id.get());
        self.opted_out.clear();
        self.link.connected();
        // Re-evaluate the policy for the channel just entered before resuming
        // anything: a handoff into an excluded channel must not reopen logical
        // sessions.
        match self.channel_is_excluded(channel_id).await {
            Ok(false) => {
                self.policy.entered_channel(true);
                self.resume_users_in_channel(channel_id, connected_at_ms)
                    .await;
            }
            Ok(true) => {
                info!(
                    guild_id = self.guild_id.get(),
                    channel_id = channel_id.get(),
                    "joined a channel excluded by guild policy; not recording while connected here"
                );
                self.policy.entered_channel(false);
            }
            Err(error) => {
                warn!(
                    guild_id = self.guild_id.get(),
                    channel_id = channel_id.get(),
                    "recording policy check failed after channel change; recording stays suspended: {}",
                    error
                );
                self.metrics.db_query_errors.fetch_add(1, Ordering::Relaxed);
                self.policy.entered_channel(false);
            }
        }
    }

    /// Re-evaluates the guild's channel exclusion for the channel the actor is
    /// connected to, and each speaker's recording opt-out, at most once per
    /// second.
    ///
    /// A violation suspends recording without terminating the actor. Terminating
    /// it here used to leave the Songbird receiver attached to a dead handle, so
    /// the guild could not record again until the call was torn down and
    /// rejoined from scratch.
    pub(super) async fn refresh_recording_policy(&mut self, at_ms: i64) {
        if !self.policy.check_due(at_ms) {
            return;
        }
        match self.channel_is_excluded(self.channel_id).await {
            Ok(true) => self.suspend_recording_for_policy(at_ms).await,
            Ok(false) => {
                self.resume_recording_after_policy(at_ms).await;
                self.apply_user_opt_outs(at_ms).await;
            }
            Err(error) => {
                warn!(
                    guild_id = self.guild_id.get(),
                    channel_id = self.channel_id.get(),
                    "recording policy check failed; suspending recording to preserve privacy: {}",
                    error
                );
                self.metrics.db_query_errors.fetch_add(1, Ordering::Relaxed);
                self.suspend_recording_for_policy(at_ms).await;
            }
        }
    }

    /// Closes the writers of users who opted out of recording since the last
    /// check and reopens those who opted back in, so a change from the web app
    /// or `/recording` takes effect within a second. A failed check suspends
    /// recording, like a failed channel policy check.
    async fn apply_user_opt_outs(&mut self, at_ms: i64) {
        let recorded = self.recordings.user_ssrc_pairs();
        let remembered = self.opted_out.user_ids();
        if recorded.is_empty() && remembered.is_empty() {
            return;
        }
        let candidates: Vec<i64> = recorded
            .iter()
            .map(|(user_id, _)| user_id.to_i64())
            .chain(remembered.iter().map(|user_id| user_id.to_i64()))
            .collect();
        let opted_out: HashSet<i64> = match crate::database::opt_outs::opted_out_among(
            &self.pool,
            self.guild_id.to_i64(),
            &candidates,
        )
        .await
        {
            Ok(user_ids) => user_ids.into_iter().collect(),
            Err(error) => {
                warn!(
                    guild_id = self.guild_id.get(),
                    channel_id = self.channel_id.get(),
                    "recording opt-out check failed; suspending recording to preserve privacy: {}",
                    error
                );
                self.metrics.db_query_errors.fetch_add(1, Ordering::Relaxed);
                self.suspend_recording_for_policy(at_ms).await;
                return;
            }
        };

        for (user_id, ssrc) in recorded {
            if !opted_out.contains(&user_id.to_i64()) {
                continue;
            }
            info!(
                guild_id = self.guild_id.get(),
                user_id, "closing writer: the user opted out of recording"
            );
            self.pause_user_recording(user_id, Some(self.channel_id), "opted_out", false, at_ms)
                .await;
            self.opted_out.remember(user_id, ssrc);
        }

        let opted_back_in: Vec<(u64, u32)> = remembered
            .into_iter()
            .filter(|user_id| !opted_out.contains(&user_id.to_i64()))
            .filter_map(|user_id| self.opted_out.forget(user_id).map(|ssrc| (user_id, ssrc)))
            .collect();
        if !opted_back_in.is_empty() {
            let reopened = self.reopen_suspended_speakers(opted_back_in).await;
            info!(
                guild_id = self.guild_id.get(),
                reopened, "users opted back in to recording"
            );
        }
    }

    async fn channel_is_excluded(&self, channel_id: ChannelId) -> crate::database::DbResult<bool> {
        let excluded = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM guild_recording_policy WHERE guild_id = $1 AND $2 = ANY(excluded_channel_ids))",
            self.guild_id.to_i64(),
            channel_id.to_i64(),
        )
        .fetch_one(&self.pool)
        .await?;
        // `EXISTS` is NOT NULL; a NULL means the driver lost the query's shape,
        // which must fail closed (error) rather than read as "permitted".
        match excluded {
            Some(excluded) => Ok(excluded),
            None => Err(sqlx::Error::RowNotFound.into()),
        }
    }

    async fn suspend_recording_for_policy(&mut self, at_ms: i64) {
        // Keep the users with an open writer so their writers can be reopened
        // the moment the channel is allowed again.
        let Some(speakers) = self.policy.suspend(self.recordings.user_ssrc_pairs()) else {
            return;
        };
        self.metrics.record_recording_policy_suspension();
        info!(
            guild_id = self.guild_id.get(),
            channel_id = self.channel_id.get(),
            speakers,
            "recording suspended: channel is excluded by guild policy; the voice call stays up"
        );
        self.pause_all_for_departure(
            Departure {
                from_channel_id: self.channel_id,
                to_channel_id: Some(self.channel_id),
                reason: "channel_excluded",
                starts_grace: false,
            },
            at_ms,
        )
        .await;
    }

    async fn resume_recording_after_policy(&mut self, at_ms: i64) {
        let Some(speakers) = self.policy.allow() else {
            return;
        };
        let resumed_sessions = self.resume_users_in_channel(self.channel_id, at_ms).await;
        let reopened_writers = self.reopen_suspended_speakers(speakers).await;
        info!(
            guild_id = self.guild_id.get(),
            channel_id = self.channel_id.get(),
            resumed_sessions,
            reopened_writers,
            "recording resumed: channel is no longer excluded by guild policy"
        );
    }

    /// Reopens a writer for every user still in the channel who was speaking
    /// around the suspension.
    ///
    /// Discord does not repeat a speaking update while an utterance continues,
    /// so a speaker whose writer was closed by the suspension would otherwise
    /// stay unrecorded until their next transition - and the timeline would show
    /// synthetic silence for the whole remainder of that utterance.
    async fn reopen_suspended_speakers(&mut self, speakers: Vec<(u64, u32)>) -> usize {
        if speakers.is_empty() {
            return 0;
        }
        let present = self.human_users_in_channel(self.channel_id);
        let mut reopened = 0;
        for (user_id, ssrc) in speakers {
            if !present.contains(&user_id) {
                continue;
            }
            let Some(member) = self.resolve_member(user_id).await else {
                continue;
            };
            if member.user.bot {
                self.recordings.insert_bot(user_id, ssrc);
                continue;
            }
            if self.recordings.has_active_ssrc(ssrc) {
                continue;
            }
            self.open_user_recording(user_id, ssrc, &member).await;
            if self.recordings.has_active_ssrc(ssrc) {
                reopened += 1;
            }
        }
        reopened
    }

    async fn resume_users_in_channel(&self, channel_id: ChannelId, at_ms: i64) -> usize {
        let mut resumed = 0;
        for user_id in self.human_users_in_channel(channel_id) {
            if self.resume_pending_user(user_id, channel_id, at_ms).await {
                resumed += 1;
            }
        }
        resumed
    }

    /// Returns the departure timestamp when the actor must exit, or `None` to
    /// keep running. The run loop that calls this performs the departure
    /// handling and terminates, so this must never await the actor's own
    /// termination: the signal it would wait for is only sent after the loop it
    /// is running inside returns.
    pub(super) async fn handle_deadlines(&mut self, now_ms: i64) -> Option<i64> {
        if self.link.reconnect_deadline_passed(now_ms) {
            // The run loop must never block on the guild operation mutex: a
            // holder may be awaiting this actor's termination, which would
            // deadlock. If the lock is contended, keep the deadline state and
            // retry on the next tick — or exit via the shutdown signal if the
            // holder is a teardown that already signalled us.
            let report = crate::events::voice::try_teardown_voice_session(
                &self.env.data,
                &self.pool,
                self.guild_id,
            )
            .await?;
            warn!(
                guild_id = self.guild_id.get(),
                "voice recovery timed out; tearing down stale call"
            );
            self.link.torn_down();
            self.metrics.record_recovery_teardown();
            if report.manager_missing {
                self.metrics.record_recovery_teardown_manager_missing();
                warn!(
                    guild_id = self.guild_id.get(),
                    "voice teardown found no Songbird manager; recorder exiting"
                );
            }
            if report.connected_after {
                warn!(
                    guild_id = self.guild_id.get(),
                    remove_error = report.remove_error,
                    "voice call remained connected after recovery timeout teardown"
                );
            }
            let exit_at_ms = deadline_exit(report.connected_after, now_ms)?;
            // Publish the exit before the run loop records the departure, so a
            // concurrent get_or_create waits for termination instead of
            // attaching its receiver to an actor committed to exiting.
            self.stopping.store(true, Ordering::Release);
            return Some(exit_at_ms);
        }
        None
    }

    /// The guild's voice call was removed (or the actor lost every handle).
    /// Pause all open logical sessions as a bot departure, then let the run
    /// loop terminate. Pending-session expiry is handled by the global expiry
    /// task, so nothing needs this actor alive after the departure is recorded.
    pub(super) async fn handle_voice_session_ended(&mut self, at_ms: i64) {
        if let Some(departure) = self.link.end_session(self.channel_id) {
            self.pause_all_for_departure(departure, at_ms).await;
        }
    }

    /// Drops this guild's entry from the recorder registry so the sender is
    /// released and a future reconnect starts with a fresh actor.
    pub(super) async fn remove_from_registry(&self) {
        if let Some(registry) = &self.registry {
            registry.remove_if(self.guild_id, &self.actor_id).await;
        }
    }

    pub(super) async fn reap_stale_users(&mut self) {
        if self.link.is_reconnecting() || !self.recordings.has_users() {
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
        if let Some(guild) = self.env.cache.guild(self.guild_id) {
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
        let Some(guild) = self.env.cache.guild(self.guild_id) else {
            return Vec::new();
        };
        guild
            .voice_states
            .iter()
            .filter_map(|(user_id, voice)| {
                if voice.channel_id != Some(channel_id) {
                    return None;
                }
                // `voice.member` travels with the voice-state payload. The guild
                // member cache can lack users the bot never chunked, and treating
                // a present user as absent silently skipped the resume, so an
                // unknown bot flag counts as human (resuming a bot's session is a
                // no-op because bots have none).
                let is_bot = voice
                    .member
                    .as_ref()
                    .map(|member| member.user.bot)
                    .or_else(|| guild.members.get(user_id).map(|member| member.user.bot));
                (is_bot != Some(true)).then_some(user_id.get())
            })
            .collect()
    }
}

/// Whether a teardown result commits the actor to exiting.
///
/// A call that is still connected after teardown is a failed teardown, not a
/// departure: the actor stays alive so a later attempt can finish the job.
fn deadline_exit(connected_after: bool, now_ms: i64) -> Option<i64> {
    (!connected_after).then_some(now_ms)
}

#[cfg(test)]
mod tests {
    use super::deadline_exit;

    #[test]
    fn a_call_that_survives_teardown_keeps_the_actor_alive() {
        assert_eq!(deadline_exit(false, 61_000), Some(61_000));
        assert_eq!(deadline_exit(true, 61_000), None);
    }
}
