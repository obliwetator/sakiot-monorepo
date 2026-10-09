use super::*;

/// One Ogg page with a body of `body` bytes.
fn ogg_page(body: usize) -> Vec<u8> {
    let mut lacing = vec![255_u8; body / 255];
    lacing.push(u8::try_from(body % 255).unwrap());
    let mut page = b"OggS".to_vec();
    page.extend_from_slice(&[0; 22]);
    page.push(u8::try_from(lacing.len()).unwrap());
    page.extend_from_slice(&lacing);
    page.extend(std::iter::repeat_n(0_u8, body));
    page
}

#[test]
fn a_file_is_audible_once_its_first_audio_page_is_complete() {
    let headers = [ogg_page(19), ogg_page(26)].concat();
    let audio = ogg_page(3_900);
    let whole = [headers.clone(), audio.clone()].concat();

    assert_eq!(complete_ogg_pages(&[], FIRST_AUDIBLE_PAGES), 0);
    assert_eq!(complete_ogg_pages(&headers, FIRST_AUDIBLE_PAGES), 2);
    assert_eq!(
        complete_ogg_pages(&whole[..whole.len() - 1], FIRST_AUDIBLE_PAGES),
        2
    );
    assert_eq!(complete_ogg_pages(&whole, FIRST_AUDIBLE_PAGES), 3);
    assert_eq!(
        complete_ogg_pages(b"not an ogg file at all, no capture pattern", 3),
        0
    );
}

#[test]
fn the_starting_playlist_is_live_and_empty() {
    assert!(STARTING_PLAYLIST.starts_with("#EXTM3U\n"));
    assert!(STARTING_PLAYLIST.contains("#EXT-X-PLAYLIST-TYPE:EVENT"));
    assert!(!STARTING_PLAYLIST.contains("#EXT-X-ENDLIST"));
    assert!(!STARTING_PLAYLIST.contains("#EXTINF"));
}

#[tokio::test]
async fn creation_lock_survives_failed_job_retries() {
    let container = LiveContainer::default();
    let first = container.key_lock("recording").await;
    let retry = container.key_lock("recording").await;
    let other = container.key_lock("other-recording").await;

    assert!(Arc::ptr_eq(&first, &retry));
    assert!(!Arc::ptr_eq(&first, &other));
}

#[tokio::test]
async fn released_creation_lock_is_pruned_unless_a_waiter_holds_it() {
    let container = LiveContainer::default();
    let lock = container.key_lock("recording").await;

    // A waiter still holding a clone keeps the entry: it must observe the
    // same lock or two spawns could race for one recording.
    let waiter = container.key_lock("recording").await;
    container.release_key_lock("recording", lock).await;
    assert_eq!(container.locks.lock().await.len(), 1);

    // With nobody waiting, the entry goes away and the map cannot grow
    // with every recording ever streamed.
    container.release_key_lock("recording", waiter).await;
    assert!(container.locks.lock().await.is_empty());
}

#[tokio::test]
async fn pruned_lock_is_recreated_for_later_requests() {
    let container = LiveContainer::default();
    let lock = container.key_lock("recording").await;
    container.release_key_lock("recording", lock).await;

    let recreated = container.key_lock("recording").await;
    let stored = container.locks.lock().await.get("recording").cloned();
    assert_eq!(container.locks.lock().await.len(), 1);
    assert!(stored.is_some_and(|stored| Arc::ptr_eq(&recreated, &stored)));
}

/// A running live job whose ffmpeg is stood in for by `sleep`. Returns the
/// receiver of the follower's stop signal.
async fn running_live_job(
    container: &LiveContainer,
    id: &str,
) -> Result<(Arc<Mutex<JobState>>, watch::Receiver<bool>), Box<dyn std::error::Error>> {
    let child = Command::new("sleep")
        .arg("600")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let (stop, stopped) = watch::channel(false);
    let job = Arc::new(Mutex::new(JobState {
        finalized: false,
        child: Some(child),
        follow_stop: Some(stop),
    }));
    container
        .jobs
        .write()
        .await
        .insert(id.to_owned(), job.clone());
    container.track_requests(id);
    Ok((job, stopped))
}

#[tokio::test]
async fn a_live_job_with_listeners_keeps_running() -> Result<(), Box<dyn std::error::Error>> {
    let container = LiveContainer::default();
    let (job, stopped) = running_live_job(&container, "recording").await?;
    container.touch("recording");

    assert!(!stop_if_idle(&container, "recording", &job, LIVE_IDLE_STOP).await);
    assert!(container.jobs.read().await.contains_key("recording"));
    assert!(job.lock().await.child.is_some());
    assert!(!*stopped.borrow());
    Ok(())
}

#[tokio::test]
async fn an_idle_live_job_stops_and_is_forgotten() -> Result<(), Box<dyn std::error::Error>> {
    let container = LiveContainer::default();
    let (job, stopped) = running_live_job(&container, "recording").await?;

    assert!(stop_if_idle(&container, "recording", &job, Duration::ZERO).await);
    assert!(container.jobs.read().await.is_empty());
    assert!(job.lock().await.child.is_none());
    assert!(*stopped.borrow());
    assert!(container.locks.lock().await.is_empty());

    // Requests for the stopped job do not track it again; the job that
    // replaces it starts tracking when it spawns.
    container.touch("recording");
    assert!(container.requested().is_empty());
    Ok(())
}

#[tokio::test]
async fn only_running_live_jobs_count_as_idle() -> Result<(), Box<dyn std::error::Error>> {
    let container = LiveContainer::default();
    let (job, _stopped) = running_live_job(&container, "recording").await?;
    container.untrack_requests("recording");

    // A finished or VOD job has no request tracking and never stops.
    assert_eq!(container.idle_for("recording"), Duration::ZERO);
    assert!(!stop_if_idle(&container, "recording", &job, Duration::from_millis(1)).await);
    assert!(job.lock().await.child.is_some());
    Ok(())
}

#[tokio::test]
async fn hls_cache_action_builds_when_playlist_missing() {
    let dir =
        std::env::temp_dir().join(format!("sakiot-live-test-missing-{}", uuid::Uuid::new_v4()));
    let playlist = dir.join("playlist.m3u8");

    assert_eq!(
        hls_cache_action(&playlist).await,
        HlsCacheAction::BuildFresh
    );
}

#[tokio::test]
async fn hls_cache_action_reuses_finalized_playlist() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("sakiot-live-test-final-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir).await?;
    let playlist = dir.join("playlist.m3u8");
    tokio::fs::write(&playlist, "#EXTM3U\n#EXT-X-ENDLIST\n").await?;

    assert_eq!(
        hls_cache_action(&playlist).await,
        HlsCacheAction::ReuseFinalized
    );

    let _ = tokio::fs::remove_dir_all(&dir).await;
    Ok(())
}

#[tokio::test]
async fn hls_cache_action_purges_unfinalized_playlist_even_for_vod()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("sakiot-live-test-stale-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir).await?;
    let playlist = dir.join("playlist.m3u8");
    tokio::fs::write(&playlist, "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:EVENT\n").await?;

    // A non-finalized playlist means the previous run never completed,
    // whether it was live or a VOD rebuild: both must purge, otherwise
    // the VOD command (stdin null) refuses the overwrite and the dead
    // playlist is served as finalized.
    assert_eq!(
        hls_cache_action(&playlist).await,
        HlsCacheAction::PurgeStale
    );

    let _ = tokio::fs::remove_dir_all(&dir).await;
    Ok(())
}

#[test]
fn live_ffmpeg_flags_do_not_append_existing_playlist() -> Result<(), Box<dyn std::error::Error>> {
    let args = ffmpeg_output_args(Path::new("/tmp/live"), true);
    let flags_pos = args
        .iter()
        .position(|arg| arg == "-hls_flags")
        .ok_or_else(|| std::io::Error::other("hls flags option should exist"))?;
    let flags = &args[flags_pos + 1];

    assert!(flags.contains("omit_endlist"));
    assert!(!flags.contains("append_list"));
    Ok(())
}

/// Spawns `cat` as a stand-in for ffmpeg and returns it with its stdin,
/// so a follower can be pointed at a real pipe and the result compared
/// byte for byte.
fn spawn_sink(out: &Path) -> Result<(Child, ChildStdin), Box<dyn std::error::Error>> {
    let outfile = std::fs::File::create(out)?;
    let mut child = Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::from(outfile))
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let stdin = child.stdin.take().expect("stdin was piped");
    Ok((child, stdin))
}

async fn append(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut f = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await?;
    f.write_all(bytes).await?;
    f.flush().await?;
    Ok(())
}

#[tokio::test]
async fn drain_live_pipeline_preserves_all_source_bytes() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = std::env::temp_dir().join(format!("sakiot-live-test-drain-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir).await?;
    let src = dir.join("src.ogg");
    let out = dir.join("out.bin");
    let payload = vec![7u8; 300_000];
    tokio::fs::write(&src, &payload).await?;

    let (mut child, stdin) = spawn_sink(&out)?;
    let (stop_tx, stop_rx) = watch::channel(false);
    let follower = tokio::spawn(follow_source_into(
        src.clone(),
        stdin,
        stop_rx,
        "drain-test".into(),
    ));

    // Let the follower reach EOF, then append the "last writes" that the
    // old fixed 2s sleep used to race against.
    tokio::time::sleep(Duration::from_millis(300)).await;
    append(&src, &payload).await?;

    drain_live_pipeline(&mut child, Some(&stop_tx), "drain-test").await;
    follower.await?;

    let written = tokio::fs::read(&out).await?;
    assert_eq!(written.len(), payload.len() * 2);
    assert!(written.iter().all(|b| *b == 7));

    let _ = tokio::fs::remove_dir_all(&dir).await;
    Ok(())
}

#[tokio::test]
async fn follower_keeps_streaming_across_repeated_end_of_file()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("sakiot-live-test-grow-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir).await?;
    let src = dir.join("src.ogg");
    let out = dir.join("out.bin");
    let chunk = vec![3u8; 64 * 1024];
    tokio::fs::write(&src, &chunk).await?;

    let (mut child, stdin) = spawn_sink(&out)?;
    let (stop_tx, stop_rx) = watch::channel(false);
    let follower = tokio::spawn(follow_source_into(
        src.clone(),
        stdin,
        stop_rx,
        "grow-test".into(),
    ));

    // A live recording is written in bursts, so the follower hits EOF
    // repeatedly before the recording ends. Each sleep is long enough for
    // it to reach EOF and park; it has to resume on its own every time.
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(300)).await;
        append(&src, &chunk).await?;
    }

    tokio::time::sleep(Duration::from_millis(300)).await;
    drain_live_pipeline(&mut child, Some(&stop_tx), "grow-test").await;
    follower.await?;

    let written = tokio::fs::read(&out).await?;
    assert_eq!(written.len(), chunk.len() * 4);

    let _ = tokio::fs::remove_dir_all(&dir).await;
    Ok(())
}

#[tokio::test]
async fn follower_closes_the_pipe_so_the_child_exits_on_its_own()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("sakiot-live-test-eof-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir).await?;
    let src = dir.join("src.ogg");
    let out = dir.join("out.bin");
    tokio::fs::write(&src, b"hello").await?;

    let (mut child, stdin) = spawn_sink(&out)?;
    let (stop_tx, stop_rx) = watch::channel(false);
    tokio::spawn(follow_source_into(
        src.clone(),
        stdin,
        stop_rx,
        "eof-test".into(),
    ));

    let _ = stop_tx.send(true);
    // No signal is sent to the child here: closing the pipe has to be
    // enough for it to finish by itself.
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait()).await??;
    assert!(status.success());
    assert_eq!(tokio::fs::read(&out).await?, b"hello");

    let _ = tokio::fs::remove_dir_all(&dir).await;
    Ok(())
}

/// End-to-end against the real encoder: a growing Ogg/Opus source, the
/// production ffmpeg arguments, and the follower in between. `cat` cannot
/// show that ffmpeg is happy reading a pipe this process fills, or that
/// closing that pipe is enough for it to finalize its segments.
#[tokio::test]
async fn live_hls_output_covers_a_source_that_grows_while_ffmpeg_reads_it()
-> Result<(), Box<dyn std::error::Error>> {
    // CI runners have no ffmpeg; skip there like the other media-tool tests.
    let ffmpeg_available = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success());
    if !ffmpeg_available {
        return Ok(());
    }
    let dir = std::env::temp_dir().join(format!("sakiot-live-test-e2e-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir).await?;
    let complete = dir.join("complete.ogg");
    let src = dir.join("src.ogg");
    let out_dir = dir.join("hls");
    tokio::fs::create_dir_all(&out_dir).await?;

    // A six second Opus-in-Ogg recording, the shape the bot writes.
    let encode = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=6"])
        .args(["-c:a", "libopus", "-b:a", "64k"])
        .arg(&complete)
        .stderr(Stdio::piped())
        .output()
        .await?;
    assert!(encode.status.success(), "fixture encode failed");
    let payload = tokio::fs::read(&complete).await?;

    let mut c = Command::new("ffmpeg");
    c.arg("-hide_banner")
        .args(["-loglevel", "error"])
        .args(["-f", "ogg", "-i", "pipe:0"]);
    for a in ffmpeg_output_args(&out_dir, true) {
        c.arg(a);
    }
    let mut child = c
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let stdin = child.stdin.take().expect("stdin was piped");

    tokio::fs::write(&src, &[] as &[u8]).await?;
    let (stop_tx, stop_rx) = watch::channel(false);
    let follower = tokio::spawn(follow_source_into(
        src.clone(),
        stdin,
        stop_rx,
        "e2e-test".into(),
    ));

    // Reveal the recording in pieces, as the bot would.
    for piece in payload.chunks(payload.len() / 6 + 1) {
        append(&src, piece).await?;
        tokio::time::sleep(Duration::from_millis(120)).await;
    }

    drain_live_pipeline(&mut child, Some(&stop_tx), "e2e-test").await;
    follower.await?;

    let playlist = out_dir.join("playlist.m3u8");
    append_endlist(&playlist).await?;
    assert!(
        out_dir.join("init.mp4").exists(),
        "missing fMP4 init segment"
    );

    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration"])
        .args(["-of", "default=noprint_wrappers=1:nokey=1"])
        .arg(&playlist)
        .output()
        .await?;
    assert!(probe.status.success(), "ffprobe rejected the playlist");
    let duration: f64 = String::from_utf8(probe.stdout)?.trim().parse()?;
    assert!(
        (duration - 6.0).abs() < 0.5,
        "expected the full six seconds, got {duration}"
    );

    let _ = tokio::fs::remove_dir_all(&dir).await;
    Ok(())
}

#[tokio::test]
async fn stderr_drain_allows_noisy_child_to_exit() -> Result<(), Box<dyn std::error::Error>> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(
            "i=0; while [ \"$i\" -lt 20000 ]; do \
             echo 0123456789012345678901234567890123456789 >&2; \
             i=$((i + 1)); done",
        )
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    drain_child_stderr(&mut child, "stderr-drain-test".into());

    let status = tokio::time::timeout(Duration::from_secs(5), child.wait()).await??;

    assert!(status.success());
    Ok(())
}
