//! The worker loop: running claimed jobs with a renewed lease and a deadline,
//! and cleaning up finished ones.

use super::*;

async fn run_attempt(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    job: &ClaimedMediaJob,
) -> Result<(Option<String>, Option<PathBuf>), AppError> {
    report_progress(pool, &job.id, &job.token, "processing", 1).await?;
    let attempt = JobAttempt {
        pool,
        media,
        requester: job.requester,
        id: &job.id,
        token: &job.token,
    };
    match &job.request {
        MediaJobRequest::RecordingSilence {
            guild_id,
            channel_id,
            year,
            month,
            file_name,
        } => {
            crate::audio::silence::run_recording_silence_job(
                &attempt,
                *guild_id,
                *channel_id,
                *year,
                *month,
                file_name,
            )
            .await
        }
        MediaJobRequest::RecordingWaveform {
            guild_id,
            channel_id,
            year,
            month,
            file_name,
            silence_free,
        } => {
            crate::audio::peaks::run_recording_waveform_job(
                &attempt,
                *guild_id,
                *channel_id,
                *year,
                *month,
                file_name,
                *silence_free,
            )
            .await
        }
        MediaJobRequest::ClipWaveform { guild_id, clip_id } => {
            crate::audio::peaks::run_clip_waveform_job(&attempt, *guild_id, clip_id).await
        }
        MediaJobRequest::SessionWaveform {
            session_id,
            silence_free,
        } => {
            crate::audio::sessions::run_session_waveform_job(&attempt, *session_id, *silence_free)
                .await
        }
        MediaJobRequest::SessionSilence { session_id } => {
            crate::audio::sessions::run_session_silence_job(&attempt, *session_id).await
        }
        MediaJobRequest::SessionMix {
            session_id,
            scope,
            participants,
        } => {
            crate::audio::sessions::run_session_mix_job(
                &attempt,
                *session_id,
                scope,
                participants.clone(),
            )
            .await
        }
        MediaJobRequest::SessionDownload {
            session_id,
            start,
            end,
            remove_silence,
        } => {
            crate::audio::sessions::run_download_job(
                &attempt,
                *session_id,
                *start,
                *end,
                *remove_silence,
            )
            .await
        }
    }
}

async fn execute(pool: Pool<Postgres>, media: MediaArchive, job: ClaimedMediaJob) {
    let started = std::time::Instant::now();
    let result = {
        let mut interval = tokio::time::interval(Duration::from_secs(20));
        interval.tick().await;
        let mut lease_confirmed_at = tokio::time::Instant::now();
        let deadline = tokio::time::sleep(Duration::from_secs(30 * 60));
        tokio::pin!(deadline);
        let attempt = run_attempt(&pool, &media, &job);
        tokio::pin!(attempt);
        loop {
            tokio::select! {
                result = &mut attempt => break result,
                _ = &mut deadline => break Err(AppError::ExecutionTimedOut),
                _ = interval.tick() => match renew(&pool, &job).await {
                    Ok(true) => lease_confirmed_at = tokio::time::Instant::now(),
                    Ok(false) => break Err(AppError::JobLeaseLost),
                    Err(error) => {
                        tracing::warn!(job_id=%job.id, ?error, "media lease renewal failed");
                        if lease_confirmed_at.elapsed() >= Duration::from_secs(40) {
                            break Err(AppError::WorkerInterrupted(
                                "media job lease could not be renewed".into(),
                            ));
                        }
                    }
                },
            }
        }
    };
    crate::job_metrics::record(
        job.request.kind(),
        crate::job_metrics::outcome(&result),
        started.elapsed(),
    );
    if let Err(error) = finish(&pool, &job, result).await {
        tracing::error!(job_id=%job.id, ?error, "media job completion failed");
    }
}

pub fn spawn_worker(pool: Pool<Postgres>, media: MediaArchive) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_cleanup = tokio::time::Instant::now() - Duration::from_secs(60 * 60);
        loop {
            if last_cleanup.elapsed() >= Duration::from_secs(60 * 60) {
                if let Err(error) = cleanup(&pool).await {
                    tracing::warn!(?error, "media job cleanup failed");
                }
                last_cleanup = tokio::time::Instant::now();
            }
            match claim(&pool).await {
                Ok(Some(job)) => {
                    let pool = pool.clone();
                    let media = media.clone();
                    tokio::spawn(execute(pool, media, job));
                }
                Ok(None) => tokio::time::sleep(Duration::from_millis(500)).await,
                Err(error) => {
                    tracing::error!(?error, "media queue claim failed");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    })
}

async fn cleanup(pool: &Pool<Postgres>) -> Result<(), AppError> {
    let managed = PathBuf::from(crate::audio::paths::recording_path()).join(".media-jobs");
    let rows = sqlx::query!(
        "SELECT id,result_path FROM media_jobs WHERE state IN ('ready','failed') AND finished_at < now()-interval '30 days' ORDER BY finished_at LIMIT 100",
    )
    .fetch_all(pool)
    .await?;
    for row in rows {
        let id = row.id;
        if let Some(path) = row.result_path {
            let path = PathBuf::from(path);
            if !path.starts_with(&managed) {
                tracing::error!(job_id=%id, path=%path.display(), "refusing to clean unmanaged media result");
                continue;
            }
            match tokio::fs::remove_file(&path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(job_id=%id, ?error, "could not clean media result");
                    continue;
                }
            }
        }
        sqlx::query!(
            "DELETE FROM media_jobs WHERE id=$1 AND state IN ('ready','failed') AND finished_at < now()-interval '30 days'",
            id
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}
