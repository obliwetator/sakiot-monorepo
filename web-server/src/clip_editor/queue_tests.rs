use super::{queue::*, *};
use crate::errors::ErrorKind;
use sqlx::PgPool;
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn snapshot() -> Snapshot {
    Snapshot {
        body: tests::body(vec![tests::segment()]),
        sources: vec!["source.ogg".into()],
        overwrite: None,
        channel_id: 10,
        name: "Export".into(),
    }
}
async fn submit(pool: &PgPool, key: &str, snapshot: &Snapshot) -> Result<String, AppError> {
    enqueue(
        pool,
        1,
        crate::permissions::Viewer::discord(100),
        key,
        &serde_json::to_value(&snapshot.body).unwrap(),
        snapshot,
    )
    .await
}
async fn claimed(pool: &PgPool) -> Job {
    let (id, token) = claim(pool).await.unwrap().unwrap();
    load(pool, &id, &token).await.unwrap()
}
async fn target(pool: &PgPool) -> Snapshot {
    sqlx::query("INSERT INTO clips (clip_id, guild_id, user_id, start_time, saved_file_name, original_file_name) VALUES ('target',1,100,0,'old.ogg','compose')").execute(pool).await.unwrap();
    let mut snapshot = snapshot();
    snapshot.body.overwrite_clip_id = Some("target".into());
    snapshot.overwrite = Some(ComposeOverwrite {
        clip_id: "target".into(),
        old_saved_file_name: "old.ogg".into(),
        fallback_name: None,
    });
    snapshot
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn concurrent_submissions_are_idempotent_and_payload_bound(pool: PgPool) -> TestResult {
    let snapshot = snapshot();
    let (a, b) = tokio::join!(
        submit(&pool, "same-key", &snapshot),
        submit(&pool, "same-key", &snapshot)
    );
    assert_eq!(a?, b?);
    let mut changed = snapshot.clone();
    changed.body.name = Some("different".into());
    assert!(matches!(
        submit(&pool, "same-key", &changed).await,
        Err(AppError::Conflict(_))
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM composition_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn queue_backpressure_keeps_idempotent_retries_available(pool: PgPool) -> TestResult {
    let snapshot = snapshot();
    let first = submit(&pool, "one", &snapshot).await?;
    submit(&pool, "two", &snapshot).await?;
    submit(&pool, "three", &snapshot).await?;
    assert!(matches!(
        submit(&pool, "four", &snapshot).await,
        Err(AppError::UserJobLimitReached)
    ));
    assert_eq!(submit(&pool, "one", &snapshot).await?, first);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn shared_capacity_is_reported_apart_from_the_personal_limit(pool: PgPool) -> TestResult {
    // Other users fill the shared queue; this user has nothing active.
    sqlx::query("INSERT INTO composition_jobs (id,guild_id,user_id,idempotency_key,request,snapshot,result_clip_id) SELECT 'other-' || n, 1, 1000 + n, 'key', '{}', '{}', 'clip-' || n FROM generate_series(1, 100) n")
        .execute(&pool)
        .await?;
    let error = submit(&pool, "blocked", &snapshot())
        .await
        .expect_err("the shared queue is full");
    assert!(matches!(error, AppError::ExportQueueFull));
    let response = actix_web::ResponseError::error_response(&error);
    assert_eq!(
        response.status(),
        actix_web::http::StatusCode::SERVICE_UNAVAILABLE
    );
    let body = actix_web::body::to_bytes(response.into_body()).await?;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body)?,
        serde_json::json!({
            "code": 503,
            "kind": "export_queue_full",
            "message": ErrorKind::ExportQueueFull.default_message(),
        })
    );

    // A caller at their own limit hears about that first: it is the part
    // they can resolve.
    sqlx::query("INSERT INTO media_jobs (id,kind,user_id,idempotency_key,resource_key,request) SELECT 'mine-' || n, 'session_download', 100, 'key-' || n, 'resource-' || n, '{}' FROM generate_series(1, 3) n")
        .execute(&pool)
        .await?;
    assert!(matches!(
        submit(&pool, "blocked", &snapshot()).await,
        Err(AppError::UserJobLimitReached)
    ));
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn claims_are_globally_bounded_and_old_attempt_cannot_publish(pool: PgPool) -> TestResult {
    let id = submit(&pool, "first", &snapshot()).await?;
    submit(&pool, "second", &snapshot()).await?;
    let (a, b) = tokio::join!(claim(&pool), claim(&pool));
    let claims: Vec<_> = [a?, b?].into_iter().flatten().collect();
    assert_eq!(claims.len(), 1);
    let old = load(&pool, &claims[0].0, &claims[0].1).await?;
    sqlx::query(
        "UPDATE composition_jobs SET lease_expires_at = now() - interval '1 second' WHERE id = $1",
    )
    .bind(&id)
    .execute(&pool)
    .await?;
    let recovered = claimed(&pool).await;
    assert_eq!(recovered.id, id);
    assert_ne!(recovered.token, old.token);
    assert!(!renew(&pool, &id, &old.token).await?);
    assert!(!report(&pool, &id, &old.token, "rendering", 80).await?);
    assert!(publish(&pool, &old, "stale.ogg", 1.0, 100).await.is_err());
    publish(&pool, &recovered, "recovered.ogg", 1.0, 100).await?;
    // A late error from the old attempt must not replace success.
    fail(&pool, &id, &old.token, ErrorKind::InternalError, true).await?;
    assert_eq!(status(&pool, 1, 100, &id).await?.status, "ready");
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn completion_and_failure_are_stable_and_owner_scoped(pool: PgPool) -> TestResult {
    let snapshot = target(&pool).await;
    let id = submit(&pool, "overwrite", &snapshot).await?;
    let job = claimed(&pool).await;
    publish(&pool, &job, "new.ogg", 1.0, 100).await?;
    for _ in 0..3 {
        let result = status(&pool, 1, 100, &id).await?;
        assert_eq!(result.status, "ready");
        assert_eq!(result.result_clip_id.as_deref(), Some("target"));
    }
    assert!(status(&pool, 2, 100, &id).await.is_err());
    assert!(status(&pool, 1, 101, &id).await.is_err());
    let failed_id = submit(&pool, "failed", &snapshot).await?;
    let job = claimed(&pool).await;
    fail(&pool, &job.id, &job.token, ErrorKind::SourceChanged, false).await?;
    for _ in 0..3 {
        let failed = status(&pool, 1, 100, &failed_id).await?;
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.error_kind, Some(ErrorKind::SourceChanged));
        assert_eq!(
            failed.error.as_deref(),
            Some(ErrorKind::SourceChanged.default_message())
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn retries_have_a_terminal_budget(pool: PgPool) -> TestResult {
    let id = submit(&pool, "retry", &snapshot()).await?;
    for attempt in 1..=MAX_ATTEMPTS {
        let job = claimed(&pool).await;
        fail(&pool, &id, &job.token, ErrorKind::ExecutionTimedOut, true).await?;
        let expected = if attempt < MAX_ATTEMPTS {
            ("queued", "retrying")
        } else {
            ("failed", "failed")
        };
        let result = status(&pool, 1, 100, &id).await?;
        assert_eq!((result.status.as_str(), result.stage.as_str()), expected);
        // The deadline is reported as such, never as a source-clip problem,
        // and the text does not promise a retry the queue may not make.
        assert_eq!(result.error_kind, Some(ErrorKind::ExecutionTimedOut));
        let message = result.error.unwrap_or_default();
        assert!(!message.to_lowercase().contains("retry"), "{message}");
        assert!(!message.to_lowercase().contains("source"), "{message}");
        sqlx::query("UPDATE composition_jobs SET retry_at = now()")
            .execute(&pool)
            .await?;
    }
    assert!(claim(&pool).await?.is_none());
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn repeatedly_crashed_workers_eventually_fail(pool: PgPool) -> TestResult {
    let id = submit(&pool, "crash", &snapshot()).await?;
    for _ in 0..MAX_ATTEMPTS {
        claimed(&pool).await;
        sqlx::query("UPDATE composition_jobs SET lease_expires_at = now() - interval '1 second'")
            .execute(&pool)
            .await?;
    }
    assert!(claim(&pool).await?.is_none());
    assert_eq!(status(&pool, 1, 100, &id).await?.status, "failed");
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn overwrite_and_archive_publication_roll_back_together(pool: PgPool) -> TestResult {
    let snapshot = target(&pool).await;
    sqlx::query("INSERT INTO media_objects (clip_id) VALUES ('target')")
        .execute(&pool)
        .await?;
    sqlx::raw_sql("CREATE FUNCTION reject_archive_update() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected archive failure'; END $$; CREATE TRIGGER reject_archive BEFORE UPDATE ON media_objects FOR EACH ROW EXECUTE FUNCTION reject_archive_update();").execute(&pool).await?;
    let id = submit(&pool, "atomic", &snapshot).await?;
    let job = claimed(&pool).await;
    assert!(publish(&pool, &job, "new.ogg", 1.0, 100).await.is_err());
    let saved: String =
        sqlx::query_scalar("SELECT saved_file_name FROM clips WHERE clip_id = 'target'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(saved, "old.ogg");
    assert_eq!(status(&pool, 1, 100, &id).await?.status, "running");
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn overwrite_fences_archive_worker_and_checks_revision(pool: PgPool) -> TestResult {
    let snapshot = target(&pool).await;
    sqlx::query("INSERT INTO media_objects (clip_id, state, lease_owner, lease_expires_at) VALUES ('target','uploading','old-upload',now() + interval '5 minutes')").execute(&pool).await?;
    let first = submit(&pool, "first", &snapshot).await?;
    let second = submit(&pool, "second", &snapshot).await?;
    let job = claimed(&pool).await;
    assert_eq!(job.id, first);
    publish(&pool, &job, "new.ogg", 1.0, 100).await?;
    let row: (String, String, Option<String>) = sqlx::query_as("SELECT state, clip_saved_file_name, lease_owner FROM media_objects WHERE clip_id = 'target'").fetch_one(&pool).await?;
    assert_eq!(row, ("pending".into(), "new.ogg".into(), None));
    let changed = sqlx::query("UPDATE media_objects SET last_error = 'stale upload' WHERE lease_owner = 'old-upload' AND state = 'uploading'").execute(&pool).await?;
    assert_eq!(changed.rows_affected(), 0);
    let stale = claimed(&pool).await;
    assert_eq!(stale.id, second);
    assert!(matches!(
        publish(&pool, &stale, "stale.ogg", 1.0, 100).await,
        Err(AppError::DestinationChanged)
    ));
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn deleted_overwrite_target_cannot_be_reported_ready(pool: PgPool) -> TestResult {
    let snapshot = target(&pool).await;
    let id = submit(&pool, "deleted", &snapshot).await?;
    let job = claimed(&pool).await;
    sqlx::query("UPDATE clips SET deleted_at = now() WHERE clip_id = 'target'")
        .execute(&pool)
        .await?;
    assert!(matches!(
        publish(&pool, &job, "new.ogg", 1.0, 100).await,
        Err(AppError::DestinationChanged)
    ));
    assert_ne!(status(&pool, 1, 100, &id).await?.status, "ready");
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn queued_v1_jobs_are_explicitly_migrated_to_shared_stereo_renderer(
    pool: PgPool,
) -> TestResult {
    let id = submit(&pool, "v1-queued", &snapshot()).await?;
    sqlx::query("UPDATE composition_jobs SET renderer_version = 1 WHERE id = $1")
        .bind(&id)
        .execute(&pool)
        .await?;
    let job = claimed(&pool).await;
    assert_eq!(job.id, id);
    let version: i32 =
        sqlx::query_scalar("SELECT renderer_version FROM composition_jobs WHERE id = $1")
            .bind(&id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(version, RENDERER_VERSION);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn active_v1_lease_is_preserved_then_expired_work_migrates_with_new_token(
    pool: PgPool,
) -> TestResult {
    let id = submit(&pool, "v1-running", &snapshot()).await?;
    let old = claimed(&pool).await;
    sqlx::query("UPDATE composition_jobs SET renderer_version = 1 WHERE id = $1")
        .bind(&id)
        .execute(&pool)
        .await?;
    assert!(claim(&pool).await?.is_none());
    let version: i32 =
        sqlx::query_scalar("SELECT renderer_version FROM composition_jobs WHERE id = $1")
            .bind(&id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(version, 1);
    sqlx::query(
        "UPDATE composition_jobs SET lease_expires_at = now() - interval '1 second' WHERE id = $1",
    )
    .bind(&id)
    .execute(&pool)
    .await?;
    let recovered = claimed(&pool).await;
    assert_eq!(recovered.id, id);
    assert_ne!(old.token, recovered.token);
    assert!(!renew(&pool, &id, &old.token).await?);
    assert!(
        publish(&pool, &old, "stale-v1.ogg", 1.0, 100)
            .await
            .is_err()
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn legacy_export_errors_are_not_echoed(pool: PgPool) -> TestResult {
    let id = submit(&pool, "legacy", &snapshot()).await?;
    sqlx::query("UPDATE composition_jobs SET state='failed', stage='failed', error='FFmpeg failed: /srv/sakiot/data/clips/x.ogg: Invalid data', error_kind=NULL WHERE id=$1")
        .bind(&id)
        .execute(&pool)
        .await?;
    let result = status(&pool, 1, 100, &id).await?;
    let body = serde_json::to_string(&result)?;
    assert!(!body.contains("/srv"), "{body}");
    assert!(!body.contains("FFmpeg"), "{body}");
    assert_eq!(result.error_kind, None);
    assert_eq!(
        result.error.as_deref(),
        Some(crate::errors::UNCLASSIFIED_JOB_ERROR)
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn waveform_builds_do_not_use_up_the_callers_share(pool: PgPool) -> TestResult {
    // A cold channel mix left more waveform builds running for this caller
    // than their own limit; their export is still admitted.
    sqlx::query("INSERT INTO media_jobs (id,kind,user_id,idempotency_key,resource_key,request) SELECT 'wave-' || n, 'recording_waveform', 100, 'wave-key-' || n, 'wave-resource-' || n, '{}' FROM generate_series(1, 5) n")
        .execute(&pool)
        .await?;
    submit(&pool, "export", &snapshot()).await?;
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn lease_updates_skip_the_attempts_own_publication(pool: PgPool) -> TestResult {
    submit(&pool, "export", &snapshot()).await?;
    let job = claimed(&pool).await;
    // Hold the row the way `jobs::publish` does until it commits. The worker
    // renews and reports from the loop that drives that publication, so
    // waiting for the lock deadlocked the attempt.
    let mut publication = pool.begin().await?;
    sqlx::query("SELECT id FROM composition_jobs WHERE id = $1 FOR UPDATE")
        .bind(&job.id)
        .execute(&mut *publication)
        .await?;
    let within = std::time::Duration::from_secs(5);
    assert!(tokio::time::timeout(within, renew(&pool, &job.id, &job.token)).await??);
    assert!(
        tokio::time::timeout(within, report(&pool, &job.id, &job.token, "rendering", 50)).await??
    );
    publication.rollback().await?;

    assert!(report(&pool, &job.id, &job.token, "rendering", 60).await?);
    let progress: i16 = sqlx::query_scalar("SELECT progress FROM composition_jobs WHERE id = $1")
        .bind(&job.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(progress, 60);
    publish(&pool, &job, "export.ogg", 1.0, 100).await?;
    assert!(!renew(&pool, &job.id, &job.token).await?);
    Ok(())
}
