use chrono::Datelike;
use sakiot_paths::{DataRoots, RecordingKey};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::{error, warn};

use super::InnerReceiver;
use super::state::VoiceEventType;

pub(super) async fn insert_voice_event_audit(
    inner: &Arc<InnerReceiver>,
    user_id: u64,
    ssrc: u32,
    event_type: VoiceEventType,
    details: &str,
) {
    let _ = sqlx::query(
        "INSERT INTO voice_events_audit (guild_id, user_id, ssrc, event_type_id, details) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(inner.guild_id.get() as i64)
    .bind(user_id as i64)
    .bind(ssrc as i64)
    .bind(event_type as i32)
    .bind(details)
    .execute(&inner.pool)
    .await;
}

#[tracing::instrument(skip_all, name = "create_path")]
pub(super) async fn create_path(
    inner: &Arc<InnerReceiver>,
    now: chrono::DateTime<chrono::Utc>,
    user_id: u64,
    is_channel_empty: bool,
) -> Option<String> {
    let guild_id = inner.guild_id;
    let channel_id = inner.channel_id;
    let file_name = RecordingKey::stem_for(now.timestamp_millis(), user_id as i64);
    let key = RecordingKey::new(
        guild_id.get() as i64,
        channel_id.get() as i64,
        now.year(),
        now.month(),
        file_name.clone(),
    );

    let recording_root = DataRoots::from_env().recordings_str();
    let dir_path = key.recording_dir(&recording_root);
    let combined_path = key.recording_dir(&recording_root).join(&file_name);

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
        inner.recording_owner_instance_id.clone()
    )
    .execute(&inner.pool)
    .await
    {
        Ok(ok) => ok,
        Err(err) => {
            error!("{}", err);
            inner
                .metrics
                .db_insert_failures
                .fetch_add(1, Ordering::Relaxed);
            inner
                .metrics
                .db_query_errors
                .fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };

    Some(combined_path.to_string_lossy().into_owned())
}

pub(super) async fn heartbeat_active_recordings(inner: &Arc<InnerReceiver>) {
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
