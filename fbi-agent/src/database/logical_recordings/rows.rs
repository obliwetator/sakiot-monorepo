//! Session row operations that run inside a caller's transaction, under the
//! user's session lock.

use super::*;

#[derive(Debug)]
pub(super) struct OpenSessionRow {
    pub(super) id: i64,
    pub(super) state: String,
    pub(super) last_segment_index: i32,
    pub(super) next_fragment_start_ms: Option<i64>,
}

pub(super) async fn lock_user_session(
    tx: &mut Transaction<'_, Postgres>,
    guild_id: i64,
    user_id: i64,
) -> DbResult<()> {
    let key = guild_id
        .wrapping_mul(6_364_136_223_846_793_005_i64)
        .wrapping_add(user_id.rotate_left(17));
    sqlx::query!("SELECT pg_advisory_xact_lock($1)", key)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn select_open_session(
    tx: &mut Transaction<'_, Postgres>,
    guild_id: i64,
    user_id: i64,
) -> DbResult<Option<OpenSessionRow>> {
    let row = sqlx::query!(
        r#"SELECT id,
                state,
                last_segment_index,
                (EXTRACT(EPOCH FROM next_fragment_start_at) * 1000)::bigint AS next_fragment_start_ms
           FROM recording_sessions
          WHERE guild_id = $1 AND user_id = $2 AND state <> 'finalized'
          ORDER BY started_at DESC, id DESC
          LIMIT 1
          FOR UPDATE"#,
        guild_id,
        user_id
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| {
        Ok(OpenSessionRow {
            id: row.id,
            state: row.state,
            last_segment_index: row.last_segment_index,
            next_fragment_start_ms: row.next_fragment_start_ms,
        })
    })
    .transpose()
}

pub(super) async fn create_session_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    guild_id: i64,
    channel_id: i64,
    user_id: i64,
    now_ms: i64,
    owner_instance_id: &str,
) -> DbResult<OpenSessionRow> {
    let id = sqlx::query_scalar!(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, current_channel_id, state,
             started_at, owner_instance_id)
         VALUES
            ($1, $2, $3, $3, 'active',
             to_timestamp($4::double precision / 1000.0), $5)
         RETURNING id",
        guild_id,
        user_id,
        channel_id,
        now_ms as f64,
        owner_instance_id
    )
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

pub(super) async fn resume_row_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    session_id: i64,
    channel_id: i64,
    at_ms: i64,
    owner_instance_id: &str,
) -> DbResult<()> {
    let row = sqlx::query!(
        r#"SELECT (EXTRACT(EPOCH FROM pause_started_at) * 1000)::bigint AS pause_ms,
                pending_reason,
                pending_from_channel_id,
                pending_to_channel_id
           FROM recording_sessions
          WHERE id = $1 AND state = 'pending'
          FOR UPDATE"#,
        session_id
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let pause_ms = row.pause_ms;
    let reason = row.pending_reason;
    let from_channel_id = row.pending_from_channel_id;
    let planned_to_channel_id = row.pending_to_channel_id;

    if let Some(pause_ms) = pause_ms
        && at_ms >= pause_ms
    {
        sqlx::query!(
            "INSERT INTO recording_gaps
                (recording_session_id, started_at, ended_at, reason,
                 from_channel_id, to_channel_id)
             VALUES
                ($1,
                 to_timestamp($2::double precision / 1000.0),
                 to_timestamp($3::double precision / 1000.0),
                 $4, $5, $6)",
            session_id,
            pause_ms as f64,
            at_ms as f64,
            reason.as_deref().unwrap_or("handoff"),
            from_channel_id,
            Some(channel_id)
        )
        .execute(&mut **tx)
        .await?;
    }

    sqlx::query!(
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
        session_id,
        channel_id,
        at_ms as f64,
        owner_instance_id
    )
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

pub(super) async fn expire_user_pending_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    guild_id: i64,
    user_id: i64,
    now_ms: i64,
) -> DbResult<()> {
    let row = sqlx::query!(
        r#"SELECT id,
                (EXTRACT(EPOCH FROM pause_started_at) * 1000)::bigint AS pause_ms,
                (EXTRACT(EPOCH FROM pending_deadline_at) * 1000)::bigint AS deadline_ms,
                (EXTRACT(EPOCH FROM absolute_cap_deadline_at) * 1000)::bigint AS cap_ms,
                current_channel_id
           FROM recording_sessions
          WHERE guild_id = $1 AND user_id = $2 AND state = 'pending'
          ORDER BY started_at DESC, id DESC
          LIMIT 1
          FOR UPDATE"#,
        guild_id,
        user_id
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let deadline_ms = row.deadline_ms;
    let cap_ms = row.cap_ms;
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
        row.id,
        row.pause_ms.unwrap_or(now_ms),
        deadline_ms,
        cap_ms,
        row.current_channel_id,
    )
    .await
}

pub(super) async fn finalize_pending_row_in_tx(
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

    sqlx::query!(
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
        session_id,
        pause_ms as f64,
        end_reason
    )
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

pub(super) async fn insert_session_event_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    recording_session_id: i64,
    at_ms: i64,
    event_type: &str,
    channel_id: Option<i64>,
    previous_channel_id: Option<i64>,
    details: serde_json::Value,
) -> DbResult<()> {
    sqlx::query!(
        "INSERT INTO recording_session_events
            (recording_session_id, occurred_at, event_type, channel_id,
             previous_channel_id, details)
         VALUES
            ($1, to_timestamp($2::double precision / 1000.0), $3, $4, $5, $6::jsonb)",
        recording_session_id,
        at_ms as f64,
        event_type,
        channel_id,
        previous_channel_id,
        details
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
