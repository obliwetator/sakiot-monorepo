use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serenity::{
    model::id::{ChannelId, GuildId},
    prelude::{RwLock, TypeMap},
};
use sqlx::{PgPool, migrate::Migrator};

use crate::cooldown::JamCooldown;
use crate::database::{DbError, logical_recordings, recordings, stamps};
use crate::runtime::{BotRole, RuntimeConfig, RuntimeState};

static FULL_MIGRATOR: Migrator = sqlx::migrate!("../sakiot-db/migrations");
const MEDIA_ARCHIVE_MIGRATION: i64 = 20_260_716_000_000;
const LOGICAL_RECORDINGS_MIGRATION: i64 = 20_260_712_000_000;
const STAMP_RECORDING_SESSIONS_MIGRATION: i64 = 20_260_731_000_000;

fn unique_id() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after unix epoch")
        .as_millis() as i64;
    9_000_000_000_000 + (millis % 1_000_000_000)
}

async fn insert_test_instance(
    pool: &PgPool,
    owner: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query!(
        "INSERT INTO bot_instances (instance_id, role, state, heartbeat_at, started_at)
         VALUES ($1, 'active', 'active', now(), now())
         ON CONFLICT (instance_id) DO UPDATE SET state = 'active', heartbeat_at = now()",
        owner
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn test_runtime(instance_id: String) -> Arc<RuntimeState> {
    RuntimeState::new(RuntimeConfig {
        instance_id,
        initial_role: BotRole::Active,
        drain_timeout: Duration::from_secs(30),
    })
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn recording_create_heartbeat_finalize_uses_audio_file_id(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let base = unique_id();
    let guild_id = base;
    let channel_id = base + 1;
    let user_id = base + 2;
    let owner = format!("test-recording-{base}");
    insert_test_instance(&pool, &owner).await?;

    let handle = recordings::create_recording_for_test(
        &pool,
        guild_id,
        channel_id,
        user_id,
        chrono::Utc::now(),
        &owner,
        temporary.path(),
    )
    .await?;

    assert!(handle.audio_file_id > 0);
    assert!(!handle.file_name.is_empty());
    assert_eq!(
        recordings::heartbeat_active_recordings(&pool, &[handle.audio_file_id], &owner).await?,
        1
    );
    recordings::finalize_recording(
        &pool,
        handle.audio_file_id,
        &owner,
        1_234,
        recordings::FINALIZE_REASON_WRITER_CLOSE,
    )
    .await?;

    let row = sqlx::query!(
        "SELECT id, file_name, end_ts - start_ts AS duration_ms, finalize_reason_id
           FROM audio_files
          WHERE id = $1",
        handle.audio_file_id
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.id, handle.audio_file_id);
    assert_eq!(row.file_name, handle.file_name);
    assert_eq!(row.duration_ms, Some(1_234));
    assert_eq!(
        row.finalize_reason_id,
        Some(recordings::FINALIZE_REASON_WRITER_CLOSE)
    );

    sqlx::query!(
        "DELETE FROM audio_files WHERE id = $1",
        handle.audio_file_id
    )
    .execute(&pool)
    .await?;
    sqlx::query!("DELETE FROM bot_instances WHERE instance_id = $1", owner)
        .execute(&pool)
        .await?;

    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn stamp_creation_persists_fragment_and_logical_session(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let base = unique_id();
    let guild_id = base;
    let channel_id = base + 1;
    let user_id = base + 2;
    let stamper_user_id = base + 3;
    let owner = format!("test-stamp-recording-{base}");
    insert_test_instance(&pool, &owner).await?;
    let now = chrono::Utc::now();

    let handle = recordings::create_recording_for_test(
        &pool,
        guild_id,
        channel_id,
        user_id,
        now,
        &owner,
        temporary.path(),
    )
    .await?;
    let active = stamps::active_recording_for_stamp(
        &pool,
        user_id,
        guild_id,
        channel_id,
        now.timestamp_millis(),
    )
    .await?
    .expect("active recording");
    assert_eq!(active.audio_file_id, handle.audio_file_id);
    assert_eq!(
        active.recording_session_id,
        Some(handle.recording_session_id)
    );

    let stamp_id = stamps::create_stamp(
        &pool,
        guild_id,
        channel_id,
        user_id,
        stamper_user_id,
        now.timestamp_millis(),
        -5_000,
        Some(active.audio_file_id),
        active.recording_session_id,
        Some("test stamp"),
    )
    .await?;
    let stored: (Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT audio_file_id, recording_session_id
           FROM stamps
          WHERE id = $1",
    )
    .bind(stamp_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(stored.0, Some(handle.audio_file_id));
    assert_eq!(stored.1, Some(handle.recording_session_id));
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn logical_recording_resumes_with_one_gap_and_next_fragment(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    use chrono::TimeZone;

    let temporary = tempfile::tempdir()?;
    let base = unique_id();
    let owner = format!("test-logical-resume-{base}");
    insert_test_instance(&pool, &owner).await?;
    let start_ms = 1_700_000_000_000;
    let first = logical_recordings::create_fragment_in(
        &pool,
        base,
        base + 1,
        base + 2,
        chrono::Utc.timestamp_millis_opt(start_ms).single().unwrap(),
        &owner,
        temporary.path(),
    )
    .await?;
    recordings::finalize_recording(
        &pool,
        first.audio_file_id,
        &owner,
        1_000,
        recordings::FINALIZE_REASON_WRITER_CLOSE,
    )
    .await?;

    assert!(
        logical_recordings::pause_session(
            &pool,
            logical_recordings::PauseRequest {
                recording_session_id: first.recording_session_id,
                at_ms: start_ms + 1_000,
                reason: "handoff",
                from_channel_id: Some(base + 1),
                to_channel_id: Some(base + 3),
                has_afk_channel: false,
                starts_grace: false,
                pending_cap_seconds: logical_recordings::DEFAULT_PENDING_CAP_SECONDS,
                owner_instance_id: &owner,
            },
        )
        .await?
    );
    assert_eq!(
        logical_recordings::resume_pending_user(
            &pool,
            base,
            base + 2,
            base + 3,
            start_ms + 4_000,
            &owner,
        )
        .await?,
        Some(first.recording_session_id)
    );

    let second = logical_recordings::create_fragment_in(
        &pool,
        base,
        base + 3,
        base + 2,
        chrono::Utc
            .timestamp_millis_opt(start_ms + 4_000)
            .single()
            .unwrap(),
        &owner,
        temporary.path(),
    )
    .await?;
    assert_eq!(second.recording_session_id, first.recording_session_id);
    assert_eq!(second.segment_index, 1);
    assert_eq!(second.initial_silence_ms, 0);

    let gap = sqlx::query_as::<_, (i64, i64, String, Option<i64>, Option<i64>)>(
        "SELECT
            (EXTRACT(EPOCH FROM started_at) * 1000)::bigint,
            (EXTRACT(EPOCH FROM ended_at) * 1000)::bigint,
            reason,
            from_channel_id,
            to_channel_id
           FROM recording_gaps
          WHERE recording_session_id = $1",
    )
    .bind(first.recording_session_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        gap,
        (
            start_ms + 1_000,
            start_ms + 4_000,
            "handoff".to_string(),
            Some(base + 1),
            Some(base + 3),
        )
    );

    let (state, channel_id, cap_is_null): (String, Option<i64>, bool) = sqlx::query_as(
        "SELECT state, current_channel_id, absolute_cap_deadline_at IS NULL
           FROM recording_sessions
          WHERE id = $1",
    )
    .bind(first.recording_session_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(state, "active");
    assert_eq!(channel_id, Some(base + 3));
    assert!(cap_is_null);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn unavailable_grace_expiry_ends_at_departure_without_silence(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    use chrono::TimeZone;

    let temporary = tempfile::tempdir()?;
    let base = unique_id();
    let owner = format!("test-logical-timeout-{base}");
    insert_test_instance(&pool, &owner).await?;
    let start_ms = 1_700_100_000_000;
    let pause_ms = start_ms + 1_000;
    let unavailable_ms = pause_ms + 1_000;
    let handle = logical_recordings::create_fragment_in(
        &pool,
        base,
        base + 1,
        base + 2,
        chrono::Utc.timestamp_millis_opt(start_ms).single().unwrap(),
        &owner,
        temporary.path(),
    )
    .await?;
    recordings::finalize_recording(
        &pool,
        handle.audio_file_id,
        &owner,
        pause_ms - start_ms,
        recordings::FINALIZE_REASON_WRITER_CLOSE,
    )
    .await?;
    logical_recordings::pause_session(
        &pool,
        logical_recordings::PauseRequest {
            recording_session_id: handle.recording_session_id,
            at_ms: pause_ms,
            reason: "handoff",
            from_channel_id: Some(base + 1),
            to_channel_id: Some(base + 3),
            has_afk_channel: true,
            starts_grace: false,
            pending_cap_seconds: logical_recordings::DEFAULT_PENDING_CAP_SECONDS,
            owner_instance_id: &owner,
        },
    )
    .await?;
    logical_recordings::mark_pending_user_unavailable(
        &pool,
        logical_recordings::PendingUserUnavailableRequest {
            guild_id: base,
            user_id: base + 2,
            at_ms: unavailable_ms,
            reason: "disconnect",
            channel_id: None,
            has_afk_channel: true,
            pending_cap_seconds: logical_recordings::DEFAULT_PENDING_CAP_SECONDS,
            owner_instance_id: &owner,
        },
    )
    .await?;
    let deadline_ms =
        unavailable_ms + logical_recordings::USER_UNAVAILABLE_GRACE_SECONDS.saturating_mul(1_000);
    assert_eq!(
        logical_recordings::expire_pending_sessions(&pool, deadline_ms).await?,
        1
    );

    let row: (String, i64, Option<String>) = sqlx::query_as(
        "SELECT state,
                (EXTRACT(EPOCH FROM ended_at) * 1000)::bigint,
                end_reason
           FROM recording_sessions
          WHERE id = $1",
    )
    .bind(handle.recording_session_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        row,
        (
            "finalized".to_string(),
            pause_ms,
            Some("pending_grace_expired".to_string())
        )
    );
    let gaps: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM recording_gaps WHERE recording_session_id = $1")
            .bind(handle.recording_session_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(gaps, 0);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn silent_resumed_session_can_pause_before_next_fragment(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let base = unique_id();
    let owner = format!("test-silent-resume-{base}");
    insert_test_instance(&pool, &owner).await?;
    let departure_ms = 1_700_200_010_000;
    let session_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, current_channel_id, state,
             started_at, resumed_at, next_fragment_start_at, owner_instance_id)
         VALUES
            ($1, $2, $3, $3, 'active',
             to_timestamp(1700200000), to_timestamp(1700200005),
             to_timestamp(1700200005), $4)
         RETURNING id",
    )
    .bind(base)
    .bind(base + 1)
    .bind(base + 2)
    .bind(&owner)
    .fetch_one(&pool)
    .await?;

    assert!(
        logical_recordings::pause_active_user(
            &pool,
            logical_recordings::PauseActiveUserRequest {
                guild_id: base,
                user_id: base + 1,
                at_ms: departure_ms,
                reason: "handoff",
                from_channel_id: Some(base + 2),
                to_channel_id: Some(base + 3),
                has_afk_channel: false,
                starts_grace: false,
                pending_cap_seconds: logical_recordings::DEFAULT_PENDING_CAP_SECONDS,
                owner_instance_id: &owner,
            },
        )
        .await?
    );
    let (state, pause_ms, cap_ms): (String, i64, i64) = sqlx::query_as(
        "SELECT state,
                (EXTRACT(EPOCH FROM pause_started_at) * 1000)::bigint,
                (EXTRACT(EPOCH FROM absolute_cap_deadline_at) * 1000)::bigint
           FROM recording_sessions
          WHERE id = $1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(state, "pending");
    assert_eq!(pause_ms, departure_ms);
    assert_eq!(
        cap_ms,
        departure_ms + logical_recordings::DEFAULT_PENDING_CAP_SECONDS * 1_000
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn startup_recovery_reclaims_same_instance_even_with_fresh_heartbeat(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let base = unique_id();
    let owner = format!("test-restarted-owner-{base}");
    insert_test_instance(&pool, &owner).await?;
    let active_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, current_channel_id, state,
             started_at, owner_instance_id)
         VALUES ($1, $2, $3, $3, 'active', to_timestamp(1700300000), $4)
         RETURNING id",
    )
    .bind(base)
    .bind(base + 1)
    .bind(base + 2)
    .bind(&owner)
    .fetch_one(&pool)
    .await?;
    let pending_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, current_channel_id, state,
             started_at, pause_started_at, owner_instance_id)
         VALUES ($1, $2, $3, $3, 'pending', to_timestamp(1700300000),
                 to_timestamp(1700300010), $4)
         RETURNING id",
    )
    .bind(base)
    .bind(base + 4)
    .bind(base + 2)
    .bind(&owner)
    .fetch_one(&pool)
    .await?;

    let report =
        logical_recordings::recover_stale_sessions(&pool, 1_700_300_020_000, 45, Some(&owner))
            .await?;
    assert_eq!(report.stale_active_finalized, 1);
    assert_eq!(report.stale_pending_released, 1);
    let active: (String, Option<String>) =
        sqlx::query_as("SELECT state, end_reason FROM recording_sessions WHERE id = $1")
            .bind(active_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(active.0, "finalized");
    assert_eq!(active.1.as_deref(), Some("owner_lost"));
    let pending: (String, Option<String>) =
        sqlx::query_as("SELECT state, owner_instance_id FROM recording_sessions WHERE id = $1")
            .bind(pending_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(pending, ("pending".to_string(), None));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn logical_recording_migration_backfills_one_session_per_file(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let prior = Migrator::with_migrations(
        FULL_MIGRATOR
            .iter()
            .filter(|migration| migration.version < LOGICAL_RECORDINGS_MIGRATION)
            .cloned()
            .collect(),
    );
    prior.run(&pool).await?;
    sqlx::query(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month, start_ts, end_ts)
         VALUES
            ('historical-finalized', 1, 10, 100, 2026, 7, 1000, 2000),
            ('historical-active', 1, 11, 100, 2026, 7, 3000, NULL)",
    )
    .execute(&pool)
    .await?;

    FULL_MIGRATOR.run(&pool).await?;

    let rows = sqlx::query_as::<_, (String, i64, i32, String, Option<String>)>(
        "SELECT af.file_name,
                af.recording_session_id,
                af.segment_index,
                rs.state,
                rs.end_reason
           FROM audio_files af
           JOIN recording_sessions rs ON rs.id = af.recording_session_id
          WHERE af.file_name LIKE 'historical-%'
          ORDER BY af.file_name",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].1, rows[1].1);
    assert!(rows.iter().all(|row| row.2 == 0));
    assert_eq!(rows[0].3, "active");
    assert_eq!(rows[0].4, None);
    assert_eq!(rows[1].3, "finalized");
    assert_eq!(rows[1].4.as_deref(), Some("historical_backfill"));

    let events: Vec<(String, i64)> = sqlx::query_as(
        "SELECT event_type, COUNT(*)
           FROM recording_session_events
          GROUP BY event_type
          ORDER BY event_type",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        events,
        vec![
            ("fragment_close".to_string(), 1),
            ("fragment_open".to_string(), 2),
        ]
    );

    // Nullable link remains accepted for old draining writers during phase one.
    sqlx::query(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month)
         VALUES ('rolling-old-writer', 1, 10, 100, 2026, 7)",
    )
    .execute(&pool)
    .await?;
    let nullable_link: Option<i64> = sqlx::query_scalar(
        "SELECT recording_session_id
           FROM audio_files
          WHERE file_name = 'rolling-old-writer'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(nullable_link, None);

    let (fk_validated, index_exists): (bool, bool) = sqlx::query_as(
        "SELECT
            (SELECT convalidated
               FROM pg_constraint
              WHERE conname = 'audio_files_recording_session_id_fkey'),
            to_regclass('public.audio_files_session_segment_idx') IS NOT NULL",
    )
    .fetch_one(&pool)
    .await?;
    assert!(!fk_validated);
    assert!(index_exists);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn stamp_session_migration_backfills_and_preserves_logical_session(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let prior = Migrator::with_migrations(
        FULL_MIGRATOR
            .iter()
            .filter(|migration| migration.version < STAMP_RECORDING_SESSIONS_MIGRATION)
            .cloned()
            .collect(),
    );
    prior.run(&pool).await?;

    let session_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, current_channel_id, state,
             started_at, ended_at, end_reason, last_segment_index)
         VALUES (1, 100, 10, 10, 'finalized',
                 to_timestamp(1), to_timestamp(2), 'test', 0)
         RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    let audio_file_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month,
             start_ts, end_ts, recording_session_id, segment_index)
         VALUES ('stamp-backfill', 1, 10, 100, 1970, 1,
                 1000, 2000, $1, 0)
         RETURNING id",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await?;
    let stamp_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO stamps
            (guild_id, channel_id, target_user_id, stamper_user_id,
             stamp_ts, audio_file_id)
         VALUES (1, 10, 100, 101, 1500, $1)
         RETURNING id",
    )
    .bind(audio_file_id)
    .fetch_one(&pool)
    .await?;

    FULL_MIGRATOR.run(&pool).await?;

    let migrated_session_id: Option<i64> = sqlx::query_scalar(
        "SELECT recording_session_id
           FROM stamps
          WHERE id = $1",
    )
    .bind(stamp_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(migrated_session_id, Some(session_id));

    sqlx::query("DELETE FROM audio_files WHERE id = $1")
        .bind(audio_file_id)
        .execute(&pool)
        .await?;
    let links: (Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT audio_file_id, recording_session_id
           FROM stamps
          WHERE id = $1",
    )
    .bind(stamp_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(links, (None, Some(session_id)));

    let (foreign_key_exists, index_exists): (bool, bool) = sqlx::query_as(
        "SELECT
            EXISTS (
                SELECT 1
                  FROM pg_constraint
                 WHERE conname = 'stamps_recording_session_id_fkey'
            ),
            to_regclass('public.stamps_by_recording_session') IS NOT NULL",
    )
    .fetch_one(&pool)
    .await?;
    assert!(foreign_key_exists);
    assert!(index_exists);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn media_archive_migration_backfills_finalized_recordings_and_saved_clips(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let prior = Migrator::with_migrations(
        FULL_MIGRATOR
            .iter()
            .filter(|migration| migration.version < MEDIA_ARCHIVE_MIGRATION)
            .cloned()
            .collect(),
    );
    prior.run(&pool).await?;
    sqlx::query(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month, start_ts, end_ts)
         VALUES
            ('archive-finalized', 1, 10, 100, 2026, 7, 1000, 2000),
            ('archive-active', 1, 10, 100, 2026, 7, 3000, NULL)",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO clips (clip_id, start_time, saved_file_name)
         VALUES
            ('archive-saved-clip', 0, '2026/07/archive-saved-clip.ogg'),
            ('archive-unsaved-clip', 0, NULL)",
    )
    .execute(&pool)
    .await?;

    FULL_MIGRATOR.run(&pool).await?;

    let sources: Vec<(Option<String>, Option<String>, String)> = sqlx::query_as(
        "SELECT af.file_name, mo.clip_id, mo.state
           FROM media_objects mo
           LEFT JOIN audio_files af ON af.id = mo.audio_file_id
          ORDER BY COALESCE(af.file_name, mo.clip_id)",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        sources,
        vec![
            (
                Some("archive-finalized".to_owned()),
                None,
                "pending".to_owned()
            ),
            (
                None,
                Some("archive-saved-clip".to_owned()),
                "pending".to_owned()
            ),
        ]
    );

    let duplicate =
        sqlx::query("INSERT INTO media_objects (clip_id) VALUES ('archive-saved-clip')")
            .execute(&pool)
            .await;
    assert!(duplicate.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn recording_finalize_reports_zero_row_mismatch(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let err = recordings::finalize_recording(
        &pool,
        -unique_id(),
        "missing-owner",
        1,
        recordings::FINALIZE_REASON_WRITER_CLOSE,
    )
    .await
    .expect_err("missing recording should report row mismatch");

    assert!(matches!(
        err,
        DbError::UnexpectedRows {
            operation: "finalize recording",
            expected: 1,
            actual: 0
        }
    ));
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn cooldown_db_failure_propagates(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    pool.close().await;

    let cooldown = JamCooldown::new();
    let result = cooldown.check_and_record(&pool, 1, 2).await;
    assert!(result.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn db_constraints_reject_negative_cooldown(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let guild_id = unique_id();

    let negative_cooldown = sqlx::query!(
        "INSERT INTO guild_jam_cooldowns (guild_id, cooldown_seconds)
         VALUES ($1, -1)",
        guild_id
    )
    .execute(&pool)
    .await;
    assert!(negative_cooldown.is_err());

    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn guild_cache_accepts_unknown_discord_channel_types(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let guild_id = unique_id();
    let channel_id = guild_id + 1;

    sqlx::query("INSERT INTO guilds (id, owner_id) VALUES ($1, $2)")
        .bind(guild_id)
        .bind(guild_id)
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         VALUES ($1, $2, 255, 'future-channel-type')",
    )
    .bind(channel_id)
    .bind(guild_id)
    .execute(&pool)
    .await?;

    let channel_type =
        sqlx::query_scalar::<_, i32>("SELECT type FROM channels WHERE channel_id = $1")
            .bind(channel_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(channel_type, 255);

    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn guild_cache_prune_removes_stale_roles_channels_and_dependents(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let guild_id = unique_id();
    let owner_id = guild_id + 1;
    let keep_role = guild_id + 10;
    let stale_role = guild_id + 11;
    let keep_channel = guild_id + 20;
    let stale_channel = guild_id + 21;
    let user_id = guild_id + 30;

    sqlx::query!(
        "INSERT INTO guilds (id, owner_id) VALUES ($1, $2)",
        guild_id,
        owner_id
    )
    .execute(&pool)
    .await?;
    sqlx::query!(
        "INSERT INTO channel_type (id, type)
         VALUES (2, 'voice')
         ON CONFLICT (id) DO NOTHING"
    )
    .execute(&pool)
    .await?;
    sqlx::query!(
        "INSERT INTO roles (guild_id, role_id, permission, name)
         VALUES ($1, $2, 0, 'keep'), ($1, $3, 0, 'stale')",
        guild_id,
        keep_role,
        stale_role
    )
    .execute(&pool)
    .await?;
    sqlx::query!(
        "INSERT INTO user_roles (user_id, role_id)
         VALUES ($1, $2), ($1, $3)",
        user_id,
        keep_role,
        stale_role
    )
    .execute(&pool)
    .await?;
    sqlx::query!(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         VALUES ($1, $2, 2, 'keep'), ($3, $2, 2, 'stale')",
        keep_channel,
        guild_id,
        stale_channel
    )
    .execute(&pool)
    .await?;
    sqlx::query!(
        "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny)
         VALUES ($1, $3, 'role', 0, 0), ($2, $3, 'role', 0, 0)",
        keep_channel,
        stale_channel,
        keep_role
    )
    .execute(&pool)
    .await?;

    crate::database::guild_cache::prune_stale_roles_for_test(&pool, guild_id, &[keep_role]).await?;
    crate::database::guild_cache::prune_stale_channels_for_test(&pool, guild_id, &[keep_channel])
        .await?;

    let stale_roles =
        sqlx::query_scalar!("SELECT COUNT(*) FROM roles WHERE role_id = $1", stale_role)
            .fetch_one(&pool)
            .await?
            .unwrap_or(0);
    let keep_roles =
        sqlx::query_scalar!("SELECT COUNT(*) FROM roles WHERE role_id = $1", keep_role)
            .fetch_one(&pool)
            .await?
            .unwrap_or(0);
    let stale_user_roles = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM user_roles WHERE role_id = $1",
        stale_role
    )
    .fetch_one(&pool)
    .await?
    .unwrap_or(0);
    let keep_user_roles = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM user_roles WHERE role_id = $1",
        keep_role
    )
    .fetch_one(&pool)
    .await?
    .unwrap_or(0);
    let stale_channels = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM channels WHERE channel_id = $1",
        stale_channel
    )
    .fetch_one(&pool)
    .await?
    .unwrap_or(0);
    let keep_channels = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM channels WHERE channel_id = $1",
        keep_channel
    )
    .fetch_one(&pool)
    .await?
    .unwrap_or(0);
    let stale_permissions = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM channel_permissions WHERE channel_id = $1",
        stale_channel
    )
    .fetch_one(&pool)
    .await?
    .unwrap_or(0);
    let keep_permissions = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM channel_permissions WHERE channel_id = $1",
        keep_channel
    )
    .fetch_one(&pool)
    .await?
    .unwrap_or(0);

    assert_eq!(stale_roles, 0);
    assert_eq!(keep_roles, 1);
    assert_eq!(stale_user_roles, 0);
    assert_eq!(keep_user_roles, 1);
    assert_eq!(stale_channels, 0);
    assert_eq!(keep_channels, 1);
    assert_eq!(stale_permissions, 0);
    assert_eq!(keep_permissions, 1);

    sqlx::query!("DELETE FROM guilds WHERE id = $1", guild_id)
        .execute(&pool)
        .await?;

    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn local_disconnect_releases_only_current_owner_lease(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let base = unique_id();
    let own_guild = GuildId::new(base as u64);
    let other_guild = GuildId::new((base + 1) as u64);
    let own_runtime = test_runtime(format!("test-local-disconnect-{base}"));
    let other_runtime = test_runtime(format!("test-other-owner-{base}"));

    crate::deployment::upsert_instance(&pool, &own_runtime).await?;
    crate::deployment::upsert_instance(&pool, &other_runtime).await?;
    crate::deployment::claim_voice_session(
        &pool,
        &own_runtime,
        own_guild,
        ChannelId::new((base + 10) as u64),
    )
    .await?;
    crate::deployment::claim_voice_session(
        &pool,
        &other_runtime,
        other_guild,
        ChannelId::new((base + 11) as u64),
    )
    .await?;

    let data = Arc::new(RwLock::new(TypeMap::new()));
    {
        let mut data_write = data.write().await;
        data_write.insert::<songbird::SongbirdKey>(songbird::Songbird::serenity());
        data_write.insert::<crate::runtime::RuntimeStateKey>(own_runtime.clone());
    }

    let report = crate::events::voice::teardown_voice_session(&data, &pool, own_guild).await;
    assert!(!report.had_call);
    assert!(!report.connected_after);

    let own_leases = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM voice_session_leases
          WHERE guild_id = $1 AND owner_instance_id = $2",
    )
    .bind(own_guild.get() as i64)
    .bind(&own_runtime.config().instance_id)
    .fetch_one(&pool)
    .await?;
    let other_leases = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM voice_session_leases
          WHERE guild_id = $1 AND owner_instance_id = $2",
    )
    .bind(other_guild.get() as i64)
    .bind(&other_runtime.config().instance_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(own_leases, 0);
    assert_eq!(other_leases, 1);
    Ok(())
}
