//! Recording lifecycle: speaking-state driven writer creation, per-tick audio
//! writes, heartbeats, and finalization (including the database bookkeeping
//! that goes with each step).

use std::{collections::HashMap, fs::File, io::BufWriter, sync::atomic::Ordering};

use serenity::model::{guild::Member, id::UserId};
use tracing::{debug, error, info, warn};

use super::{RecorderActor, VoicePacket};
use crate::cast::ToI64;
use crate::events::ogg_opus_writer::OggOpusWriter;
use crate::events::voice_receiver::{
    recordings::RemapOutcome,
    state::{RecordingFinalizeReason, UserRecording, VoiceEventType},
};

impl RecorderActor {
    pub(super) async fn handle_speaking_state_update(&mut self, user_id: Option<u64>, ssrc: u32) {
        debug!(
            "Speaking state update: user {:?} has SSRC {}",
            user_id, ssrc
        );

        let Some(user_id) = user_id else {
            error!("No user_id in SpeakingStateUpdate");
            return;
        };

        if self.recordings.remap_bot(user_id, ssrc) {
            return;
        }

        match self.recordings.remap_active_user(user_id, ssrc) {
            RemapOutcome::AlreadyActive => {
                debug!("Writer already active for ssrc {}", ssrc);
                return;
            }
            RemapOutcome::Remapped { from, to } => {
                info!(
                    "Remapped active writer for user {} from ssrc {} to {}",
                    user_id, from, to
                );
                return;
            }
            RemapOutcome::NotActive | RemapOutcome::Stale => {}
        }

        let Some(member) = self.resolve_member(user_id).await else {
            return;
        };

        if member.user.bot {
            self.recordings.insert_bot(user_id, ssrc);
            return;
        }

        if self.resume_paused_recording(user_id, ssrc).await {
            return;
        }

        if self.recordings.has_active_ssrc(ssrc) {
            debug!("Writer already active for ssrc {}", ssrc);
            return;
        }

        self.open_user_recording(user_id, ssrc, &member).await;
    }

    async fn resolve_member(&self, user_id: u64) -> Option<Member> {
        let guild = match self.ctx.cache.guild(self.guild_id) {
            Some(guild) => guild.to_owned(),
            None => {
                error!("Guild {} not in cache", self.guild_id);
                return None;
            }
        };

        if let Some(member) = guild.members.get(&UserId::new(user_id)).cloned() {
            return Some(member);
        }

        match guild.member(&self.ctx, UserId::new(user_id)).await {
            Ok(member) => Some(member.into_owned()),
            Err(err) => {
                error!("Failed to get member: {}", err);
                None
            }
        }
    }

    async fn open_user_recording(&mut self, user_id: u64, ssrc: u32, member: &Member) {
        info!("New writer for ssrc {}", ssrc);
        let now = chrono::Utc::now();

        let Some(recording_handle) = self.create_recording(now, user_id).await else {
            error!("Failed to create recording path for ssrc {}", ssrc);
            return;
        };

        let file = match File::create(format!("{}.ogg", recording_handle.path)) {
            Ok(file) => file,
            Err(err) => {
                error!("Failed to create file for ssrc {}: {}", ssrc, err);
                self.metrics
                    .track_writer_setup_failure(&self.guild_metrics, &self.channel_metrics);
                self.mark_recording_setup_failed(
                    recording_handle.audio_file_id,
                    RecordingFinalizeReason::FileCreate,
                )
                .await;
                return;
            }
        };

        let writer = match OggOpusWriter::new(BufWriter::new(file), ssrc, 0) {
            Ok(writer) => writer,
            Err(err) => {
                error!("Failed to init OggOpusWriter for ssrc {}: {}", ssrc, err);
                self.metrics
                    .track_writer_setup_failure(&self.guild_metrics, &self.channel_metrics);
                self.mark_recording_setup_failed(
                    recording_handle.audio_file_id,
                    RecordingFinalizeReason::WriterInit,
                )
                .await;
                return;
            }
        };

        self.recordings.insert_active(
            user_id,
            ssrc,
            UserRecording {
                writer,
                audio_file_id: recording_handle.audio_file_id,
                file_name: recording_handle.file_name,
                start_time: now,
                user_id,
                ssrc,
            },
        );

        crate::database::user_names::observe(
            &self.pool,
            self.guild_id.get(),
            &member.user,
            Some(member),
        )
        .await;

        self.metrics.track_recording_started(
            &self.guild_metrics,
            &self.channel_metrics,
            self.guild_id.get(),
            self.channel_id.get(),
            user_id,
        );

        self.insert_receiver_voice_event(
            user_id,
            ssrc,
            VoiceEventType::WriterOpen,
            "Writer opened",
        )
        .await;

        info!("1 file created for ssrc: {}", ssrc);
    }

    pub(super) async fn handle_voice_tick(&mut self, at_ms: i64, packets: Vec<VoicePacket>) {
        let packet_map: HashMap<u32, Vec<u8>> = packets
            .into_iter()
            .filter(|packet| !packet.opus.is_empty())
            .map(|packet| (packet.ssrc, packet.opus))
            .collect();
        let active_ssrcs = self.recordings.active_non_bot_ssrcs();

        for ssrc in active_ssrcs {
            let Some(recording) = self.recordings.active_get_mut(ssrc) else {
                continue;
            };
            let result = match packet_map.get(&ssrc) {
                Some(bytes) => {
                    self.stats
                        .last_voice_packet_time
                        .store(at_ms, Ordering::Relaxed);
                    self.metrics.track_last_voice_packet(
                        &self.guild_metrics,
                        &self.channel_metrics,
                        at_ms,
                    );
                    self.metrics
                        .track_audio_packet_received(&self.guild_metrics, &self.channel_metrics);
                    recording.writer.write_packet(bytes)
                }
                None => recording.writer.write_silence(1),
            };

            if let Err(err) = result {
                error!("Writer error for ssrc {}: {}", ssrc, err);
            }
        }
    }

    pub(super) async fn heartbeat_active_recordings(&self) {
        let audio_file_ids = self.recordings.tracked_audio_file_ids();
        if audio_file_ids.is_empty() {
            return;
        }

        if let Err(err) = crate::database::recordings::heartbeat_active_recordings(
            &self.pool,
            &audio_file_ids,
            &self.recording_owner_instance_id,
        )
        .await
        {
            warn!("recording heartbeat failed: {}", err);
            self.metrics.db_query_errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(super) async fn finalize_all_active_recordings(
        &mut self,
        event_type: VoiceEventType,
        close_time: chrono::DateTime<chrono::Utc>,
    ) {
        for (ssrc, recording) in self.recordings.take_all_active() {
            self.finalize_recording(ssrc, recording, event_type, close_time)
                .await;
        }

        for paused in self.recordings.take_all_paused() {
            self.finalize_recording(paused.ssrc, paused.recording, event_type, paused.paused_at)
                .await;
        }
    }

    pub(super) async fn finalize_writer_at(
        &mut self,
        ssrc: u32,
        event_type: VoiceEventType,
        close_time: chrono::DateTime<chrono::Utc>,
    ) {
        let Some(recording) = self.recordings.remove_active_by_ssrc(ssrc) else {
            return;
        };
        self.finalize_recording(ssrc, recording, event_type, close_time)
            .await;
    }

    pub(super) async fn finalize_recording(
        &self,
        ssrc: u32,
        mut recording: UserRecording,
        event_type: VoiceEventType,
        close_time: chrono::DateTime<chrono::Utc>,
    ) {
        let mut finalize_reason = finalize_reason(event_type);
        if let Err(err) = recording.writer.finish() {
            error!("Failed to finalize writer for ssrc {}: {}", ssrc, err);
            finalize_reason = RecordingFinalizeReason::WriterError;
            self.metrics.track_recording_finalize_error();
            self.insert_receiver_voice_event(
                recording.user_id,
                ssrc,
                VoiceEventType::WriterError,
                &format!("finish: {}", err),
            )
            .await;
        }

        let time_elapsed = close_time
            .signed_duration_since(recording.start_time)
            .num_milliseconds();
        self.metrics.track_recording_finished(
            &self.guild_metrics,
            &self.channel_metrics,
            self.guild_id.get(),
            self.channel_id.get(),
            recording.user_id,
            time_elapsed as f64 / 1000.0,
        );
        let file_name = recording.file_name.clone();
        let audio_file_id = recording.audio_file_id;
        let user_id = recording.user_id;
        let rec_ssrc = recording.ssrc;

        if let Err(err) = crate::database::recordings::finalize_recording(
            &self.pool,
            audio_file_id,
            &self.recording_owner_instance_id,
            time_elapsed,
            finalize_reason.id(),
        )
        .await
        {
            error!(
                file_name,
                audio_file_id, "failed to finalize recording row: {}", err
            );
            self.metrics.db_query_errors.fetch_add(1, Ordering::Relaxed);
        }

        self.insert_receiver_voice_event(user_id, rec_ssrc, event_type, "Writer closed")
            .await;
    }

    async fn create_recording(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        user_id: u64,
    ) -> Option<crate::database::recordings::RecordingHandle> {
        match crate::database::recordings::create_recording(
            &self.pool,
            self.guild_id.to_i64(),
            self.channel_id.to_i64(),
            user_id.to_i64(),
            now,
            &self.recording_owner_instance_id,
        )
        .await
        {
            Ok(handle) => Some(handle),
            Err(err) => {
                error!("failed to create recording db/path handle: {}", err);
                self.metrics
                    .db_insert_failures
                    .fetch_add(1, Ordering::Relaxed);
                self.metrics.db_query_errors.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    async fn insert_receiver_voice_event(
        &self,
        user_id: u64,
        ssrc: u32,
        event_type: VoiceEventType,
        details: &str,
    ) {
        if let Err(err) = crate::database::voice_events::insert_receiver_voice_event(
            &self.pool,
            self.guild_id.to_i64(),
            user_id.to_i64(),
            ssrc.to_i64(),
            event_type as i32,
            details,
        )
        .await
        {
            warn!(
                guild_id = self.guild_id.get(),
                user_id,
                ssrc,
                event_type_id = event_type as i32,
                "failed to insert receiver voice event: {}",
                err
            );
            self.metrics
                .db_insert_failures
                .fetch_add(1, Ordering::Relaxed);
            self.metrics.db_query_errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    async fn mark_recording_setup_failed(
        &self,
        audio_file_id: i64,
        reason: RecordingFinalizeReason,
    ) {
        if let Err(err) = crate::database::recordings::mark_recording_setup_failed(
            &self.pool,
            audio_file_id,
            &self.recording_owner_instance_id,
            reason.id(),
        )
        .await
        {
            warn!(
                audio_file_id,
                reason = reason.as_str(),
                "failed to mark recording setup failure: {}",
                err
            );
            self.metrics.db_query_errors.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn finalize_reason(event_type: VoiceEventType) -> RecordingFinalizeReason {
    match event_type {
        VoiceEventType::WriterOpen => RecordingFinalizeReason::Unknown,
        VoiceEventType::WriterClose => RecordingFinalizeReason::WriterClose,
        VoiceEventType::WriterError => RecordingFinalizeReason::WriterError,
        VoiceEventType::ZombieReaped => RecordingFinalizeReason::ZombieReaped,
    }
}
