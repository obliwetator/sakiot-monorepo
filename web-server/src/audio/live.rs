//! On-demand HLS for a single per-user recording.
//!
//! First request for a recording's `playlist.m3u8` spawns ffmpeg that copies
//! (no re-encode) the source `.ogg` into fMP4 HLS segments. Output cached at
//! `{root}/{guild}/{ch}/{y}/{m}/hls-{stem}/`. Subsequent requests serve from
//! disk.
//!
//! While the recording is still being written (DB row has `end_ts IS NULL`
//! and a fresh recording heartbeat), ffmpeg consumes a `tail -F` of the source
//! so the playlist grows in real time. A background task polls the DB; when
//! the row is no longer live it waits for tail to reach the end of the (now
//! complete) source, stops tail so ffmpeg drains to EOF and exits, then we
//! append `ENDLIST`.

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
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

use crate::auth::{Access, Token};
use crate::errors::AppError;

use super::paths::recording_path;

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

/// How long the drain waits for tail to reach the end of the finalized
/// source file before terminating it anyway.
const TAIL_CATCHUP_TIMEOUT: Duration = Duration::from_secs(15);
/// Poll interval for the tail catch-up wait.
const TAIL_CATCHUP_POLL: Duration = Duration::from_millis(250);
/// How long the pipeline gets to drain and exit after tail terminates.
const PIPELINE_EXIT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long after a group SIGTERM before escalating to SIGKILL.
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
}

#[derive(Debug)]
pub struct JobState {
    pub finalized: bool,
    pub child: Option<Child>,
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
    if s.is_empty() || s.contains('/') || s.contains("..") || s.contains('\\') || s.contains('\'') {
        return Err(AppError::BadRequest("Invalid stem".into()));
    }
    Ok(())
}

fn validate_seg(s: &str) -> Result<(), AppError> {
    if s.is_empty() || s.contains('/') || s.contains("..") || s.contains('\\') {
        return Err(AppError::BadRequest("Invalid segment name".into()));
    }
    Ok(())
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
    PurgeStaleLive,
    BuildFresh,
}

async fn hls_cache_action(playlist: &Path, is_live: bool) -> HlsCacheAction {
    if !tokio::fs::try_exists(playlist).await.unwrap_or(false) {
        return HlsCacheAction::BuildFresh;
    }

    if playlist_finalized(playlist).await {
        return HlsCacheAction::ReuseFinalized;
    }

    if is_live {
        return HlsCacheAction::PurgeStaleLive;
    }

    HlsCacheAction::BuildFresh
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

/// Parses `/proc/<pid>/stat` into (comm, pgrp). The comm field is
/// parenthesized and may itself contain spaces or parentheses, so it ends at
/// the *last* `)`.
fn parse_stat_comm_pgrp(stat: &str) -> Option<(&str, i64)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let comm = stat.get(open + 1..close)?;
    // Fields after the comm: state, ppid, pgrp, ...
    let pgrp = stat.get(close + 1..)?.split_whitespace().nth(2)?;
    Some((comm, pgrp.parse().ok()?))
}

/// The `pos:` line of a /proc fdinfo file.
fn parse_fdinfo_pos(fdinfo: &str) -> Option<u64> {
    fdinfo.lines().find_map(|line| {
        line.strip_prefix("pos:")
            .and_then(|v| v.trim().parse().ok())
    })
}

/// Finds a process in group `pgid` with command name `comm` by scanning
/// /proc. procfs reads are memory-backed and never wait on storage, so plain
/// std::fs is fine on the runtime here and below.
fn find_group_member(pgid: u32, comm: &str) -> Option<u32> {
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        if let Some((proc_comm, proc_pgrp)) = parse_stat_comm_pgrp(&stat)
            && proc_comm == comm
            && proc_pgrp == i64::from(pgid)
        {
            return Some(pid);
        }
    }
    None
}

/// The read offset of `pid`'s open fd on `canonical` (a canonicalized path),
/// from /proc fdinfo. None when the process or fd is gone.
fn proc_read_pos(pid: u32, canonical: &Path) -> Option<u64> {
    for entry in std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?.flatten() {
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        if target != canonical {
            continue;
        }
        let fd = entry.file_name();
        let fdinfo =
            std::fs::read_to_string(format!("/proc/{pid}/fdinfo/{}", fd.to_string_lossy())).ok()?;
        return parse_fdinfo_pos(&fdinfo);
    }
    None
}

/// Waits until `tail_pid` has read `src` to the end twice in a row (the
/// second observation gives tail time to flush the final chunk into the
/// pipe), bounded by TAIL_CATCHUP_TIMEOUT.
async fn wait_for_tail_catchup(tail_pid: u32, src: &Path, job_id: &str) {
    let Ok(canonical) = tokio::fs::canonicalize(src).await else {
        warn!(stem = %job_id, "cannot canonicalize source; skipping tail catch-up wait");
        return;
    };
    let deadline = std::time::Instant::now() + TAIL_CATCHUP_TIMEOUT;
    let mut caught_up_once = false;
    loop {
        let len = tokio::fs::metadata(src).await.ok().map(|meta| meta.len());
        match (proc_read_pos(tail_pid, &canonical), len) {
            (Some(pos), Some(len)) if pos >= len => {
                if caught_up_once {
                    return;
                }
                caught_up_once = true;
            }
            (None, _) if !Path::new(&format!("/proc/{tail_pid}")).exists() => {
                // tail already exited; nothing left to wait for.
                return;
            }
            _ => caught_up_once = false,
        }
        if std::time::Instant::now() >= deadline {
            warn!(stem = %job_id, "tail did not catch up with the source before timeout");
            return;
        }
        tokio::time::sleep(TAIL_CATCHUP_POLL).await;
    }
}

/// Terminates the live `tail -F | ffmpeg` pipeline without truncating the
/// output. The source is already complete (the bot closes its writer before
/// the DB row stops being live), so wait for tail to read to EOF, then
/// SIGTERM tail alone: ffmpeg sees EOF on stdin, flushes the final partial
/// segment, and the pipeline exits on its own. The process group is signalled
/// only as an escalation. Replaces a fixed 2-second sleep that could truncate
/// the end of a recording on a slow host.
async fn drain_live_pipeline(child: &mut Child, src: &Path, job_id: &str) {
    let Some(pgid) = child.id() else {
        // Already exited; just reap it.
        let _ = child.wait().await;
        return;
    };

    // The pipeline is `setsid sh -c "tail -F … | ffmpeg …"`: tail and ffmpeg
    // run as siblings in the group whose pgid is child.id() (setsid makes the
    // leader's pid the pgid). tokio's Child::kill would only signal the
    // shell, orphaning both — hence raw kill(2) here and below.
    if let Some(tail_pid) = find_group_member(pgid, "tail") {
        wait_for_tail_catchup(tail_pid, src, job_id).await;
        // SAFETY: SIGTERM to the single pid located above; if it exited in
        // the meantime the signal is a harmless ESRCH.
        unsafe {
            libc::kill(tail_pid as i32, libc::SIGTERM);
        }
    } else {
        warn!(stem = %job_id, "tail not found in pipeline group; terminating whole group");
        // SAFETY: negative pid signals the whole process group; ffmpeg
        // treats SIGTERM as a graceful quit and still writes its trailer.
        unsafe {
            libc::kill(-(pgid as i32), libc::SIGTERM);
        }
    }

    if tokio::time::timeout(PIPELINE_EXIT_TIMEOUT, child.wait())
        .await
        .is_ok()
    {
        return;
    }
    warn!(stem = %job_id, "pipeline did not exit after drain; terminating group");
    // SAFETY: group SIGTERM, as above.
    unsafe {
        libc::kill(-(pgid as i32), libc::SIGTERM);
    }
    if tokio::time::timeout(PIPELINE_KILL_TIMEOUT, child.wait())
        .await
        .is_err()
    {
        warn!(stem = %job_id, "pipeline ignored SIGTERM; killing group");
        // SAFETY: last-resort group SIGKILL.
        unsafe {
            libc::kill(-(pgid as i32), libc::SIGKILL);
        }
        let _ = child.wait().await;
    }
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
    let mut child = if is_live {
        // Shell pipeline so we don't have to wire ChildStdout -> Stdio manually.
        // `exec` on the ffmpeg side means the shell's pid IS ffmpeg's pid once
        // tail starts producing bytes — but tail still runs as a sibling under
        // the shell's process group. We kill the whole group via setsid below.
        let src_q = src.to_string_lossy().replace('\'', "'\\''");
        let mut ff_args = vec![
            "-hide_banner".into(),
            "-loglevel".into(),
            "warning".into(),
            "-f".into(),
            "ogg".into(),
            "-i".into(),
            "pipe:0".into(),
        ];
        ff_args.extend(ffmpeg_output_args(&out_dir, true));
        // Shell-quote each ffmpeg arg.
        let ff_quoted = ff_args
            .iter()
            .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
            .collect::<Vec<_>>()
            .join(" ");
        let cmd = format!("tail -F -c +0 -- '{}' | ffmpeg {}", src_q, ff_quoted);

        Command::new("setsid")
            .arg("sh")
            .arg("-c")
            .arg(&cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(AppError::IoError)?
    } else {
        let mut c = Command::new("ffmpeg");
        c.arg("-hide_banner")
            .args(["-loglevel", "warning"])
            .arg("-i")
            .arg(&src);
        for a in ffmpeg_output_args(&out_dir, false) {
            c.arg(a);
        }
        c.stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(AppError::IoError)?
    };
    drain_child_stderr(&mut child, id.clone());

    let state = Arc::new(Mutex::new(JobState {
        finalized: false,
        child: Some(child),
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
                drain_live_pipeline(&mut child, &src, &id).await;
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
    let _key_guard = key_guard.lock().await;
    if let Some(s) = container.jobs.read().await.get(&id).cloned() {
        return Ok(s);
    }

    ensure_job_locked(container, pool, key).await
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

    match hls_cache_action(&playlist, is_live).await {
        HlsCacheAction::ReuseFinalized => {
            let s = Arc::new(Mutex::new(JobState {
                finalized: true,
                child: None,
            }));
            container.jobs.write().await.insert(id, s.clone());
            return Ok(s);
        }
        HlsCacheAction::PurgeStaleLive => {
            warn!(
                stem = %key.stem,
                path = %out_dir.display(),
                "purging stale non-finalized live HLS cache before respawn"
            );
            tokio::fs::remove_dir_all(&out_dir)
                .await
                .map_err(AppError::IoError)?;
        }
        HlsCacheAction::BuildFresh => {}
    }

    spawn_job(container, pool, key, src, out_dir, is_live).await
}

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
    let ended_at = db.end_ts.or(if db.live { None } else { db.start_ts });
    Ok(HttpResponse::Ok().json(StateResponse {
        live: db.live,
        started_at: db.start_ts,
        ended_at,
    }))
}

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
    async fn hls_cache_action_builds_when_playlist_missing() {
        let dir =
            std::env::temp_dir().join(format!("sakiot-live-test-missing-{}", uuid::Uuid::new_v4()));
        let playlist = dir.join("playlist.m3u8");

        assert_eq!(
            hls_cache_action(&playlist, true).await,
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
            hls_cache_action(&playlist, true).await,
            HlsCacheAction::ReuseFinalized
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
        Ok(())
    }

    #[tokio::test]
    async fn hls_cache_action_purges_unfinalized_live_playlist()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir =
            std::env::temp_dir().join(format!("sakiot-live-test-stale-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await?;
        let playlist = dir.join("playlist.m3u8");
        tokio::fs::write(&playlist, "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:EVENT\n").await?;

        assert_eq!(
            hls_cache_action(&playlist, true).await,
            HlsCacheAction::PurgeStaleLive
        );
        assert_eq!(
            hls_cache_action(&playlist, false).await,
            HlsCacheAction::BuildFresh
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

    #[test]
    fn stat_parser_handles_awkward_comm_names() {
        let stat = "1234 (tail) S 1 5678 5678 0 -1 4194304 0";
        assert_eq!(parse_stat_comm_pgrp(stat), Some(("tail", 5678)));

        // comm may contain spaces and parentheses; it ends at the last ')'.
        let nested = "99 ((sd-pam) x) S 1 42 42 0 -1";
        assert_eq!(parse_stat_comm_pgrp(nested), Some(("(sd-pam) x", 42)));

        assert_eq!(parse_stat_comm_pgrp("garbage"), None);
    }

    #[test]
    fn fdinfo_parser_reads_pos() {
        let fdinfo = "pos:\t123456\nflags:\t0100000\nmnt_id:\t29\n";
        assert_eq!(parse_fdinfo_pos(fdinfo), Some(123456));
        assert_eq!(parse_fdinfo_pos("flags:\t0\n"), None);
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

        // Same shape as the production pipeline, with cat standing in for
        // ffmpeg so the assertion is byte-exact.
        let cmd = format!(
            "tail -F -c +0 -- '{}' | cat > '{}'",
            src.display(),
            out.display()
        );
        let mut child = Command::new("setsid")
            .arg("sh")
            .arg("-c")
            .arg(&cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;

        // Let the pipeline start, then append the "last writes" the old
        // fixed 2s sleep used to race against.
        tokio::time::sleep(Duration::from_millis(300)).await;
        {
            use tokio::io::AsyncWriteExt;
            let mut f = tokio::fs::OpenOptions::new()
                .append(true)
                .open(&src)
                .await?;
            f.write_all(&payload).await?;
            f.flush().await?;
        }

        drain_live_pipeline(&mut child, &src, "drain-test").await;

        let written = tokio::fs::read(&out).await?;
        assert_eq!(written.len(), payload.len() * 2);

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
