use serenity::model::id::UserId;
use songbird::model::payload::ClientDisconnect;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::{error, info, warn};

use super::InnerReceiver;
use super::finalize::finalize_recording_arc;
use super::state::{PausedRecording, VoiceEventType};

pub(super) const USER_REJOIN_RESUME_TIMEOUT_MS: u64 = 10 * 60 * 1000;

pub(super) fn silence_frames_for_gap_ms(gap_ms: i64) -> u64 {
    if gap_ms <= 0 {
        0
    } else {
        (gap_ms as u64).div_ceil(20)
    }
}

pub(super) fn paused_timeout_matches(current_token: Option<u64>, timeout_token: u64) -> bool {
    current_token == Some(timeout_token)
}

pub(super) async fn handle_client_disconnect(
    inner: &Arc<InnerReceiver>,
    ClientDisconnect { user_id }: &ClientDisconnect,
) {
    use tracing::Instrument;
    let _ = async {
        info!("client disconnected id: {}", user_id);

        let is_bot_ssrc = inner.bot_user_id_hashmap.write().await.remove(&user_id.0);
        if let Some(bot_ssrc) = is_bot_ssrc {
            warn!("Removed bot with id: {} and ssrc: {}", user_id.0, bot_ssrc);
            inner.bot_ssrcs.write().await.remove(&bot_ssrc);
            return;
        }

        let ssrc = match inner.user_id_hashmap.write().await.remove(&user_id.0) {
            Some(ok) => ok,
            None => {
                warn!("tried to remove bot");
                return;
            }
        };

        pause_recording_for_rejoin(inner, user_id.0, ssrc).await;
    }
    .instrument(tracing::info_span!("ClientDisconnect", user_id = %user_id))
    .await;
}

pub(super) async fn pause_recording_for_rejoin(
    inner: &Arc<InnerReceiver>,
    user_id: u64,
    ssrc: u32,
) {
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

    crate::events::voice::insert_voice_event(
        &inner.pool,
        inner.guild_id.get() as i64,
        Some(inner.channel_id.get() as i64),
        user_id as i64,
        crate::events::voice::EVT_USER_RECORDING_PAUSE,
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

pub(super) async fn resume_paused_recording(
    inner: &Arc<InnerReceiver>,
    user_id: u64,
    ssrc: u32,
) -> bool {
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

    crate::events::voice::insert_voice_event(
        &inner.pool,
        inner.guild_id.get() as i64,
        Some(inner.channel_id.get() as i64),
        user_id as i64,
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

// Re-exported for the disconnect module's reaper-style scan.
pub(super) async fn scan_users_no_longer_in_voice_state(
    inner: &Arc<InnerReceiver>,
) -> Vec<(u64, u32)> {
    let mut users_to_remove = Vec::new();
    let user_map = inner.user_id_hashmap.read().await;
    if let Some(guild) = inner.ctx_main.cache.guild(inner.guild_id) {
        for (&uid, &ssrc) in user_map.iter() {
            if !guild.voice_states.contains_key(&UserId::new(uid)) {
                users_to_remove.push((uid, ssrc));
            }
        }
    }
    users_to_remove
}
