//! The web process supervises a disposable copy of its own executable. This
//! makes blocking DSP and its FFmpeg descendants killable as one process group.
use super::*;
use std::time::Duration;

const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const DISK_BUDGET: u64 = 8 * 1024 * 1024 * 1024;

pub fn spawn_compose_worker(pool: Pool<Postgres>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_cleanup = None;
        loop {
            if last_cleanup
                .is_none_or(|last: std::time::Instant| last.elapsed() > Duration::from_secs(60))
            {
                if let Err(error) = cleanup(&pool).await {
                    tracing::warn!(?error, "composition cleanup failed");
                }
                last_cleanup = Some(std::time::Instant::now());
            }
            match queue::claim(&pool).await {
                Ok(Some((id, token))) => {
                    if let Err(error) = supervise(&pool, &id, &token).await {
                        tracing::error!(job_id = %id, ?error, "composition worker failed");
                        let _ = queue::fail(
                            &pool,
                            &id,
                            &token,
                            "Export worker was interrupted. Retrying automatically when possible.",
                            true,
                        )
                        .await;
                    }
                }
                Ok(None) => tokio::time::sleep(Duration::from_secs(2)).await,
                Err(error) => {
                    tracing::error!(?error, "composition queue claim failed");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    })
}

struct ProcessGroup(i32);
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // The child is started in its own process group. Also stop descendants
        // on normal exit, cancellation, or a lost lease.
        unsafe {
            libc::kill(-self.0, libc::SIGKILL);
        }
    }
}

async fn supervise(pool: &Pool<Postgres>, id: &str, token: &str) -> Result<(), AppError> {
    use std::os::unix::process::CommandExt;
    let mut command = tokio::process::Command::new(std::env::current_exe()?);
    command
        .args(["compose-worker", id, token])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    command.as_std_mut().process_group(0);
    let mut child = command.spawn()?;
    let group = ProcessGroup(
        child
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .ok_or(AppError::InternalError)?,
    );
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    let deadline = tokio::time::sleep(ATTEMPT_TIMEOUT);
    tokio::pin!(deadline);
    let result = loop {
        tokio::select! {
            status = child.wait() => {
                break match status {
                    Ok(status) if status.success() => Ok(()),
                    Ok(_) => Err(AppError::InternalError),
                    Err(error) => Err(error.into()),
                };
            }
            _ = heartbeat.tick() => {
                // A transient database error must not kill a long render: the
                // lease is valid for 60s and the next heartbeat retries. Only a
                // definitive "not renewed" answer means the lease is gone.
                match tokio::time::timeout(Duration::from_secs(5), queue::renew(pool, id, token)).await {
                    Ok(Ok(true)) => {},
                    Ok(Ok(false)) => break Err(AppError::Conflict("Export lease lost".into())),
                    Ok(Err(error)) => tracing::warn!(job_id = %id, ?error, "composition lease renewal failed; retrying next heartbeat"),
                    Err(_) => tracing::warn!(job_id = %id, "composition lease renewal timed out; retrying next heartbeat"),
                }
            }
            _ = &mut deadline => break Err(AppError::ServiceUnavailable("Export exceeded the execution deadline".into())),
        }
    };
    drop(group);
    let _ = child.wait().await;
    result
}

/// Internal child-process entry point, not an HTTP endpoint. It reads only the
/// database settings here; no listening socket or Discord connection is opened.
pub async fn run_compose_worker_command(arguments: &[String]) -> Result<(), AppError> {
    if arguments.len() != 2
        || arguments
            .iter()
            .any(|value| uuid::Uuid::parse_str(value).is_err())
    {
        return Err(AppError::BadRequest(
            "Expected job id and attempt token".into(),
        ));
    }
    if unsafe { libc::getpgrp() != libc::getpid() } {
        return Err(AppError::BadRequest(
            "Composition workers must run in their own process group".into(),
        ));
    }
    let id = &arguments[0];
    let token = &arguments[1];
    // Bound allocations and individual temporary files in this child only.
    unsafe {
        let memory = libc::rlimit {
            rlim_cur: 2 * 1024 * 1024 * 1024,
            rlim_max: 2 * 1024 * 1024 * 1024,
        };
        let file = libc::rlimit {
            rlim_cur: DISK_BUDGET,
            rlim_max: DISK_BUDGET,
        };
        if libc::setrlimit(libc::RLIMIT_AS, &memory) != 0
            || libc::setrlimit(libc::RLIMIT_FSIZE, &file) != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    // This thread still runs if a DSP blocking task stalls the async executor.
    // It also prevents descendants surviving an abruptly terminated supervisor.
    let parent = unsafe { libc::getppid() };
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if started.elapsed() > ATTEMPT_TIMEOUT || unsafe { libc::getppid() } != parent {
                unsafe {
                    libc::kill(0, libc::SIGKILL);
                }
                return;
            }
        }
    });
    let database_url = std::env::var("DATABASE_URL").map_err(|_| AppError::InternalError)?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(3)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await?;
    let job = queue::load(&pool, id, token).await?;
    let media = MediaArchive::from_env()
        .await
        .map_err(|_| AppError::InternalError)?;
    let progress = web::Data::new(WaveformProgressContainer(tokio::sync::RwLock::new(
        std::collections::HashMap::new(),
    )));
    let work = execute(&pool, &media, &job, &progress);
    tokio::pin!(work);
    let mut ticker = tokio::time::interval(Duration::from_secs(2));
    let result = loop {
        tokio::select! {
            result = &mut work => break result,
            _ = ticker.tick() => {
                let pct = progress.0.read().await.get(&compose_progress_key(id)).copied().unwrap_or(0).clamp(0, 99);
                let stage = if pct == 0 { "preparing" } else { "rendering" };
                // A transient database error here must not fail the export: the
                // lease still has time left and the next tick retries. Only a
                // definitive "not running" answer means the lease was lost.
                match queue::report(&pool, id, token, stage, pct).await {
                    Ok(true) => {}
                    Ok(false) => {
                        // Publication may have just committed. Returning lets the
                        // supervisor end the group; no new attempt may publish.
                        break Err(AppError::Conflict("Export lease lost".into()));
                    }
                    Err(error) => tracing::warn!(
                        job_id = %id,
                        ?error,
                        "composition progress report failed; retrying next tick"
                    ),
                }
                if workspace_bytes(&attempt_dir(id, token)).await? > DISK_BUDGET {
                    break Err(AppError::BadRequest("Export exceeded the temporary storage budget".into()));
                }
            }
        }
    };
    if let Err(error) = result {
        tracing::error!(job_id = %id, ?error, "composition attempt failed");
        let retryable = !matches!(
            error,
            AppError::BadRequest(_)
                | AppError::Conflict(_)
                | AppError::Forbidden
                | AppError::ClipNotFound
        );
        let message = match &error {
            AppError::Conflict(message) | AppError::BadRequest(message) => message.clone(),
            AppError::Forbidden => "Access to a source clip was removed.".into(),
            AppError::ClipNotFound => "A source or destination clip was deleted.".into(),
            _ => "The export could not finish. Check the source clips and retry.".into(),
        };
        queue::fail(&pool, id, token, &message, retryable).await?;
    }
    Ok(())
}

pub(super) fn attempt_dir(id: &str, token: &str) -> PathBuf {
    PathBuf::from(clips_path())
        .join(".composition-jobs")
        .join(id)
        .join(token)
}

async fn execute(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    job: &queue::Job,
    progress: &web::Data<WaveformProgressContainer>,
) -> Result<(), AppError> {
    let data = web::Data::new(pool.clone());
    let resolved = resolve_composition(
        &data,
        Some(media),
        job.guild_id,
        job.user_id,
        validate_composition(job.snapshot.body.clone())?,
    )
    .await?;
    let directory = attempt_dir(&job.id, &job.token);
    tokio::fs::create_dir_all(&directory).await?;
    let mut segments = Vec::new();
    for (index, (segment, source)) in job
        .snapshot
        .body
        .segments
        .iter()
        .zip(resolved.sources)
        .enumerate()
    {
        if job.snapshot.sources.get(index) != Some(&source.saved_file_name) {
            return Err(AppError::Conflict(
                "A source clip changed after this export was submitted".into(),
            ));
        }
        let pinned = directory.join(format!("source-{index}.ogg"));
        // All media lives on the same filesystem. A hard link keeps the exact
        // bytes alive when archive eviction unlinks the source during rendering.
        tokio::fs::hard_link(&source.path, &pinned).await?;
        segments.push(SegmentRender {
            path: pinned,
            source_in: segment.source_in,
            source_out: segment.source_out,
            effects: shared_effects_from_dto(&segment.effects),
            timeline_start: segment.timeline_start,
            muted: job
                .snapshot
                .body
                .muted_tracks
                .get(segment.track as usize)
                .copied()
                .unwrap_or(false),
        });
    }
    let output = directory.join("render.ogg");
    render_compose(
        &segments,
        job.snapshot.body.master_volume_db,
        &output,
        expected_duration_ms(&segments),
        progress,
        &compose_progress_key(&job.id),
    )
    .await?;
    let duration = probe_duration(&output).await? as f32;
    let size = i64::try_from(tokio::fs::metadata(&output).await?.len())
        .map_err(|_| AppError::InternalError)?;
    if size == 0 {
        return Err(AppError::FfmpegError("Empty export".into()));
    }
    if !queue::report(pool, &job.id, &job.token, "publishing", 99).await? {
        return Err(AppError::Conflict("Export lease lost".into()));
    }
    let saved = format!("compositions/{}-{}.ogg", job.id, job.token);
    let final_path = crate::media_archive::clip_local_path(&saved)?;
    let parent = final_path.parent().ok_or(AppError::InternalError)?;
    tokio::fs::create_dir_all(parent).await?;
    if let Some(root) = parent.parent() {
        tokio::fs::File::open(root).await?.sync_all().await?;
    }
    tokio::fs::File::open(&output).await?.sync_all().await?;
    tokio::fs::rename(&output, &final_path).await?;
    tokio::fs::File::open(parent).await?.sync_all().await?;
    publish(pool, job, &saved, duration, size).await
}

async fn workspace_bytes(path: &Path) -> Result<u64, AppError> {
    let mut entries = match tokio::fs::read_dir(path).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = 0u64;
    while let Some(entry) = entries.next_entry().await? {
        bytes = bytes.saturating_add(entry.metadata().await.map_or(0, |metadata| metadata.len()));
    }
    Ok(bytes)
}

/// Only managed attempt directories and immutable composition outputs are
/// eligible. Never remove a file referenced by any clip, even a soft-deleted one.
async fn cleanup(pool: &Pool<Postgres>) -> Result<(), AppError> {
    let old_paths: Vec<String> = sqlx::query_scalar("SELECT DISTINCT snapshot->'overwrite'->>'old_saved_file_name' FROM composition_jobs WHERE state = 'ready' AND finished_at < now() - interval '1 hour' AND snapshot->'overwrite'->>'old_saved_file_name' IS NOT NULL")
        .fetch_all(pool).await?;
    for saved in old_paths {
        let used: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM clips WHERE saved_file_name = $1)")
                .bind(&saved)
                .fetch_one(pool)
                .await?;
        if !used {
            let _ = tokio::fs::remove_file(crate::media_archive::clip_local_path(&saved)?).await;
        }
    }
    sqlx::query("DELETE FROM composition_jobs WHERE state IN ('ready', 'failed') AND finished_at < now() - interval '30 days'").execute(pool).await?;
    let root = PathBuf::from(clips_path());
    let work = root.join(".composition-jobs");
    if let Ok(mut jobs) = tokio::fs::read_dir(&work).await {
        while let Some(job) = jobs.next_entry().await? {
            if !job.file_type().await?.is_dir() {
                continue;
            }
            let id = job.file_name().to_string_lossy().into_owned();
            let mut attempts = tokio::fs::read_dir(job.path()).await?;
            while let Some(attempt) = attempts.next_entry().await? {
                if !attempt.file_type().await?.is_dir() || !old_enough(&attempt).await {
                    continue;
                }
                let token = attempt.file_name().to_string_lossy().into_owned();
                let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM composition_jobs WHERE id = $1 AND attempt_token = $2 AND state = 'running' AND lease_expires_at > now())")
                    .bind(&id).bind(&token).fetch_one(pool).await?;
                if !active {
                    let _ = tokio::fs::remove_dir_all(attempt.path()).await;
                }
            }
            let _ = tokio::fs::remove_dir(job.path()).await;
        }
    }
    if let Ok(mut files) = tokio::fs::read_dir(root.join("compositions")).await {
        while let Some(file) = files.next_entry().await? {
            if !file.file_type().await?.is_file() || !old_enough(&file).await {
                continue;
            }
            let saved = format!("compositions/{}", file.file_name().to_string_lossy());
            let used: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM clips WHERE saved_file_name = $1)")
                    .bind(&saved)
                    .fetch_one(pool)
                    .await?;
            if !used {
                let _ = tokio::fs::remove_file(file.path()).await;
            }
        }
    }
    Ok(())
}

async fn old_enough(entry: &tokio::fs::DirEntry) -> bool {
    entry
        .metadata()
        .await
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.elapsed().ok())
        .is_some_and(|age| age > Duration::from_secs(3600))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;

    #[test]
    fn dropping_supervisor_guard_terminates_child_group() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 30 & wait"]).process_group(0);
        let mut child = command.spawn()?;
        let group = ProcessGroup(i32::try_from(child.id())?);
        drop(group);
        assert!(!child.wait()?.success());
        Ok(())
    }
}
