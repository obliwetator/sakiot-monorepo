use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::Mutex;
use tracing::error;

use super::InnerReceiver;
use super::persistence::insert_receiver_voice_event;
use super::state::{PausedRecording, RecordingFinalizeReason, UserRecording, VoiceEventType};

pub(super) async fn finalize_all_active_recordings(
    inner: &Arc<InnerReceiver>,
    event_type: VoiceEventType,
) {
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

pub(super) async fn clear_receiver_state(inner: &Arc<InnerReceiver>) {
    inner.user_id_hashmap.write().await.clear();
    inner.paused_recordings.write().await.clear();
    inner.bot_ssrcs.write().await.clear();
    inner.bot_user_id_hashmap.write().await.clear();
    inner.session_start_ms.store(0, Ordering::SeqCst);
}

/// Close the writer for `ssrc`, run the audio_files DB update, decrement
/// active counters. Idempotent — silently no-ops if the writer is already gone.
pub(super) async fn finalize_writer(
    inner: &Arc<InnerReceiver>,
    ssrc: u32,
    event_type: VoiceEventType,
) {
    finalize_writer_at(inner, ssrc, event_type, chrono::Utc::now()).await;
}

/// Same as `finalize_writer`, but with an explicit close time when the real
/// leave happened during an outage and only the reconnect time is knowable.
pub(super) async fn finalize_writer_at(
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

pub(super) async fn finalize_recording_arc(
    inner: &Arc<InnerReceiver>,
    ssrc: u32,
    arc: Arc<Mutex<UserRecording>>,
    event_type: VoiceEventType,
    close_time: chrono::DateTime<chrono::Utc>,
) {
    let mut rec = arc.lock().await;

    let mut finalize_reason = finalize_reason(event_type);
    if let Err(e) = rec.writer.finish() {
        error!("Failed to finalize writer for ssrc {}: {}", ssrc, e);
        finalize_reason = RecordingFinalizeReason::WriterError;
        inner.metrics.track_recording_finalize_error();
        insert_receiver_voice_event(
            inner,
            rec.user_id,
            ssrc,
            VoiceEventType::WriterError,
            &format!("finish: {}", e),
        )
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
    let file_name = rec.file_name.clone();
    let audio_file_id = rec.audio_file_id;
    let user_id = rec.user_id;
    let rec_ssrc = rec.ssrc;
    drop(rec);

    if let Err(err) = crate::database::recordings::finalize_recording(
        &inner.pool,
        audio_file_id,
        &inner.recording_owner_instance_id,
        time_elapsed,
        finalize_reason.id(),
    )
    .await
    {
        error!(
            file_name,
            audio_file_id, "failed to finalize recording row: {}", err
        );
        inner
            .metrics
            .db_query_errors
            .fetch_add(1, Ordering::Relaxed);
    }

    insert_receiver_voice_event(inner, user_id, rec_ssrc, event_type, "Writer closed").await;
}

fn finalize_reason(event_type: VoiceEventType) -> RecordingFinalizeReason {
    match event_type {
        VoiceEventType::WriterOpen => RecordingFinalizeReason::Unknown,
        VoiceEventType::WriterClose => RecordingFinalizeReason::WriterClose,
        VoiceEventType::WriterError => RecordingFinalizeReason::WriterError,
        VoiceEventType::ZombieReaped => RecordingFinalizeReason::ZombieReaped,
    }
}
