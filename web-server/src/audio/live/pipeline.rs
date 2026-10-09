//! The ffmpeg pipeline behind a live job: spawning it, feeding a growing
//! source into it, stopping it when idle, and draining it at the end.

use super::*;

/// Build the ffmpeg command tail (everything past the input args).
pub(super) fn ffmpeg_output_args(out_dir: &Path, live: bool) -> Vec<String> {
    let seg_pattern = out_dir.join("seg_%05d.m4s");
    let playlist = out_dir.join("playlist.m3u8");
    let flags = if live {
        "independent_segments+omit_endlist"
    } else {
        "independent_segments"
    };
    let playlist_type = if live { "event" } else { "vod" };
    vec![
        "-c:a".into(),
        "copy".into(),
        "-map".into(),
        "0:a:0".into(),
        "-f".into(),
        "hls".into(),
        "-hls_time".into(),
        "2".into(),
        "-hls_list_size".into(),
        "0".into(),
        "-hls_flags".into(),
        flags.into(),
        "-hls_playlist_type".into(),
        playlist_type.into(),
        "-hls_segment_type".into(),
        "fmp4".into(),
        "-hls_fmp4_init_filename".into(),
        "init.mp4".into(),
        "-hls_segment_filename".into(),
        seg_pattern.to_string_lossy().into_owned(),
        playlist.to_string_lossy().into_owned(),
    ]
}

pub(super) fn drain_child_stderr(child: &mut Child, job_id: String) {
    let Some(mut stderr) = child.stderr.take() else {
        return;
    };

    tokio::spawn(async move {
        if let Err(error) = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await {
            warn!(stem = %job_id, ?error, "failed to drain ffmpeg stderr");
        }
    });
}

/// Streams `src` into `sink` as the recording grows, and closes `sink` once
/// the source is complete.
///
/// This replaces a `tail -F` subprocess. Owning the read loop means the job
/// knows exactly how many bytes it has handed to ffmpeg, so finishing is just
/// "read to EOF, then drop the pipe" - no locating another process in the
/// group and no reading its fd offset out of procfs to guess whether it had
/// caught up.
///
/// `stop` going true means the DB row is no longer live. The bot closes its
/// writer before that happens, so the file is complete by then and the loop
/// only breaks on an EOF observed *after* the signal - every byte written is
/// forwarded.
pub(super) async fn follow_source_into(
    src: PathBuf,
    mut sink: ChildStdin,
    mut stop: watch::Receiver<bool>,
    job_id: String,
) {
    let mut file = match tokio::fs::File::open(&src).await {
        Ok(file) => file,
        Err(error) => {
            error!(stem = %job_id, ?error, "cannot open live source");
            return;
        }
    };

    let mut buf = vec![0u8; FOLLOW_CHUNK];
    loop {
        match file.read(&mut buf).await {
            Ok(0) => {
                // Caught up with the writer. If the recording has finished,
                // this EOF is the real end of the file.
                if *stop.borrow() {
                    break;
                }
                tokio::select! {
                    _ = tokio::time::sleep(FOLLOW_POLL) => {}
                    _ = stop.changed() => {}
                }
            }
            Ok(read) => {
                if let Err(error) = sink.write_all(&buf[..read]).await {
                    // ffmpeg exited or closed stdin; nothing left to feed.
                    warn!(stem = %job_id, ?error, "live pipe write failed");
                    return;
                }
            }
            Err(error) => {
                error!(stem = %job_id, ?error, "live source read failed");
                return;
            }
        }
    }

    if let Err(error) = sink.shutdown().await {
        warn!(stem = %job_id, ?error, "closing the live pipe failed");
    }
}

/// Finishes the live job: close ffmpeg's stdin at the true end of the source
/// and let it write its trailer.
///
/// Signalling the follower is the graceful path - ffmpeg sees EOF on stdin,
/// flushes the final partial segment and exits on its own. The signals below
/// are escalation only. ffmpeg is a direct child now, so they address one pid
/// rather than a process group.
pub(super) async fn drain_live_pipeline(
    child: &mut Child,
    stop: Option<&watch::Sender<bool>>,
    job_id: &str,
) {
    if let Some(stop) = stop {
        let _ = stop.send(true);
    }

    if tokio::time::timeout(PIPELINE_EXIT_TIMEOUT, child.wait())
        .await
        .is_ok()
    {
        return;
    }

    if let Some(pid) = child.id() {
        warn!(stem = %job_id, "ffmpeg did not exit after its input closed; terminating");
        // SAFETY: SIGTERM to this job's own child pid, which `child` is still
        // holding open, so the pid cannot have been recycled. ffmpeg treats
        // SIGTERM as a graceful quit and still writes its trailer.
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        if tokio::time::timeout(PIPELINE_KILL_TIMEOUT, child.wait())
            .await
            .is_ok()
        {
            return;
        }
    }

    warn!(stem = %job_id, "ffmpeg ignored SIGTERM; killing");
    let _ = child.kill().await;
}

/// Stops a running live job whose playlist nobody has requested for
/// `idle_after`, and forgets it, so the next request starts a new job. That
/// job finds the unfinished playlist and rebuilds the output from the start of
/// the recording.
///
/// Holds the job's creation lock throughout, so a new job cannot start writing
/// the same directory before the old ffmpeg has exited. A request that took
/// the old job from the map just before it was stopped still gets the old
/// playlist; the player's next request starts the new job.
pub(super) async fn stop_if_idle(
    container: &LiveContainer,
    id: &str,
    state: &Arc<Mutex<JobState>>,
    idle_after: Duration,
) -> bool {
    if container.idle_for(id) < idle_after {
        return false;
    }
    let key_guard = container.key_lock(id).await;
    let stopped = {
        let _creation = key_guard.lock().await;
        // A request may have arrived while this waited for the lock.
        if container.idle_for(id) < idle_after {
            false
        } else {
            {
                let mut jobs = container.jobs.write().await;
                if jobs.get(id).is_some_and(|job| Arc::ptr_eq(job, state)) {
                    jobs.remove(id);
                }
            }
            container.untrack_requests(id);
            let mut job = state.lock().await;
            if let Some(stop) = job.follow_stop.take() {
                let _ = stop.send(true);
            }
            if let Some(mut child) = job.child.take()
                && let Err(error) = child.kill().await
            {
                warn!(stem = %id, ?error, "stopping an idle live ffmpeg failed");
            }
            true
        }
    };
    container.release_key_lock(id, key_guard).await;
    stopped
}

pub(super) async fn spawn_job(
    container: web::Data<LiveContainer>,
    pool: web::Data<Pool<Postgres>>,
    key: RecordingKey,
    src: PathBuf,
    out_dir: PathBuf,
    is_live: bool,
) -> Result<Arc<Mutex<JobState>>, AppError> {
    tokio::fs::create_dir_all(&out_dir)
        .await
        .map_err(AppError::IoError)?;

    let id = key_id(&key);
    let mut follow_stop = None;
    let mut child = if is_live {
        // ffmpeg reads the growing recording from a pipe this process owns and
        // fills (see `follow_source_into`). Spawned directly: no shell, so no
        // argument quoting, no process group, and `Child` refers to ffmpeg
        // itself rather than a shell standing in front of it.
        let mut c = Command::new("ffmpeg");
        c.arg("-hide_banner")
            .args(["-loglevel", "warning"])
            .args(["-f", "ogg", "-i", "pipe:0"]);
        for a in ffmpeg_output_args(&out_dir, true) {
            c.arg(a);
        }
        let mut child = c
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| crate::ffmpeg::tool_error("ffmpeg", error))?;

        let stdin = child.stdin.take().ok_or_else(|| {
            AppError::IoError(std::io::Error::other("ffmpeg stdin was not piped"))
        })?;
        let (stop_tx, stop_rx) = watch::channel(false);
        tokio::spawn(follow_source_into(src.clone(), stdin, stop_rx, id.clone()));
        follow_stop = Some(stop_tx);
        child
    } else {
        // `-y` + stdin null: never block on the overwrite prompt if a file
        // from an earlier build is still present (stdin null means ffmpeg
        // answers prompts with "no" and exits).
        let mut c = Command::new("ffmpeg");
        c.arg("-hide_banner")
            .args(["-loglevel", "warning", "-y"])
            .arg("-i")
            .arg(&src);
        for a in ffmpeg_output_args(&out_dir, false) {
            c.arg(a);
        }
        c.stdout(Stdio::null())
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| crate::ffmpeg::tool_error("ffmpeg", error))?
    };
    drain_child_stderr(&mut child, id.clone());

    let state = Arc::new(Mutex::new(JobState {
        finalized: false,
        child: Some(child),
        follow_stop,
    }));
    container
        .jobs
        .write()
        .await
        .insert(key_id(&key), state.clone());
    if is_live {
        container.track_requests(&id);
    }

    // Lifecycle task.
    let container_c = container.clone();
    let state_c = state.clone();
    let pool_c = pool.clone();
    let stem = key.stem.clone();
    let out_dir_c = out_dir.clone();
    tokio::spawn(async move {
        if is_live {
            // Poll DB until the row is no longer lease-backed live, then kill
            // the pipeline. Stop early once nobody listens.
            loop {
                tokio::time::sleep(LIVE_POLL).await;
                if stop_if_idle(&container_c, &id, &state_c, LIVE_IDLE_STOP).await {
                    info!(stem = %id, "live job stopped: no listeners");
                    return;
                }
                match db_state(&pool_c, &stem).await {
                    Ok(state) if !state.live => break,
                    Ok(_) => {}
                    Err(e) => {
                        error!(stem = %id, error = ?e, "db poll error");
                    }
                }
            }
            let mut g = state_c.lock().await;
            if let Some(mut child) = g.child.take() {
                let stop = g.follow_stop.take();
                drain_live_pipeline(&mut child, stop.as_ref(), &id).await;
            }
            drop(g);
            let pl = out_dir_c.join("playlist.m3u8");
            if let Err(e) = append_endlist(&pl).await {
                error!(stem = %id, error = ?e, "append_endlist failed");
            }
            state_c.lock().await.finalized = true;
            container_c.untrack_requests(&id);
            info!(stem = %id, "live job finalized");
        } else {
            let mut g = state_c.lock().await;
            if let Some(mut child) = g.child.take() {
                let _ = child.wait().await;
            }
            g.finalized = true;
            info!(stem = %id, "vod job finished");
        }
    });

    // Wait briefly for ffmpeg to write playlist + init.mp4 before returning.
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let pl = out_dir.join("playlist.m3u8");
    let init = out_dir.join("init.mp4");
    while std::time::Instant::now() < deadline {
        if tokio::fs::try_exists(&pl).await.unwrap_or(false)
            && tokio::fs::try_exists(&init).await.unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(state)
}
