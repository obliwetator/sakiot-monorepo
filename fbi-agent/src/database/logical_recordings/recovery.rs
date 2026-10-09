//! Recovering sessions a previous agent left open, and closing fragments and
//! sessions whose setup failed.

use super::*;

pub async fn recover_stale_sessions(
    pool: &Pool<Postgres>,
    now_ms: i64,
    stale_after_seconds: i64,
    starting_instance_id: Option<&str>,
) -> DbResult<RecoveryReport> {
    let stale_pending_released = sqlx::query!(
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
        stale_after_seconds as f64,
        starting_instance_id
    )
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
    let stale_active_finalized = sqlx::query!(
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
        stale_after_seconds as f64,
        starting_instance_id
    )
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
    sqlx::query!(
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
        recording_session_id,
        at_ms as f64
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
