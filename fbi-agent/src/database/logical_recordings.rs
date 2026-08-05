use std::path::Path;

use chrono::{Datelike, TimeZone, Utc};
use sakiot_paths::{DataRoots, RecordingKey};
use sqlx::{Pool, Postgres, Row, Transaction};

use crate::database::DbResult;
use crate::database::recordings::RecordingHandle;

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
    let value = sqlx::query_scalar::<_, i32>(
        "SELECT pending_cap_seconds FROM guild_voice_settings WHERE guild_id = $1",
    )
    .bind(guild_id)
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

    let previous_start_ms = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(start_ts), -1)
           FROM audio_files
          WHERE recording_session_id = $1",
    )
    .bind(session.id)
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

    let audio_file_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month, start_ts, end_ts,
             recording_owner_instance_id, recording_heartbeat_at,
             recording_session_id, segment_index)
         VALUES
            ($1, $2, $3, $4, $5, $6, $7, NULL, $8, now(), $9, $10)
         RETURNING id",
    )
    .bind(&file_name)
    .bind(guild_id)
    .bind(channel_id)
    .bind(user_id)
    .bind(fragment_start.year())
    .bind(fragment_start.month() as i32)
    .bind(fragment_start_ms)
    .bind(owner_instance_id)
    .bind(session.id)
    .bind(segment_index)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE recording_sessions
            SET state = 'active',
                current_channel_id = $2,
                owner_instance_id = $3,
                last_segment_index = $4,
                next_fragment_start_at = NULL,
                resumed_at = NULL,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(session.id)
    .bind(channel_id)
    .bind(owner_instance_id)
    .bind(segment_index)
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

pub async fn pause_session(pool: &Pool<Postgres>, request: PauseRequest<'_>) -> DbResult<bool> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT state,
                (EXTRACT(EPOCH FROM pause_started_at) * 1000)::bigint AS pause_ms,
                (EXTRACT(EPOCH FROM absolute_cap_deadline_at) * 1000)::bigint AS cap_ms
           FROM recording_sessions
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(request.recording_session_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.commit().await?;
        return Ok(false);
    };
    let state: String = row.try_get("state")?;
    if state == "finalized" {
        tx.commit().await?;
        return Ok(false);
    }

    let existing_pause_ms: Option<i64> = row.try_get("pause_ms")?;
    let existing_cap_ms: Option<i64> = row.try_get("cap_ms")?;
    let pause_at_ms = existing_pause_ms.unwrap_or(request.at_ms);
    let computed = pending_deadlines(
        pause_at_ms,
        request.starts_grace.then_some(request.at_ms),
        request.has_afk_channel,
        request.pending_cap_seconds,
    );
    let absolute_cap_ms = existing_cap_ms.or(computed.absolute_cap_ms);
    let grace_ms = request.starts_grace.then(|| {
        request
            .at_ms
            .saturating_add(USER_UNAVAILABLE_GRACE_SECONDS.saturating_mul(1_000))
    });
    let pending_deadline_ms = match (absolute_cap_ms, grace_ms) {
        (Some(cap), Some(grace)) => Some(cap.min(grace)),
        (Some(cap), None) => Some(cap),
        (None, Some(grace)) => Some(grace),
        (None, None) => None,
    };

    sqlx::query(
        "UPDATE recording_sessions
            SET state = 'pending',
                pause_started_at = COALESCE(pause_started_at, to_timestamp($2::double precision / 1000.0)),
                pending_deadline_at = CASE
                    WHEN $3::bigint IS NULL THEN pending_deadline_at
                    ELSE to_timestamp($3::double precision / 1000.0)
                END,
                absolute_cap_deadline_at = CASE
                    WHEN $4::bigint IS NULL THEN absolute_cap_deadline_at
                    ELSE to_timestamp($4::double precision / 1000.0)
                END,
                next_fragment_start_at = NULL,
                pending_reason = $5,
                pending_from_channel_id = COALESCE(pending_from_channel_id, $6),
                pending_to_channel_id = $7,
                owner_instance_id = $8,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(request.recording_session_id)
    .bind(pause_at_ms)
    .bind(pending_deadline_ms)
    .bind(absolute_cap_ms)
    .bind(request.reason)
    .bind(request.from_channel_id)
    .bind(request.to_channel_id)
    .bind(request.owner_instance_id)
    .execute(&mut *tx)
    .await?;

    let event_type = match request.reason {
        "afk" => "afk",
        "disconnect" => "disconnect",
        "network" => "network_pause",
        _ => "pause",
    };
    insert_session_event_in_tx(
        &mut tx,
        request.recording_session_id,
        request.at_ms,
        event_type,
        request.to_channel_id.or(request.from_channel_id),
        request.from_channel_id,
        serde_json::json!({
            "reason": request.reason,
            "pending_deadline_ms": pending_deadline_ms,
            "absolute_cap_deadline_ms": absolute_cap_ms,
        }),
    )
    .await?;

    tx.commit().await?;
    Ok(true)
}

pub struct PendingUserUnavailableRequest<'a> {
    pub guild_id: i64,
    pub user_id: i64,
    pub at_ms: i64,
    pub reason: &'a str,
    pub channel_id: Option<i64>,
    pub has_afk_channel: bool,
    pub pending_cap_seconds: i64,
    pub owner_instance_id: &'a str,
}

pub struct PauseActiveUserRequest<'a> {
    pub guild_id: i64,
    pub user_id: i64,
    pub at_ms: i64,
    pub reason: &'a str,
    pub from_channel_id: Option<i64>,
    pub to_channel_id: Option<i64>,
    pub has_afk_channel: bool,
    pub starts_grace: bool,
    pub pending_cap_seconds: i64,
    pub owner_instance_id: &'a str,
}

/// Pauses a logical session that was resumed when the user reached the bot but
/// has not opened its next physical fragment yet. Without this path, a silent
/// user could leave—or the bot could hand off again—while the session stayed
/// incorrectly active forever.
pub async fn pause_active_user(
    pool: &Pool<Postgres>,
    request: PauseActiveUserRequest<'_>,
) -> DbResult<bool> {
    let session_id = sqlx::query_scalar::<_, i64>(
        "SELECT id
           FROM recording_sessions
          WHERE guild_id = $1
            AND user_id = $2
            AND state = 'active'
            AND owner_instance_id = $3
          ORDER BY started_at DESC, id DESC
          LIMIT 1",
    )
    .bind(request.guild_id)
    .bind(request.user_id)
    .bind(request.owner_instance_id)
    .fetch_optional(pool)
    .await?;
    let Some(recording_session_id) = session_id else {
        return Ok(false);
    };

    pause_session(
        pool,
        PauseRequest {
            recording_session_id,
            at_ms: request.at_ms,
            reason: request.reason,
            from_channel_id: request.from_channel_id,
            to_channel_id: request.to_channel_id,
            has_afk_channel: request.has_afk_channel,
            starts_grace: request.starts_grace,
            pending_cap_seconds: request.pending_cap_seconds,
            owner_instance_id: request.owner_instance_id,
        },
    )
    .await
}

pub async fn owned_active_sessions(
    pool: &Pool<Postgres>,
    guild_id: i64,
    owner_instance_id: &str,
) -> DbResult<Vec<(i64, i64)>> {
    Ok(sqlx::query_as::<_, (i64, i64)>(
        "SELECT id, user_id
           FROM recording_sessions
          WHERE guild_id = $1
            AND state = 'active'
            AND owner_instance_id = $2
          ORDER BY id",
    )
    .bind(guild_id)
    .bind(owner_instance_id)
    .fetch_all(pool)
    .await?)
}

pub async fn mark_pending_user_unavailable(
    pool: &Pool<Postgres>,
    request: PendingUserUnavailableRequest<'_>,
) -> DbResult<bool> {
    let session_id = sqlx::query_scalar::<_, i64>(
        "SELECT id
           FROM recording_sessions
          WHERE guild_id = $1 AND user_id = $2 AND state = 'pending'
          ORDER BY started_at DESC, id DESC
          LIMIT 1",
    )
    .bind(request.guild_id)
    .bind(request.user_id)
    .fetch_optional(pool)
    .await?;
    let Some(recording_session_id) = session_id else {
        return Ok(false);
    };

    pause_session(
        pool,
        PauseRequest {
            recording_session_id,
            at_ms: request.at_ms,
            reason: request.reason,
            from_channel_id: request.channel_id,
            to_channel_id: request.channel_id,
            has_afk_channel: request.has_afk_channel,
            starts_grace: true,
            pending_cap_seconds: request.pending_cap_seconds,
            owner_instance_id: request.owner_instance_id,
        },
    )
    .await
}

pub async fn resume_pending_user(
    pool: &Pool<Postgres>,
    guild_id: i64,
    user_id: i64,
    channel_id: i64,
    at_ms: i64,
    owner_instance_id: &str,
) -> DbResult<Option<i64>> {
    let mut tx = pool.begin().await?;
    lock_user_session(&mut tx, guild_id, user_id).await?;
    expire_user_pending_in_tx(&mut tx, guild_id, user_id, at_ms).await?;
    let row = select_open_session(&mut tx, guild_id, user_id).await?;
    let Some(row) = row.filter(|row| row.state == "pending") else {
        tx.commit().await?;
        return Ok(None);
    };

    resume_row_in_tx(&mut tx, row.id, channel_id, at_ms, owner_instance_id).await?;
    tx.commit().await?;
    Ok(Some(row.id))
}

pub async fn expire_pending_sessions(pool: &Pool<Postgres>, now_ms: i64) -> DbResult<u64> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query(
        "SELECT id,
                (EXTRACT(EPOCH FROM pause_started_at) * 1000)::bigint AS pause_ms,
                (EXTRACT(EPOCH FROM pending_deadline_at) * 1000)::bigint AS deadline_ms,
                (EXTRACT(EPOCH FROM absolute_cap_deadline_at) * 1000)::bigint AS cap_ms,
                current_channel_id
           FROM recording_sessions
          WHERE state = 'pending'
            AND (
                (pending_deadline_at IS NOT NULL
                    AND pending_deadline_at <= to_timestamp($1::double precision / 1000.0))
                OR (absolute_cap_deadline_at IS NOT NULL
                    AND absolute_cap_deadline_at <= to_timestamp($1::double precision / 1000.0))
            )
          FOR UPDATE",
    )
    .bind(now_ms)
    .fetch_all(&mut *tx)
    .await?;

    for row in &rows {
        let session_id: i64 = row.try_get("id")?;
        let pause_ms: Option<i64> = row.try_get("pause_ms")?;
        let deadline_ms: Option<i64> = row.try_get("deadline_ms")?;
        let cap_ms: Option<i64> = row.try_get("cap_ms")?;
        let channel_id: Option<i64> = row.try_get("current_channel_id")?;
        finalize_pending_row_in_tx(
            &mut tx,
            session_id,
            pause_ms.unwrap_or(now_ms),
            deadline_ms,
            cap_ms,
            channel_id,
        )
        .await?;
    }
    let count = rows.len() as u64;
    tx.commit().await?;
    Ok(count)
}

pub async fn recover_stale_sessions(
    pool: &Pool<Postgres>,
    now_ms: i64,
    stale_after_seconds: i64,
    starting_instance_id: Option<&str>,
) -> DbResult<RecoveryReport> {
    let stale_pending_released = sqlx::query(
        "UPDATE recording_sessions rs
            SET owner_instance_id = NULL, updated_at = now()
          WHERE rs.state = 'pending'
            AND rs.owner_instance_id IS NOT NULL
            AND (
                ($2::text IS NOT NULL AND rs.owner_instance_id = $2)
                OR NOT EXISTS (
                    SELECT 1
                      FROM bot_instances bi
                     WHERE bi.instance_id = rs.owner_instance_id
                       AND bi.heartbeat_at > now() - ($1::double precision * interval '1 second')
                       AND bi.state <> 'stopped'
                )
            )",
    )
    .bind(stale_after_seconds as f64)
    .bind(starting_instance_id)
    .execute(pool)
    .await?
    .rows_affected();

    // Reclaiming a session whose owner instance is gone. The `starting_instance_id`
    // branch must NOT fire while the old process with the same instance id is
    // still alive (an overlapping release during a drain keeps its own
    // `bot_instances` heartbeat fresh, so that row cannot distinguish the two
    // processes). Fragments the old process is still writing carry a fresh
    // `recording_heartbeat_at`, so gate the reclaim on that instead. Require a
    // stale unfinished/reaped fragment as crash evidence: an active session
    // with no such fragment can be a live, silent post-handoff session and is
    // safe for the restarted process to reuse.
    let stale_active_finalized = sqlx::query(
        "UPDATE recording_sessions rs
            SET state = 'finalized',
                ended_at = COALESCE(
                    (
                        SELECT to_timestamp(COALESCE(MAX(af.end_ts), MAX(af.start_ts))::double precision / 1000.0)
                          FROM audio_files af
                         WHERE af.recording_session_id = rs.id
                    ),
                    rs.started_at
                ),
                owner_instance_id = NULL,
                end_reason = 'owner_lost',
                updated_at = now()
          WHERE rs.state = 'active'
            AND rs.owner_instance_id IS NOT NULL
            AND (
                ($2::text IS NOT NULL AND rs.owner_instance_id = $2
                    AND EXISTS (
                        SELECT 1 FROM audio_files af
                         WHERE af.recording_session_id = rs.id
                           AND af.recording_owner_instance_id = rs.owner_instance_id
                           AND (af.end_ts IS NULL OR af.reaped IS TRUE)
                    )
                    AND NOT EXISTS (
                        SELECT 1 FROM audio_files af
                         WHERE af.recording_session_id = rs.id
                          AND af.recording_owner_instance_id = rs.owner_instance_id
                           AND af.end_ts IS NULL
                           AND af.recording_heartbeat_at
                               > now() - ($1::double precision * interval '1 second')
                    ))
                OR NOT EXISTS (
                    SELECT 1
                      FROM bot_instances bi
                     WHERE bi.instance_id = rs.owner_instance_id
                       AND bi.heartbeat_at > now() - ($1::double precision * interval '1 second')
                       AND bi.state <> 'stopped'
                )
            )",
    )
    .bind(stale_after_seconds as f64)
    .bind(starting_instance_id)
    .execute(pool)
    .await?
    .rows_affected();

    let overdue_finalized = expire_pending_sessions(pool, now_ms).await?;
    Ok(RecoveryReport {
        stale_pending_released,
        stale_active_finalized,
        overdue_finalized,
    })
}

pub async fn insert_fragment_close_event(
    tx: &mut Transaction<'_, Postgres>,
    recording_session_id: i64,
    at_ms: i64,
    channel_id: i64,
    audio_file_id: i64,
    segment_index: Option<i32>,
    reason: &str,
) -> DbResult<()> {
    insert_session_event_in_tx(
        tx,
        recording_session_id,
        at_ms,
        "fragment_close",
        Some(channel_id),
        None,
        serde_json::json!({
            "audio_file_id": audio_file_id,
            "segment_index": segment_index,
            "reason": reason,
        }),
    )
    .await
}

pub async fn finalize_setup_failed_session(
    tx: &mut Transaction<'_, Postgres>,
    recording_session_id: i64,
    at_ms: i64,
) -> DbResult<()> {
    sqlx::query(
        "UPDATE recording_sessions rs
            SET state = 'finalized',
                ended_at = to_timestamp($2::double precision / 1000.0),
                end_reason = 'fragment_setup_failed',
                updated_at = now()
          WHERE rs.id = $1
            AND NOT EXISTS (
                SELECT 1
                  FROM audio_files af
                 WHERE af.recording_session_id = rs.id
                   AND af.end_ts IS NOT NULL
                   AND af.reaped IS FALSE
            )",
    )
    .bind(recording_session_id)
    .bind(at_ms)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[derive(Debug)]
struct OpenSessionRow {
    id: i64,
    state: String,
    last_segment_index: i32,
    next_fragment_start_ms: Option<i64>,
}

async fn lock_user_session(
    tx: &mut Transaction<'_, Postgres>,
    guild_id: i64,
    user_id: i64,
) -> DbResult<()> {
    let key = guild_id
        .wrapping_mul(6_364_136_223_846_793_005_i64)
        .wrapping_add(user_id.rotate_left(17));
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn select_open_session(
    tx: &mut Transaction<'_, Postgres>,
    guild_id: i64,
    user_id: i64,
) -> DbResult<Option<OpenSessionRow>> {
    let row = sqlx::query(
        "SELECT id,
                state,
                last_segment_index,
                (EXTRACT(EPOCH FROM next_fragment_start_at) * 1000)::bigint AS next_fragment_start_ms
           FROM recording_sessions
          WHERE guild_id = $1 AND user_id = $2 AND state <> 'finalized'
          ORDER BY started_at DESC, id DESC
          LIMIT 1
          FOR UPDATE",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| {
        Ok(OpenSessionRow {
            id: row.try_get("id")?,
            state: row.try_get("state")?,
            last_segment_index: row.try_get("last_segment_index")?,
            next_fragment_start_ms: row.try_get("next_fragment_start_ms")?,
        })
    })
    .transpose()
}

async fn create_session_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    guild_id: i64,
    channel_id: i64,
    user_id: i64,
    now_ms: i64,
    owner_instance_id: &str,
) -> DbResult<OpenSessionRow> {
    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, current_channel_id, state,
             started_at, owner_instance_id)
         VALUES
            ($1, $2, $3, $3, 'active',
             to_timestamp($4::double precision / 1000.0), $5)
         RETURNING id",
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(channel_id)
    .bind(now_ms)
    .bind(owner_instance_id)
    .fetch_one(&mut **tx)
    .await?;

    insert_session_event_in_tx(
        tx,
        id,
        now_ms,
        "session_start",
        Some(channel_id),
        None,
        serde_json::json!({}),
    )
    .await?;
    Ok(OpenSessionRow {
        id,
        state: "active".to_string(),
        last_segment_index: -1,
        next_fragment_start_ms: None,
    })
}

async fn resume_row_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    session_id: i64,
    channel_id: i64,
    at_ms: i64,
    owner_instance_id: &str,
) -> DbResult<()> {
    let row = sqlx::query(
        "SELECT (EXTRACT(EPOCH FROM pause_started_at) * 1000)::bigint AS pause_ms,
                pending_reason,
                pending_from_channel_id,
                pending_to_channel_id
           FROM recording_sessions
          WHERE id = $1 AND state = 'pending'
          FOR UPDATE",
    )
    .bind(session_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let pause_ms: Option<i64> = row.try_get("pause_ms")?;
    let reason: Option<String> = row.try_get("pending_reason")?;
    let from_channel_id: Option<i64> = row.try_get("pending_from_channel_id")?;
    let planned_to_channel_id: Option<i64> = row.try_get("pending_to_channel_id")?;

    if let Some(pause_ms) = pause_ms
        && at_ms >= pause_ms
    {
        sqlx::query(
            "INSERT INTO recording_gaps
                (recording_session_id, started_at, ended_at, reason,
                 from_channel_id, to_channel_id)
             VALUES
                ($1,
                 to_timestamp($2::double precision / 1000.0),
                 to_timestamp($3::double precision / 1000.0),
                 $4, $5, $6)",
        )
        .bind(session_id)
        .bind(pause_ms)
        .bind(at_ms)
        .bind(reason.as_deref().unwrap_or("handoff"))
        .bind(from_channel_id)
        .bind(Some(channel_id))
        .execute(&mut **tx)
        .await?;
    }

    sqlx::query(
        "UPDATE recording_sessions
            SET state = 'active',
                current_channel_id = $2,
                resumed_at = to_timestamp($3::double precision / 1000.0),
                next_fragment_start_at = to_timestamp($3::double precision / 1000.0),
                pause_started_at = NULL,
                pending_deadline_at = NULL,
                absolute_cap_deadline_at = NULL,
                pending_reason = NULL,
                pending_from_channel_id = NULL,
                pending_to_channel_id = NULL,
                owner_instance_id = $4,
                updated_at = now()
          WHERE id = $1 AND state = 'pending'",
    )
    .bind(session_id)
    .bind(channel_id)
    .bind(at_ms)
    .bind(owner_instance_id)
    .execute(&mut **tx)
    .await?;

    insert_session_event_in_tx(
        tx,
        session_id,
        at_ms,
        "resume",
        Some(channel_id),
        from_channel_id,
        serde_json::json!({
            "reason": reason,
            "planned_to_channel_id": planned_to_channel_id,
        }),
    )
    .await?;
    Ok(())
}

async fn expire_user_pending_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    guild_id: i64,
    user_id: i64,
    now_ms: i64,
) -> DbResult<()> {
    let row = sqlx::query(
        "SELECT id,
                (EXTRACT(EPOCH FROM pause_started_at) * 1000)::bigint AS pause_ms,
                (EXTRACT(EPOCH FROM pending_deadline_at) * 1000)::bigint AS deadline_ms,
                (EXTRACT(EPOCH FROM absolute_cap_deadline_at) * 1000)::bigint AS cap_ms,
                current_channel_id
           FROM recording_sessions
          WHERE guild_id = $1 AND user_id = $2 AND state = 'pending'
          ORDER BY started_at DESC, id DESC
          LIMIT 1
          FOR UPDATE",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let deadline_ms: Option<i64> = row.try_get("deadline_ms")?;
    let cap_ms: Option<i64> = row.try_get("cap_ms")?;
    let effective = match (deadline_ms, cap_ms) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    if effective.is_none_or(|deadline| now_ms < deadline) {
        return Ok(());
    }

    finalize_pending_row_in_tx(
        tx,
        row.try_get("id")?,
        row.try_get::<Option<i64>, _>("pause_ms")?.unwrap_or(now_ms),
        deadline_ms,
        cap_ms,
        row.try_get("current_channel_id")?,
    )
    .await
}

async fn finalize_pending_row_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    session_id: i64,
    pause_ms: i64,
    deadline_ms: Option<i64>,
    cap_ms: Option<i64>,
    channel_id: Option<i64>,
) -> DbResult<()> {
    let cap_expired = cap_ms.is_some_and(|cap| deadline_ms.is_none_or(|deadline| cap <= deadline));
    let event_type = if cap_expired { "cap_expiry" } else { "timeout" };
    let end_reason = if cap_expired {
        "pending_cap_expired"
    } else {
        "pending_grace_expired"
    };

    sqlx::query(
        "UPDATE recording_sessions
            SET state = 'finalized',
                ended_at = to_timestamp($2::double precision / 1000.0),
                end_reason = $3,
                pending_deadline_at = NULL,
                absolute_cap_deadline_at = NULL,
                next_fragment_start_at = NULL,
                owner_instance_id = NULL,
                updated_at = now()
          WHERE id = $1 AND state = 'pending'",
    )
    .bind(session_id)
    .bind(pause_ms)
    .bind(end_reason)
    .execute(&mut **tx)
    .await?;

    insert_session_event_in_tx(
        tx,
        session_id,
        deadline_ms.or(cap_ms).unwrap_or(pause_ms),
        event_type,
        channel_id,
        None,
        serde_json::json!({ "ended_at_ms": pause_ms, "end_reason": end_reason }),
    )
    .await?;
    Ok(())
}

async fn insert_session_event_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    recording_session_id: i64,
    at_ms: i64,
    event_type: &str,
    channel_id: Option<i64>,
    previous_channel_id: Option<i64>,
    details: serde_json::Value,
) -> DbResult<()> {
    sqlx::query(
        "INSERT INTO recording_session_events
            (recording_session_id, occurred_at, event_type, channel_id,
             previous_channel_id, details)
         VALUES
            ($1, to_timestamp($2::double precision / 1000.0), $3, $4, $5, $6::jsonb)",
    )
    .bind(recording_session_id)
    .bind(at_ms)
    .bind(event_type)
    .bind(channel_id)
    .bind(previous_channel_id)
    .bind(details.to_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_PENDING_CAP_SECONDS, USER_UNAVAILABLE_GRACE_SECONDS, pending_deadlines};

    #[test]
    fn default_cap_starts_at_departure() {
        let deadlines = pending_deadlines(1_000, None, false, DEFAULT_PENDING_CAP_SECONDS);
        assert_eq!(
            deadlines.absolute_cap_ms,
            Some(1_000 + DEFAULT_PENDING_CAP_SECONDS * 1_000)
        );
        assert_eq!(deadlines.pending_deadline_ms, deadlines.absolute_cap_ms);
    }

    #[test]
    fn afk_guild_has_no_absolute_cap() {
        let deadlines = pending_deadlines(1_000, None, true, DEFAULT_PENDING_CAP_SECONDS);
        assert_eq!(deadlines.absolute_cap_ms, None);
        assert_eq!(deadlines.pending_deadline_ms, None);
    }

    #[test]
    fn disconnect_grace_is_bounded_by_existing_cap() {
        let cap_seconds = 60;
        let deadlines = pending_deadlines(1_000, Some(50_000), false, cap_seconds);
        assert_eq!(deadlines.absolute_cap_ms, Some(61_000));
        assert_eq!(deadlines.pending_deadline_ms, Some(61_000));
    }

    #[test]
    fn afk_or_disconnect_starts_sixty_second_grace() {
        let deadlines = pending_deadlines(1_000, Some(5_000), true, DEFAULT_PENDING_CAP_SECONDS);
        assert_eq!(deadlines.absolute_cap_ms, None);
        assert_eq!(
            deadlines.pending_deadline_ms,
            Some(5_000 + USER_UNAVAILABLE_GRACE_SECONDS * 1_000)
        );
    }
}
