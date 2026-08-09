use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::{fs::File, io::BufWriter, io::Write};

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
const NORMALIZED_MIN: f32 = 0.0;
const NORMALIZED_MAX: f32 = 1.0;
const DELAY_MAX_SECONDS: f32 = 5.0;
const MID_FREQUENCY_HZ: u16 = 1_000;
const SAMPLE_RATE: f64 = 48_000.0;
const MAX_FFMPEG_ERROR_BYTES: usize = 4096;
// The offline phase-vocoder path keeps one segment in memory. Longer source
// windows retain the existing FFmpeg/Rubber Band renderer until the shared DSP
// gains a streaming length-changing API.
const MAX_SHARED_DSP_SEGMENT_SECONDS: f32 = 60.0;
const OUTPUT_CHANNELS: usize = 2;

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
    /// Id of the merged unit this segment belongs to, when the clip editor
    /// merged several snapped segments into one element. Segments sharing an
    /// id re-import as one unit. Purely an editing hint: the render ignores
    /// it. Optional so compositions from before the field existed still
    /// deserialize.
    #[serde(default)]
    pub merge_group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SegmentEffectsDto {
    pub volume_db: f32,
    pub pitch_cents: f32,
    pub rate: f32,
    pub bass_db: f32,
    /// 1 kHz peaking EQ gain. Defaults to zero for saved compositions from
    /// before the parity-matched mid control existed.
    #[serde(default)]
    pub mid_db: f32,
    pub treble_db: f32,
    /// Plays the trimmed content backwards. Defaults to false so requests and
    /// stored compositions from before the flag existed still deserialize.
    #[serde(default)]
    pub reverse: bool,
    /// Stateful and modulation effects added by the shared DSP integration.
    /// The entire group defaults to bypass-compatible DSP defaults for older
    /// saved compositions.
    #[serde(default)]
    pub advanced: AdvancedSegmentEffectsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct AdvancedSegmentEffectsDto {
    /// Fixed silence appended after reverse/pitch/rate and processed through
    /// the shared effect chain, making effect ring-out part of the timeline.
    pub tail_seconds: f32,
    pub distortion_amount: f32,
    pub distortion_wet: f32,
    pub delay_seconds: f32,
    pub delay_feedback: f32,
    pub delay_wet: f32,
    pub compressor_enabled: bool,
    pub compressor_threshold_db: f32,
    pub compressor_knee_db: f32,
    pub compressor_ratio: f32,
    pub compressor_attack_seconds: f32,
    pub compressor_release_seconds: f32,
    pub chorus_enabled: bool,
    pub chorus_frequency_hz: f32,
    pub chorus_delay_ms: f32,
    pub chorus_depth: f32,
    pub chorus_spread_degrees: f32,
    pub chorus_feedback: f32,
    pub chorus_wet: f32,
    pub reverb_enabled: bool,
    pub reverb_decay_seconds: f32,
    pub reverb_pre_delay_seconds: f32,
    pub reverb_wet: f32,
    pub reverb_seed: u32,
}

impl Default for AdvancedSegmentEffectsDto {
    fn default() -> Self {
        let effects = sakiot_dsp::SegmentEffects::default();
        Self {
            tail_seconds: effects.tail_seconds,
            distortion_amount: effects.distortion_amount,
            distortion_wet: effects.distortion_wet,
            delay_seconds: effects.delay_seconds,
            delay_feedback: effects.delay_feedback,
            delay_wet: effects.delay_wet,
            compressor_enabled: effects.compressor_enabled,
            compressor_threshold_db: effects.compressor_threshold_db,
            compressor_knee_db: effects.compressor_knee_db,
            compressor_ratio: effects.compressor_ratio,
            compressor_attack_seconds: effects.compressor_attack_seconds,
            compressor_release_seconds: effects.compressor_release_seconds,
            chorus_enabled: effects.chorus_enabled,
            chorus_frequency_hz: effects.chorus_frequency_hz,
            chorus_delay_ms: effects.chorus_delay_ms,
            chorus_depth: effects.chorus_depth,
            chorus_spread_degrees: effects.chorus_spread_degrees,
            chorus_feedback: effects.chorus_feedback,
            chorus_wet: effects.chorus_wet,
            reverb_enabled: effects.reverb_enabled,
            reverb_decay_seconds: effects.reverb_decay_seconds,
            reverb_pre_delay_seconds: effects.reverb_pre_delay_seconds,
            reverb_wet: effects.reverb_wet,
            reverb_seed: effects.reverb_seed,
        }
    }
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
    effects: sakiot_dsp::SegmentEffects,
    timeline_start: f32,
}

struct TemporaryRawFiles {
    paths: Vec<PathBuf>,
}

impl Drop for TemporaryRawFiles {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn is_valid_clip_id(clip_id: &str) -> bool {
    !clip_id.is_empty()
        && !clip_id.contains("..")
        && !clip_id.contains('/')
        && !clip_id.contains('\\')
        && !clip_id.chars().any(char::is_control)
}

fn validate_effects(effects: &SegmentEffectsDto, index: usize) -> Result<(), AppError> {
    let advanced = &effects.advanced;
    let all_finite = effects.volume_db.is_finite()
        && effects.pitch_cents.is_finite()
        && effects.rate.is_finite()
        && effects.bass_db.is_finite()
        && effects.mid_db.is_finite()
        && effects.treble_db.is_finite()
        && advanced.tail_seconds.is_finite()
        && advanced.distortion_amount.is_finite()
        && advanced.distortion_wet.is_finite()
        && advanced.delay_seconds.is_finite()
        && advanced.delay_feedback.is_finite()
        && advanced.delay_wet.is_finite()
        && advanced.compressor_threshold_db.is_finite()
        && advanced.compressor_knee_db.is_finite()
        && advanced.compressor_ratio.is_finite()
        && advanced.compressor_attack_seconds.is_finite()
        && advanced.compressor_release_seconds.is_finite()
        && advanced.chorus_frequency_hz.is_finite()
        && advanced.chorus_delay_ms.is_finite()
        && advanced.chorus_depth.is_finite()
        && advanced.chorus_spread_degrees.is_finite()
        && advanced.chorus_feedback.is_finite()
        && advanced.chorus_wet.is_finite()
        && advanced.reverb_decay_seconds.is_finite()
        && advanced.reverb_pre_delay_seconds.is_finite()
        && advanced.reverb_wet.is_finite();
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
        || !(SHELF_MIN..=SHELF_MAX).contains(&effects.mid_db)
        || !(SHELF_MIN..=SHELF_MAX).contains(&effects.treble_db)
    {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: EQ gains must be between {SHELF_MIN} and {SHELF_MAX} dB"
        )));
    }
    if !(0.0..=sakiot_dsp::MAX_EFFECT_TAIL_SECONDS).contains(&advanced.tail_seconds) {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: effect tail must be between 0 and {} seconds",
            sakiot_dsp::MAX_EFFECT_TAIL_SECONDS
        )));
    }
    if !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.distortion_amount)
        || !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.distortion_wet)
    {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: distortion amount and wet must be between 0 and 1"
        )));
    }
    if !(0.0..=DELAY_MAX_SECONDS).contains(&advanced.delay_seconds)
        || !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.delay_feedback)
        || !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.delay_wet)
    {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: delay parameters are outside the supported ranges"
        )));
    }
    if !(-100.0..=0.0).contains(&advanced.compressor_threshold_db)
        || !(0.0..=40.0).contains(&advanced.compressor_knee_db)
        || !(1.0..=20.0).contains(&advanced.compressor_ratio)
        || !(0.0..=1.0).contains(&advanced.compressor_attack_seconds)
        || !(0.0..=1.0).contains(&advanced.compressor_release_seconds)
    {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: compressor parameters are outside the supported ranges"
        )));
    }
    if !(0.0..=20.0).contains(&advanced.chorus_frequency_hz)
        || !(0.0..=100.0).contains(&advanced.chorus_delay_ms)
        || !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.chorus_depth)
        || !(0.0..=360.0).contains(&advanced.chorus_spread_degrees)
        || !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.chorus_feedback)
        || !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.chorus_wet)
    {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: chorus parameters are outside the supported ranges"
        )));
    }
    if !(0.001..=30.0).contains(&advanced.reverb_decay_seconds)
        || !(0.0..=5.0).contains(&advanced.reverb_pre_delay_seconds)
        || !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.reverb_wet)
    {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: reverb parameters are outside the supported ranges"
        )));
    }
    Ok(())
}

fn advanced_effects_active(effects: &SegmentEffectsDto) -> bool {
    effects.advanced.tail_seconds > 0.0
        || effects.advanced.distortion_wet > 0.0
        || effects.advanced.delay_wet > 0.0
        || effects.advanced.compressor_enabled
        || effects.advanced.chorus_enabled
        || effects.advanced.reverb_enabled
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
        if let Some(group) = &segment.merge_group
            && !is_valid_clip_id(group)
        {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: invalid merge group id"
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
        if segment.source_out - segment.source_in > MAX_SHARED_DSP_SEGMENT_SECONDS
            && advanced_effects_active(&segment.effects)
        {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: shared tail, distortion, delay, compressor, chorus, and reverb currently support source windows up to {MAX_SHARED_DSP_SEGMENT_SECONDS} seconds"
            )));
        }
    }
    let total_seconds = body
        .segments
        .iter()
        .map(|segment| {
            f64::from(segment.timeline_start)
                + f64::from(segment.source_out - segment.source_in)
                    / f64::from(segment.effects.rate)
                + f64::from(segment.effects.advanced.tail_seconds)
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
            effects: shared_effects_from_dto(&segment.effects),
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
                + f64::from(segment.effects.tail_seconds)
        })
        .fold(0.0, f64::max);
    (max_end * 1_000.0).round() as i64
}

/// Timeline consumption rate of a segment. Pitch shifting preserves duration,
/// so only the speed control changes the visible and exported extent.
fn effective_rate(segment: &SegmentRender) -> f64 {
    f64::from(segment.effects.rate)
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
    if segments
        .iter()
        .any(|segment| segment.source_out - segment.source_in > MAX_SHARED_DSP_SEGMENT_SECONDS)
    {
        return render_compose_legacy(
            segments,
            master_volume_db,
            output,
            expected_total_ms,
            progress,
            cache_key,
        )
        .await;
    }

    render_compose_shared(
        segments,
        master_volume_db,
        output,
        expected_total_ms,
        progress,
        cache_key,
    )
    .await
}

async fn render_compose_shared(
    segments: &[SegmentRender],
    master_volume_db: f32,
    output: &Path,
    expected_total_ms: i64,
    progress: &web::Data<WaveformProgressContainer>,
    cache_key: &str,
) -> Result<(), AppError> {
    let raw_files = prepare_shared_dsp_segments(segments, output).await?;
    let filter_graph = build_shared_mix_graph(segments, master_volume_db);
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .arg("-y")
        .args(["-hide_banner", "-loglevel", "error"]);
    for path in &raw_files.paths {
        command
            .args(["-f", "f32le", "-ar", "48000", "-ac", "2", "-i"])
            .arg(path);
    }
    command
        .args(["-filter_complex", &filter_graph, "-map", "[out]"])
        .args(["-c:a", "libopus", "-b:a", "96k"])
        .args(["-progress", "pipe:2", "-nostats"])
        .arg(output);
    run_ffmpeg_with_progress(command, expected_total_ms, progress, cache_key).await
}

async fn render_compose_legacy(
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
        .arg(output);
    run_ffmpeg_with_progress(command, expected_total_ms, progress, cache_key).await
}

async fn run_ffmpeg_with_progress(
    mut command: tokio::process::Command,
    expected_total_ms: i64,
    progress: &web::Data<WaveformProgressContainer>,
    cache_key: &str,
) -> Result<(), AppError> {
    command
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

async fn prepare_shared_dsp_segments(
    segments: &[SegmentRender],
    output: &Path,
) -> Result<TemporaryRawFiles, AppError> {
    let mut files = TemporaryRawFiles { paths: Vec::new() };
    for (index, segment) in segments.iter().enumerate() {
        let path = temporary_segment_path(output, index);
        files.paths.push(path.clone());
        let input = decode_segment_f32(segment).await?;
        let effects = shared_segment_effects(segment);
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let rendered =
                sakiot_dsp::render_clip_interleaved(&input, SAMPLE_RATE, OUTPUT_CHANNELS, effects)
                    .map_err(|error| error.to_string())?;
            let file = File::create(&path).map_err(|error| error.to_string())?;
            let mut writer = BufWriter::new(file);
            for sample in rendered {
                writer
                    .write_all(&sample.to_le_bytes())
                    .map_err(|error| error.to_string())?;
            }
            writer.flush().map_err(|error| error.to_string())
        })
        .await
        .map_err(|_| AppError::InternalError)?
        .map_err(|error| AppError::FfmpegError(format!("shared DSP failed: {error}")))?;
    }
    Ok(files)
}

async fn decode_segment_f32(segment: &SegmentRender) -> Result<Vec<f32>, AppError> {
    let start_frame = (f64::from(segment.source_in) * SAMPLE_RATE).round() as u64;
    let end_frame = (f64::from(segment.source_out) * SAMPLE_RATE).round() as u64;
    let filter = format!(
        "aresample={SAMPLE_RATE:.0},atrim=start_sample={start_frame}:end_sample={end_frame},asetpts=PTS-STARTPTS,aformat=sample_fmts=flt:channel_layouts=stereo"
    );
    let output = tokio::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(&segment.path)
        .args([
            "-map",
            "0:a:0",
            "-af",
            &filter,
            "-ar",
            "48000",
            "-ac",
            "2",
            "-c:a",
            "pcm_f32le",
            "-f",
            "f32le",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AppError::ServiceUnavailable(
                    "ffmpeg executable is unavailable; install FFmpeg on the web server".into(),
                )
            } else {
                AppError::IoError(error)
            }
        })?;
    if !output.status.success() {
        return Err(AppError::FfmpegError(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    if !output
        .stdout
        .len()
        .is_multiple_of(size_of::<f32>() * OUTPUT_CHANNELS)
    {
        return Err(AppError::FfmpegError(
            "decoded segment returned incomplete stereo f32 frames".into(),
        ));
    }
    Ok(output
        .stdout
        .chunks_exact(size_of::<f32>())
        .map(|sample| f32::from_le_bytes(sample.try_into().unwrap_or_default()))
        .collect())
}

fn temporary_segment_path(output: &Path, index: usize) -> PathBuf {
    let stem = output
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("compose");
    output.with_file_name(format!(".{stem}.dsp-segment-{index}.f32"))
}

fn shared_segment_effects(segment: &SegmentRender) -> sakiot_dsp::SegmentEffects {
    segment.effects
}

fn shared_effects_from_dto(effects: &SegmentEffectsDto) -> sakiot_dsp::SegmentEffects {
    let advanced = &effects.advanced;
    sakiot_dsp::SegmentEffects {
        volume_db: effects.volume_db,
        pitch_cents: effects.pitch_cents,
        rate: effects.rate,
        tail_seconds: advanced.tail_seconds,
        bass_db: effects.bass_db,
        mid_db: effects.mid_db,
        treble_db: effects.treble_db,
        distortion_amount: advanced.distortion_amount,
        distortion_wet: advanced.distortion_wet,
        delay_seconds: advanced.delay_seconds,
        delay_feedback: advanced.delay_feedback,
        delay_wet: advanced.delay_wet,
        compressor_enabled: advanced.compressor_enabled,
        compressor_threshold_db: advanced.compressor_threshold_db,
        compressor_knee_db: advanced.compressor_knee_db,
        compressor_ratio: advanced.compressor_ratio,
        compressor_attack_seconds: advanced.compressor_attack_seconds,
        compressor_release_seconds: advanced.compressor_release_seconds,
        chorus_enabled: advanced.chorus_enabled,
        chorus_frequency_hz: advanced.chorus_frequency_hz,
        chorus_delay_ms: advanced.chorus_delay_ms,
        chorus_depth: advanced.chorus_depth,
        chorus_spread_degrees: advanced.chorus_spread_degrees,
        chorus_feedback: advanced.chorus_feedback,
        chorus_wet: advanced.chorus_wet,
        reverb_enabled: advanced.reverb_enabled,
        reverb_decay_seconds: advanced.reverb_decay_seconds,
        reverb_pre_delay_seconds: advanced.reverb_pre_delay_seconds,
        reverb_wet: advanced.reverb_wet,
        reverb_seed: advanced.reverb_seed,
        reverse: effects.reverse,
    }
}

fn build_shared_mix_graph(segments: &[SegmentRender], master_volume_db: f32) -> String {
    let mut graph = String::new();
    for (index, segment) in segments.iter().enumerate() {
        let delay_ms = (f64::from(segment.timeline_start) * 1_000.0).round() as i64;
        graph.push_str(&format!("[{index}:a]adelay={delay_ms}:all=1[s{index}];"));
    }
    for index in 0..segments.len() {
        graph.push_str(&format!("[s{index}]"));
    }
    graph.push_str(&format!(
        "amix=inputs={}:duration=longest:normalize=0,aformat=sample_fmts=fltp:channel_layouts=stereo,volume={:.6}[out]",
        segments.len(),
        db_to_linear(master_volume_db),
    ));
    graph
}

fn db_to_linear(db: f32) -> f64 {
    10f64.powf(f64::from(db) / 20.0)
}

fn build_filter_graph(segments: &[SegmentRender], master_volume_db: f32) -> String {
    let mut graph = String::new();
    for (index, segment) in segments.iter().enumerate() {
        let tempo = effective_rate(segment);
        let pitch = pitch_factor(segment.effects.pitch_cents);
        let time_pitch = if (tempo - 1.0).abs() < f64::EPSILON && (pitch - 1.0).abs() < f64::EPSILON
        {
            String::new()
        } else {
            format!(",rubberband=tempo={tempo:.6}:pitch={pitch:.6}")
        };
        let volume = db_to_linear(segment.effects.volume_db);
        let delay_ms = (f64::from(segment.timeline_start) * 1_000.0).round() as i64;
        // Reversed segments trim the source window first, then flip it, so the
        // audible content is exactly the [source_in, source_out] window played
        // backwards - the same as the client's negative playbackRate preview.
        let reverse = if segment.effects.reverse {
            ",areverse"
        } else {
            ""
        };
        graph.push_str(&format!(
            "[{index}:a]atrim=start={:.6}:end={:.6}{reverse},aresample={SAMPLE_RATE:.4}{time_pitch},volume={volume:.6},bass=g={:.3}:f=250:t=s:w=1,equalizer=g={:.3}:f={MID_FREQUENCY_HZ}:t=q:w=1,treble=g={:.3}:f=3000:t=s:w=1,aresample={SAMPLE_RATE:.4},aformat=sample_fmts=fltp:channel_layouts=mono,adelay={delay_ms}:all=1[s{index}];",
            segment.source_in,
            segment.source_out,
            segment.effects.bass_db,
            segment.effects.mid_db,
            segment.effects.treble_db,
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
        DELAY_MAX_SECONDS, MAX_SEGMENTS, MAX_SHARED_DSP_SEGMENT_SECONDS, MAX_TOTAL_SECONDS,
        SegmentRender, build_filter_graph, build_shared_mix_graph, compose_progress_percent,
        expected_duration_ms, is_valid_clip_id, probe_duration, render_compose_shared,
        shared_effects_from_dto, validate_edit,
    };
    use crate::audio::types::WaveformProgressContainer;
    use crate::clip_editor::{
        AdvancedSegmentEffectsDto, ComposeClipBody, ComposeSegment, SegmentEffectsDto,
    };
    use actix_web::web;
    use std::{collections::HashMap, path::PathBuf};

    fn segment_render() -> SegmentRender {
        SegmentRender {
            path: PathBuf::from("src.ogg"),
            source_in: 1.0,
            source_out: 5.0,
            effects: sakiot_dsp::SegmentEffects {
                rate: 2.0,
                pitch_cents: 1200.0,
                volume_db: 6.0,
                bass_db: -3.0,
                mid_db: 2.0,
                treble_db: 3.0,
                ..sakiot_dsp::SegmentEffects::default()
            },
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
                mid_db: 0.0,
                treble_db: 0.0,
                reverse: false,
                advanced: AdvancedSegmentEffectsDto::default(),
            },
            merge_group: None,
        }
    }

    #[test]
    fn builds_filter_graph_with_pitch_and_rate() {
        let graph = build_filter_graph(&[segment_render()], -6.0);
        assert!(graph.contains("atrim=start=1.000000:end=5.000000"));
        // Tempo and pitch are independent: 2x is half the duration while
        // +1200 cents raises the result one octave without another resize.
        assert!(graph.contains("rubberband=tempo=2.000000:pitch=2.000000"));
        // 10^(6/20) = 1.995262
        assert!(graph.contains("volume=1.995262"));
        assert!(graph.contains("bass=g=-3.000:f=250:t=s:w=1"));
        assert!(graph.contains("equalizer=g=2.000:f=1000:t=q:w=1"));
        assert!(graph.contains("treble=g=3.000:f=3000:t=s:w=1"));
        assert!(graph.contains("adelay=2500:all=1"));
        assert!(graph.contains("amix=inputs=1:duration=longest:normalize=0"));
        // master volume 10^(-6/20) = 0.501187
        assert!(graph.ends_with("volume=0.501187[out]"));
    }

    #[test]
    fn builds_filter_graph_without_effects() {
        let mut render = segment_render();
        render.effects.rate = 1.0;
        render.effects.pitch_cents = 0.0;
        render.effects.volume_db = 0.0;
        let graph = build_filter_graph(&[render], 0.0);
        assert!(!graph.contains("rubberband"));
        assert!(graph.contains("volume=1.000000"));
    }

    #[test]
    fn builds_filter_graph_with_reverse_after_the_trim() {
        let mut render = segment_render();
        render.effects.reverse = true;
        let graph = build_filter_graph(&[render], 0.0);
        // The window is trimmed first and then flipped, so the audible
        // content is [source_in, source_out] played backwards.
        assert!(graph.contains("atrim=start=1.000000:end=5.000000,areverse"));
        assert!(graph.contains("adelay=2500:all=1"));
    }

    #[test]
    fn forward_segments_omit_the_reverse_filter() {
        let graph = build_filter_graph(&[segment_render()], 0.0);
        assert!(!graph.contains("areverse"));
    }

    #[test]
    fn mixes_two_segments_into_one_output() {
        let graph = build_filter_graph(&[segment_render(), segment_render()], 0.0);
        assert!(graph.contains("[s0][s1]amix=inputs=2:duration=longest:normalize=0"));
    }

    #[test]
    fn shared_mix_graph_only_places_and_sums_preprocessed_segments() {
        let graph = build_shared_mix_graph(&[segment_render(), segment_render()], -6.0);
        assert!(graph.contains("[0:a]adelay=2500:all=1[s0]"));
        assert!(graph.contains("[1:a]adelay=2500:all=1[s1]"));
        assert!(graph.contains("[s0][s1]amix=inputs=2:duration=longest:normalize=0"));
        assert!(graph.contains("channel_layouts=stereo"));
        assert!(graph.ends_with("volume=0.501187[out]"));
        assert!(!graph.contains("rubberband"));
        assert!(!graph.contains("equalizer"));
    }

    #[test]
    fn maps_product_effects_into_the_shared_contract() {
        let mut dto = segment().effects;
        dto.volume_db = 6.0;
        dto.pitch_cents = 700.0;
        dto.rate = 1.35;
        dto.bass_db = -3.0;
        dto.mid_db = 2.0;
        dto.treble_db = 3.0;
        dto.reverse = true;
        dto.advanced.tail_seconds = 2.0;
        dto.advanced.distortion_amount = 0.8;
        dto.advanced.distortion_wet = 0.6;
        dto.advanced.delay_seconds = 0.4;
        dto.advanced.delay_feedback = 0.35;
        dto.advanced.delay_wet = 0.25;
        dto.advanced.compressor_enabled = true;
        dto.advanced.chorus_enabled = true;
        dto.advanced.reverb_enabled = true;
        dto.advanced.reverb_seed = 42;
        let effects = shared_effects_from_dto(&dto);
        assert_eq!(effects.volume_db, 6.0);
        assert_eq!(effects.pitch_cents, 700.0);
        assert_eq!(effects.rate, 1.35);
        assert_eq!(effects.bass_db, -3.0);
        assert_eq!(effects.mid_db, 2.0);
        assert_eq!(effects.treble_db, 3.0);
        assert!(effects.reverse);
        assert_eq!(effects.tail_seconds, 2.0);
        assert_eq!(effects.distortion_amount, 0.8);
        assert_eq!(effects.distortion_wet, 0.6);
        assert_eq!(effects.delay_seconds, 0.4);
        assert_eq!(effects.delay_feedback, 0.35);
        assert_eq!(effects.delay_wet, 0.25);
        assert!(effects.compressor_enabled);
        assert!(effects.chorus_enabled);
        assert!(effects.reverb_enabled);
        assert_eq!(effects.reverb_seed, 42);
    }

    #[actix_rt::test]
    #[ignore = "manual real-media integration check"]
    async fn renders_real_clip_through_shared_server_pipeline() {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../sakiot-DSP/examples/8161a145-9b3d-4bdb-bca1-4d12fd6781a2/source.ogg");
        assert!(source.is_file(), "real-media fixture is missing");
        let temporary = tempfile::tempdir().unwrap();
        let output = temporary.path().join("shared-render.ogg");
        let mut render = segment_render();
        render.path = source;
        render.source_in = 0.25;
        render.source_out = 5.25;
        render.timeline_start = 0.125;
        render.effects.rate = 1.35;
        render.effects.pitch_cents = 700.0;
        render.effects.volume_db = -3.0;
        render.effects.reverse = true;
        render.effects.tail_seconds = 0.2;
        render.effects.distortion_amount = 0.6;
        render.effects.distortion_wet = 0.25;
        render.effects.delay_seconds = 0.05;
        render.effects.delay_feedback = 0.2;
        render.effects.delay_wet = 0.2;
        render.effects.compressor_enabled = true;
        render.effects.chorus_enabled = true;
        render.effects.reverb_enabled = true;
        render.effects.reverb_decay_seconds = 0.2;
        render.effects.reverb_wet = 0.2;
        render.effects.reverb_seed = 42;
        let expected_ms = expected_duration_ms(std::slice::from_ref(&render));
        let progress = web::Data::new(WaveformProgressContainer(tokio::sync::RwLock::new(
            HashMap::new(),
        )));

        render_compose_shared(
            &[render],
            -2.0,
            &output,
            expected_ms,
            &progress,
            "real-media-test",
        )
        .await
        .unwrap();

        let duration = probe_duration(&output).await.unwrap();
        let expected_seconds =
            std::time::Duration::from_millis(u64::try_from(expected_ms).unwrap()).as_secs_f64();
        assert!((duration - expected_seconds).abs() < 0.1);
        assert!(std::fs::metadata(output).unwrap().len() > 1_000);
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
        // Content 1.0..5.0 = 4s at 2x tempo -> 2s. The +1200-cent pitch
        // shift preserves that duration, so the timeline ends at 4.5s.
        assert_eq!(expected_duration_ms(&[segment_render()]), 4_500);
    }

    #[test]
    fn effect_tail_extends_the_expected_timeline_after_rate_processing() {
        let mut render = segment_render();
        render.effects.tail_seconds = 2.0;
        // Four content seconds at 2x consume 2s; the fixed 2s tail is not
        // rate-scaled, and the segment starts at 2.5s.
        assert_eq!(expected_duration_ms(&[render]), 6_500);
    }

    #[test]
    fn expected_duration_ignores_pitch_and_follows_speed() {
        let mut render = segment_render();
        render.effects.pitch_cents = -1200.0;
        render.source_in = 0.0;
        render.source_out = 10.0;
        render.timeline_start = 0.0;
        // Pitch is duration-preserving; only the 2x tempo resizes the clip.
        assert_eq!(expected_duration_ms(&[render]), 5_000);
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
    fn rejects_invalid_merge_group_ids() {
        let mut seg = segment();
        seg.merge_group = Some("../escape".into());
        assert!(validate_edit(&body(vec![seg.clone()])).is_err());
        seg.merge_group = Some("".into());
        assert!(validate_edit(&body(vec![seg])).is_err());
    }

    #[test]
    fn merge_groups_round_trip_through_the_stored_json() {
        let mut first = segment();
        first.merge_group = Some("group-1".into());
        let mut second = segment();
        second.source_id = "def".into();
        second.source_out = 20.0;
        second.timeline_start = 10.0;
        second.merge_group = Some("group-1".into());
        let value = serde_json::to_value(body(vec![first, second])).unwrap();
        assert_eq!(value["segments"][0]["merge_group"], "group-1");
        assert_eq!(value["segments"][1]["merge_group"], "group-1");
        let restored: ComposeClipBody = serde_json::from_value(value).unwrap();
        assert_eq!(restored.segments[0].merge_group.as_deref(), Some("group-1"));
        assert_eq!(restored.segments[1].merge_group.as_deref(), Some("group-1"));
    }

    #[test]
    fn merge_group_defaults_to_none_when_missing() {
        let mut value = serde_json::to_value(body(vec![segment()])).unwrap();
        value["segments"][0]
            .as_object_mut()
            .unwrap()
            .remove("merge_group");
        let restored: ComposeClipBody = serde_json::from_value(value).unwrap();
        assert_eq!(restored.segments[0].merge_group, None);
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
        assert!(validate_edit(&body(vec![seg.clone()])).is_err());
        seg.effects.bass_db = 0.0;
        seg.effects.advanced.distortion_wet = 1.1;
        assert!(validate_edit(&body(vec![seg.clone()])).is_err());
        seg.effects.advanced.distortion_wet = 0.0;
        seg.effects.advanced.tail_seconds = sakiot_dsp::MAX_EFFECT_TAIL_SECONDS + 0.1;
        assert!(validate_edit(&body(vec![seg.clone()])).is_err());
        seg.effects.advanced.tail_seconds = 0.0;
        seg.effects.advanced.delay_seconds = DELAY_MAX_SECONDS + 0.1;
        assert!(validate_edit(&body(vec![seg.clone()])).is_err());
        seg.effects.advanced.delay_seconds = 0.25;
        seg.effects.advanced.compressor_ratio = 21.0;
        assert!(validate_edit(&body(vec![seg.clone()])).is_err());
        seg.effects.advanced.compressor_ratio = 12.0;
        seg.effects.advanced.chorus_depth = -0.1;
        assert!(validate_edit(&body(vec![seg.clone()])).is_err());
        seg.effects.advanced.chorus_depth = 0.7;
        seg.effects.advanced.reverb_decay_seconds = 0.0;
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
    fn rejects_advanced_effects_on_oversized_in_memory_segments() {
        let mut seg = segment();
        seg.source_out = MAX_SHARED_DSP_SEGMENT_SECONDS + 1.0;
        seg.effects.advanced.distortion_wet = 0.5;
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
        assert_eq!(value["segments"][0]["effects"]["mid_db"], 0.0);
        assert_eq!(value["segments"][0]["effects"]["treble_db"], 0.0);
        let distortion_amount = value["segments"][0]["effects"]["advanced"]["distortion_amount"]
            .as_f64()
            .unwrap();
        assert!((distortion_amount - 0.4).abs() < 1e-6);
        assert_eq!(
            value["segments"][0]["effects"]["advanced"]["tail_seconds"],
            0.0
        );
        assert_eq!(
            value["segments"][0]["effects"]["advanced"]["reverb_seed"],
            0x5341_4b49_u32
        );
        assert_eq!(value["segments"][0]["effects"]["reverse"], false);
    }

    #[test]
    fn effects_default_to_forward_playback_when_reverse_is_missing() {
        let mut seg = segment();
        let mut value = serde_json::to_value(body(vec![seg.clone()])).unwrap();
        value["segments"][0]["effects"]
            .as_object_mut()
            .unwrap()
            .remove("reverse");
        let restored: ComposeClipBody = serde_json::from_value(value).unwrap();
        assert!(!restored.segments[0].effects.reverse);
        seg.effects.reverse = true;
        let value = serde_json::to_value(body(vec![seg])).unwrap();
        let restored: ComposeClipBody = serde_json::from_value(value).unwrap();
        assert!(restored.segments[0].effects.reverse);
    }

    #[test]
    fn effects_default_to_flat_mid_eq_when_mid_is_missing() {
        let mut value = serde_json::to_value(body(vec![segment()])).unwrap();
        value["segments"][0]["effects"]
            .as_object_mut()
            .unwrap()
            .remove("mid_db");
        let restored: ComposeClipBody = serde_json::from_value(value).unwrap();
        assert_eq!(restored.segments[0].effects.mid_db, 0.0);
    }

    #[test]
    fn advanced_effects_default_to_shared_bypass_when_missing() {
        let mut value = serde_json::to_value(body(vec![segment()])).unwrap();
        value["segments"][0]["effects"]
            .as_object_mut()
            .unwrap()
            .remove("advanced");
        let restored: ComposeClipBody = serde_json::from_value(value).unwrap();
        let advanced = &restored.segments[0].effects.advanced;
        assert_eq!(advanced.distortion_amount, 0.4);
        assert_eq!(advanced.tail_seconds, 0.0);
        assert_eq!(advanced.distortion_wet, 0.0);
        assert_eq!(advanced.delay_wet, 0.0);
        assert!(!advanced.compressor_enabled);
        assert!(!advanced.chorus_enabled);
        assert!(!advanced.reverb_enabled);
        assert_eq!(advanced.reverb_seed, 0x5341_4b49);
    }
}
