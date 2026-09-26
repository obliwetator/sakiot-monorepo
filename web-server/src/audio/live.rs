//! On-demand HLS for a single per-user recording.
//!
//! First request for a recording's `playlist.m3u8` spawns ffmpeg that copies
//! (no re-encode) the source `.ogg` into fMP4 HLS segments. Output cached at
//! `{root}/{guild}/{ch}/{y}/{m}/hls-{stem}/`. Subsequent requests serve from
//! disk.
//!
//! While the recording is still being written (DB row has `end_ts IS NULL`
//! and a fresh recording heartbeat), a follower task reads the source as it
//! grows and feeds ffmpeg's stdin, so the playlist grows in real time. A
//! background task polls the DB; when the row is no longer live it signals the
//! follower, which reads the (now complete) source to EOF and closes ffmpeg's
//! stdin. ffmpeg flushes its final segment and exits, then we append
//! `ENDLIST`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use actix_files::NamedFile;
use actix_web::{HttpRequest, HttpResponse, Responder, get, http::header, web};
use sakiot_paths::RecordingKey;
use serde::Serialize;
use sqlx::{Pool, Postgres};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, RwLock, watch};
use tracing::{error, info, warn};

use crate::auth::{Access, Token};
use crate::errors::AppError;

use super::paths::recording_path;
use super::util::is_valid_file_segment;

pub(crate) const CACHE_ACCESS_MARKER: &str = ".sakiot-cache-access";
const CACHE_ACCESS_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Refreshes rebuildable-cache activity for cap eviction and stale reaping.
/// Best effort: serving media remains more important than marker persistence.
pub(crate) async fn mark_cache_access(directory: &Path) {
    let marker = directory.join(CACHE_ACCESS_MARKER);
    if let Ok(metadata) = tokio::fs::metadata(&marker).await
        && let Ok(modified) = metadata.modified()
        && modified
            .elapsed()
            .is_ok_and(|age| age < CACHE_ACCESS_REFRESH_INTERVAL)
    {
        return;
    }
    if let Err(error) =
        tokio::fs::write(marker, chrono::Utc::now().timestamp_millis().to_string()).await
    {
        warn!(path = %directory.display(), ?error, "cache access marker update failed");
    }
}

/// How long the follower waits before re-checking a source that has stopped
/// growing while the recording is still live.
const FOLLOW_POLL: Duration = Duration::from_millis(200);
/// Read size for the source follower.
const FOLLOW_CHUNK: usize = 64 * 1024;
/// How long ffmpeg gets to flush and exit after its stdin closes.
const PIPELINE_EXIT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long after SIGTERM before escalating to SIGKILL.
const PIPELINE_KILL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default, Debug)]
pub struct LiveContainer {
    pub(crate) jobs: RwLock<HashMap<String, Arc<Mutex<JobState>>>>,
    /// Per-key creation locks: only one job may spawn per recording, so two
    /// concurrent first requests cannot start duplicate ffmpeg pipelines into
    /// the same `hls-{stem}` directory. Locks live as long as the job map;
    /// retaining them also serializes retries after a failed spawn.
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl LiveContainer {
    /// Serializes job creation for one recording. The returned guard is held
    /// for the whole spawn; while it is held, any other request for the same
    /// key waits and then reuses the completed entry.
    async fn key_lock(&self, id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().await;
        locks
            .entry(id.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Drops a creation lock once nobody is waiting on it. The job map already
    /// deduplicates later requests, so keeping one lock per recording ever
    /// streamed would grow without bound.
    async fn release_key_lock(&self, id: &str, lock: Arc<Mutex<()>>) {
        drop(lock);
        let mut locks = self.locks.lock().await;
        if locks
            .get(id)
            .is_some_and(|current| Arc::strong_count(current) == 1)
        {
            locks.remove(id);
        }
    }
}

#[derive(Debug)]
pub struct JobState {
    pub finalized: bool,
    pub child: Option<Child>,
    /// Tells the source follower the recording is complete, so it reads to
    /// true EOF and closes ffmpeg's stdin. `None` for VOD jobs, which read a
    /// finished file directly.
    follow_stop: Option<watch::Sender<bool>>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct StateResponse {
    pub live: bool,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

struct DbRecordingState {
    start_ts: Option<i64>,
    end_ts: Option<i64>,
    live: bool,
}

fn key_id(k: &RecordingKey) -> String {
    format!(
        "{}/{}/{:04}/{:02}/{}",
        k.guild_id, k.channel_id, k.year, k.month, k.stem
    )
}

fn validate_stem(s: &str) -> Result<(), AppError> {
    if is_valid_file_segment(s) {
        Ok(())
    } else {
        Err(AppError::BadRequest("Invalid stem".into()))
    }
}

fn validate_seg(s: &str) -> Result<(), AppError> {
    if is_valid_file_segment(s) {
        Ok(())
    } else {
        Err(AppError::BadRequest("Invalid segment name".into()))
    }
}

async fn source_path(k: &RecordingKey) -> Option<PathBuf> {
    let recording_root = recording_path();
    let padded = k.recording_path(&recording_root);
    if tokio::fs::try_exists(&padded).await.unwrap_or(false) {
        return Some(padded);
    }
    let root = recording_root.trim_end_matches('/');
    let unpadded = PathBuf::from(root)
        .join(format!(
            "{}/{}/{}/{}",
            k.guild_id, k.channel_id, k.year, k.month
        ))
        .join(format!("{}.ogg", k.stem));
    if tokio::fs::try_exists(&unpadded).await.unwrap_or(false) {
        Some(unpadded)
    } else {
        None
    }
}

/// Probe the audio codec of `src`. Returns the lowercase codec name
/// (e.g. "opus", "vorbis"). On any ffprobe failure returns Err.
async fn probe_codec(src: &Path) -> Result<String, AppError> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "csv=p=0",
        ])
        .arg(src)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(AppError::IoError)?;
    if !out.status.success() {
        return Err(AppError::FfmpegError("ffprobe failed".into()));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .trim()
        .to_ascii_lowercase())
}

async fn db_state(pool: &Pool<Postgres>, stem: &str) -> Result<DbRecordingState, AppError> {
    let row = sqlx::query!(
        "SELECT af.start_ts,
                af.end_ts,
                (
                    af.end_ts IS NULL
                    AND af.reaped IS FALSE
                    AND EXISTS (
                        SELECT 1
                          FROM bot_instances bi
                         WHERE bi.instance_id = af.recording_owner_instance_id
                           AND af.recording_heartbeat_at > now() - interval '120 seconds'
                           AND bi.heartbeat_at > now() - interval '120 seconds'
                           AND bi.state <> 'stopped'
                    )
                ) AS live
           FROM audio_files af
          WHERE af.file_name = $1",
        stem
    )
    .fetch_optional(pool)
    .await?;
    Ok(row
        .map(|r| DbRecordingState {
            start_ts: r.start_ts,
            end_ts: r.end_ts,
            live: r.live.unwrap_or(false),
        })
        .unwrap_or(DbRecordingState {
            start_ts: None,
            end_ts: None,
            live: false,
        }))
}

async fn playlist_finalized(p: &Path) -> bool {
    matches!(tokio::fs::read_to_string(p).await, Ok(s) if s.contains("#EXT-X-ENDLIST"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HlsCacheAction {
    ReuseFinalized,
    PurgeStale,
    BuildFresh,
}

async fn hls_cache_action(playlist: &Path) -> HlsCacheAction {
    if !tokio::fs::try_exists(playlist).await.unwrap_or(false) {
        return HlsCacheAction::BuildFresh;
    }

    if playlist_finalized(playlist).await {
        return HlsCacheAction::ReuseFinalized;
    }

    // A playlist without #EXT-X-ENDLIST means the previous ffmpeg run (live
    // or VOD) never finished — e.g. a crash between spawn and the ENDLIST
    // append. The rebuild cannot reuse it: the VOD command answers prompts
    // with stdin null, so ffmpeg would refuse to overwrite and exit,
    // finalizing the dead playlist. Purge and rebuild from the source.
    HlsCacheAction::PurgeStale
}

async fn append_endlist(p: &Path) -> std::io::Result<()> {
    let mut content = tokio::fs::read_to_string(p).await?;
    if content.contains("#EXT-X-ENDLIST") {
        return Ok(());
    }
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str("#EXT-X-ENDLIST\n");
    tokio::fs::write(p, content).await
}

/// Build the ffmpeg command tail (everything past the input args).
fn ffmpeg_output_args(out_dir: &Path, live: bool) -> Vec<String> {
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

fn drain_child_stderr(child: &mut Child, job_id: String) {
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
async fn follow_source_into(
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
async fn drain_live_pipeline(child: &mut Child, stop: Option<&watch::Sender<bool>>, job_id: &str) {
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

async fn spawn_job(
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
            .map_err(AppError::IoError)?;

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
            .map_err(AppError::IoError)?
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

    // Lifecycle task.
    let state_c = state.clone();
    let pool_c = pool.clone();
    let stem = key.stem.clone();
    let out_dir_c = out_dir.clone();
    tokio::spawn(async move {
        if is_live {
            // Poll DB until the row is no longer lease-backed live, then kill
            // the pipeline.
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
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

pub(crate) async fn ensure_job(
    container: web::Data<LiveContainer>,
    pool: web::Data<Pool<Postgres>>,
    key: RecordingKey,
) -> Result<Arc<Mutex<JobState>>, AppError> {
    let id = key_id(&key);
    if let Some(s) = container.jobs.read().await.get(&id).cloned() {
        return Ok(s);
    }

    // Serialize creation for this key: a concurrent first request waits here
    // and then reuses the entry the winner inserts, so only one ffmpeg
    // pipeline writes the shared `hls-{stem}` directory.
    let key_guard = container.key_lock(&id).await;
    let result = {
        let _key_guard = key_guard.lock().await;
        if let Some(s) = container.jobs.read().await.get(&id).cloned() {
            Ok(s)
        } else {
            ensure_job_locked(container.clone(), pool, key).await
        }
    };
    container.release_key_lock(&id, key_guard).await;
    result
}

async fn ensure_job_locked(
    container: web::Data<LiveContainer>,
    pool: web::Data<Pool<Postgres>>,
    key: RecordingKey,
) -> Result<Arc<Mutex<JobState>>, AppError> {
    let id = key_id(&key);
    let src = source_path(&key).await.ok_or(AppError::FileNotFound)?;

    // Probe BEFORE the on-disk cache shortcut: a stale `hls-*` dir from a
    // pre-gate run can otherwise serve vorbis-in-fmp4 that MSE refuses,
    // making hls.js spin on seg_00000.
    match probe_codec(&src).await {
        Ok(c) if c == "opus" => {}
        Ok(c) => {
            info!(stem = %key.stem, codec = %c, "non-opus input; HLS unsupported");
            return Err(AppError::BadRequest(format!("unsupported codec: {}", c)));
        }
        Err(e) => {
            error!(stem = %key.stem, error = ?e, "ffprobe failed");
            return Err(AppError::FfmpegError("codec probe failed".into()));
        }
    }

    let recording_root = recording_path();
    let out_dir = key.live_dir(&recording_root);
    let playlist = out_dir.join("playlist.m3u8");
    let db = db_state(&pool, &key.stem).await?;
    let is_live = db.live;

    match hls_cache_action(&playlist).await {
        HlsCacheAction::ReuseFinalized => {
            let s = Arc::new(Mutex::new(JobState {
                finalized: true,
                child: None,
                follow_stop: None,
            }));
            container.jobs.write().await.insert(id, s.clone());
            return Ok(s);
        }
        HlsCacheAction::PurgeStale => {
            warn!(
                stem = %key.stem,
                path = %out_dir.display(),
                "purging stale non-finalized HLS cache before rebuild"
            );
            tokio::fs::remove_dir_all(&out_dir)
                .await
                .map_err(AppError::IoError)?;
        }
        HlsCacheAction::BuildFresh => {}
    }

    spawn_job(container, pool, key, src, out_dir, is_live).await
}

#[utoipa::path(
    get,
    path = "/api/audio/live/{guild_id}/{channel_id}/{year}/{month}/{stem}/playlist.m3u8",
    tag = "audio",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("channel_id" = i64, Path, description = "Discord channel id"),
        ("year" = i32, Path, description = "Recording year"),
        ("month" = u32, Path, description = "Recording month"),
        ("stem" = String, Path, description = "Recording file stem"),
    ),
    responses(
        (status = 200, description = "HLS playlist for the live recording", content_type = "application/vnd.apple.mpegurl"),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 404, description = "Live recording not found", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/live/{guild_id}/{channel_id}/{year}/{month}/{stem}/playlist.m3u8")]
pub async fn live_playlist(
    path: web::Path<(i64, i64, i32, u32, String)>,
    container: web::Data<LiveContainer>,
    pool: web::Data<Pool<Postgres>>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, channel_id, year, month, stem) = path.into_inner();
    validate_stem(&stem)?;
    let token = token.ok_or(AppError::Unauthorized)?;
    let month_i32 = i32::try_from(month)
        .map_err(|_| AppError::InvalidParam("invalid recording month".to_owned()))?;
    super::sessions::require_recording_access(
        &pool,
        guild_id,
        channel_id,
        year,
        month_i32,
        &stem,
        token.user_id,
    )
    .await?;
    let key = RecordingKey::new(guild_id, channel_id, year, month, stem);
    let _ = ensure_job(container, pool, key.clone()).await?;
    mark_cache_access(&key.live_dir(&recording_path())).await;
    let pl = key.live_playlist_path(&recording_path());
    let body = tokio::fs::read(&pl)
        .await
        .map_err(|_| AppError::FileNotFound)?;
    let final_ = std::str::from_utf8(&body)
        .map(|s| s.contains("#EXT-X-ENDLIST"))
        .unwrap_or(false);
    let cache = if final_ {
        "public, max-age=300"
    } else {
        "no-cache"
    };
    Ok(HttpResponse::Ok()
        .content_type("application/vnd.apple.mpegurl")
        .insert_header((header::CACHE_CONTROL, cache))
        .body(body))
}

#[utoipa::path(
    get,
    path = "/api/audio/live/{guild_id}/{channel_id}/{year}/{month}/{stem}/state",
    tag = "audio",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("channel_id" = i64, Path, description = "Discord channel id"),
        ("year" = i32, Path, description = "Recording year"),
        ("month" = u32, Path, description = "Recording month"),
        ("stem" = String, Path, description = "Recording file stem"),
    ),
    responses(
        (status = 200, description = "Live recording state", body = StateResponse),
        (status = 400, description = "Invalid stem", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "Missing channel permission", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/live/{guild_id}/{channel_id}/{year}/{month}/{stem}/state")]
pub async fn live_state(
    path: web::Path<(i64, i64, i32, u32, String)>,
    pool: web::Data<Pool<Postgres>>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, channel_id, year, month, stem) = path.into_inner();
    validate_stem(&stem)?;
    let token = token.ok_or(AppError::Unauthorized)?;
    let month = i32::try_from(month)
        .map_err(|_| AppError::InvalidParam("invalid recording month".to_owned()))?;
    super::sessions::require_recording_access(
        &pool,
        guild_id,
        channel_id,
        year,
        month,
        &stem,
        token.user_id,
    )
    .await?;
    let db = db_state(&pool, &stem).await?;
    // A finalized recording with no end timestamp has an unknown end. Falling
    // back to the start timestamp reported every such recording as zero
    // seconds long, which the UI showed as an empty timeline.
    let ended_at = db.end_ts;
    Ok(HttpResponse::Ok().json(StateResponse {
        live: db.live,
        started_at: db.start_ts,
        ended_at,
    }))
}

#[utoipa::path(
    get,
    path = "/api/audio/live/{guild_id}/{channel_id}/{year}/{month}/{stem}/{seg}",
    tag = "audio",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("channel_id" = i64, Path, description = "Discord channel id"),
        ("year" = i32, Path, description = "Recording year"),
        ("month" = u32, Path, description = "Recording month"),
        ("stem" = String, Path, description = "Recording file stem"),
        ("seg" = String, Path, description = "HLS segment name"),
    ),
    responses(
        (status = 200, description = "HLS media segment", content_type = "video/mp2t"),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 404, description = "Segment not found", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/live/{guild_id}/{channel_id}/{year}/{month}/{stem}/{seg}")]
pub async fn live_segment(
    req: HttpRequest,
    path: web::Path<(i64, i64, i32, u32, String, String)>,
    pool: web::Data<Pool<Postgres>>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<impl Responder, AppError> {
    let (guild_id, channel_id, year, month, stem, seg) = path.into_inner();
    validate_stem(&stem)?;
    validate_seg(&seg)?;
    let token = token.ok_or(AppError::Unauthorized)?;
    let month_i32 = i32::try_from(month)
        .map_err(|_| AppError::InvalidParam("invalid recording month".to_owned()))?;
    super::sessions::require_recording_access(
        &pool,
        guild_id,
        channel_id,
        year,
        month_i32,
        &stem,
        token.user_id,
    )
    .await?;
    if seg == "playlist.m3u8" || seg == "state" {
        return Err(AppError::BadRequest("reserved name".into()));
    }
    let key = RecordingKey::new(guild_id, channel_id, year, month, stem);
    mark_cache_access(&key.live_dir(&recording_path())).await;
    let path = key.live_segment_path(&recording_path(), &seg);
    let f = NamedFile::open_async(&path)
        .await
        .map_err(|_| AppError::FileNotFound)?;
    let mut resp = f.into_response(&req);
    let cache = if seg.starts_with("seg_") {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=3600"
    };
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static(cache),
    );
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn hls_cache_action_reuses_finalized_playlist() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir =
            std::env::temp_dir().join(format!("sakiot-live-test-final-{}", uuid::Uuid::new_v4()));
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
        let dir =
            std::env::temp_dir().join(format!("sakiot-live-test-stale-{}", uuid::Uuid::new_v4()));
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
    fn live_ffmpeg_flags_do_not_append_existing_playlist() -> Result<(), Box<dyn std::error::Error>>
    {
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
    async fn drain_live_pipeline_preserves_all_source_bytes()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir =
            std::env::temp_dir().join(format!("sakiot-live-test-drain-{}", uuid::Uuid::new_v4()));
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
        let dir =
            std::env::temp_dir().join(format!("sakiot-live-test-grow-{}", uuid::Uuid::new_v4()));
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
        let dir =
            std::env::temp_dir().join(format!("sakiot-live-test-eof-{}", uuid::Uuid::new_v4()));
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
        let dir =
            std::env::temp_dir().join(format!("sakiot-live-test-e2e-{}", uuid::Uuid::new_v4()));
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
}
