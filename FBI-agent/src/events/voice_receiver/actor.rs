use crate::cast::ToI64;
use std::{
    collections::HashMap,
    fs::File,
    io::BufWriter,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use serenity::{
    client::Context,
    model::{
        guild::Member,
        id::{ChannelId, GuildId, UserId},
    },
};
use songbird::packet::{Packet, PacketSize, rtp::RtpExtensionPacket};
use sqlx::{Pool, Postgres};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::{
    disconnect::{RECOVERABLE_DISCONNECT_TIMEOUT_MS, recording_channel_has_human_members},
    pause::{USER_REJOIN_RESUME_TIMEOUT_MS, paused_timeout_matches, silence_frames_for_gap_ms},
    recordings::{RecorderStats, Recordings, RemapOutcome},
    state::{PausedRecording, RecordingFinalizeReason, UserRecording, VoiceEventType},
};
use crate::events::ogg_opus_writer::OggOpusWriter;

const COMMAND_CAPACITY: usize = 256;
const CONTROL_SEND_TIMEOUT: Duration = Duration::from_millis(250);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const REAPER_INTERVAL: Duration = Duration::from_secs(60);
const DEADLINE_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub(super) struct RecorderHandle {
    tx: mpsc::Sender<RecorderCommand>,
    stats: Arc<RecorderStats>,
    metrics: Arc<crate::BotMetrics>,
    guild_metrics: Arc<crate::GuildRecordingMetrics>,
    channel_metrics: Arc<crate::GuildRecordingMetrics>,
}

impl RecorderHandle {
    pub(super) async fn new(
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
        let stats = Arc::new(RecorderStats::default());
        let (tx, rx) = mpsc::channel(COMMAND_CAPACITY);
        let actor = RecorderActor {
            pool,
            ctx,
            guild_id,
            channel_id,
            metrics: metrics.clone(),
            guild_metrics: guild_metrics.clone(),
            channel_metrics: channel_metrics.clone(),
            recording_owner_instance_id,
            stats: stats.clone(),
            recordings: Recordings::new(stats.clone()),
            paused_token: 1,
            disconnected_at_ms: 0,
            recoverable_disconnect_deadline_ms: 0,
        };
        tokio::spawn(actor.run(rx));

        Self {
            tx,
            stats,
            metrics,
            guild_metrics,
            channel_metrics,
        }
    }

    pub(super) fn stats(&self) -> &RecorderStats {
        &self.stats
    }

    pub(super) async fn send_control(&self, command: RecorderCommand) {
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

    pub(super) fn try_send_tick(&self, at_ms: i64, packets: Vec<VoicePacket>) {
        let packet_count = packets.len();
        match self
            .tx
            .try_send(RecorderCommand::VoiceTick { at_ms, packets })
        {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                let drop_count =
                    voice_tick_drop_count(self.stats.active_user_count(), packet_count);
                self.metrics.track_audio_packets_dropped(
                    &self.guild_metrics,
                    &self.channel_metrics,
                    drop_count,
                );
                warn!(
                    drop_count,
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
pub(super) struct VoicePacket {
    pub(super) ssrc: u32,
    pub(super) opus: Vec<u8>,
}

#[derive(Debug)]
pub(super) enum RecorderCommand {
    SpeakingState {
        user_id: Option<u64>,
        ssrc: u32,
    },
    VoiceTick {
        at_ms: i64,
        packets: Vec<VoicePacket>,
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
}

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

    async fn handle_speaking_state_update(&mut self, user_id: Option<u64>, ssrc: u32) {
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

    async fn handle_voice_tick(&mut self, at_ms: i64, packets: Vec<VoicePacket>) {
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

    async fn handle_client_disconnect(&mut self, user_id: u64, at_ms: i64) {
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

    async fn resume_paused_recording(&mut self, user_id: u64, ssrc: u32) -> bool {
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

    async fn handle_driver_disconnect(
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

    async fn handle_driver_connected(&mut self, reconnect: bool, at_ms: i64) {
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

    async fn handle_deadlines(&mut self, now_ms: i64) {
        if self.disconnected_at_ms > 0
            && self.recoverable_disconnect_deadline_ms > 0
            && now_ms >= self.recoverable_disconnect_deadline_ms
        {
            warn!(
                "Recoverable disconnect timed out after {}ms. Closing active recordings.",
                RECOVERABLE_DISCONNECT_TIMEOUT_MS
            );
            self.disconnected_at_ms = 0;
            self.recoverable_disconnect_deadline_ms = 0;
            self.finalize_all_active_recordings(VoiceEventType::WriterClose, chrono::Utc::now())
                .await;
            self.clear_receiver_state();
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

    async fn reap_stale_users(&mut self) {
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

    async fn heartbeat_active_recordings(&self) {
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

    async fn finalize_all_active_recordings(
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

    async fn finalize_writer_at(
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

    async fn finalize_recording(
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

    fn clear_receiver_state(&mut self) {
        self.recordings.clear();
        self.disconnected_at_ms = 0;
        self.recoverable_disconnect_deadline_ms = 0;
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

fn voice_tick_drop_count(active_user_count: usize, packet_count: usize) -> u64 {
    active_user_count.max(packet_count).max(1) as u64
}

fn finalize_reason(event_type: VoiceEventType) -> RecordingFinalizeReason {
    match event_type {
        VoiceEventType::WriterOpen => RecordingFinalizeReason::Unknown,
        VoiceEventType::WriterClose => RecordingFinalizeReason::WriterClose,
        VoiceEventType::WriterError => RecordingFinalizeReason::WriterError,
        VoiceEventType::ZombieReaped => RecordingFinalizeReason::ZombieReaped,
    }
}

pub(super) fn extract_opus_payload(
    data: &songbird::events::context_data::VoiceData,
) -> Option<Vec<u8>> {
    data.packet.as_ref().map(|rtp| {
        let view = rtp.rtp();
        let payload = view.payload();
        let start = rtp.payload_offset.min(payload.len());
        let end = rtp.payload_end_pad.min(payload.len());
        if end <= start {
            return Vec::new();
        }
        let body = &payload[start..end];
        let opus = if view.get_extension() != 0 {
            match RtpExtensionPacket::new(body) {
                Some(ext) => {
                    let off = ext.packet_size();
                    if off >= body.len() {
                        &[][..]
                    } else {
                        &body[off..]
                    }
                }
                None => body,
            }
        } else {
            body
        };
        opus.to_vec()
    })
}

pub(super) fn disconnect_command(
    ctx: &Context,
    guild_id: GuildId,
    channel_id: ChannelId,
    data: &songbird::events::context_data::DisconnectData<'_>,
    at_ms: i64,
) -> RecorderCommand {
    use songbird::events::context_data::{DisconnectKind, DisconnectReason};

    let should_count_disconnect = matches!(data.kind, DisconnectKind::Runtime)
        || !matches!(data.reason, Some(DisconnectReason::Requested));
    let channel_has_human_members = recording_channel_has_human_members(ctx, guild_id, channel_id);
    let finalize_empty_channel = super::disconnect::should_finalize_empty_channel_disconnect(
        data.reason.as_ref(),
        channel_has_human_members,
    );
    let recoverable = super::disconnect::should_resume_recordings_for_disconnect(
        data.reason.as_ref(),
        super::disconnect::RESUME_INTENTIONAL_DISCONNECTS_FOR_TESTING,
    ) && !finalize_empty_channel;

    RecorderCommand::DriverDisconnected {
        should_count_disconnect,
        recoverable,
        finalize_empty_channel,
        at_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::voice_tick_drop_count;

    #[test]
    fn voice_tick_drop_count_uses_largest_available_signal() {
        assert_eq!(voice_tick_drop_count(0, 0), 1);
        assert_eq!(voice_tick_drop_count(3, 0), 3);
        assert_eq!(voice_tick_drop_count(1, 4), 4);
    }
}
