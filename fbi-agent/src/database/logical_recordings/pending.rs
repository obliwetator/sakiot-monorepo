//! Pausing a user's session, the pending episode that follows, and resuming
//! or expiring it.

use super::*;

pub async fn pause_session(pool: &Pool<Postgres>, request: PauseRequest<'_>) -> DbResult<bool> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query!(
        r#"SELECT state,
                (EXTRACT(EPOCH FROM pause_started_at) * 1000)::bigint AS pause_ms,
                (EXTRACT(EPOCH FROM absolute_cap_deadline_at) * 1000)::bigint AS cap_ms
           FROM recording_sessions
          WHERE id = $1
          FOR UPDATE"#,
        request.recording_session_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.commit().await?;
        return Ok(false);
    };
    if row.state == "finalized" {
        tx.commit().await?;
        return Ok(false);
    }

    let existing_pause_ms = row.pause_ms;
    let existing_cap_ms = row.cap_ms;
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

    sqlx::query!(
        "UPDATE recording_sessions
            SET state = 'pending',
                pause_started_at = COALESCE(pause_started_at, to_timestamp($2::bigint / 1000.0)),
                pending_deadline_at = CASE
                    WHEN $3::bigint IS NULL THEN pending_deadline_at
                    ELSE to_timestamp($3::bigint / 1000.0)
                END,
                absolute_cap_deadline_at = CASE
                    WHEN $4::bigint IS NULL THEN absolute_cap_deadline_at
                    ELSE to_timestamp($4::bigint / 1000.0)
                END,
                next_fragment_start_at = NULL,
                pending_reason = $5,
                pending_from_channel_id = COALESCE(pending_from_channel_id, $6),
                pending_to_channel_id = $7,
                owner_instance_id = $8,
                updated_at = now()
          WHERE id = $1",
        request.recording_session_id,
        pause_at_ms,
        pending_deadline_ms,
        absolute_cap_ms,
        request.reason,
        request.from_channel_id,
        request.to_channel_id,
        request.owner_instance_id
    )
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
    let session_id = sqlx::query_scalar!(
        "SELECT id
           FROM recording_sessions
          WHERE guild_id = $1
            AND user_id = $2
            AND state = 'active'
            AND owner_instance_id = $3
          ORDER BY started_at DESC, id DESC
          LIMIT 1",
        request.guild_id,
        request.user_id,
        request.owner_instance_id
    )
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
    let rows = sqlx::query!(
        "SELECT id, user_id
           FROM recording_sessions
          WHERE guild_id = $1
            AND state = 'active'
            AND owner_instance_id = $2
          ORDER BY id",
        guild_id,
        owner_instance_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|row| (row.id, row.user_id)).collect())
}

pub async fn mark_pending_user_unavailable(
    pool: &Pool<Postgres>,
    request: PendingUserUnavailableRequest<'_>,
) -> DbResult<bool> {
    let session_id = sqlx::query_scalar!(
        "SELECT id
           FROM recording_sessions
          WHERE guild_id = $1 AND user_id = $2 AND state = 'pending'
          ORDER BY started_at DESC, id DESC
          LIMIT 1",
        request.guild_id,
        request.user_id
    )
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
    // A logical session must never be reopened in a channel the guild excludes
    // from recording. This is the data-layer counterpart to the recorder
    // actor's in-memory suspension flag.
    let excluded = match sqlx::query_scalar!(
        "SELECT COALESCE($2 = ANY(excluded_channel_ids), false) FROM guild_recording_policy WHERE guild_id = $1",
        guild_id,
        channel_id,
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        // No policy row means the guild has no exclusions.
        None => false,
        Some(Some(excluded)) => excluded,
        // `COALESCE(..., false)` is NOT NULL; a NULL means the driver lost the
        // query's shape, which must fail closed rather than read as permitted.
        Some(None) => return Err(sqlx::Error::RowNotFound.into()),
    };
    if excluded {
        tx.commit().await?;
        return Ok(None);
    }
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
    let rows = sqlx::query!(
        r#"SELECT id,
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
          FOR UPDATE"#,
        now_ms as f64
    )
    .fetch_all(&mut *tx)
    .await?;

    for row in &rows {
        finalize_pending_row_in_tx(
            &mut tx,
            row.id,
            row.pause_ms.unwrap_or(now_ms),
            row.deadline_ms,
            row.cap_ms,
            row.current_channel_id,
        )
        .await?;
    }
    let count = rows.len() as u64;
    tx.commit().await?;
    Ok(count)
}
