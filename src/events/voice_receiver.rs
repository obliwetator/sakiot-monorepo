use chrono::Datelike;
use sakiot_paths::{CLIPS_ROOT, RECORDING_ROOT, RecordingKey};
use serenity::{
    async_trait,
    client::Context,
    model::id::{ChannelId, GuildId},
};
use songbird::{
    Event, EventContext, EventHandler as VoiceEventHandler,
    events::context_data::{ConnectData, DisconnectData, DisconnectReason},
    model::payload::{ClientDisconnect, Speaking},
    packet::{Packet, PacketSize, rtp::RtpExtensionPacket},
};
use sqlx::{Pool, Postgres};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, warn};

use crate::events::ogg_opus_writer::OggOpusWriter;

pub const RECORDING_FILE_PATH: &str = RECORDING_ROOT;
pub const CLIPS_FILE_PATH: &str = CLIPS_ROOT;
const RECOVERABLE_DISCONNECT_TIMEOUT_MS: u64 = 60_000;
const USER_REJOIN_RESUME_TIMEOUT_MS: u64 = 10 * 60 * 1000;
// Without this only way to test if pray discord randomly disconncts our bot. Need to manually toggle
// CAVEAT: this includes bot self disconnects
// TODO: Remote toggle for easier testing. No need to recompile
const RESUME_INTENTIONAL_DISCONNECTS_FOR_TESTING: bool = true;

#[repr(i32)]
#[derive(Clone, Copy)]
pub enum VoiceEventType {
    WriterOpen = 1,
    WriterClose = 2,
    WriterError = 3,
    ZombieReaped = 4,
}

/// One per-user recording: the streaming writer plus the metadata needed to
/// finalize the audio_files row when the writer closes.
struct UserRecording {
    writer: OggOpusWriter<BufWriter<File>>,
    file_name: String,
    start_time: chrono::DateTime<chrono::Utc>,
    user_id: u64,
    ssrc: u32,
}

#[derive(Clone)]
struct PausedRecording {
    recording: Arc<Mutex<UserRecording>>,
    ssrc: u32,
    paused_at: chrono::DateTime<chrono::Utc>,
    token: u64,
}

#[derive(Clone)]
pub struct Receiver {
    inner: Arc<InnerReceiver>,
}

pub struct InnerReceiver {
    pool: Pool<Postgres>,
    channel_id: ChannelId,
    ctx_main: Arc<Context>,
    guild_id: GuildId,
    /// Active per-user recordings keyed by SSRC.
    ssrc_writer_hashmap: Arc<RwLock<HashMap<u32, Arc<Mutex<UserRecording>>>>>,
    user_id_hashmap: Arc<RwLock<HashMap<u64, u32>>>,
    paused_recordings: Arc<RwLock<HashMap<u64, PausedRecording>>>,
    paused_recording_token: AtomicU64,
    bot_ssrcs: Arc<RwLock<HashSet<u32>>>,
    bot_user_id_hashmap: Arc<RwLock<HashMap<u64, u32>>>,
    metrics: Arc<crate::BotMetrics>,
    guild_metrics: Arc<crate::GuildRecordingMetrics>,
    channel_metrics: Arc<crate::GuildRecordingMetrics>,
    recording_owner_instance_id: String,
    pub last_voice_packet_time: AtomicI64,
    /// Wallclock millisecond when the first non-bot user joined this session.
    /// 0 = inactive. Used to pad new joiners' files with leading silence so
    /// every per-user .ogg shares granule-zero = session-start.
    session_start_ms: AtomicI64,
    /// Wallclock millisecond when a recoverable driver disconnect began.
    /// 0 = active/no pending resume.
    disconnected_at_ms: AtomicI64,
}

impl Drop for Receiver {
    fn drop(&mut self) {
        // info!("Receiver dropped");
    }
}

impl Drop for InnerReceiver {
    fn drop(&mut self) {
        // info!("Inner Receiver dropped");
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

        let heartbeat_inner_weak = Arc::downgrade(&inner);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                let Some(inner_clone) = heartbeat_inner_weak.upgrade() else {
                    break;
                };
                heartbeat_active_recordings(&inner_clone).await;
            }
        });

        // Spawn Health Checker / Reaper
        let inner_weak = Arc::downgrade(&inner);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;

                let inner_clone = match inner_weak.upgrade() {
                    Some(i) => i,
                    None => break,
                };

                if inner_clone.disconnected_at_ms.load(Ordering::SeqCst) > 0 {
                    continue;
                }

                let mut users_to_remove = Vec::new();

                let user_map = inner_clone.user_id_hashmap.read().await;
                if user_map.is_empty() {
                    continue;
                }

                if let Some(guild) = inner_clone.ctx_main.cache.guild(inner_clone.guild_id) {
                    for (&uid, &ssrc) in user_map.iter() {
                        if !guild
                            .voice_states
                            .contains_key(&serenity::model::id::UserId::new(uid))
                        {
                            users_to_remove.push((uid, ssrc));
                        }
                    }
                }
                drop(user_map);

                for (uid, ssrc) in users_to_remove {
                    warn!(
                        "Reaper: User {} (SSRC {}) is no longer in voice state. Closing writer.",
                        uid, ssrc
                    );
                    inner_clone.user_id_hashmap.write().await.remove(&uid);
                    finalize_writer(&inner_clone, ssrc, VoiceEventType::ZombieReaped).await;
                }
            }
        });

        Self { inner }
    }

    pub fn last_voice_packet_time(&self) -> i64 {
        self.inner.last_voice_packet_time.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl VoiceEventHandler for Receiver {
    #[tracing::instrument(level = "trace",  skip_all, name = "receiver_act", fields(guild_id = %self.inner.guild_id))]
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        use EventContext as Ctx;
        use tracing::Instrument;
        match ctx {
            Ctx::SpeakingStateUpdate(Speaking {
                speaking,
                ssrc,
                user_id,
                ..
            }) => {
                let _ = async {
                debug!(
                    "Speaking state update: user {:?} has SSRC {:?}, using {:?}",
                    user_id, ssrc, speaking,
                );

                let Some(user_id) = user_id else {
                    error!("No user_id in SpeakingStateUpdate");
                    return None;
                };

                let previous_bot_ssrc = {
                    self.inner
                        .bot_user_id_hashmap
                        .read()
                        .await
                        .get(&user_id.0)
                        .copied()
                };
                if let Some(previous_bot_ssrc) = previous_bot_ssrc {
                    if previous_bot_ssrc != *ssrc {
                        let mut bot_ssrcs = self.inner.bot_ssrcs.write().await;
                        bot_ssrcs.remove(&previous_bot_ssrc);
                        bot_ssrcs.insert(*ssrc);
                        self.inner
                            .bot_user_id_hashmap
                            .write()
                            .await
                            .insert(user_id.0, *ssrc);
                    }
                    return None;
                }

                let (is_channel_empty, previous_ssrc) = {
                    let users = self.inner.user_id_hashmap.read().await;
                    (users.is_empty(), users.get(&user_id.0).copied())
                };

                if let Some(previous_ssrc) = previous_ssrc {
                    if previous_ssrc == *ssrc {
                        debug!("Writer already active for ssrc {}", ssrc);
                        return None;
                    }

                    let recording = {
                        let mut writer_map = self.inner.ssrc_writer_hashmap.write().await;
                        if let Some(recording) = writer_map.remove(&previous_ssrc) {
                            writer_map.insert(*ssrc, recording.clone());
                            Some(recording)
                        } else {
                            None
                        }
                    };

                    if let Some(recording) = recording {
                        self.inner
                            .user_id_hashmap
                            .write()
                            .await
                            .insert(user_id.0, *ssrc);

                        let mut rec = recording.lock().await;
                        rec.ssrc = *ssrc;
                        info!(
                            "Remapped active writer for user {} from ssrc {} to {}",
                            user_id.0, previous_ssrc, ssrc
                        );
                        return None;
                    }
                }

                let guild = match self.inner.ctx_main.cache.guild(self.inner.guild_id) {
                    Some(g) => g.to_owned(),
                    None => {
                        error!("Guild {} not in cache", self.inner.guild_id);
                        return None;
                    }
                };

                let member = match guild
                    .members
                    .get(&serenity::model::id::UserId::new(user_id.0))
                    .cloned()
                {
                    Some(m) => m,
                    None => match guild.member(&self.inner.ctx_main, user_id.0).await {
                        Ok(m) => m.into_owned(),
                        Err(e) => {
                            error!("Failed to get member: {}", e);
                            return None;
                        }
                    }
                };

                if member.user.bot {
                    self.inner.bot_ssrcs.write().await.insert(*ssrc);
                    self.inner
                        .bot_user_id_hashmap
                        .write()
                        .await
                        .insert(user_id.0, *ssrc);
                } else {
                    if resume_paused_recording(&self.inner, user_id.0, *ssrc).await {
                        return None;
                    }

                    {
                        self.inner
                            .user_id_hashmap
                            .write()
                            .await
                            .insert(user_id.0, *ssrc);
                    }

                    {
                        // Single write-lock for the check-and-insert to avoid TOCTOU.
                        let mut writer_map = self.inner.ssrc_writer_hashmap.write().await;
                        if writer_map.contains_key(ssrc) {
                            debug!("Writer already active for ssrc {}", ssrc);
                        } else {
                            info!("New writer for ssrc {}", ssrc);
                            let now = chrono::Utc::now();
                            let now_ms = now.timestamp_millis();

                            let Some(path) = create_path(
                                self,
                                now,
                                user_id.0,
                                is_channel_empty,
                            )
                            .await
                            else {
                                error!("Failed to create recording path for ssrc {}", ssrc);
                                return None;
                            };

                            let file = match File::create(format!("{}.ogg", path)) {
                                Ok(f) => f,
                                Err(e) => {
                                    error!("Failed to create file for ssrc {}: {}", ssrc, e);
                                    self.inner
                                        .metrics
                                        .track_writer_setup_failure(
                                            &self.inner.guild_metrics,
                                            &self.inner.channel_metrics,
                                        );
                                    return None;
                                }
                            };

                            let writer = match OggOpusWriter::new(BufWriter::new(file), *ssrc, 0) {
                                Ok(w) => w,
                                Err(e) => {
                                    error!("Failed to init OggOpusWriter for ssrc {}: {}", ssrc, e);
                                    return None;
                                }
                            };

                            let file_name = RecordingKey::stem_for(now_ms, user_id.0 as i64);
                            let recording = UserRecording {
                                writer,
                                file_name,
                                start_time: now,
                                user_id: user_id.0,
                                ssrc: *ssrc,
                            };
                            writer_map.insert(*ssrc, Arc::new(Mutex::new(recording)));
                            drop(writer_map);

                            crate::database::user_names::observe(
                                &self.inner.pool,
                                self.inner.guild_id.get(),
                                &member.user,
                                Some(&member),
                            )
                            .await;

                            self.inner.metrics.track_recording_started(
                                &self.inner.guild_metrics,
                                &self.inner.channel_metrics,
                                self.inner.guild_id.get(),
                                self.inner.channel_id.get(),
                                user_id.0,
                            );

                            let _ = sqlx::query(
                                "INSERT INTO voice_events_audit (guild_id, user_id, ssrc, event_type_id, details) VALUES ($1, $2, $3, $4, $5)"
                            )
                            .bind(self.inner.guild_id.get() as i64)
                            .bind(user_id.0 as i64)
                            .bind(*ssrc as i64)
                            .bind(VoiceEventType::WriterOpen as i32)
                            .bind("Writer opened")
                            .execute(&self.inner.pool)
                            .await;

                            info!("1 file created for ssrc: {}", *ssrc);
                        }
                    }
                }
                None::<Event>
            }.instrument(tracing::debug_span!("SpeakingStateUpdate", ssrc = %ssrc, user_id = ?user_id)).await;
            }

            Ctx::RtpPacket(_packet) => {
                // Raw RTP — unused; we read Opus payload from VoiceTick instead.
            }
            Ctx::VoiceTick(tick) => {
                // Snapshot the active SSRCs (skip bots).
                let active: Vec<(u32, Arc<Mutex<UserRecording>>)> = {
                    let map = self.inner.ssrc_writer_hashmap.read().await;
                    let bots = self.inner.bot_ssrcs.read().await;
                    map.iter()
                        .filter(|(s, _)| !bots.contains(s))
                        .map(|(s, w)| (*s, w.clone()))
                        .collect()
                };

                for (ssrc, recording) in active {
                    let speaking_data = tick.speaking.get(&ssrc);

                    // Pull the raw Opus payload bytes if the user spoke this tick.
                    let opus_bytes: Option<Vec<u8>> = speaking_data.and_then(|d| {
                        d.packet.as_ref().map(|rtp| {
                            let view = rtp.rtp();
                            let payload = view.payload();
                            // NB: in songbird 0.6 VoiceTick, `payload_end_pad` is an
                            // absolute end index into `payload`, not a tail-pad count
                            // (see songbird/src/driver/tasks/udp_rx/ssrc_state.rs:86).
                            let start = rtp.payload_offset.min(payload.len());
                            let end = rtp.payload_end_pad.min(payload.len());
                            if end <= start {
                                return Vec::new();
                            }
                            let body = &payload[start..end];
                            // RTP header extension (Discord uses one-byte form) sits
                            // inside the body — skip it before handing bytes to Opus.
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
                    });

                    let mut rec = recording.lock().await;
                    let result = match opus_bytes.as_deref() {
                        Some(bytes) if !bytes.is_empty() => {
                            let now = chrono::Utc::now().timestamp_millis();
                            self.inner
                                .last_voice_packet_time
                                .store(now, Ordering::Relaxed);
                            self.inner.metrics.track_last_voice_packet(
                                &self.inner.guild_metrics,
                                &self.inner.channel_metrics,
                                now,
                            );
                            self.inner.metrics.track_audio_packet_received(
                                &self.inner.guild_metrics,
                                &self.inner.channel_metrics,
                            );
                            rec.writer.write_packet(bytes)
                        }
                        _ => rec.writer.write_silence(1),
                    };
                    if let Err(e) = result {
                        error!("Writer error for ssrc {}: {}", ssrc, e);
                    }
                }
            }
            Ctx::RtcpPacket(_data) => {}

            Ctx::DriverDisconnect(DisconnectData { kind, reason, .. }) => {
                info!("Disconnected \n kind: {:?} \n reason {:?}", kind, reason);

                // TODO: Log only  unexpected driver discoenncets and unrequested
                if *kind == songbird::events::context_data::DisconnectKind::Runtime
                    || *reason != Some(DisconnectReason::Requested)
                {
                    self.inner
                        .metrics
                        .driver_disconnects
                        .fetch_add(1, Ordering::Relaxed);
                }

                let channel_has_human_members = recording_channel_has_human_members(&self.inner);
                let should_finalize_empty_channel_disconnect =
                    should_finalize_empty_channel_disconnect(
                        reason.as_ref(),
                        channel_has_human_members,
                    );

                if should_resume_recordings_for_disconnect(
                    reason.as_ref(),
                    RESUME_INTENTIONAL_DISCONNECTS_FOR_TESTING,
                ) && !should_finalize_empty_channel_disconnect
                {
                    let now = chrono::Utc::now().timestamp_millis();
                    if self
                        .inner
                        .disconnected_at_ms
                        .compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                    {
                        info!("Recoverable disconnect recorded at {}", now);
                        let users: Vec<i64> = self
                            .inner
                            .user_id_hashmap
                            .read()
                            .await
                            .keys()
                            .map(|u| *u as i64)
                            .collect();
                        let guild_id = self.inner.guild_id.get() as i64;
                        let channel_id = Some(self.inner.channel_id.get() as i64);
                        for uid in users {
                            super::voice::insert_voice_event(
                                &self.inner.pool,
                                guild_id,
                                channel_id,
                                uid,
                                super::voice::EVT_RECORDING_PAUSE,
                            )
                            .await;
                        }
                        schedule_recoverable_disconnect_timeout(&self.inner, now);
                    }
                    return None;
                }

                if should_finalize_empty_channel_disconnect {
                    info!(
                        "Intentional disconnect with no human users in channel. Closing active recordings."
                    );
                }

                self.inner.disconnected_at_ms.store(0, Ordering::SeqCst);
                finalize_all_active_recordings(&self.inner, VoiceEventType::WriterClose).await;
                clear_receiver_state(&self.inner).await;
            }
            Ctx::DriverConnect(ConnectData { .. }) => {
                info!("Connected");
                resume_after_recoverable_disconnect(&self.inner).await;
            }
            Ctx::DriverReconnect(ConnectData { .. }) => {
                info!("Reconnected");
                self.inner
                    .metrics
                    .driver_reconnects
                    .fetch_add(1, Ordering::Relaxed);
                resume_after_recoverable_disconnect(&self.inner).await;
            }

            Ctx::ClientDisconnect(ClientDisconnect { user_id }) => {
                let _ = async {
                    info!("client disconnected id: {}", user_id);

                    let is_bot_ssrc = self
                        .inner
                        .bot_user_id_hashmap
                        .write()
                        .await
                        .remove(&user_id.0);
                    if let Some(bot_ssrc) = is_bot_ssrc {
                        warn!("Removed bot with id: {} and ssrc: {}", user_id.0, bot_ssrc);
                        self.inner.bot_ssrcs.write().await.remove(&bot_ssrc);
                        return None;
                    }

                    let ssrc = match self.inner.user_id_hashmap.write().await.remove(&user_id.0) {
                        Some(ok) => ok,
                        None => {
                            warn!("tried to remove bot");
                            return None;
                        }
                    };

                    pause_recording_for_rejoin(&self.inner, user_id.0, ssrc).await;
                    None::<Event>
                }
                .instrument(tracing::info_span!("ClientDisconnect", user_id = %user_id))
                .await;
            }
            _ => {
                warn!("Unhandled voice event context");
            }
        }

        None
    }
}

fn silence_frames_for_gap_ms(gap_ms: i64) -> u64 {
    if gap_ms <= 0 {
        0
    } else {
        (gap_ms as u64).div_ceil(20)
    }
}

fn is_intentional_driver_disconnect(reason: Option<&DisconnectReason>) -> bool {
    reason.is_none() || matches!(reason, Some(DisconnectReason::Requested))
}

fn should_resume_recordings_for_disconnect(
    reason: Option<&DisconnectReason>,
    resume_intentional_disconnects: bool,
) -> bool {
    resume_intentional_disconnects || !is_intentional_driver_disconnect(reason)
}

fn should_finalize_empty_channel_disconnect(
    reason: Option<&DisconnectReason>,
    channel_has_human_members: Option<bool>,
) -> bool {
    is_intentional_driver_disconnect(reason) && channel_has_human_members == Some(false)
}

fn recording_channel_has_human_members(inner: &InnerReceiver) -> Option<bool> {
    let guild = inner.ctx_main.cache.guild(inner.guild_id)?;
    let bot_id = inner.ctx_main.cache.current_user().id;

    for (user_id, voice_state) in &guild.voice_states {
        if voice_state.channel_id != Some(inner.channel_id) {
            continue;
        }

        if *user_id == bot_id {
            continue;
        }

        let Some(member) = guild.members.get(user_id) else {
            // Missing member cache for a non-bot user: keep recoverable behavior.
            return Some(true);
        };

        if !member.user.bot {
            return Some(true);
        }
    }

    Some(false)
}

fn paused_timeout_matches(current_token: Option<u64>, timeout_token: u64) -> bool {
    current_token == Some(timeout_token)
}

async fn pause_recording_for_rejoin(inner: &Arc<InnerReceiver>, user_id: u64, ssrc: u32) {
    let recording = inner.ssrc_writer_hashmap.write().await.remove(&ssrc);
    let Some(recording) = recording else {
        warn!(
            user_id,
            ssrc, "ClientDisconnect had no active writer to pause"
        );
        return;
    };

    {
        let _rec = recording.lock().await;
    }
    let paused_at = chrono::Utc::now();
    let token = inner.paused_recording_token.fetch_add(1, Ordering::SeqCst);
    let paused = PausedRecording {
        recording,
        ssrc,
        paused_at,
        token,
    };

    let previous = inner
        .paused_recordings
        .write()
        .await
        .insert(user_id, paused.clone());
    if let Some(previous) = previous {
        warn!(
            user_id,
            previous_ssrc = previous.ssrc,
            "Replacing existing paused recording for user"
        );
        finalize_recording_arc(
            inner,
            previous.ssrc,
            previous.recording,
            VoiceEventType::WriterClose,
            previous.paused_at,
        )
        .await;
    }

    super::voice::insert_voice_event(
        &inner.pool,
        inner.guild_id.get() as i64,
        Some(inner.channel_id.get() as i64),
        user_id as i64,
        super::voice::EVT_USER_RECORDING_PAUSE,
    )
    .await;

    info!(
        user_id,
        ssrc,
        timeout_ms = USER_REJOIN_RESUME_TIMEOUT_MS,
        "Paused recording for user rejoin"
    );
    schedule_user_rejoin_resume_timeout(inner, user_id, token);
}

async fn resume_paused_recording(inner: &Arc<InnerReceiver>, user_id: u64, ssrc: u32) -> bool {
    let paused = inner.paused_recordings.write().await.remove(&user_id);
    let Some(paused) = paused else {
        return false;
    };

    let now = chrono::Utc::now();
    let gap_ms = now
        .signed_duration_since(paused.paused_at)
        .num_milliseconds();
    let frames = silence_frames_for_gap_ms(gap_ms);

    {
        let mut rec = paused.recording.lock().await;
        if let Err(err) = rec.writer.write_silence(frames) {
            error!(
                user_id,
                old_ssrc = paused.ssrc,
                new_ssrc = ssrc,
                "Failed to write user rejoin silence: {}",
                err
            );
        }
        rec.ssrc = ssrc;
    }

    inner
        .ssrc_writer_hashmap
        .write()
        .await
        .insert(ssrc, paused.recording);
    inner.user_id_hashmap.write().await.insert(user_id, ssrc);

    super::voice::insert_voice_event(
        &inner.pool,
        inner.guild_id.get() as i64,
        Some(inner.channel_id.get() as i64),
        user_id as i64,
        super::voice::EVT_USER_RECORDING_RESUME,
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

fn schedule_user_rejoin_resume_timeout(inner: &Arc<InnerReceiver>, user_id: u64, token: u64) {
    let inner = Arc::clone(inner);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(
            USER_REJOIN_RESUME_TIMEOUT_MS,
        ))
        .await;

        let paused = {
            let mut paused_recordings = inner.paused_recordings.write().await;
            if paused_timeout_matches(paused_recordings.get(&user_id).map(|p| p.token), token) {
                paused_recordings.remove(&user_id)
            } else {
                None
            }
        };

        let Some(paused) = paused else {
            return;
        };

        warn!(
            user_id,
            ssrc = paused.ssrc,
            timeout_ms = USER_REJOIN_RESUME_TIMEOUT_MS,
            "User rejoin resume timed out. Closing recording."
        );
        finalize_recording_arc(
            &inner,
            paused.ssrc,
            paused.recording,
            VoiceEventType::WriterClose,
            paused.paused_at,
        )
        .await;
    });
}

fn schedule_recoverable_disconnect_timeout(inner: &Arc<InnerReceiver>, disconnected_at_ms: i64) {
    let inner = Arc::clone(inner);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(
            RECOVERABLE_DISCONNECT_TIMEOUT_MS,
        ))
        .await;

        if inner
            .disconnected_at_ms
            .compare_exchange(disconnected_at_ms, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        warn!(
            "Recoverable disconnect timed out after {}ms. Closing active recordings.",
            RECOVERABLE_DISCONNECT_TIMEOUT_MS
        );
        finalize_all_active_recordings(&inner, VoiceEventType::WriterClose).await;
        clear_receiver_state(&inner).await;
    });
}

async fn resume_after_recoverable_disconnect(inner: &Arc<InnerReceiver>) {
    let disconnected_at_ms = inner.disconnected_at_ms.swap(0, Ordering::SeqCst);
    if disconnected_at_ms == 0 {
        return;
    }

    let reconnect_time = chrono::Utc::now();
    let reconnect_ms = reconnect_time.timestamp_millis();
    let frames = silence_frames_for_gap_ms(reconnect_ms - disconnected_at_ms);
    info!(
        "Resuming recordings after {}ms disconnect with {} silence frames",
        reconnect_ms - disconnected_at_ms,
        frames
    );

    {
        let users: Vec<i64> = inner
            .user_id_hashmap
            .read()
            .await
            .keys()
            .map(|u| *u as i64)
            .collect();
        let guild_id = inner.guild_id.get() as i64;
        let channel_id = Some(inner.channel_id.get() as i64);
        for uid in users {
            super::voice::insert_voice_event(
                &inner.pool,
                guild_id,
                channel_id,
                uid,
                super::voice::EVT_RECORDING_RESUME,
            )
            .await;
        }
    }

    let active: Vec<(u32, Arc<Mutex<UserRecording>>)> = {
        let map = inner.ssrc_writer_hashmap.read().await;
        let bots = inner.bot_ssrcs.read().await;
        map.iter()
            .filter(|(ssrc, _)| !bots.contains(ssrc))
            .map(|(ssrc, writer)| (*ssrc, writer.clone()))
            .collect()
    };

    for (ssrc, recording) in active {
        let mut rec = recording.lock().await;
        if let Err(err) = rec.writer.write_silence(frames) {
            error!(
                "Failed to write reconnect gap silence for ssrc {}: {}",
                ssrc, err
            );
        }
    }

    let mut users_to_remove = Vec::new();
    {
        let user_map = inner.user_id_hashmap.read().await;
        if let Some(guild) = inner.ctx_main.cache.guild(inner.guild_id) {
            for (&uid, &ssrc) in user_map.iter() {
                if !guild
                    .voice_states
                    .contains_key(&serenity::model::id::UserId::new(uid))
                {
                    users_to_remove.push((uid, ssrc));
                }
            }
        }
    }

    for (uid, ssrc) in users_to_remove {
        warn!(
            "User {} (SSRC {}) is no longer in voice after reconnect. Closing writer.",
            uid, ssrc
        );
        inner.user_id_hashmap.write().await.remove(&uid);
        finalize_writer_at(inner, ssrc, VoiceEventType::WriterClose, reconnect_time).await;
    }
}

async fn finalize_all_active_recordings(inner: &Arc<InnerReceiver>, event_type: VoiceEventType) {
    let close_time = chrono::Utc::now();
    let ssrcs: Vec<u32> = {
        let map = inner.ssrc_writer_hashmap.read().await;
        map.keys().copied().collect()
    };
    for ssrc in ssrcs {
        finalize_writer_at(inner, ssrc, event_type, close_time).await;
    }

    let paused_recordings: Vec<PausedRecording> = {
        let mut paused = inner.paused_recordings.write().await;
        paused.drain().map(|(_, recording)| recording).collect()
    };
    for paused in paused_recordings {
        finalize_recording_arc(
            inner,
            paused.ssrc,
            paused.recording,
            event_type,
            paused.paused_at,
        )
        .await;
    }
}

async fn clear_receiver_state(inner: &Arc<InnerReceiver>) {
    inner.user_id_hashmap.write().await.clear();
    inner.paused_recordings.write().await.clear();
    inner.bot_ssrcs.write().await.clear();
    inner.bot_user_id_hashmap.write().await.clear();
    inner.session_start_ms.store(0, Ordering::SeqCst);
}

/// Close the writer for `ssrc`, run the audio_files DB update, decrement
/// active counters. Idempotent — silently no-ops if the writer is already gone.
async fn finalize_writer(inner: &Arc<InnerReceiver>, ssrc: u32, event_type: VoiceEventType) {
    finalize_writer_at(inner, ssrc, event_type, chrono::Utc::now()).await;
}

/// Same as `finalize_writer`, but with an explicit close time when the real
/// leave happened during an outage and only the reconnect time is knowable.
async fn finalize_writer_at(
    inner: &Arc<InnerReceiver>,
    ssrc: u32,
    event_type: VoiceEventType,
    close_time: chrono::DateTime<chrono::Utc>,
) {
    let entry = inner.ssrc_writer_hashmap.write().await.remove(&ssrc);
    let Some(arc) = entry else {
        return;
    };

    finalize_recording_arc(inner, ssrc, arc, event_type, close_time).await;
}

async fn finalize_recording_arc(
    inner: &Arc<InnerReceiver>,
    ssrc: u32,
    arc: Arc<Mutex<UserRecording>>,
    event_type: VoiceEventType,
    close_time: chrono::DateTime<chrono::Utc>,
) {
    // Lock the writer to wait out any in-flight tick write, then finalize.
    // We don't try to unwrap the Arc; we just clone the metadata we need
    // and let the Arc drop naturally after this scope.
    let mut rec = arc.lock().await;

    if let Err(e) = rec.writer.finish() {
        error!("Failed to finalize writer for ssrc {}: {}", ssrc, e);
        inner.metrics.track_recording_finalize_error();
        let _ = sqlx::query(
            "INSERT INTO voice_events_audit (guild_id, user_id, ssrc, event_type_id, details) VALUES ($1, $2, $3, $4, $5)"
        )
        .bind(inner.guild_id.get() as i64)
        .bind(rec.user_id as i64)
        .bind(ssrc as i64)
        .bind(VoiceEventType::WriterError as i32)
        .bind(format!("finish: {}", e))
        .execute(&inner.pool)
        .await;
    }

    let time_elapsed = close_time
        .signed_duration_since(rec.start_time)
        .num_milliseconds();
    inner.metrics.track_recording_finished(
        &inner.guild_metrics,
        &inner.channel_metrics,
        inner.guild_id.get(),
        inner.channel_id.get(),
        rec.user_id,
        time_elapsed as f64 / 1000.0,
    );
    let last_person_in_channel = inner.user_id_hashmap.read().await.is_empty();
    // 2 = JOINED 3 = LAST
    let state = if last_person_in_channel { 3 } else { 2 };

    let file_name = rec.file_name.clone();
    let user_id = rec.user_id;
    let rec_ssrc = rec.ssrc;
    drop(rec);

    if let Err(err) = sqlx::query!(
        "UPDATE audio_files
            SET end_ts = audio_files.start_ts + $1,
                state_leave = $2,
                recording_heartbeat_at = NULL
            WHERE file_name = $3",
        time_elapsed,
        state,
        file_name
    )
    .execute(&inner.pool)
    .await
    {
        error!("{}", err);
        inner
            .metrics
            .db_query_errors
            .fetch_add(1, Ordering::Relaxed);
    }

    let _ = sqlx::query(
        "INSERT INTO voice_events_audit (guild_id, user_id, ssrc, event_type_id, details) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(inner.guild_id.get() as i64)
    .bind(user_id as i64)
    .bind(rec_ssrc as i64)
    .bind(event_type as i32)
    .bind("Writer closed")
    .execute(&inner.pool)
    .await;
}

#[tracing::instrument(skip_all, name = "create_path")]
async fn create_path(
    _self: &Receiver,
    now: chrono::DateTime<chrono::Utc>,
    user_id: u64,
    is_channel_empty: bool,
) -> Option<String> {
    let guild_id = _self.inner.guild_id;
    let channel_id = _self.inner.channel_id;
    let file_name = RecordingKey::stem_for(now.timestamp_millis(), user_id as i64);
    let key = RecordingKey::new(
        guild_id.get() as i64,
        channel_id.get() as i64,
        now.year(),
        now.month(),
        file_name.clone(),
    );

    let dir_path = key.recording_dir(RECORDING_ROOT);
    let combined_path = key.recording_dir(RECORDING_ROOT).join(&file_name);

    if let Err(err) = std::fs::create_dir_all(&dir_path) {
        error!("cannot create path {}: {}", dir_path.display(), err);
        return None;
    };

    let null: Option<i64> = None;

    match sqlx::query!(
        "INSERT INTO audio_files
	(file_name, guild_id, channel_id, user_id, year, month, start_ts, end_ts, state_enter, recording_owner_instance_id, recording_heartbeat_at) VALUES
	($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, now())",
        file_name,
        guild_id.get() as i64,
        channel_id.get() as i64,
        user_id as i64,
        now.year(),
        now.month() as i32,
        now.timestamp_millis(),
        null,
        if is_channel_empty { 1 } else { 2 },
        _self.inner.recording_owner_instance_id.clone()
    )
    .execute(&_self.inner.pool)
    .await
    {
        Ok(ok) => ok,
        Err(err) => {
            error!("{}", err);
            _self
                .inner
                .metrics
                .db_insert_failures
                .fetch_add(1, Ordering::Relaxed);
            _self
                .inner
                .metrics
                .db_query_errors
                .fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };

    Some(combined_path.to_string_lossy().into_owned())
}

async fn heartbeat_active_recordings(inner: &Arc<InnerReceiver>) {
    let mut file_names = HashSet::new();
    {
        let active = inner.ssrc_writer_hashmap.read().await;
        for recording in active.values() {
            let recording = recording.lock().await;
            file_names.insert(recording.file_name.clone());
        }
    }
    {
        let paused = inner.paused_recordings.read().await;
        for recording in paused.values() {
            let recording = recording.recording.lock().await;
            file_names.insert(recording.file_name.clone());
        }
    }

    if file_names.is_empty() {
        return;
    }

    let file_names = file_names.into_iter().collect::<Vec<_>>();
    if let Err(err) = sqlx::query(
        "UPDATE audio_files
            SET recording_heartbeat_at = now()
          WHERE file_name = ANY($1)
            AND recording_owner_instance_id = $2
            AND end_ts IS NULL",
    )
    .bind(&file_names)
    .bind(&inner.recording_owner_instance_id)
    .execute(&inner.pool)
    .await
    {
        warn!("recording heartbeat failed: {}", err);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            super::super::voice::EVT_USER_RECORDING_PAUSE,
            super::super::voice::EVT_RECORDING_PAUSE
        );
        assert_ne!(
            super::super::voice::EVT_USER_RECORDING_RESUME,
            super::super::voice::EVT_RECORDING_RESUME
        );
        assert_eq!(super::super::voice::EVT_USER_RECORDING_PAUSE, 20);
        assert_eq!(super::super::voice::EVT_USER_RECORDING_RESUME, 21);
    }

    #[test]
    fn none_and_requested_disconnects_are_intentional() {
        assert!(is_intentional_driver_disconnect(None));
        assert!(is_intentional_driver_disconnect(Some(
            &DisconnectReason::Requested
        )));
        assert!(!is_intentional_driver_disconnect(Some(
            &DisconnectReason::TimedOut
        )));
    }

    #[test]
    fn optional_testing_flag_can_resume_intentional_disconnects() {
        assert!(!should_resume_recordings_for_disconnect(None, false));
        assert!(should_resume_recordings_for_disconnect(None, true));
        assert!(should_resume_recordings_for_disconnect(
            Some(&DisconnectReason::TimedOut),
            false
        ));
    }

    #[test]
    fn empty_channel_intentional_disconnects_finalize_even_when_resume_testing_enabled() {
        assert!(should_finalize_empty_channel_disconnect(None, Some(false)));
        assert!(should_finalize_empty_channel_disconnect(
            Some(&DisconnectReason::Requested),
            Some(false)
        ));
        assert!(!should_finalize_empty_channel_disconnect(
            Some(&DisconnectReason::Requested),
            Some(true)
        ));
        assert!(!should_finalize_empty_channel_disconnect(
            Some(&DisconnectReason::Requested),
            None
        ));
        assert!(!should_finalize_empty_channel_disconnect(
            Some(&DisconnectReason::TimedOut),
            Some(false)
        ));
    }
}
