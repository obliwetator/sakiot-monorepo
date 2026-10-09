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
//!
//! A live job also stops once nobody has asked for its playlist for
//! [`LIVE_IDLE_STOP`]: each one holds an ffmpeg process, and a recording
//! somebody listened to once would otherwise keep one until the recording
//! ends. The next request rebuilds the output from the start of the recording.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::PoisonError;
use std::time::{Duration, Instant};

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

mod pipeline;
mod playlist;
mod source;

pub(crate) use playlist::starting_playlist;

use pipeline::*;
use playlist::*;
use source::*;

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
/// How often a live job checks whether its recording is still live and
/// whether anyone still listens.
const LIVE_POLL: Duration = Duration::from_secs(5);
/// How long a live job keeps its ffmpeg after the last playlist request.
/// Players re-read a live playlist every target duration (2 s), so a minute
/// without one means nobody is listening.
const LIVE_IDLE_STOP: Duration = Duration::from_secs(60);

#[derive(Default, Debug)]
pub struct LiveContainer {
    pub(crate) jobs: RwLock<HashMap<String, Arc<Mutex<JobState>>>>,
    /// Per-key creation locks: only one job may spawn per recording, so two
    /// concurrent first requests cannot start duplicate ffmpeg pipelines into
    /// the same `hls-{stem}` directory. Locks live as long as the job map;
    /// retaining them also serializes retries after a failed spawn.
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// When each running live job's playlist was last requested. Only running
    /// live jobs have an entry, so finished recordings do not accumulate.
    requested: std::sync::Mutex<HashMap<String, Instant>>,
}

impl LiveContainer {
    fn requested(&self) -> std::sync::MutexGuard<'_, HashMap<String, Instant>> {
        self.requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts tracking playlist requests for a live job that has just started.
    fn track_requests(&self, id: &str) {
        self.requested().insert(id.to_owned(), Instant::now());
    }

    /// Records a playlist request. A no-op unless the job is running live.
    fn touch(&self, id: &str) {
        if let Some(at) = self.requested().get_mut(id) {
            *at = Instant::now();
        }
    }

    /// Time since the job's playlist was last requested. Zero for a job that
    /// is not tracked, so only a running live job is ever considered idle.
    fn idle_for(&self, id: &str) -> Duration {
        self.requested()
            .get(id)
            .map_or(Duration::ZERO, Instant::elapsed)
    }

    fn untrack_requests(&self, id: &str) {
        self.requested().remove(id);
    }

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

/// Start (or reuse) the HLS job behind a live playlist request. `None` while
/// the recording is live but its file has no audio yet: the handler answers
/// [`starting_playlist`] and the player asks again, instead of the probe
/// failing on the empty file. Only a recording's first request checks.
/// Every call counts as a listener for the job's idle stop.
pub(crate) async fn ensure_playlist_job(
    container: web::Data<LiveContainer>,
    pool: web::Data<Pool<Postgres>>,
    key: RecordingKey,
) -> Result<Option<Arc<Mutex<JobState>>>, AppError> {
    let id = key_id(&key);
    let started = container.jobs.read().await.contains_key(&id);
    if !started
        && let Some(src) = source_path(&key).await
        && !has_first_audio_page(&src)
            .await
            .map_err(AppError::IoError)?
        && db_state(&pool, &key.stem).await?.live
    {
        return Ok(None);
    }
    let job = ensure_job(container.clone(), pool, key).await?;
    container.touch(&id);
    Ok(Some(job))
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
        (status = 200, description = "HLS playlist for the live recording; live and empty until its first audio is written", content_type = "application/vnd.apple.mpegurl"),
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
        crate::permissions::Viewer::of(&token),
    )
    .await?;
    let key = RecordingKey::new(guild_id, channel_id, year, month, stem);
    if ensure_playlist_job(container, pool, key.clone())
        .await?
        .is_none()
    {
        return Ok(starting_playlist());
    }
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
        crate::permissions::Viewer::of(&token),
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
        crate::permissions::Viewer::of(&token),
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
mod tests;
