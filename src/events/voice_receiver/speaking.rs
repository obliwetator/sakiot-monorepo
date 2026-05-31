use sakiot_paths::RecordingKey;
use serenity::model::id::UserId;
use songbird::model::payload::Speaking;
use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{Instrument, debug, error, info};

use super::InnerReceiver;
use super::pause::resume_paused_recording;
use super::persistence::{create_path, insert_receiver_voice_event, mark_recording_setup_failed};
use super::state::{RecordingFinalizeReason, UserRecording, VoiceEventType};
use crate::events::ogg_opus_writer::OggOpusWriter;

pub(super) async fn handle_speaking_state_update(
    inner: &Arc<InnerReceiver>,
    Speaking {
        speaking,
        ssrc,
        user_id,
        ..
    }: &Speaking,
) {
    let ssrc = *ssrc;
    let user_id_opt = *user_id;
    let speaking_copy = *speaking;
    let inner = Arc::clone(inner);
    async move {
        debug!(
            "Speaking state update: user {:?} has SSRC {:?}, using {:?}",
            user_id_opt, ssrc, speaking_copy,
        );

        let Some(user_id) = user_id_opt else {
            error!("No user_id in SpeakingStateUpdate");
            return;
        };

        let previous_bot_ssrc = {
            inner
                .bot_user_id_hashmap
                .read()
                .await
                .get(&user_id.0)
                .copied()
        };
        if let Some(previous_bot_ssrc) = previous_bot_ssrc {
            if previous_bot_ssrc != ssrc {
                let mut bot_ssrcs = inner.bot_ssrcs.write().await;
                bot_ssrcs.remove(&previous_bot_ssrc);
                bot_ssrcs.insert(ssrc);
                inner
                    .bot_user_id_hashmap
                    .write()
                    .await
                    .insert(user_id.0, ssrc);
            }
            return;
        }

        let previous_ssrc = {
            let users = inner.user_id_hashmap.read().await;
            users.get(&user_id.0).copied()
        };

        if let Some(previous_ssrc) = previous_ssrc {
            if previous_ssrc == ssrc {
                debug!("Writer already active for ssrc {}", ssrc);
                return;
            }

            let recording = {
                let mut writer_map = inner.ssrc_writer_hashmap.write().await;
                if let Some(recording) = writer_map.remove(&previous_ssrc) {
                    writer_map.insert(ssrc, recording.clone());
                    Some(recording)
                } else {
                    None
                }
            };

            if let Some(recording) = recording {
                inner.user_id_hashmap.write().await.insert(user_id.0, ssrc);

                {
                    let mut rec = recording.lock().await;
                    rec.ssrc = ssrc;
                }
                info!(
                    "Remapped active writer for user {} from ssrc {} to {}",
                    user_id.0, previous_ssrc, ssrc
                );
                return;
            }
        }

        let guild = match inner.ctx_main.cache.guild(inner.guild_id) {
            Some(g) => g.to_owned(),
            None => {
                error!("Guild {} not in cache", inner.guild_id);
                return;
            }
        };

        let member = match guild.members.get(&UserId::new(user_id.0)).cloned() {
            Some(m) => m,
            None => match guild.member(&inner.ctx_main, user_id.0).await {
                Ok(m) => m.into_owned(),
                Err(e) => {
                    error!("Failed to get member: {}", e);
                    return;
                }
            },
        };

        if member.user.bot {
            inner.bot_ssrcs.write().await.insert(ssrc);
            inner
                .bot_user_id_hashmap
                .write()
                .await
                .insert(user_id.0, ssrc);
            return;
        }

        if resume_paused_recording(&inner, user_id.0, ssrc).await {
            return;
        }

        {
            inner.user_id_hashmap.write().await.insert(user_id.0, ssrc);
        }

        // Single write-lock for the check-and-insert to avoid TOCTOU.
        let mut writer_map = inner.ssrc_writer_hashmap.write().await;
        if writer_map.contains_key(&ssrc) {
            debug!("Writer already active for ssrc {}", ssrc);
            return;
        }

        info!("New writer for ssrc {}", ssrc);
        let now = chrono::Utc::now();
        let now_ms = now.timestamp_millis();
        let file_name = RecordingKey::stem_for(now_ms, user_id.0 as i64);

        let Some(path) = create_path(&inner, now, user_id.0).await else {
            error!("Failed to create recording path for ssrc {}", ssrc);
            return;
        };

        let file = match File::create(format!("{}.ogg", path)) {
            Ok(f) => f,
            Err(e) => {
                error!("Failed to create file for ssrc {}: {}", ssrc, e);
                inner
                    .metrics
                    .track_writer_setup_failure(&inner.guild_metrics, &inner.channel_metrics);
                mark_recording_setup_failed(
                    &inner,
                    &file_name,
                    RecordingFinalizeReason::FileCreate,
                )
                .await;
                return;
            }
        };

        let writer = match OggOpusWriter::new(BufWriter::new(file), ssrc, 0) {
            Ok(w) => w,
            Err(e) => {
                error!("Failed to init OggOpusWriter for ssrc {}: {}", ssrc, e);
                inner
                    .metrics
                    .track_writer_setup_failure(&inner.guild_metrics, &inner.channel_metrics);
                mark_recording_setup_failed(
                    &inner,
                    &file_name,
                    RecordingFinalizeReason::WriterInit,
                )
                .await;
                return;
            }
        };

        let recording = UserRecording {
            writer,
            file_name,
            start_time: now,
            user_id: user_id.0,
            ssrc,
        };
        writer_map.insert(ssrc, Arc::new(Mutex::new(recording)));
        drop(writer_map);

        crate::database::user_names::observe(
            &inner.pool,
            inner.guild_id.get(),
            &member.user,
            Some(&member),
        )
        .await;

        inner.metrics.track_recording_started(
            &inner.guild_metrics,
            &inner.channel_metrics,
            inner.guild_id.get(),
            inner.channel_id.get(),
            user_id.0,
        );

        insert_receiver_voice_event(
            &inner,
            user_id.0,
            ssrc,
            VoiceEventType::WriterOpen,
            "Writer opened",
        )
        .await;

        info!("1 file created for ssrc: {}", ssrc);
    }
    .instrument(tracing::debug_span!("SpeakingStateUpdate", ssrc = %ssrc, user_id = ?user_id_opt))
    .await;
}
