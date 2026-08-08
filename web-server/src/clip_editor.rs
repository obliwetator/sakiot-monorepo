use std::path::{Path, PathBuf};
use std::process::Stdio;

use actix_web::{HttpResponse, get, post, web};
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres, Row};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::error;

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::media_archive::MediaArchive;

use crate::audio::clips_path;
use crate::audio::types::WaveformProgressContainer;

const MAX_SEGMENTS: usize = 200;
const MAX_TRACKS: i32 = 32;
const MAX_TOTAL_SECONDS: f64 = 3600.0;
const MIN_SEGMENT_SECONDS: f32 = 0.05;
const VOLUME_MIN: f32 = -40.0;
const VOLUME_MAX: f32 = 12.0;
const PITCH_MIN: f32 = -1200.0;
const PITCH_MAX: f32 = 1200.0;
const RATE_MIN: f32 = 0.5;
const RATE_MAX: f32 = 2.0;
const SHELF_MIN: f32 = -12.0;
const SHELF_MAX: f32 = 12.0;
const SAMPLE_RATE: f64 = 48_000.0;
const MAX_FFMPEG_ERROR_BYTES: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ComposeClipBody {
    pub name: Option<String>,
    pub master_volume_db: f32,
    pub segments: Vec<ComposeSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ComposeSegment {
    pub source: String,
    pub source_id: String,
    pub source_in: f32,
    pub source_out: f32,
    pub timeline_start: f32,
    pub track: i32,
    pub effects: SegmentEffectsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SegmentEffectsDto {
    pub volume_db: f32,
    pub pitch_cents: f32,
    pub rate: f32,
    pub bass_db: f32,
    pub treble_db: f32,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ComposeClipAccepted {
    pub status: &'static str,
    pub progress: i16,
    pub id: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ComposeClipStatus {
    pub status: String,
    pub progress: i16,
}

struct ResolvedSource {
    path: PathBuf,
    channel_id: i64,
    length: f32,
}

struct SegmentRender {
    path: PathBuf,
    source_in: f32,
    source_out: f32,
    rate: f32,
    pitch_cents: f32,
    volume_db: f32,
    bass_db: f32,
    treble_db: f32,
    timeline_start: f32,
}

fn is_valid_clip_id(clip_id: &str) -> bool {
    !clip_id.is_empty()
        && !clip_id.contains("..")
        && !clip_id.contains('/')
        && !clip_id.contains('\\')
        && !clip_id.chars().any(char::is_control)
}

fn validate_effects(effects: &SegmentEffectsDto, index: usize) -> Result<(), AppError> {
    let all_finite = effects.volume_db.is_finite()
        && effects.pitch_cents.is_finite()
        && effects.rate.is_finite()
        && effects.bass_db.is_finite()
        && effects.treble_db.is_finite();
    if !all_finite {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: effect values must be finite"
        )));
    }
    if !(VOLUME_MIN..=VOLUME_MAX).contains(&effects.volume_db) {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: volume must be between {VOLUME_MIN} and {VOLUME_MAX} dB"
        )));
    }
    if !(PITCH_MIN..=PITCH_MAX).contains(&effects.pitch_cents) {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: pitch must be between {PITCH_MIN} and {PITCH_MAX} cents"
        )));
    }
    if !(RATE_MIN..=RATE_MAX).contains(&effects.rate) {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: rate must be between {RATE_MIN} and {RATE_MAX}"
        )));
    }
    if !(SHELF_MIN..=SHELF_MAX).contains(&effects.bass_db)
        || !(SHELF_MIN..=SHELF_MAX).contains(&effects.treble_db)
    {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: bass and treble must be between {SHELF_MIN} and {SHELF_MAX} dB"
        )));
    }
    Ok(())
}

fn validate_edit(body: &ComposeClipBody) -> Result<(), AppError> {
    if body.segments.is_empty() || body.segments.len() > MAX_SEGMENTS {
        return Err(AppError::BadRequest(format!(
            "Composition must contain between 1 and {MAX_SEGMENTS} segments"
        )));
    }
    if !body.master_volume_db.is_finite()
        || !(VOLUME_MIN..=VOLUME_MAX).contains(&body.master_volume_db)
    {
        return Err(AppError::BadRequest(format!(
            "Master volume must be between {VOLUME_MIN} and {VOLUME_MAX} dB"
        )));
    }
    for (index, segment) in body.segments.iter().enumerate() {
        if segment.source != "clip" {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: only \"clip\" sources are supported"
            )));
        }
        if !is_valid_clip_id(&segment.source_id) {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: invalid source clip id"
            )));
        }
        if !(0..=MAX_TRACKS).contains(&segment.track) {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: track must be between 0 and {MAX_TRACKS}"
            )));
        }
        if !segment.source_in.is_finite()
            || !segment.source_out.is_finite()
            || segment.source_in < 0.0
            || segment.source_out - segment.source_in < MIN_SEGMENT_SECONDS
        {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: source range is invalid"
            )));
        }
        if !segment.timeline_start.is_finite() || segment.timeline_start < 0.0 {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: timeline_start must be non-negative"
            )));
        }
        validate_effects(&segment.effects, index)?;
    }
    let total_seconds = body
        .segments
        .iter()
        .map(|segment| {
            f64::from(segment.timeline_start)
                + f64::from(segment.source_out - segment.source_in)
                    / f64::from(segment.effects.rate)
        })
        .fold(0.0, f64::max);
    if total_seconds > MAX_TOTAL_SECONDS {
        return Err(AppError::BadRequest(format!(
            "Composition is longer than {MAX_TOTAL_SECONDS} seconds"
        )));
    }
    Ok(())
}

async fn resolve_sources(
    pool: &web::Data<Pool<Postgres>>,
    media: &MediaArchive,
    guild_id: i64,
    user_id: i64,
    segments: &[ComposeSegment],
) -> Result<Vec<ResolvedSource>, AppError> {
    let mut resolved = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        let row = sqlx::query(
            "SELECT saved_file_name, channel_id, recording_session_id, length,
                    original_file_name
               FROM clips
              WHERE guild_id = $1 AND clip_id = $2 AND deleted_at IS NULL",
        )
        .bind(guild_id)
        .bind(&segment.source_id)
        .fetch_optional(pool.get_ref())
        .await?
        .ok_or(AppError::ClipNotFound)?;
        if row
            .try_get::<Option<String>, _>("original_file_name")?
            .as_deref()
            == Some("compose")
        {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: composed clips cannot be used as sources"
            )));
        }
        let recording_session_id: Option<i64> = row.try_get("recording_session_id")?;
        if let Some(recording_session_id) = recording_session_id {
            crate::audio::sessions::require_session_access(pool, recording_session_id, user_id)
                .await?;
        } else {
            let channel_id: Option<i64> = row.try_get("channel_id")?;
            crate::permissions::require_channel_access(
                pool,
                guild_id,
                channel_id.ok_or(AppError::ClipNotFound)?,
                user_id,
            )
            .await?;
        }
        let saved_file_name: Option<String> = row.try_get("saved_file_name")?;
        let saved_file_name = saved_file_name.ok_or(AppError::ClipNotFound)?;
        let path = crate::media_archive::clip_local_path(&saved_file_name)?;
        media
            .ensure_clip_local(pool.get_ref(), &segment.source_id, &path)
            .await?;
        let length: Option<f32> = row.try_get("length")?;
        let length = match length {
            Some(length) if length.is_finite() && length > 0.0 => length,
            _ => probe_duration(&path).await? as f32,
        };
        let channel_id: i64 = row
            .try_get::<Option<i64>, _>("channel_id")?
            .ok_or(AppError::ClipNotFound)?;
        resolved.push(ResolvedSource {
            path,
            channel_id,
            length,
        });
    }
    Ok(resolved)
}

#[utoipa::path(
    post,
    path = "/api/audio/clips/{guild_id}/compose",
    tag = "clips",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    request_body = ComposeClipBody,
    responses(
        (status = 202, description = "Clip composition started", body = ComposeClipAccepted),
        (status = 400, description = "Invalid composition request", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "Missing channel permission", body = crate::errors::ApiError),
        (status = 404, description = "Source clip not found", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/audio/clips/{guild_id}/compose")]
pub async fn compose_clip(
    path: web::Path<i64>,
    body: web::Json<ComposeClipBody>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    media: web::Data<MediaArchive>,
    progress: web::Data<WaveformProgressContainer>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let user_id = token.user_id;
    let guild_id = path.into_inner();

    validate_edit(&body)?;
    let resolved = resolve_sources(&pool, &media, guild_id, user_id, &body.segments).await?;
    for (segment, source) in body.segments.iter().zip(&resolved) {
        if segment.source_out > source.length + 0.1 {
            return Err(AppError::BadRequest(format!(
                "Segment source_out {:.3}s exceeds clip length {:.3}s",
                segment.source_out, source.length
            )));
        }
    }
    let channel_id = resolved
        .first()
        .map(|source| source.channel_id)
        .unwrap_or_default();

    let segments: Vec<SegmentRender> = body
        .segments
        .iter()
        .zip(resolved)
        .map(|(segment, source)| SegmentRender {
            path: source.path,
            source_in: segment.source_in,
            source_out: segment.source_out,
            rate: segment.effects.rate,
            pitch_cents: segment.effects.pitch_cents,
            volume_db: segment.effects.volume_db,
            bass_db: segment.effects.bass_db,
            treble_db: segment.effects.treble_db,
            timeline_start: segment.timeline_start,
        })
        .collect();
    let expected_total_ms = expected_duration_ms(&segments);
    let name = body
        .name
        .as_deref()
        .and_then(crate::clips::normalized_clip_name)
        .unwrap_or("composed-clip")
        .to_string();
    let master_volume_db = body.master_volume_db;
    let composition =
        serde_json::to_value(body.into_inner()).map_err(|_| AppError::InternalError)?;

    let clip_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let target_dir = PathBuf::from(clips_path())
        .join(now.year().to_string())
        .join(format!("{:02}", now.month()));
    tokio::fs::create_dir_all(&target_dir).await?;
    let full_path = target_dir.join(format!("{clip_id}.ogg"));
    let saved_file_name = format!("{}/{:02}/{clip_id}.ogg", now.year(), now.month());
    let cache_key = compose_progress_key(&clip_id);
    let clip_id_task = clip_id.clone();

    {
        let mut values = progress.0.write().await;
        if values.get(&cache_key).copied() == Some(-1) {
            values.remove(&cache_key);
        }
        values.insert(cache_key.clone(), 0);
    }

    let pool_clone = pool.clone();
    let progress_clone = progress.clone();
    tokio::spawn(async move {
        progress_clone.0.write().await.insert(cache_key.clone(), 1);
        let result = run_compose_job(
            &pool_clone,
            &segments,
            master_volume_db,
            &full_path,
            expected_total_ms,
            &progress_clone,
            &cache_key,
            &clip_id_task,
            guild_id,
            user_id,
            channel_id,
            &name,
            &saved_file_name,
            &composition,
        )
        .await;
        match result {
            Ok(()) => {
                progress_clone.0.write().await.remove(&cache_key);
            }
            Err(error) => {
                error!(clip_id = %clip_id_task, "clip composition failed: {}", error);
                let _ = tokio::fs::remove_file(&full_path).await;
                progress_clone.0.write().await.insert(cache_key, -1);
            }
        }
    });

    Ok(HttpResponse::Accepted().json(ComposeClipAccepted {
        status: "processing",
        progress: 0,
        id: clip_id,
    }))
}

#[utoipa::path(
    get,
    path = "/api/audio/clips/{guild_id}/compose/{clip_id}",
    tag = "clips",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("clip_id" = String, Path, description = "Composition clip id"),
    ),
    responses(
        (status = 200, description = "Composition status", body = ComposeClipStatus),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/clips/{guild_id}/compose/{clip_id}")]
pub async fn compose_clip_status(
    path: web::Path<(i64, String)>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    progress: web::Data<WaveformProgressContainer>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, clip_id) = path.into_inner();
    token.ok_or(AppError::Unauthorized)?;
    let cache_key = compose_progress_key(&clip_id);

    {
        let mut map = progress.0.write().await;
        if let Some(value) = map.get(&cache_key).copied() {
            if value < 0 {
                map.remove(&cache_key);
                return Ok(HttpResponse::Ok().json(ComposeClipStatus {
                    status: "failed".into(),
                    progress: 0,
                }));
            }
            return Ok(HttpResponse::Ok().json(ComposeClipStatus {
                status: "processing".into(),
                progress: value.min(99),
            }));
        }
    }

    let row = sqlx::query(
        "SELECT 1 FROM clips WHERE guild_id = $1 AND clip_id = $2 AND deleted_at IS NULL",
    )
    .bind(guild_id)
    .bind(&clip_id)
    .fetch_optional(pool.get_ref())
    .await?;
    if row.is_some() {
        return Ok(HttpResponse::Ok().json(ComposeClipStatus {
            status: "ready".into(),
            progress: 100,
        }));
    }
    Ok(HttpResponse::Ok().json(ComposeClipStatus {
        status: "idle".into(),
        progress: 0,
    }))
}

fn compose_progress_key(clip_id: &str) -> String {
    format!("clip-compose-{clip_id}")
}

fn expected_duration_ms(segments: &[SegmentRender]) -> i64 {
    let max_end = segments
        .iter()
        .map(|segment| {
            f64::from(segment.timeline_start)
                + f64::from(segment.source_out - segment.source_in) / effective_rate(segment)
        })
        .fold(0.0, f64::max);
    (max_end * 1_000.0).round() as i64
}

/// Combined playback factor of a segment: speed times the pitch shift, so a
/// pitch raise plays the content faster and a pitch drop slower, mirroring
/// the Web Audio computed playback rate (playbackRate * 2^(detune/1200)).
fn effective_rate(segment: &SegmentRender) -> f64 {
    f64::from(segment.rate) * pitch_factor(segment.pitch_cents)
}

fn pitch_factor(pitch_cents: f32) -> f64 {
    2f64.powf(f64::from(pitch_cents) / 1200.0)
}

#[allow(clippy::too_many_arguments)]
async fn run_compose_job(
    pool: &web::Data<Pool<Postgres>>,
    segments: &[SegmentRender],
    master_volume_db: f32,
    full_path: &Path,
    expected_total_ms: i64,
    progress: &web::Data<WaveformProgressContainer>,
    cache_key: &str,
    clip_id: &str,
    guild_id: i64,
    user_id: i64,
    channel_id: i64,
    name: &str,
    saved_file_name: &str,
    composition: &serde_json::Value,
) -> Result<(), AppError> {
    render_compose(
        segments,
        master_volume_db,
        full_path,
        expected_total_ms,
        progress,
        cache_key,
    )
    .await?;
    let duration = probe_duration(full_path).await?;
    let size = tokio::fs::metadata(full_path).await?.len() as i64;

    let insert = sqlx::query(
        "INSERT INTO clips
            (clip_id, length, size, channel_id, guild_id, user_id,
             original_file_name, saved_file_name, name, start_time, composition)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(clip_id)
    .bind(duration as f32)
    .bind(size)
    .bind(channel_id)
    .bind(guild_id)
    .bind(user_id)
    .bind("compose")
    .bind(saved_file_name)
    .bind(name)
    .bind(0.0f32)
    .bind(composition)
    .execute(pool.get_ref())
    .await;
    if let Err(err) = insert {
        return Err(AppError::DbError(err));
    }
    crate::audio::spawn_clip_waveform(
        clip_id.to_string(),
        full_path.to_path_buf(),
        progress.clone(),
    );
    Ok(())
}

async fn render_compose(
    segments: &[SegmentRender],
    master_volume_db: f32,
    output: &Path,
    expected_total_ms: i64,
    progress: &web::Data<WaveformProgressContainer>,
    cache_key: &str,
) -> Result<(), AppError> {
    let filter_graph = build_filter_graph(segments, master_volume_db);
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .arg("-y")
        .args(["-hide_banner", "-loglevel", "error"]);
    for segment in segments {
        command.arg("-i").arg(&segment.path);
    }
    command
        .args(["-filter_complex", &filter_graph, "-map", "[out]"])
        .args(["-c:a", "libopus", "-b:a", "96k"])
        .args(["-progress", "pipe:2", "-nostats"])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::ServiceUnavailable(
                "ffmpeg executable is unavailable; install FFmpeg on the web server".into(),
            )
        } else {
            AppError::IoError(error)
        }
    })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::FfmpegError("FFmpeg stderr pipe unavailable".into()))?;
    let mut lines = BufReader::new(stderr).lines();
    let mut error_output = Vec::new();
    while let Some(line) = lines.next_line().await.map_err(AppError::IoError)? {
        if let Some(value) = compose_progress_percent(&line, expected_total_ms) {
            let mut values = progress.0.write().await;
            let current = values.entry(cache_key.to_owned()).or_insert(0);
            if *current >= 0 && value > *current {
                *current = value;
            }
        } else if !is_ffmpeg_progress_line(&line) && error_output.len() < MAX_FFMPEG_ERROR_BYTES {
            let remaining = MAX_FFMPEG_ERROR_BYTES - error_output.len();
            error_output.extend_from_slice(&line.as_bytes()[..line.len().min(remaining)]);
            error_output.push(b'\n');
        }
    }

    let status = child.wait().await.map_err(AppError::IoError)?;
    if !status.success() {
        return Err(AppError::FfmpegError(
            String::from_utf8_lossy(&error_output).into_owned(),
        ));
    }
    Ok(())
}

fn db_to_linear(db: f32) -> f64 {
    10f64.powf(f64::from(db) / 20.0)
}

fn build_filter_graph(segments: &[SegmentRender], master_volume_db: f32) -> String {
    let mut graph = String::new();
    for (index, segment) in segments.iter().enumerate() {
        let combined_rate = effective_rate(segment);
        let volume = db_to_linear(segment.volume_db);
        let delay_ms = (f64::from(segment.timeline_start) * 1_000.0).round() as i64;
        graph.push_str(&format!(
            "[{index}:a]atrim=start={:.6}:end={:.6},asetrate={:.4},aresample={SAMPLE_RATE:.4},volume={volume:.6},bass=g={:.3}:f=250,treble=g={:.3}:f=3000,aresample={SAMPLE_RATE:.4},aformat=sample_fmts=fltp:channel_layouts=mono,adelay={delay_ms}:all=1[s{index}];",
            segment.source_in,
            segment.source_out,
            SAMPLE_RATE * combined_rate,
            segment.bass_db,
            segment.treble_db,
        ));
    }
    for index in 0..segments.len() {
        graph.push_str(&format!("[s{index}]"));
    }
    graph.push_str(&format!(
        "amix=inputs={}:duration=longest:normalize=0,volume={:.6}[out]",
        segments.len(),
        db_to_linear(master_volume_db),
    ));
    graph
}

fn compose_progress_percent(line: &str, total_ms: i64) -> Option<i16> {
    let elapsed_us = line.strip_prefix("out_time_us=")?.parse::<u64>().ok()?;
    let total_ms = u64::try_from(total_ms).ok()?.max(1);
    let progress = (elapsed_us / 1_000)
        .saturating_mul(99)
        .checked_div(total_ms)?
        .clamp(1, 99);
    i16::try_from(progress).ok()
}

fn is_ffmpeg_progress_line(line: &str) -> bool {
    matches!(
        line.split_once('=').map(|(key, _)| key),
        Some(
            "bitrate"
                | "drop_frames"
                | "dup_frames"
                | "fps"
                | "frame"
                | "out_time"
                | "out_time_ms"
                | "out_time_us"
                | "progress"
                | "speed"
                | "stream_0_0_q"
                | "total_size"
        )
    )
}

async fn probe_duration(path: &Path) -> Result<f64, AppError> {
    let probe = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AppError::ServiceUnavailable(
                    "ffprobe executable is unavailable; install FFmpeg on the web server".into(),
                )
            } else {
                AppError::IoError(error)
            }
        })?;
    if !probe.status.success() {
        return Err(AppError::FfmpegError(
            String::from_utf8_lossy(&probe.stderr).into_owned(),
        ));
    }
    String::from_utf8_lossy(&probe.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .ok_or_else(|| AppError::FfmpegError("ffprobe returned no audio duration".into()))
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_SEGMENTS, MAX_TOTAL_SECONDS, SegmentRender, build_filter_graph,
        compose_progress_percent, expected_duration_ms, is_valid_clip_id, validate_edit,
    };
    use crate::clip_editor::{ComposeClipBody, ComposeSegment, SegmentEffectsDto};
    use std::path::PathBuf;

    fn segment_render() -> SegmentRender {
        SegmentRender {
            path: PathBuf::from("src.ogg"),
            source_in: 1.0,
            source_out: 5.0,
            rate: 2.0,
            pitch_cents: 1200.0,
            volume_db: 6.0,
            bass_db: -3.0,
            treble_db: 3.0,
            timeline_start: 2.5,
        }
    }

    fn body(segments: Vec<ComposeSegment>) -> ComposeClipBody {
        ComposeClipBody {
            name: None,
            master_volume_db: 0.0,
            segments,
        }
    }

    fn segment() -> ComposeSegment {
        ComposeSegment {
            source: "clip".into(),
            source_id: "abc".into(),
            source_in: 0.0,
            source_out: 10.0,
            timeline_start: 0.0,
            track: 0,
            effects: SegmentEffectsDto {
                volume_db: 0.0,
                pitch_cents: 0.0,
                rate: 1.0,
                bass_db: 0.0,
                treble_db: 0.0,
            },
        }
    }

    #[test]
    fn builds_filter_graph_with_pitch_and_rate() {
        let graph = build_filter_graph(&[segment_render()], -6.0);
        assert!(graph.contains("atrim=start=1.000000:end=5.000000"));
        // pitch factor 2, rate 2 -> combined 4 -> 192 kHz
        assert!(graph.contains("asetrate=192000"));
        // the audible duration follows the combined rate (content / 4), so no
        // atempo compensation is applied; the box shows the same extent
        assert!(!graph.contains("atempo"));
        // 10^(6/20) = 1.995262
        assert!(graph.contains("volume=1.995262"));
        assert!(graph.contains("bass=g=-3.000:f=250"));
        assert!(graph.contains("treble=g=3.000:f=3000"));
        assert!(graph.contains("adelay=2500:all=1"));
        assert!(graph.contains("amix=inputs=1:duration=longest:normalize=0"));
        // master volume 10^(-6/20) = 0.501187
        assert!(graph.ends_with("volume=0.501187[out]"));
    }

    #[test]
    fn builds_filter_graph_without_effects() {
        let mut render = segment_render();
        render.rate = 1.0;
        render.pitch_cents = 0.0;
        render.volume_db = 0.0;
        let graph = build_filter_graph(&[render], 0.0);
        assert!(graph.contains("asetrate=48000"));
        assert!(graph.contains("volume=1.000000"));
    }

    #[test]
    fn mixes_two_segments_into_one_output() {
        let graph = build_filter_graph(&[segment_render(), segment_render()], 0.0);
        assert!(graph.contains("[s0][s1]amix=inputs=2:duration=longest:normalize=0"));
    }

    #[test]
    fn maps_ffmpeg_output_time_into_composition_progress() {
        assert_eq!(
            compose_progress_percent("out_time_us=5000000", 10_000),
            Some(49)
        );
        assert_eq!(
            compose_progress_percent("out_time_us=10000000", 10_000),
            Some(99)
        );
        assert_eq!(compose_progress_percent("progress=end", 10_000), None);
    }

    #[test]
    fn computes_expected_duration_from_the_longest_segment() {
        // content 1.0..5.0 = 4s at combined rate 2 * 2 (pitch one octave up)
        // = 4 -> 1s, so the timeline ends at 2.5 + 1 = 3.5s.
        assert_eq!(expected_duration_ms(&[segment_render()]), 3_500);
    }

    #[test]
    fn expected_duration_follows_pitch_and_speed() {
        let mut render = segment_render();
        render.pitch_cents = -1200.0; // half-speed playback
        render.source_in = 0.0;
        render.source_out = 10.0;
        render.timeline_start = 0.0;
        // 10s at rate 2 * pitch 0.5 = 1 -> 10s.
        assert_eq!(expected_duration_ms(&[render]), 10_000);
    }

    #[test]
    fn validates_clip_ids() {
        assert!(is_valid_clip_id("abc-123"));
        assert!(!is_valid_clip_id(""));
        assert!(!is_valid_clip_id("../secret"));
        assert!(!is_valid_clip_id("a/b"));
        assert!(!is_valid_clip_id("a\\b"));
        assert!(!is_valid_clip_id("a\nb"));
    }

    #[test]
    fn rejects_empty_and_oversized_compositions() {
        assert!(validate_edit(&body(vec![])).is_err());
        assert!(validate_edit(&body(vec![segment(); MAX_SEGMENTS + 1])).is_err());
    }

    #[test]
    fn rejects_non_clip_sources() {
        let mut seg = segment();
        seg.source = "session".into();
        assert!(validate_edit(&body(vec![seg])).is_err());
    }

    #[test]
    fn rejects_out_of_bounds_effects() {
        let mut seg = segment();
        seg.effects.volume_db = 13.0;
        assert!(validate_edit(&body(vec![seg.clone()])).is_err());
        seg.effects.volume_db = 0.0;
        seg.effects.pitch_cents = 1201.0;
        assert!(validate_edit(&body(vec![seg.clone()])).is_err());
        seg.effects.pitch_cents = 0.0;
        seg.effects.rate = 0.4;
        assert!(validate_edit(&body(vec![seg.clone()])).is_err());
        seg.effects.rate = 1.0;
        seg.effects.bass_db = -13.0;
        assert!(validate_edit(&body(vec![seg])).is_err());
    }

    #[test]
    fn rejects_invalid_source_ranges_and_timeline_starts() {
        let mut seg = segment();
        seg.source_out = 0.0;
        assert!(validate_edit(&body(vec![seg.clone()])).is_err());
        seg.source_out = 10.0;
        seg.timeline_start = -1.0;
        assert!(validate_edit(&body(vec![seg])).is_err());
    }

    #[test]
    fn rejects_compositions_longer_than_the_sanity_limit() {
        let mut seg = segment();
        seg.timeline_start = MAX_TOTAL_SECONDS as f32;
        assert!(validate_edit(&body(vec![seg])).is_err());
    }

    #[test]
    fn stored_edit_json_round_trips_the_request_shape() {
        let value = serde_json::to_value(body(vec![segment()])).unwrap();
        assert_eq!(value["master_volume_db"], 0.0);
        assert_eq!(value["segments"][0]["source"], "clip");
        assert_eq!(value["segments"][0]["source_id"], "abc");
        assert_eq!(value["segments"][0]["source_in"], 0.0);
        assert_eq!(value["segments"][0]["source_out"], 10.0);
        assert_eq!(value["segments"][0]["timeline_start"], 0.0);
        assert_eq!(value["segments"][0]["track"], 0);
        assert_eq!(value["segments"][0]["effects"]["volume_db"], 0.0);
        assert_eq!(value["segments"][0]["effects"]["pitch_cents"], 0.0);
        assert_eq!(value["segments"][0]["effects"]["rate"], 1.0);
        assert_eq!(value["segments"][0]["effects"]["bass_db"], 0.0);
        assert_eq!(value["segments"][0]["effects"]["treble_db"], 0.0);
    }
}
