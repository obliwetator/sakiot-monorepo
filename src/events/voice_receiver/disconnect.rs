use songbird::events::context_data::{DisconnectData, DisconnectKind, DisconnectReason};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::{error, info, warn};

use super::InnerReceiver;
use super::finalize::{
    clear_receiver_state, finalize_all_active_recordings, finalize_writer_at,
};
use super::pause::{scan_users_no_longer_in_voice_state, silence_frames_for_gap_ms};
use super::state::VoiceEventType;

pub(super) const RECOVERABLE_DISCONNECT_TIMEOUT_MS: u64 = 60_000;

// Without this only way to test if pray discord randomly disconncts our bot. Need to manually toggle
// CAVEAT: this includes bot self disconnects
// TODO: Remote toggle for easier testing. No need to recompile
pub(super) const RESUME_INTENTIONAL_DISCONNECTS_FOR_TESTING: bool = true;

pub(super) fn is_intentional_driver_disconnect(reason: Option<&DisconnectReason>) -> bool {
    reason.is_none() || matches!(reason, Some(DisconnectReason::Requested))
}

pub(super) fn should_resume_recordings_for_disconnect(
    reason: Option<&DisconnectReason>,
    resume_intentional_disconnects: bool,
) -> bool {
    resume_intentional_disconnects || !is_intentional_driver_disconnect(reason)
}

pub(super) fn should_finalize_empty_channel_disconnect(
    reason: Option<&DisconnectReason>,
    channel_has_human_members: Option<bool>,
) -> bool {
    is_intentional_driver_disconnect(reason) && channel_has_human_members == Some(false)
}

pub(super) fn recording_channel_has_human_members(inner: &InnerReceiver) -> Option<bool> {
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

pub(super) async fn handle_driver_disconnect(
    inner: &Arc<InnerReceiver>,
    DisconnectData { kind, reason, .. }: &DisconnectData<'_>,
) {
    info!("Disconnected \n kind: {:?} \n reason {:?}", kind, reason);

    // TODO: Log only  unexpected driver discoenncets and unrequested
    if *kind == DisconnectKind::Runtime || *reason != Some(DisconnectReason::Requested) {
        inner
            .metrics
            .driver_disconnects
            .fetch_add(1, Ordering::Relaxed);
    }

    let channel_has_human_members = recording_channel_has_human_members(inner);
    let should_finalize_empty_channel_disconnect =
        should_finalize_empty_channel_disconnect(reason.as_ref(), channel_has_human_members);

    if should_resume_recordings_for_disconnect(
        reason.as_ref(),
        RESUME_INTENTIONAL_DISCONNECTS_FOR_TESTING,
    ) && !should_finalize_empty_channel_disconnect
    {
        let now = chrono::Utc::now().timestamp_millis();
        if inner
            .disconnected_at_ms
            .compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            info!("Recoverable disconnect recorded at {}", now);
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
                crate::events::voice::insert_voice_event(
                    &inner.pool,
                    guild_id,
                    channel_id,
                    uid,
                    crate::events::voice::EVT_RECORDING_PAUSE,
                )
                .await;
            }
            schedule_recoverable_disconnect_timeout(inner, now);
        }
        return;
    }

    if should_finalize_empty_channel_disconnect {
        info!(
            "Intentional disconnect with no human users in channel. Closing active recordings."
        );
    }

    inner.disconnected_at_ms.store(0, Ordering::SeqCst);
    finalize_all_active_recordings(inner, VoiceEventType::WriterClose).await;
    clear_receiver_state(inner).await;
}

pub(super) async fn handle_driver_connect(inner: &Arc<InnerReceiver>) {
    info!("Connected");
    resume_after_recoverable_disconnect(inner).await;
}

pub(super) async fn handle_driver_reconnect(inner: &Arc<InnerReceiver>) {
    info!("Reconnected");
    inner
        .metrics
        .driver_reconnects
        .fetch_add(1, Ordering::Relaxed);
    resume_after_recoverable_disconnect(inner).await;
}

fn schedule_recoverable_disconnect_timeout(
    inner: &Arc<InnerReceiver>,
    disconnected_at_ms: i64,
) {
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
    use tokio::sync::Mutex;

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
            crate::events::voice::insert_voice_event(
                &inner.pool,
                guild_id,
                channel_id,
                uid,
                crate::events::voice::EVT_RECORDING_RESUME,
            )
            .await;
        }
    }

    let active: Vec<(u32, Arc<Mutex<super::state::UserRecording>>)> = {
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

    let users_to_remove = scan_users_no_longer_in_voice_state(inner).await;

    for (uid, ssrc) in users_to_remove {
        warn!(
            "User {} (SSRC {}) is no longer in voice after reconnect. Closing writer.",
            uid, ssrc
        );
        inner.user_id_hashmap.write().await.remove(&uid);
        finalize_writer_at(inner, ssrc, VoiceEventType::WriterClose, reconnect_time).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
