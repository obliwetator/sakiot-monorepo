use super::*;
use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn seed_session(pool: &PgPool, ended: &str) -> Result<(i64, i64), sqlx::Error> {
    let session_id: i64 = sqlx::query_scalar("INSERT INTO recording_sessions (guild_id,user_id,starting_channel_id,state,started_at,ended_at) VALUES (1,100,10,'finalized',now()-interval '40 days',now()-interval '39 days') RETURNING id")
        .fetch_one(pool).await?;
    let audio_id: i64 = sqlx::query_scalar("INSERT INTO audio_files (file_name,guild_id,channel_id,user_id,year,month,start_ts,end_ts,recording_session_id,segment_index) VALUES ($1,1,10,100,2026,9,1000,2000,$2,0) RETURNING id")
        .bind(ended).bind(session_id).fetch_one(pool).await?;
    Ok((session_id, audio_id))
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn deletion_tombstones_then_removes_sources_derivatives_and_audit_survives(
    pool: PgPool,
) -> TestResult {
    let temp = tempfile::tempdir()?;
    let roots = DataRoots::new(temp.path());
    let (session_id, audio_id) = seed_session(&pool, "lifecycle-recording").await?;
    let (_, unrelated_id) = seed_session(&pool, "unrelated-recording").await?;
    sqlx::query("INSERT INTO clips (clip_id,guild_id,channel_id,user_id,start_time,original_file_name,saved_file_name,recording_session_id) VALUES ('lifecycle-source',1,10,100,0,'session:1','2026/09/lifecycle-source.ogg',$1)")
        .bind(session_id).execute(&pool).await?;
    sqlx::query("INSERT INTO clips (clip_id,guild_id,channel_id,user_id,start_time,original_file_name,saved_file_name,composition) VALUES ('lifecycle-compose',1,10,100,0,'compose','compositions/lifecycle-compose.ogg',$1)")
        .bind(serde_json::json!({"segments":[{"source_id":"lifecycle-source"}]})).execute(&pool).await?;
    sqlx::query("INSERT INTO media_objects (audio_file_id) VALUES ($1),($2)")
        .bind(audio_id)
        .bind(unrelated_id)
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO media_objects (clip_id) VALUES ('lifecycle-source'),('lifecycle-compose')",
    )
    .execute(&pool)
    .await?;
    sqlx::query("INSERT INTO stamps (guild_id,channel_id,target_user_id,stamper_user_id,stamp_ts,audio_file_id,recording_session_id) VALUES (1,10,100,200,1500,$1,$2)")
        .bind(audio_id).bind(session_id).execute(&pool).await?;
    let media_id = uuid::Uuid::new_v4().to_string();
    let download = roots
        .recordings
        .join(".media-jobs")
        .join(format!("{media_id}-attempt.ogg"));
    sqlx::query("INSERT INTO media_jobs (id,kind,guild_id,user_id,idempotency_key,resource_key,request,state,result_path) VALUES ($1,'session_download',1,100,$1,$1,$2,'ready',$3)")
        .bind(&media_id).bind(serde_json::json!({"kind":"session_download","session_id":session_id})).bind(download.to_string_lossy().as_ref()).execute(&pool).await?;
    let composition_id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO composition_jobs (id,guild_id,user_id,idempotency_key,request,snapshot,result_clip_id,state) VALUES ($1,1,100,$1,'{}',$2,'lifecycle-compose','ready')")
        .bind(&composition_id).bind(serde_json::json!({"body":{"segments":[{"source_id":"lifecycle-source"}]}})).execute(&pool).await?;
    // The current editor state no longer references this recording, but
    // its previous archived revisions still do.
    sqlx::query("UPDATE clips SET composition=$1 WHERE clip_id='lifecycle-compose'")
        .bind(serde_json::json!({"segments":[{"source_id":"other-source"}]}))
        .execute(&pool)
        .await?;

    let key = RecordingKey::new(1, 10, 2026, 9, "lifecycle-recording");
    let original = key.recording_path(&roots.recordings_str());
    let no_silence = key.no_silence_path(&roots.no_silence_str());
    let waveform = key.waveform_path(&roots.waveforms_str());
    let silence_attempt = no_silence.with_extension("job.token.tmp.ogg");
    let waveform_attempt = roots
        .waveforms
        .join("lifecycle-recording.job.token.tmp.dat");
    let session_waveform_attempt = roots
        .waveforms
        .join(format!("logical-session-{session_id}.job.token.tmp.dat"));
    let session_composite_attempt = roots
        .waveforms
        .join(format!("logical-session-{session_id}-token.ogg"));
    let clip = roots.clips.join("2026/09/lifecycle-source.ogg");
    let composed = roots.clips.join("compositions/lifecycle-compose.ogg");
    let render = roots
        .clips
        .join("compositions")
        .join(format!("{composition_id}-old.ogg"));
    let paths = [
        &original,
        &no_silence,
        &waveform,
        &silence_attempt,
        &waveform_attempt,
        &session_waveform_attempt,
        &session_composite_attempt,
        &clip,
        &composed,
        &download,
        &render,
    ];
    for path in paths {
        tokio::fs::create_dir_all(path.parent().unwrap()).await?;
        tokio::fs::write(path, b"test-media").await?;
    }
    let job_id = enqueue(
        &pool,
        1,
        session_id,
        Some(200),
        "manager",
        DeletionMode::Soft,
    )
    .await?;
    assert_eq!(
        job_id,
        enqueue(
            &pool,
            1,
            session_id,
            Some(200),
            "manager",
            DeletionMode::Soft
        )
        .await?
    );
    let hidden: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM clips WHERE guild_id=1 AND deleted_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(hidden, 2);
    let soft_status = load_status(&pool, 1, &job_id).await?;
    assert_eq!(soft_status.state, "soft_deleted");
    assert_eq!(soft_status.mode, "soft");
    assert!(claim(&pool).await?.is_none());
    for path in paths {
        assert!(path.exists(), "soft deletion removed {}", path.display());
    }
    let retained_files: i64 = sqlx::query_scalar("SELECT count(*) FROM audio_files WHERE id=$1")
        .bind(audio_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(retained_files, 1);
    let retained_archive_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM media_objects WHERE audio_file_id=$1 OR clip_id IN ('lifecycle-source','lifecycle-compose')",
    )
    .bind(audio_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(retained_archive_rows, 3);
    assert_eq!(
        job_id,
        enqueue(
            &pool,
            1,
            session_id,
            Some(200),
            "manager",
            DeletionMode::Permanent
        )
        .await?
    );
    let claim = claim(&pool).await?.unwrap();
    assert_eq!(claim.id, job_id);
    process_with_roots(&pool, &MediaArchive::disabled(), &claim, &roots).await?;
    for path in paths {
        assert!(!path.exists(), "{} survived", path.display());
    }
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM audio_files WHERE id=$1")
        .bind(audio_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(remaining, 0);
    let unrelated: i64 = sqlx::query_scalar("SELECT count(*) FROM audio_files WHERE id=$1")
        .bind(unrelated_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(unrelated, 1);
    let remaining_clips: i64 = sqlx::query_scalar("SELECT count(*) FROM clips WHERE guild_id=1")
        .fetch_one(&pool)
        .await?;
    assert_eq!(remaining_clips, 0);
    let stamps: i64 =
        sqlx::query_scalar("SELECT count(*) FROM stamps WHERE recording_session_id=$1")
            .bind(session_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(stamps, 0);
    let status = load_status(&pool, 1, &job_id).await?;
    assert_eq!(status.state, "ready");
    assert_eq!(status.mode, "permanent");
    assert_eq!(status.recording_session_id, session_id.to_string());
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn retention_and_fencing_prevent_late_publication(pool: PgPool) -> TestResult {
    let (session_id, _) = seed_session(&pool, "retention-recording").await?;
    sqlx::query("INSERT INTO guild_recording_policy (guild_id,retention_days) VALUES (1,30)")
        .execute(&pool)
        .await?;
    enqueue_expired(&pool).await?;
    let retention: (String, String) = sqlx::query_as(
        "SELECT mode,state FROM recording_deletion_jobs WHERE recording_session_id=$1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(retention, ("soft".into(), "soft_deleted".into()));
    assert!(claim(&pool).await?.is_none());
    enqueue(
        &pool,
        1,
        session_id,
        Some(200),
        "manager",
        DeletionMode::Permanent,
    )
    .await?;
    let authorization: (String, Option<i64>, Option<i64>, bool) = sqlx::query_as(
        "SELECT reason,requested_by,permanent_requested_by,permanent_requested_at IS NOT NULL FROM recording_deletion_jobs WHERE recording_session_id=$1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(authorization, ("retention".into(), None, Some(200), true));
    let first = claim(&pool).await?.unwrap();
    assert_eq!(first.session_id, session_id);
    sqlx::query(
        "UPDATE recording_deletion_jobs SET lease_expires_at=now()-interval '1 second' WHERE id=$1",
    )
    .bind(&first.id)
    .execute(&pool)
    .await?;
    let second = claim(&pool).await?.unwrap();
    assert_ne!(first.token, second.token);
    assert!(stage(&pool, &first, "stale").await.is_err());
    let temp = tempfile::tempdir()?;
    process_with_roots(
        &pool,
        &MediaArchive::disabled(),
        &second,
        &DataRoots::new(temp.path()),
    )
    .await?;
    assert_eq!(load_status(&pool, 1, &second.id).await?.state, "ready");
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn tombstone_rejects_late_clip_publication(pool: PgPool) -> TestResult {
    let (session_id, _) = seed_session(&pool, "late-clip-recording").await?;
    enqueue(
        &pool,
        1,
        session_id,
        Some(200),
        "manager",
        DeletionMode::Soft,
    )
    .await?;
    let insert = sqlx::query("INSERT INTO clips (clip_id,guild_id,channel_id,user_id,start_time,original_file_name,recording_session_id) VALUES ('late-clip',1,10,100,0,'session',$1)")
        .bind(session_id).execute(&pool).await;
    assert!(insert.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn database_rejects_a_soft_job_in_the_purge_queue(pool: PgPool) -> TestResult {
    let unsafe_job = sqlx::query("INSERT INTO recording_deletion_jobs (id,recording_session_id,guild_id,reason) VALUES ('unsafe-soft-job',1,1,'manager')")
        .execute(&pool).await;
    assert!(unsafe_job.is_err());
    assert!(claim(&pool).await?.is_none());
    Ok(())
}

async fn permanent_job(pool: &PgPool, file_name: &str) -> Result<String, AppError> {
    let (session_id, _) = seed_session(pool, file_name).await?;
    enqueue(
        pool,
        1,
        session_id,
        Some(200),
        "manager",
        DeletionMode::Permanent,
    )
    .await
}

async fn make_due(pool: &PgPool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE recording_deletion_jobs SET retry_at=now() WHERE id=$1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn waiting_for_media_work_never_spends_the_failure_budget(pool: PgPool) -> TestResult {
    let temp = tempfile::tempdir()?;
    let roots = DataRoots::new(temp.path());
    let job_id = permanent_job(&pool, "waiting-recording").await?;
    sqlx::query("INSERT INTO media_jobs (id,kind,guild_id,user_id,idempotency_key,resource_key,request) VALUES ('busy','session_download',1,100,'busy','busy','{}')")
        .execute(&pool)
        .await?;
    for _ in 0..MAX_ATTEMPTS + 3 {
        make_due(&pool, &job_id).await?;
        run_one(&pool, &MediaArchive::disabled(), &roots).await?;
    }
    let waiting = load_status(&pool, 1, &job_id).await?;
    assert_eq!(
        (
            waiting.state.as_str(),
            waiting.stage.as_str(),
            waiting.attempts
        ),
        ("queued", "waiting", 0)
    );
    assert_eq!(waiting.error_kind, Some(ErrorKind::WaitingForMediaWork));
    assert_eq!(
        waiting.error.as_deref(),
        Some(ErrorKind::WaitingForMediaWork.default_message())
    );

    // Once the guild's media work drains, the same job completes.
    sqlx::query("UPDATE media_jobs SET state='ready' WHERE id='busy'")
        .execute(&pool)
        .await?;
    make_due(&pool, &job_id).await?;
    run_one(&pool, &MediaArchive::disabled(), &roots).await?;
    let done = load_status(&pool, 1, &job_id).await?;
    assert_eq!((done.state.as_str(), done.attempts), ("ready", 1));
    assert_eq!((done.error, done.error_kind), (None, None));
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn real_failures_spend_attempts_until_the_limit(pool: PgPool) -> TestResult {
    let temp = tempfile::tempdir()?;
    let roots = DataRoots::new(temp.path());
    let job_id = permanent_job(&pool, "failing-recording").await?;
    // A stored name that would escape the recordings root must stop the
    // purge, however often it is retried.
    sqlx::query("UPDATE audio_files SET file_name='../escape' WHERE file_name='failing-recording'")
        .execute(&pool)
        .await?;
    for attempt in 1..=MAX_ATTEMPTS {
        make_due(&pool, &job_id).await?;
        run_one(&pool, &MediaArchive::disabled(), &roots).await?;
        let status = load_status(&pool, 1, &job_id).await?;
        assert_eq!(status.attempts, attempt);
        let expected = if attempt < MAX_ATTEMPTS {
            ("queued", "retry")
        } else {
            ("failed", "failed")
        };
        assert_eq!((status.state.as_str(), status.stage.as_str()), expected);
        assert_eq!(status.error_kind, Some(ErrorKind::UnsafeMediaReference));
        let message = status.error.unwrap_or_default();
        assert_eq!(message, ErrorKind::UnsafeMediaReference.default_message());
        assert!(!message.contains("escape"));
        assert!(!message.to_lowercase().contains("retry"));
    }
    make_due(&pool, &job_id).await?;
    assert!(claim(&pool).await?.is_none());
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn a_stale_attempt_cannot_touch_a_reclaimed_job(pool: PgPool) -> TestResult {
    let temp = tempfile::tempdir()?;
    let roots = DataRoots::new(temp.path());
    let job_id = permanent_job(&pool, "reclaimed-recording").await?;
    let recording = RecordingKey::new(1, 10, 2026, 9, "reclaimed-recording")
        .recording_path(&roots.recordings_str());
    tokio::fs::create_dir_all(recording.parent().unwrap()).await?;
    tokio::fs::write(&recording, b"test-media").await?;

    let stale = claim(&pool).await?.unwrap();
    sqlx::query(
        "UPDATE recording_deletion_jobs SET lease_expires_at=now()-interval '1 second' WHERE id=$1",
    )
    .bind(&job_id)
    .execute(&pool)
    .await?;
    let current = claim(&pool).await?.unwrap();
    stage(&pool, &current, "purging_archive").await?;
    let snapshot = || {
        sqlx::query_as::<_, (String, String, i32, Option<String>, Option<String>, Option<String>)>(
            "SELECT state,stage,attempts,attempt_token,error,error_kind FROM recording_deletion_jobs WHERE id=$1",
        )
        .bind(&job_id)
        .fetch_one(&pool)
    };
    let before = snapshot().await?;

    assert!(matches!(
        stage(&pool, &stale, "late").await,
        Err(AttemptError::LeaseLost)
    ));
    assert!(matches!(
        process_with_roots(&pool, &MediaArchive::disabled(), &stale, &roots).await,
        Err(AttemptError::LeaseLost)
    ));
    assert!(recording.exists(), "a stale attempt purged media");
    for outcome in [
        Err(AttemptError::Waiting),
        Err(AttemptError::failed(
            ErrorKind::InternalError,
            "late failure",
        )),
        Err(AttemptError::LeaseLost),
        Ok(()),
    ] {
        record_outcome(&pool, &stale, outcome).await?;
    }
    assert_eq!(snapshot().await?, before);

    process_with_roots(&pool, &MediaArchive::disabled(), &current, &roots).await?;
    assert_eq!(load_status(&pool, 1, &job_id).await?.state, "ready");
    assert!(!recording.exists());
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn legacy_deletion_errors_are_not_echoed(pool: PgPool) -> TestResult {
    let job_id = permanent_job(&pool, "legacy-recording").await?;
    sqlx::query("UPDATE recording_deletion_jobs SET error='IO Error: /srv/sakiot/data/recordings permission denied', error_kind=NULL WHERE id=$1")
        .bind(&job_id)
        .execute(&pool)
        .await?;
    let status = load_status(&pool, 1, &job_id).await?;
    let body = serde_json::to_string(&status)?;
    assert!(!body.contains("/srv"), "{body}");
    assert!(!body.contains("IO Error"), "{body}");
    assert_eq!(status.error_kind, None);
    assert_eq!(
        status.error.as_deref(),
        Some(crate::errors::UNCLASSIFIED_JOB_ERROR)
    );
    Ok(())
}
