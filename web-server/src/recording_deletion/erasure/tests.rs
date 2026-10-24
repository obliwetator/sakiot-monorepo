use std::time::Duration;

use sqlx::PgPool;

use super::*;
use crate::recording_deletion::worker::{Subject, claim, enqueue_expired};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const GUILD: i64 = 1;
const SPEAKER: i64 = 100;
const MANAGER: i64 = 200;
const OTHER: i64 = 300;

/// A finalized session of `user_id`'s voice, ended 39 days ago, with one
/// fragment.
async fn seed_session(pool: &PgPool, user_id: i64, file_name: &str) -> Result<i64, sqlx::Error> {
    let session_id: i64 = sqlx::query_scalar(
        "INSERT INTO recording_sessions (guild_id,user_id,starting_channel_id,state,started_at,ended_at)
         VALUES ($1,$2,10,'finalized',now()-interval '40 days',now()-interval '39 days') RETURNING id",
    )
    .bind(GUILD)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "INSERT INTO audio_files (file_name,guild_id,channel_id,user_id,year,month,start_ts,end_ts,recording_session_id,segment_index)
         VALUES ($1,$2,10,$3,2026,9,1000,2000,$4,0)",
    )
    .bind(file_name)
    .bind(GUILD)
    .bind(user_id)
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(session_id)
}

async fn seed_session_clip(
    pool: &PgPool,
    clip_id: &str,
    session_id: i64,
    speaker: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO clips (clip_id,guild_id,channel_id,channel_ids,speaker_ids,user_id,start_time,original_file_name,saved_file_name,recording_session_id)
         VALUES ($1,$2,10,'{10}',ARRAY[$3::bigint],$4,0,'session:1','2026/09/' || $1 || '.ogg',$5)",
    )
    .bind(clip_id)
    .bind(GUILD)
    .bind(speaker)
    .bind(OTHER)
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// A clip composed of `sources`, made by someone else.
async fn seed_composition(pool: &PgPool, clip_id: &str, sources: &[&str]) -> TestResult {
    let segments: Vec<_> = sources
        .iter()
        .map(|source| serde_json::json!({ "source_id": source }))
        .collect();
    sqlx::query(
        "INSERT INTO clips (clip_id,guild_id,channel_id,channel_ids,speaker_ids,user_id,start_time,original_file_name,saved_file_name,composition)
         VALUES ($1,$2,10,'{10}',ARRAY[$3::bigint,$4::bigint],$4,0,'compose','compositions/' || $1 || '.ogg',$5)",
    )
    .bind(clip_id)
    .bind(GUILD)
    .bind(SPEAKER)
    .bind(OTHER)
    .bind(serde_json::json!({ "segments": segments }))
    .execute(pool)
    .await?;
    Ok(())
}

async fn hidden_clips(pool: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT clip_id FROM clips WHERE guild_id=$1 AND deleted_at IS NOT NULL ORDER BY clip_id",
    )
    .bind(GUILD)
    .fetch_all(pool)
    .await
}

async fn removed(pool: &PgPool, session_id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT deletion_requested_at IS NOT NULL FROM recording_sessions WHERE id=$1",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await
}

async fn erasure_state(pool: &PgPool, id: &str) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT state FROM recording_erasures WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
}

async fn make_due(pool: &PgPool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE recording_erasures SET purge_at=now()-interval '1 second' WHERE id=$1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// A job's (reason, requested_by, mode, permanent_requested_by).
async fn job_audit(
    pool: &PgPool,
    session_id: i64,
) -> Result<(String, Option<i64>, String, Option<i64>), sqlx::Error> {
    sqlx::query_as(
        "SELECT reason,requested_by,mode,permanent_requested_by FROM recording_deletion_jobs WHERE recording_session_id=$1",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn an_erasure_hides_what_was_visible_and_undo_reverts_exactly_that(
    pool: PgPool,
) -> TestResult {
    let session = seed_session(&pool, SPEAKER, "erased-recording").await?;
    let other_session = seed_session(&pool, OTHER, "other-recording").await?;
    seed_session_clip(&pool, "own-cut", session, SPEAKER).await?;
    seed_session_clip(&pool, "already-gone", session, SPEAKER).await?;
    seed_session_clip(&pool, "other-cut", other_session, OTHER).await?;
    seed_composition(&pool, "mix", &["own-cut", "other-cut"]).await?;
    sqlx::query("UPDATE clips SET deleted_at=now() WHERE clip_id='already-gone'")
        .execute(&pool)
        .await?;

    assert!(matches!(
        erase_session(&pool, GUILD, session, OTHER).await,
        Err(AppError::Forbidden)
    ));
    let id = erase_session(&pool, GUILD, session, SPEAKER).await?;
    assert_eq!(erase_session(&pool, GUILD, session, SPEAKER).await?, id);
    assert!(removed(&pool, session).await?);
    assert_eq!(
        hidden_clips(&pool).await?,
        ["already-gone", "mix", "own-cut"]
    );
    let (hid_session, mut hidden): (bool, Vec<String>) =
        sqlx::query_as("SELECT hid_session, hidden_clip_ids FROM recording_erasures WHERE id=$1")
            .bind(&id)
            .fetch_one(&pool)
            .await?;
    hidden.sort();
    assert!(hid_session);
    assert_eq!(hidden, ["mix", "own-cut"]);
    let status = load_erasure(&pool, GUILD, &id, SPEAKER).await?;
    assert_eq!(status.state, "grace");
    assert_eq!(
        status.purge_at_ms - status.requested_at_ms,
        i64::from(GRACE_DAYS) * 24 * 60 * 60 * 1000
    );
    assert!(matches!(
        load_erasure(&pool, GUILD, &id, OTHER).await,
        Err(AppError::NotFound)
    ));

    assert!(matches!(
        undo(&pool, GUILD, &id, OTHER).await,
        Err(AppError::NotFound)
    ));
    undo(&pool, GUILD, &id, SPEAKER).await?;
    assert!(!removed(&pool, session).await?);
    // The clip deleted before the request stays deleted.
    assert_eq!(hidden_clips(&pool).await?, ["already-gone"]);
    assert_eq!(erasure_state(&pool, &id).await?, "cancelled");
    // Undoing again changes nothing.
    undo(&pool, GUILD, &id, SPEAKER).await?;
    assert_eq!(hidden_clips(&pool).await?, ["already-gone"]);
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM recording_deletion_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(jobs, 0, "an erasure in its grace period has no job");

    // A new request opens a new erasure; the undone one stays on record.
    let again = erase_session(&pool, GUILD, session, SPEAKER).await?;
    assert_ne!(again, id);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn undoing_never_restores_what_the_server_removed(pool: PgPool) -> TestResult {
    // Removed by a manager, then erased, then undone.
    let session = seed_session(&pool, SPEAKER, "removed-first").await?;
    seed_session_clip(&pool, "removed-first-cut", session, SPEAKER).await?;
    let job = enqueue(
        &pool,
        GUILD,
        session,
        Some(MANAGER),
        "manager",
        DeletionMode::Soft,
    )
    .await?;
    let id = erase_session(&pool, GUILD, session, SPEAKER).await?;
    let hid: (bool, Vec<String>) =
        sqlx::query_as("SELECT hid_session, hidden_clip_ids FROM recording_erasures WHERE id=$1")
            .bind(&id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(hid, (false, vec![]));
    undo(&pool, GUILD, &id, SPEAKER).await?;
    assert!(removed(&pool, session).await?);
    assert_eq!(hidden_clips(&pool).await?, ["removed-first-cut"]);
    assert_eq!(
        job_audit(&pool, session).await?,
        ("manager".into(), Some(MANAGER), "soft".into(), None)
    );
    let job_id: String =
        sqlx::query_scalar("SELECT id FROM recording_deletion_jobs WHERE recording_session_id=$1")
            .bind(session)
            .fetch_one(&pool)
            .await?;
    assert_eq!(job_id, job);

    // Erased, then removed by a manager meanwhile, then undone.
    let session = seed_session(&pool, SPEAKER, "removed-meanwhile").await?;
    seed_session_clip(&pool, "removed-meanwhile-cut", session, SPEAKER).await?;
    let id = erase_session(&pool, GUILD, session, SPEAKER).await?;
    enqueue(
        &pool,
        GUILD,
        session,
        Some(MANAGER),
        "manager",
        DeletionMode::Soft,
    )
    .await?;
    undo(&pool, GUILD, &id, SPEAKER).await?;
    assert!(removed(&pool, session).await?);
    // Its clip stays hidden with the session it came from.
    assert_eq!(
        hidden_clips(&pool).await?,
        ["removed-first-cut", "removed-meanwhile-cut"]
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn erasing_frees_keeps_and_undo_leaves_the_clips_to_the_server_s_removal(
    pool: PgPool,
) -> TestResult {
    let session = seed_session(&pool, SPEAKER, "kept-recording").await?;
    seed_session_clip(&pool, "kept-cut", session, SPEAKER).await?;
    sqlx::query(
        "INSERT INTO kept_recordings (recording_session_id,user_id,guild_id) VALUES ($1,$2,$3)",
    )
    .bind(session)
    .bind(OTHER)
    .bind(GUILD)
    .execute(&pool)
    .await?;
    let id = erase_session(&pool, GUILD, session, SPEAKER).await?;
    let keeps: i64 = sqlx::query_scalar("SELECT count(*) FROM kept_recordings")
        .fetch_one(&pool)
        .await?;
    assert_eq!(keeps, 0, "an erased recording is nobody's to keep");

    // Removed by a manager meanwhile: its clip was already hidden.
    enqueue(
        &pool,
        GUILD,
        session,
        Some(MANAGER),
        "manager",
        DeletionMode::Soft,
    )
    .await?;
    undo(&pool, GUILD, &id, SPEAKER).await?;
    assert_eq!(hidden_clips(&pool).await?, ["kept-cut"]);
    // Restoring the session now shows it again.
    let mut tx = pool.begin().await?;
    lock_guild(&mut tx, GUILD).await?;
    let lifecycle = load_lifecycle(&mut tx, GUILD, session).await?;
    restore_in(
        &mut tx,
        GUILD,
        session,
        lifecycle.restorable()?,
        MANAGER,
        false,
    )
    .await?;
    tx.commit().await?;
    assert!(!removed(&pool, session).await?);
    assert!(hidden_clips(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn undo_after_retention_would_have_expired_leaves_it_to_the_next_sweep(
    pool: PgPool,
) -> TestResult {
    let session = seed_session(&pool, SPEAKER, "expiring-recording").await?;
    let id = erase_session(&pool, GUILD, session, SPEAKER).await?;
    sqlx::query("INSERT INTO guild_recording_policy (guild_id,retention_days) VALUES ($1,30)")
        .bind(GUILD)
        .execute(&pool)
        .await?;
    // Already hidden: retention skips it.
    enqueue_expired(&pool).await?;
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM recording_deletion_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(jobs, 0);

    undo(&pool, GUILD, &id, SPEAKER).await?;
    assert!(!removed(&pool, session).await?);
    enqueue_expired(&pool).await?;
    assert!(removed(&pool, session).await?);
    assert_eq!(
        job_audit(&pool, session).await?,
        ("retention".into(), None, "soft".into(), None)
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn the_sweep_hands_a_due_erasure_to_the_purge_keeping_both_in_the_audit(
    pool: PgPool,
) -> TestResult {
    // Erased with nothing removed before: a job of the speaker's own.
    let erased = seed_session(&pool, SPEAKER, "erased").await?;
    let erasure = erase_session(&pool, GUILD, erased, SPEAKER).await?;
    convert_due_erasures(&pool).await?;
    assert_eq!(
        erasure_state(&pool, &erasure).await?,
        "grace",
        "not due yet"
    );
    make_due(&pool, &erasure).await?;
    convert_due_erasures(&pool).await?;
    assert_eq!(erasure_state(&pool, &erasure).await?, "converted");
    assert_eq!(
        job_audit(&pool, erased).await?,
        (
            "speaker".into(),
            Some(SPEAKER),
            "permanent".into(),
            Some(SPEAKER)
        )
    );
    let job: String =
        sqlx::query_scalar("SELECT deletion_job_id FROM recording_erasures WHERE id=$1")
            .bind(&erasure)
            .fetch_one(&pool)
            .await?;
    // A speaker's job is theirs alone: managers get no status of it.
    assert!(matches!(
        load_status(&pool, GUILD, &job).await,
        Err(AppError::NotFound)
    ));
    assert_eq!(
        load_erasure(&pool, GUILD, &erasure, SPEAKER)
            .await?
            .purge_state
            .as_deref(),
        Some("queued")
    );
    assert!(matches!(
        undo(&pool, GUILD, &erasure, SPEAKER).await,
        Err(AppError::Conflict(_))
    ));

    // Removed by a manager first: their job is upgraded in place, and keeps
    // its reason and requester.
    let removed_first = seed_session(&pool, SPEAKER, "removed-first").await?;
    let manager_job = enqueue(
        &pool,
        GUILD,
        removed_first,
        Some(MANAGER),
        "manager",
        DeletionMode::Soft,
    )
    .await?;
    let erasure = erase_session(&pool, GUILD, removed_first, SPEAKER).await?;
    make_due(&pool, &erasure).await?;
    convert_due_erasures(&pool).await?;
    assert_eq!(
        job_audit(&pool, removed_first).await?,
        (
            "manager".into(),
            Some(MANAGER),
            "permanent".into(),
            Some(SPEAKER)
        )
    );
    let converted: (String, Option<String>) =
        sqlx::query_as("SELECT state, deletion_job_id FROM recording_erasures WHERE id=$1")
            .bind(&erasure)
            .fetch_one(&pool)
            .await?;
    assert_eq!(converted, ("converted".into(), Some(manager_job)));

    // Once converted, erasing again answers with the same erasure.
    assert_eq!(
        erase_session(&pool, GUILD, removed_first, SPEAKER).await?,
        erasure
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn undo_racing_the_sweep_at_the_deadline_has_exactly_one_winner(pool: PgPool) -> TestResult {
    let session = seed_session(&pool, SPEAKER, "racing-recording").await?;
    let id = erase_session(&pool, GUILD, session, SPEAKER).await?;
    sqlx::query(
        "UPDATE recording_erasures SET purge_at=now()+interval '300 milliseconds' WHERE id=$1",
    )
    .bind(&id)
    .execute(&pool)
    .await?;
    // Both wait on the guild while the deadline passes: the undo, asked
    // before it, and the sweep, after.
    let mut held = pool.begin().await?;
    lock_guild(&mut held, GUILD).await?;
    let undoing = tokio::spawn({
        let pool = pool.clone();
        let id = id.clone();
        async move { undo(&pool, GUILD, &id, SPEAKER).await }
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    let converting = tokio::spawn({
        let pool = pool.clone();
        let id = id.clone();
        async move { convert(&pool, GUILD, &id).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    held.rollback().await?;
    let undone = undoing.await?;
    converting.await??;

    let state = erasure_state(&pool, &id).await?;
    let jobs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM recording_deletion_jobs WHERE recording_session_id=$1",
    )
    .bind(session)
    .fetch_one(&pool)
    .await?;
    match undone {
        Ok(()) => {
            assert_eq!(state, "cancelled");
            assert_eq!(jobs, 0);
            assert!(!removed(&pool, session).await?);
        }
        Err(AppError::Conflict(_)) => {
            assert_eq!(state, "converted");
            assert_eq!(jobs, 1);
            assert!(removed(&pool, session).await?);
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn an_erased_clip_is_purged_with_every_clip_composed_from_it(pool: PgPool) -> TestResult {
    let temp = tempfile::tempdir()?;
    let roots = DataRoots::new(temp.path());
    let session = seed_session(&pool, SPEAKER, "clip-source").await?;
    seed_session_clip(&pool, "erased-cut", session, SPEAKER).await?;
    seed_session_clip(&pool, "kept-cut", session, SPEAKER).await?;
    seed_composition(&pool, "built-on-it", &["erased-cut"]).await?;
    sqlx::query(
        "INSERT INTO media_objects (clip_id) VALUES ('erased-cut'),('built-on-it'),('kept-cut')",
    )
    .execute(&pool)
    .await?;
    let composition_job = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO composition_jobs (id,guild_id,user_id,idempotency_key,request,snapshot,result_clip_id,state) VALUES ($1,$2,$3,$1,'{}',$4,'built-on-it','ready')")
        .bind(&composition_job)
        .bind(GUILD)
        .bind(OTHER)
        .bind(serde_json::json!({"body":{"segments":[{"source_id":"erased-cut"}]}}))
        .execute(&pool)
        .await?;
    let files = [
        roots.clips.join("2026/09/erased-cut.ogg"),
        roots.clips.join("compositions/built-on-it.ogg"),
        roots.waveforms.join("clip-erased-cut-1.peaks"),
        roots.waveforms.join("clip-built-on-it-1.peaks"),
        roots
            .clips
            .join("compositions")
            .join(format!("{composition_job}-render.ogg")),
    ];
    let kept = roots.clips.join("2026/09/kept-cut.ogg");
    for path in files.iter().chain([&kept]) {
        tokio::fs::create_dir_all(path.parent().unwrap()).await?;
        tokio::fs::write(path, b"test-media").await?;
    }
    // As the request leaves it: the clip and what was built on it hidden.
    sqlx::query("UPDATE clips SET deleted_at=now() WHERE clip_id IN ('erased-cut','built-on-it')")
        .execute(&pool)
        .await?;
    let erasure = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO recording_erasures (id,guild_id,clip_id,speaker_id,purge_at,hidden_clip_ids)
         VALUES ($1,$2,'erased-cut',$3,now()-interval '1 second','{erased-cut,built-on-it}')",
    )
    .bind(&erasure)
    .bind(GUILD)
    .bind(SPEAKER)
    .execute(&pool)
    .await?;

    convert_due_erasures(&pool).await?;
    let job = claim(&pool).await?.unwrap();
    assert_eq!(job.subject, Subject::Clip("erased-cut".into()));
    process_with_roots(&pool, &MediaArchive::disabled(), &job, &roots).await?;

    for path in &files {
        assert!(!path.exists(), "{} survived", path.display());
    }
    assert!(kept.exists());
    let clips: Vec<String> =
        sqlx::query_scalar("SELECT clip_id FROM clips WHERE guild_id=$1 ORDER BY clip_id")
            .bind(GUILD)
            .fetch_all(&pool)
            .await?;
    assert_eq!(clips, ["kept-cut"]);
    let objects: Vec<String> = sqlx::query_scalar("SELECT clip_id FROM media_objects")
        .fetch_all(&pool)
        .await?;
    assert_eq!(objects, ["kept-cut"]);
    let composition_jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM composition_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(composition_jobs, 0);
    let status = load_erasure(&pool, GUILD, &erasure, SPEAKER).await?;
    assert_eq!(
        (status.state.as_str(), status.purge_state.as_deref()),
        ("converted", Some("ready"))
    );
    // The session it came from is not the clip's to take.
    assert!(!removed(&pool, session).await?);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn a_clip_already_purged_with_its_session_finishes_at_once(pool: PgPool) -> TestResult {
    let temp = tempfile::tempdir()?;
    let erasure = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO recording_erasures (id,guild_id,clip_id,speaker_id,purge_at)
         VALUES ($1,$2,'long-gone',$3,now()-interval '1 second')",
    )
    .bind(&erasure)
    .bind(GUILD)
    .bind(SPEAKER)
    .execute(&pool)
    .await?;
    convert_due_erasures(&pool).await?;
    let job = claim(&pool).await?.unwrap();
    process_with_roots(
        &pool,
        &MediaArchive::disabled(),
        &job,
        &DataRoots::new(temp.path()),
    )
    .await?;
    assert_eq!(
        load_erasure(&pool, GUILD, &erasure, SPEAKER)
            .await?
            .purge_state
            .as_deref(),
        Some("ready")
    );
    Ok(())
}
