use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::Mutex;
use tracing::error;

use super::InnerReceiver;
use super::persistence::insert_voice_event_audit;
use super::state::{PausedRecording, UserRecording, VoiceEventType};

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

    if let Err(e) = rec.writer.finish() {
        error!("Failed to finalize writer for ssrc {}: {}", ssrc, e);
        inner.metrics.track_recording_finalize_error();
        insert_voice_event_audit(
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

    insert_voice_event_audit(inner, user_id, rec_ssrc, event_type, "Writer closed").await;
}
