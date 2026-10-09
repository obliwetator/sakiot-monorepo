use super::*;
use sqlx::PgPool;

fn request(session_id: i64) -> MediaJobRequest {
    MediaJobRequest::SessionDownload {
        session_id,
        start: None,
        end: None,
        remove_silence: false,
    }
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn admission_is_idempotent_payload_bound_and_shared_by_resource(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let first = enqueue(
        &pool,
        None,
        crate::permissions::Viewer::discord(10),
        "key",
        "resource",
        &request(1),
    )
    .await?;
    assert_eq!(
        enqueue(
            &pool,
            None,
            crate::permissions::Viewer::discord(10),
            "key",
            "resource",
            &request(1)
        )
        .await?
        .id,
        first.id
    );
    assert!(matches!(
        enqueue(
            &pool,
            None,
            crate::permissions::Viewer::discord(10),
            "key",
            "other",
            &request(2)
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let shared = enqueue(
        &pool,
        None,
        crate::permissions::Viewer::discord(11),
        "another",
        "resource",
        &request(1),
    )
    .await?;
    assert_eq!(shared.id, first.id);
    assert_eq!(load_status(&pool, 11, &first.id).await?.id, first.id);
    assert!(matches!(
        enqueue(
            &pool,
            None,
            crate::permissions::Viewer::discord(12),
            "different",
            "resource",
            &request(2)
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn per_user_admission_is_bounded(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    for id in 1..=3 {
        enqueue(
            &pool,
            None,
            crate::permissions::Viewer::discord(10),
            &format!("key-{id}"),
            &format!("resource-{id}"),
            &request(id),
        )
        .await?;
    }
    assert!(matches!(
        enqueue(
            &pool,
            None,
            crate::permissions::Viewer::discord(10),
            "key-4",
            "resource-4",
            &request(4)
        )
        .await,
        Err(AppError::UserJobLimitReached)
    ));
    Ok(())
}

fn recording_waveform(n: i64) -> MediaJobRequest {
    MediaJobRequest::RecordingWaveform {
        guild_id: 1,
        channel_id: 2,
        year: 2026,
        month: 10,
        file_name: format!("recording-{n}"),
        silence_free: false,
    }
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn recording_waveforms_are_not_metered_per_user(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let viewer = crate::permissions::Viewer::discord(10);
    // A cold channel mix: one waveform per source, more than the limit.
    for n in 1..=PER_USER_ACTIVE_LIMIT + 3 {
        enqueue(
            &pool,
            None,
            viewer,
            &format!("waveform-{n}"),
            &format!("waveform-resource-{n}"),
            &recording_waveform(n),
        )
        .await?;
    }
    // They leave the user's own limit to the jobs the user asked for.
    for id in 1..=PER_USER_ACTIVE_LIMIT {
        enqueue(
            &pool,
            None,
            viewer,
            &format!("key-{id}"),
            &format!("resource-{id}"),
            &request(id),
        )
        .await?;
    }
    assert!(matches!(
        enqueue(&pool, None, viewer, "key-4", "resource-4", &request(4)).await,
        Err(AppError::UserJobLimitReached)
    ));
    // And a user at that limit can still see a mix's waveforms.
    enqueue(
        &pool,
        None,
        viewer,
        "waveform-late",
        "waveform-resource-late",
        &recording_waveform(99),
    )
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn claims_are_globally_bounded_and_expired_attempts_are_fenced(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    for id in 1..=5 {
        enqueue(
            &pool,
            None,
            crate::permissions::Viewer::discord(id),
            &format!("key-{id}"),
            &format!("resource-{id}"),
            &request(id),
        )
        .await?;
    }
    let mut claimed = Vec::new();
    for _ in 0..5 {
        if let Some(job) = claim(&pool).await? {
            claimed.push(job);
        }
    }
    assert_eq!(claimed.len(), GLOBAL_RUNNING_LIMIT as usize);
    let old = &claimed[0];
    sqlx::query("UPDATE media_jobs SET lease_expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(&old.id)
        .execute(&pool)
        .await?;
    let recovered = claim(&pool).await?.ok_or("expired job was not recovered")?;
    assert_eq!(recovered.id, old.id);
    assert_ne!(recovered.token, old.token);
    assert!(!report_progress(&pool, &old.id, &old.token, "late", 90).await?);
    assert!(report_progress(&pool, &recovered.id, &recovered.token, "rendering", 50).await?);
    assert!(begin_publication(&pool, &old.id, &old.token).await.is_err());
    let tx = begin_publication(&pool, &recovered.id, &recovered.token).await?;
    complete_publication(tx, &recovered.id, &recovered.token, "/result", None).await?;
    finish(&pool, old, Err(AppError::InternalError)).await?;
    assert_eq!(
        load_status(&pool, old.requester.user_id, &old.id)
            .await?
            .status,
        "ready"
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn lease_updates_skip_the_attempts_own_publication(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    enqueue(
        &pool,
        None,
        crate::permissions::Viewer::discord(1),
        "key-1",
        "resource-1",
        &request(1),
    )
    .await?;
    let job = claim(&pool).await?.ok_or("job was not claimed")?;
    sqlx::query("UPDATE media_jobs SET lease_expires_at=now()+interval '5 seconds' WHERE id=$1")
        .bind(&job.id)
        .execute(&pool)
        .await?;
    assert!(renew(&pool, &job).await?);
    let left: f64 = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM lease_expires_at-now())::float8 FROM media_jobs WHERE id=$1",
    )
    .bind(&job.id)
    .fetch_one(&pool)
    .await?;
    assert!(left > 50.0, "the lease was not extended: {left}s left");

    // The attempt's own loops renew and report while its publication
    // holds the row. Waiting for that lock deadlocked them.
    let tx = begin_publication(&pool, &job.id, &job.token).await?;
    let within = Duration::from_secs(5);
    assert!(tokio::time::timeout(within, renew(&pool, &job)).await??);
    assert!(
        tokio::time::timeout(
            within,
            report_progress(&pool, &job.id, &job.token, "rendering", 50)
        )
        .await??
    );
    complete_publication(tx, &job.id, &job.token, "/result", None).await?;
    assert!(!renew(&pool, &job).await?);
    assert!(!report_progress(&pool, &job.id, &job.token, "late", 90).await?);
    Ok(())
}

const INTERNAL: &str =
    "Database Error: relation media_jobs at /srv/sakiot/data/.media-jobs stderr: libopus";

fn assert_safe(status: &MediaJobStatus) {
    let body = serde_json::to_string(status).unwrap();
    for secret in ["Database", "/srv", "stderr", "libopus"] {
        assert!(!body.contains(secret), "{secret} leaked: {body}");
    }
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn failures_report_a_safe_kind_and_terminal_failures_do_not_promise_retry(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let queued = enqueue(
        &pool,
        None,
        crate::permissions::Viewer::discord(10),
        "key",
        "resource",
        &request(1),
    )
    .await?;
    for attempt in 1..=MAX_ATTEMPTS {
        let job = claim(&pool).await?.ok_or("job was not claimed")?;
        finish(&pool, &job, Err(AppError::FfmpegError(INTERNAL.into()))).await?;
        let status = load_status(&pool, 10, &queued.id).await?;
        assert_safe(&status);
        assert_eq!(status.error_kind, Some(ErrorKind::MediaProcessingFailed));
        assert_eq!(
            status.error.as_deref(),
            Some(ErrorKind::MediaProcessingFailed.default_message())
        );
        let expected = if attempt < MAX_ATTEMPTS {
            "queued"
        } else {
            "failed"
        };
        assert_eq!(status.status, expected);
        assert!(
            !status
                .error
                .unwrap_or_default()
                .to_lowercase()
                .contains("retry")
        );
        sqlx::query("UPDATE media_jobs SET retry_at=now()")
            .execute(&pool)
            .await?;
    }
    // Old releases read the raw column; it holds the public text too.
    let stored: Option<String> = sqlx::query_scalar("SELECT error FROM media_jobs WHERE id=$1")
        .bind(&queued.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        stored.as_deref(),
        Some(ErrorKind::MediaProcessingFailed.default_message())
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn legacy_error_text_is_never_returned(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let queued = enqueue(
        &pool,
        None,
        crate::permissions::Viewer::discord(10),
        "key",
        "resource",
        &request(1),
    )
    .await?;
    sqlx::query("UPDATE media_jobs SET state='failed', stage='failed', error=$2, error_kind=NULL WHERE id=$1")
        .bind(&queued.id)
        .bind(INTERNAL)
        .execute(&pool)
        .await?;
    let status = load_status(&pool, 10, &queued.id).await?;
    assert_safe(&status);
    assert_eq!(status.error_kind, None);
    assert_eq!(
        status.error.as_deref(),
        Some(crate::errors::UNCLASSIFIED_JOB_ERROR)
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn a_lost_lease_writes_nothing(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let queued = enqueue(
        &pool,
        None,
        crate::permissions::Viewer::discord(10),
        "key",
        "resource",
        &request(1),
    )
    .await?;
    let job = claim(&pool).await?.ok_or("job was not claimed")?;
    finish(&pool, &job, Err(AppError::JobLeaseLost)).await?;
    let status = load_status(&pool, 10, &queued.id).await?;
    assert_eq!((status.status.as_str(), status.error), ("running", None));
    Ok(())
}
