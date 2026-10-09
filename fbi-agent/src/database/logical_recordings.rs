use std::path::Path;

use chrono::{Datelike, TimeZone, Utc};
use sakiot_paths::{DataRoots, RecordingKey};
use sqlx::{Pool, Postgres, Transaction};

use crate::database::recordings::RecordingHandle;
use crate::database::{DbError, DbResult};

mod pending;
mod recovery;
mod rows;

pub use pending::*;
pub use recovery::*;

use rows::*;

pub const DEFAULT_PENDING_CAP_SECONDS: i64 = 6 * 60 * 60;
pub const USER_UNAVAILABLE_GRACE_SECONDS: i64 = 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingDeadlines {
    pub absolute_cap_ms: Option<i64>,
    pub pending_deadline_ms: Option<i64>,
}

/// Computes deadlines for one pending episode. `pause_at_ms` never changes
/// while a user moves among channels, so the absolute cap cannot be reset by a
/// later disconnect/AFK signal.
pub fn pending_deadlines(
    pause_at_ms: i64,
    unavailable_at_ms: Option<i64>,
    has_afk_channel: bool,
    pending_cap_seconds: i64,
) -> PendingDeadlines {
    let absolute_cap_ms = (!has_afk_channel)
        .then(|| pause_at_ms.saturating_add(pending_cap_seconds.max(60).saturating_mul(1_000)));
    let grace_ms = unavailable_at_ms
        .map(|at| at.saturating_add(USER_UNAVAILABLE_GRACE_SECONDS.saturating_mul(1_000)));
    let pending_deadline_ms = match (absolute_cap_ms, grace_ms) {
        (Some(cap), Some(grace)) => Some(cap.min(grace)),
        (Some(cap), None) => Some(cap),
        (None, Some(grace)) => Some(grace),
        (None, None) => None,
    };

    PendingDeadlines {
        absolute_cap_ms,
        pending_deadline_ms,
    }
}

#[derive(Clone, Debug)]
pub struct PauseRequest<'a> {
    pub recording_session_id: i64,
    pub at_ms: i64,
    pub reason: &'a str,
    pub from_channel_id: Option<i64>,
    pub to_channel_id: Option<i64>,
    pub has_afk_channel: bool,
    pub starts_grace: bool,
    pub pending_cap_seconds: i64,
    pub owner_instance_id: &'a str,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RecoveryReport {
    pub stale_pending_released: u64,
    pub stale_active_finalized: u64,
    pub overdue_finalized: u64,
}

pub async fn pending_cap_seconds(pool: &Pool<Postgres>, guild_id: i64) -> DbResult<i64> {
    let value = sqlx::query_scalar!(
        "SELECT pending_cap_seconds FROM guild_voice_settings WHERE guild_id = $1",
        guild_id
    )
    .fetch_optional(pool)
    .await?
    .map(i64::from)
    .unwrap_or(DEFAULT_PENDING_CAP_SECONDS);
    Ok(value.max(60))
}

pub async fn create_fragment(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_id: i64,
    user_id: i64,
    now: chrono::DateTime<Utc>,
    owner_instance_id: &str,
) -> DbResult<RecordingHandle> {
    let root = DataRoots::from_env().recordings;
    create_fragment_in(
        pool,
        guild_id,
        channel_id,
        user_id,
        now,
        owner_instance_id,
        &root,
    )
    .await
}

pub async fn create_fragment_in(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_id: i64,
    user_id: i64,
    now: chrono::DateTime<Utc>,
    owner_instance_id: &str,
    recording_root: &Path,
) -> DbResult<RecordingHandle> {
    let now_ms = now.timestamp_millis();
    let mut tx = pool.begin().await?;
    sqlx::query!("SELECT pg_advisory_xact_lock($1)", guild_id)
        .execute(&mut *tx)
        .await?;
    let excluded = sqlx::query_scalar!(
        r#"SELECT COALESCE($2 = ANY(excluded_channel_ids), false) AS "excluded!" FROM guild_recording_policy WHERE guild_id=$1"#,
        guild_id,
        channel_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(false);
    if excluded {
        return Err(DbError::RecordingExcluded);
    }
    if crate::database::opt_outs::is_opted_out(&mut *tx, guild_id, user_id).await? {
        return Err(DbError::RecordingOptedOut);
    }
    lock_user_session(&mut tx, guild_id, user_id).await?;
    expire_user_pending_in_tx(&mut tx, guild_id, user_id, now_ms).await?;

    let mut session = select_open_session(&mut tx, guild_id, user_id).await?;
    if session.as_ref().is_some_and(|row| row.state == "pending") {
        resume_row_in_tx(
            &mut tx,
            session.as_ref().map(|row| row.id).unwrap_or_default(),
            channel_id,
            now_ms,
            owner_instance_id,
        )
        .await?;
        session = select_open_session(&mut tx, guild_id, user_id).await?;
    }

    let session = match session {
        Some(row) if row.state == "active" => row,
        _ => {
            create_session_in_tx(
                &mut tx,
                guild_id,
                channel_id,
                user_id,
                now_ms,
                owner_instance_id,
            )
            .await?
        }
    };

    let previous_start_ms = sqlx::query_scalar!(
        r#"SELECT COALESCE(MAX(start_ts), -1) AS "value!"
           FROM audio_files
          WHERE recording_session_id = $1"#,
        session.id
    )
    .fetch_one(&mut *tx)
    .await?;

    let requested_start_ms = session.next_fragment_start_ms.unwrap_or(now_ms).min(now_ms);
    let fragment_start_ms = requested_start_ms.max(previous_start_ms.saturating_add(1));
    let segment_index = session.last_segment_index.saturating_add(1);
    let fragment_start = Utc
        .timestamp_millis_opt(fragment_start_ms)
        .single()
        .unwrap_or(now);
    let file_name = RecordingKey::stem_for(fragment_start_ms, user_id);
    let key = RecordingKey::new(
        guild_id,
        channel_id,
        fragment_start.year(),
        fragment_start.month(),
        file_name.clone(),
    );
    let dir_path = recording_root.join(key.dir_suffix());
    std::fs::create_dir_all(&dir_path)?;
    let combined_path = dir_path.join(&file_name);

    let audio_file_id = sqlx::query_scalar!(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month, start_ts, end_ts,
             recording_owner_instance_id, recording_heartbeat_at,
             recording_session_id, segment_index)
         VALUES
            ($1, $2, $3, $4, $5, $6, $7, NULL, $8, now(), $9, $10)
         RETURNING id",
        file_name,
        guild_id,
        channel_id,
        user_id,
        fragment_start.year(),
        fragment_start.month() as i32,
        fragment_start_ms,
        owner_instance_id,
        session.id,
        segment_index
    )
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query!(
        "UPDATE recording_sessions
            SET state = 'active',
                current_channel_id = $2,
                owner_instance_id = $3,
                last_segment_index = $4,
                next_fragment_start_at = NULL,
                resumed_at = NULL,
                updated_at = now()
          WHERE id = $1",
        session.id,
        channel_id,
        owner_instance_id,
        segment_index
    )
    .execute(&mut *tx)
    .await?;

    insert_session_event_in_tx(
        &mut tx,
        session.id,
        fragment_start_ms,
        "fragment_open",
        Some(channel_id),
        None,
        serde_json::json!({
            "audio_file_id": audio_file_id,
            "segment_index": segment_index,
            "file_name": file_name,
        }),
    )
    .await?;

    tx.commit().await?;

    Ok(RecordingHandle {
        audio_file_id,
        recording_session_id: session.id,
        segment_index,
        file_name,
        path: combined_path.to_string_lossy().into_owned(),
        start_time: fragment_start,
        initial_silence_ms: now_ms.saturating_sub(fragment_start_ms),
    })
}

#[cfg(test)]
mod tests;
