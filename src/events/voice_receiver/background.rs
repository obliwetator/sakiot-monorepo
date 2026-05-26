use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::warn;

use super::InnerReceiver;
use super::finalize::finalize_writer;
use super::pause::scan_users_no_longer_in_voice_state;
use super::persistence::heartbeat_active_recordings;
use super::state::VoiceEventType;

pub(super) fn spawn_heartbeat(inner: &Arc<InnerReceiver>) {
    let weak = Arc::downgrade(inner);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            interval.tick().await;
            let Some(inner) = weak.upgrade() else {
                break;
            };
            heartbeat_active_recordings(&inner).await;
        }
    });
}

pub(super) fn spawn_reaper(inner: &Arc<InnerReceiver>) {
    let weak = Arc::downgrade(inner);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;

            let Some(inner) = weak.upgrade() else {
                break;
            };

            if inner.disconnected_at_ms.load(Ordering::SeqCst) > 0 {
                continue;
            }

            if inner.user_id_hashmap.read().await.is_empty() {
                continue;
            }

            let users_to_remove = scan_users_no_longer_in_voice_state(&inner).await;
            for (uid, ssrc) in users_to_remove {
                warn!(
                    "Reaper: User {} (SSRC {}) is no longer in voice state. Closing writer.",
                    uid, ssrc
                );
                inner.user_id_hashmap.write().await.remove(&uid);
                finalize_writer(&inner, ssrc, VoiceEventType::ZombieReaped).await;
            }
        }
    });
}
