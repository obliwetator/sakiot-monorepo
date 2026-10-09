use super::{
    DEFAULT_PENDING_CAP_SECONDS, PauseRequest, USER_UNAVAILABLE_GRACE_SECONDS, create_fragment_in,
    pause_session, pending_deadlines, resume_pending_user,
};
use crate::database::DbError;
use sqlx::PgPool;

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn excluded_channel_cannot_create_recording(pool: PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO guild_recording_policy (guild_id,excluded_channel_ids) VALUES (1,ARRAY[2::bigint])")
        .execute(&pool).await?;
    let root = tempfile::tempdir().unwrap();
    let result = create_fragment_in(&pool, 1, 2, 3, chrono::Utc::now(), "test", root.path()).await;
    assert!(matches!(result, Err(DbError::RecordingExcluded)));
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM audio_files")
        .fetch_one(&pool)
        .await?;
    assert_eq!(rows, 0);
    Ok(())
}

/// Opting out is per guild and per user: the opted-out user gets no
/// fragment, while the same user elsewhere and other users here still do.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn opted_out_user_cannot_create_recording(pool: PgPool) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO bot_instances (instance_id, role, state, heartbeat_at, started_at)
             VALUES ('test', 'active', 'active', now(), now())",
    )
    .execute(&pool)
    .await?;
    let root = tempfile::tempdir().unwrap();
    assert!(crate::database::opt_outs::set_opted_out(&pool, 1, 3, true).await?);
    assert!(!crate::database::opt_outs::set_opted_out(&pool, 1, 3, true).await?);

    let result = create_fragment_in(&pool, 1, 2, 3, chrono::Utc::now(), "test", root.path()).await;
    assert!(matches!(result, Err(DbError::RecordingOptedOut)));
    create_fragment_in(&pool, 1, 2, 4, chrono::Utc::now(), "test", root.path()).await?;
    create_fragment_in(&pool, 9, 2, 3, chrono::Utc::now(), "test", root.path()).await?;
    let recorded_users: Vec<(i64, i64)> =
        sqlx::query_as("SELECT guild_id, user_id FROM audio_files ORDER BY guild_id")
            .fetch_all(&pool)
            .await?;
    assert_eq!(recorded_users, vec![(1, 4), (9, 3)]);

    assert!(crate::database::opt_outs::set_opted_out(&pool, 1, 3, false).await?);
    create_fragment_in(&pool, 1, 2, 3, chrono::Utc::now(), "test", root.path()).await?;
    Ok(())
}

/// A pending logical session must not be reopened in a channel the guild
/// excludes. This is the data-layer counterpart to the recorder actor's
/// in-memory suspension: every resume call site funnels through here.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn excluded_channel_cannot_resume_pending_session(pool: PgPool) -> Result<(), DbError> {
    let owner = "test-excluded-resume";
    sqlx::query(
        "INSERT INTO bot_instances (instance_id, role, state, heartbeat_at, started_at)
             VALUES ($1, 'active', 'active', now(), now())
             ON CONFLICT (instance_id) DO UPDATE SET state = 'active', heartbeat_at = now()",
    )
    .bind(owner)
    .execute(&pool)
    .await?;
    let root = tempfile::tempdir().unwrap();
    let handle = create_fragment_in(&pool, 1, 2, 3, chrono::Utc::now(), owner, root.path())
        .await
        .expect("fragment");
    let now_ms = chrono::Utc::now().timestamp_millis();
    pause_session(
        &pool,
        PauseRequest {
            recording_session_id: handle.recording_session_id,
            at_ms: now_ms,
            reason: "user_moved",
            from_channel_id: Some(2),
            to_channel_id: Some(3),
            has_afk_channel: false,
            starts_grace: false,
            pending_cap_seconds: 3_600,
            owner_instance_id: owner,
        },
    )
    .await?;

    sqlx::query(
        "INSERT INTO guild_recording_policy (guild_id,excluded_channel_ids) VALUES (1,ARRAY[3::bigint])",
    )
    .execute(&pool)
    .await?;

    assert_eq!(
        resume_pending_user(&pool, 1, 3, 3, now_ms + 1_000, owner).await?,
        None,
        "an excluded channel must not reopen a logical session"
    );
    let state: String = sqlx::query_scalar("SELECT state FROM recording_sessions WHERE id = $1")
        .bind(handle.recording_session_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(state, "pending");

    sqlx::query("DELETE FROM guild_recording_policy WHERE guild_id = 1")
        .execute(&pool)
        .await?;
    assert_eq!(
        resume_pending_user(&pool, 1, 3, 3, now_ms + 2_000, owner).await?,
        Some(handle.recording_session_id),
        "a permitted channel resumes the same logical session"
    );
    Ok(())
}

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
