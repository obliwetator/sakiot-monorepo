use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::{error, warn};

use super::InnerReceiver;
use super::state::{RecordingFinalizeReason, VoiceEventType};
use crate::database::recordings::RecordingHandle;

pub(super) async fn insert_receiver_voice_event(
    inner: &Arc<InnerReceiver>,
    user_id: u64,
    ssrc: u32,
    event_type: VoiceEventType,
    details: &str,
) {
    if let Err(err) = crate::database::voice_events::insert_receiver_voice_event(
        &inner.pool,
        inner.guild_id.get() as i64,
        user_id as i64,
        ssrc as i64,
        event_type as i32,
        details,
    )
    .await
    {
        warn!(
            guild_id = inner.guild_id.get(),
            user_id,
            ssrc,
            event_type_id = event_type as i32,
            "failed to insert receiver voice event: {}",
            err
        );
        inner
            .metrics
            .db_insert_failures
            .fetch_add(1, Ordering::Relaxed);
        inner
            .metrics
            .db_query_errors
            .fetch_add(1, Ordering::Relaxed);
    }
}

#[tracing::instrument(skip_all, name = "create_recording")]
pub(super) async fn create_recording(
    inner: &Arc<InnerReceiver>,
    now: chrono::DateTime<chrono::Utc>,
    user_id: u64,
) -> Option<RecordingHandle> {
    match crate::database::recordings::create_recording(
        &inner.pool,
        inner.guild_id.get() as i64,
        inner.channel_id.get() as i64,
        user_id as i64,
        now,
        &inner.recording_owner_instance_id,
    )
    .await
    {
        Ok(handle) => Some(handle),
        Err(err) => {
            error!("failed to create recording db/path handle: {}", err);
            inner
                .metrics
                .db_insert_failures
                .fetch_add(1, Ordering::Relaxed);
            inner
                .metrics
                .db_query_errors
                .fetch_add(1, Ordering::Relaxed);
            None
        }
    }
}

pub(super) async fn heartbeat_active_recordings(inner: &Arc<InnerReceiver>) {
    let mut audio_file_ids = HashSet::new();
    {
        let active = inner.ssrc_writer_hashmap.read().await;
        for recording in active.values() {
            let recording = recording.lock().await;
            audio_file_ids.insert(recording.audio_file_id);
        }
    }
    {
        let paused = inner.paused_recordings.read().await;
        for recording in paused.values() {
            let recording = recording.recording.lock().await;
            audio_file_ids.insert(recording.audio_file_id);
        }
    }

    if audio_file_ids.is_empty() {
        return;
    }

    let audio_file_ids = audio_file_ids.into_iter().collect::<Vec<_>>();
    if let Err(err) = crate::database::recordings::heartbeat_active_recordings(
        &inner.pool,
        &audio_file_ids,
        &inner.recording_owner_instance_id,
    )
    .await
    {
        warn!("recording heartbeat failed: {}", err);
        inner
            .metrics
            .db_query_errors
            .fetch_add(1, Ordering::Relaxed);
    }
}

pub(super) async fn mark_recording_setup_failed(
    inner: &Arc<InnerReceiver>,
    audio_file_id: i64,
    reason: RecordingFinalizeReason,
) {
    if let Err(err) = crate::database::recordings::mark_recording_setup_failed(
        &inner.pool,
        audio_file_id,
        &inner.recording_owner_instance_id,
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
        inner
            .metrics
            .db_query_errors
            .fetch_add(1, Ordering::Relaxed);
    }
}
