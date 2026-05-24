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
//! the row is no longer live it kills the shell pipeline (tail's parent),
//! ffmpeg drains, then we append `ENDLIST`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use actix_files::NamedFile;
use actix_web::{get, http::header, web, HttpRequest, HttpResponse, Responder};
use sakiot_paths::RecordingKey;
use serde::Serialize;
use sqlx::{Pool, Postgres};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

use crate::errors::AppError;

use super::paths::RECORDING_PATH;

#[derive(Default, Debug)]
pub struct LiveContainer(pub RwLock<HashMap<String, Arc<Mutex<JobState>>>>);

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

fn source_path(k: &RecordingKey) -> Option<PathBuf> {
    let padded = k.recording_path(RECORDING_PATH);
    if padded.exists() {
        return Some(padded);
    }
    let root = RECORDING_PATH.trim_end_matches('/');
    let unpadded = PathBuf::from(root)
        .join(format!(
            "{}/{}/{}/{}",
            k.guild_id, k.channel_id, k.year, k.month
        ))
        .join(format!("{}.ogg", k.stem));
    if unpadded.exists() {
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
    if !playlist.exists() {
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

    let child = if is_live {
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

    let state = Arc::new(Mutex::new(JobState {
        finalized: false,
        child: Some(child),
    }));
    container
        .0
        .write()
        .await
        .insert(key_id(&key), state.clone());

    // Lifecycle task.
    let state_c = state.clone();
    let pool_c = pool.clone();
    let stem = key.stem.clone();
    let id = key_id(&key);
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
            // Give ffmpeg a moment to consume tail's last writes.
            tokio::time::sleep(Duration::from_secs(2)).await;
            let mut g = state_c.lock().await;
            if let Some(mut child) = g.child.take() {
                // Kill the whole process group, not just the shell. The live
                // pipeline is `setsid sh -c "tail -F … | ffmpeg …"`, so tail
                // and ffmpeg run as siblings under the shell's pgid (setsid
                // makes child.id() == pgid). tokio's Child::kill only signals
                // the direct child (the shell), which would orphan tail +
                // ffmpeg. `kill(-pgid, SIGTERM)` signals every process in the
                // group — that's what `libc` is here for.
                if let Some(pid) = child.id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGTERM);
                    }
                }
                let _ = child.wait().await;
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
        if pl.exists() && init.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(state)
}

async fn ensure_job(
    container: web::Data<LiveContainer>,
    pool: web::Data<Pool<Postgres>>,
    key: RecordingKey,
) -> Result<Arc<Mutex<JobState>>, AppError> {
    let id = key_id(&key);
    if let Some(s) = container.0.read().await.get(&id).cloned() {
        return Ok(s);
    }

    let src = source_path(&key).ok_or(AppError::FileNotFound)?;

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

    let out_dir = key.live_dir(RECORDING_PATH);
    let playlist = out_dir.join("playlist.m3u8");
    let db = db_state(&pool, &key.stem).await?;
    let is_live = db.live;

    match hls_cache_action(&playlist, is_live).await {
        HlsCacheAction::ReuseFinalized => {
            let s = Arc::new(Mutex::new(JobState {
                finalized: true,
                child: None,
            }));
            container.0.write().await.insert(id, s.clone());
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

    {
        let r = container.0.read().await;
        if let Some(s) = r.get(&id).cloned() {
            return Ok(s);
        }
    }
    spawn_job(container, pool, key, src, out_dir, is_live).await
}

#[get("/audio/live/{guild_id}/{channel_id}/{year}/{month}/{stem}/playlist.m3u8")]
pub async fn live_playlist(
    path: web::Path<(i64, i64, i32, u32, String)>,
    container: web::Data<LiveContainer>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, channel_id, year, month, stem) = path.into_inner();
    validate_stem(&stem)?;
    let key = RecordingKey::new(guild_id, channel_id, year, month, stem);
    let _ = ensure_job(container, pool, key.clone()).await?;
    let pl = key.live_playlist_path(RECORDING_PATH);
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
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/live/{guild_id}/{channel_id}/{year}/{month}/{stem}/state")]
pub async fn live_state(
    path: web::Path<(i64, i64, i32, u32, String)>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let (_g, _c, _y, _m, stem) = path.into_inner();
    validate_stem(&stem)?;
    let db = db_state(&pool, &stem).await?;
    let ended_at = db
        .end_ts
        .or_else(|| if db.live { None } else { db.start_ts });
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
) -> Result<impl Responder, AppError> {
    let (guild_id, channel_id, year, month, stem, seg) = path.into_inner();
    validate_stem(&stem)?;
    validate_seg(&seg)?;
    if seg == "playlist.m3u8" || seg == "state" {
        return Err(AppError::BadRequest("reserved name".into()));
    }
    let key = RecordingKey::new(guild_id, channel_id, year, month, stem);
    let path = key.live_segment_path(RECORDING_PATH, &seg);
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
    async fn hls_cache_action_purges_unfinalized_live_playlist(
    ) -> Result<(), Box<dyn std::error::Error>> {
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
}
