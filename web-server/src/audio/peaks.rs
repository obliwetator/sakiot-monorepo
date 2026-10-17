use actix_web::{HttpResponse, get, web};
use base64::prelude::*;
use sakiot_paths::DataRoots;
use sakiot_peaks::FramePeak;
use serde::Deserialize;
use serde_json::json;
use sqlx::{Pool, Postgres};
use std::path::PathBuf;

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::media_jobs::{JobAttempt, MediaJobRequest, MediaJobStatus};
use crate::permissions::require_channel_access;
use crate::waveform::{PeakDensity, decode_and_publish, read_peaks, waveform_dat};

use super::paths::{NO_SILENCE_PREFIX, no_silence_recording_path, recording_path};
use super::serve::AudioQuery;
use super::util::{file_exists, get_file_path_root, is_stale, is_valid_file_segment};

#[derive(Deserialize, Debug)]
pub struct WaveformQuery {
    /// Selects the silence-free waveform when true.
    pub silence: Option<bool>,
    /// Explicit cache-busting retry token.
    pub t: Option<u64>,
}

/// The waveform of `frames`, as the dashboard reads it.
fn waveform_response(frames: &[FramePeak], growing: bool) -> HttpResponse {
    HttpResponse::Ok().json(json!({
        "progress": 100,
        "data": BASE64_STANDARD.encode(waveform_dat(frames, PeakDensity::DEFAULT, growing)),
    }))
}

#[utoipa::path(
    get,
    path = "/api/audio/waveform/{guild_id}/{channel_id}/{year}/{month}/{file}",
    tag = "audio",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("channel_id" = i64, Path, description = "Discord channel id"),
        ("year" = i32, Path, description = "Recording year"),
        ("month" = u32, Path, description = "Recording month"),
        ("file" = String, Path, description = "Recording file name"),
        ("silence" = Option<bool>, Query, description = "Serve the silence-free waveform when true"),
    ),
    responses(
        (status = 200, description = "Base64 waveform peaks, or `built: false` while a live recording has none"),
        (status = 202, description = "Waveform is being built from the audio"),
        (status = 400, description = "Invalid file name", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 404, description = "Recording not found", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/waveform/{guild_id}/{channel_id}/{year}/{month}/{file}")]
pub async fn get_waveform_data(
    path: web::Path<(i64, i64, i32, i32, String)>,
    query: web::Query<WaveformQuery>,
    pool: web::Data<Pool<Postgres>>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let path = path.into_inner();
    if !is_valid_file_segment(&path.4) {
        return Err(AppError::BadRequest("Invalid file name".into()));
    }
    let token = token.ok_or(AppError::Unauthorized)?;
    let viewer = token.viewer();
    super::sessions::require_recording_access(
        &pool, path.0, path.1, path.2, path.3, &path.4, viewer,
    )
    .await?;
    let roots = DataRoots::from_env();

    // The silence-free version is its own file, decoded for its peaks. They
    // go stale when it is rendered again.
    if query.silence == Some(true) {
        let base = get_file_path_root(&no_silence_recording_path(), &path);
        let input = format!("{base}/{NO_SILENCE_PREFIX}{}.ogg", path.4);
        let peaks = roots.peaks_path(&format!("{NO_SILENCE_PREFIX}{}", path.4));
        if !is_stale(&input, &peaks.to_string_lossy()).await
            && let Some(frames) = read_peaks(&peaks).await?
        {
            return Ok(waveform_response(&frames, false));
        }
        if !file_exists(&input).await {
            return Err(AppError::FileNotFound);
        }
        let version = format!("final-{}", query.t.unwrap_or_default());
        return enqueue_recording_waveform(&pool, viewer, &path, true, &version).await;
    }

    // The recorder writes a recording's peaks as it records.
    let end_ts = recording_end_ts(&pool, &path.4).await?;
    if let Some(frames) = read_peaks(&roots.peaks_path(&path.4)).await? {
        return Ok(waveform_response(&frames, end_ts.is_none()));
    }
    // A recording without them (its peaks failed, were evicted, or predate
    // the recorder writing any) gets them from its audio once it has ended.
    // Decoding a live one would leave a snapshot that stops short.
    let Some(end_ts) = end_ts else {
        return Ok(HttpResponse::Ok().json(json!({ "progress": 0, "built": false })));
    };
    let version = format!("{end_ts}-{}", query.t.unwrap_or_default());
    enqueue_recording_waveform(&pool, viewer, &path, false, &version).await
}

fn recording_waveform_resource(path: &(i64, i64, i32, i32, String), silence_free: bool) -> String {
    let variant = if silence_free { "silence" } else { "original" };
    format!(
        "recording-waveform:{}/{}/{}/{}/{}:{variant}",
        path.0, path.1, path.2, path.3, path.4
    )
}

fn job_accepted(status: MediaJobStatus) -> HttpResponse {
    HttpResponse::Accepted()
        .insert_header((
            actix_web::http::header::LOCATION,
            format!("/api/media-jobs/{}", status.id),
        ))
        .json(status)
}

async fn enqueue_recording_waveform(
    pool: &web::Data<Pool<Postgres>>,
    requester: crate::permissions::Viewer,
    path: &(i64, i64, i32, i32, String),
    silence_free: bool,
    version: &str,
) -> Result<HttpResponse, AppError> {
    let request = MediaJobRequest::RecordingWaveform {
        guild_id: path.0,
        channel_id: path.1,
        year: path.2,
        month: path.3,
        file_name: path.4.clone(),
        silence_free,
    };
    let variant = if silence_free { "silence" } else { "original" };
    let resource = recording_waveform_resource(path, silence_free);
    let key = format!("waveform-{variant}-{}-{version}", path.4);
    let status = crate::media_jobs::enqueue_rebuild(
        pool,
        Some(path.0),
        requester,
        &key,
        &resource,
        &request,
    )
    .await?;
    Ok(job_accepted(status))
}

/// Decodes a finished recording, or its silence-free version, into peaks.
pub(crate) async fn run_recording_waveform_job(
    attempt: &JobAttempt<'_>,
    guild_id: i64,
    channel_id: i64,
    year: i32,
    month: i32,
    file_name: &str,
    silence_free: bool,
) -> Result<(Option<String>, Option<PathBuf>), AppError> {
    let path = (guild_id, channel_id, year, month, file_name.to_owned());
    let roots = DataRoots::from_env();
    let (input, peaks) = if silence_free {
        let base = get_file_path_root(&no_silence_recording_path(), &path);
        (
            PathBuf::from(format!("{base}/{NO_SILENCE_PREFIX}{file_name}.ogg")),
            roots.peaks_path(&format!("{NO_SILENCE_PREFIX}{file_name}")),
        )
    } else {
        let base = get_file_path_root(&recording_path(), &path);
        let input = PathBuf::from(format!("{base}/{file_name}.ogg"));
        let audio_file_id = crate::media_archive::recording_id(
            attempt.pool,
            guild_id,
            channel_id,
            year,
            month,
            file_name,
        )
        .await?
        .ok_or(AppError::FileNotFound)?;
        attempt
            .media
            .ensure_recording_local(attempt.pool, audio_file_id, &input)
            .await?;
        (input, roots.peaks_path(file_name))
    };
    if !tokio::fs::try_exists(&input).await.unwrap_or(false) {
        return Err(AppError::FileNotFound);
    }
    let url = format!(
        "/api/audio/waveform/{guild_id}/{channel_id}/{year}/{month}/{file_name}{}",
        if silence_free { "?silence=true" } else { "" }
    );
    decode_and_publish(attempt, &[(input, peaks)], &url).await?;
    Ok((Some(url), None))
}

/// When the recording ended, or `None` while it is still live.
async fn recording_end_ts(pool: &Pool<Postgres>, file_name: &str) -> Result<Option<i64>, AppError> {
    sqlx::query_scalar!(
        "SELECT end_ts FROM audio_files WHERE file_name=$1",
        file_name
    )
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::FileNotFound)
}

// Include the immutable file revision so an old generator cannot populate the
// waveform of a newly overwritten clip. The existing cache reaper removes old keys.
fn clip_waveform_key(clip_id: &str, input: &std::path::Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut hash);
    format!("clip-{clip_id}-{:016x}", hash.finish())
}

// A clip is its own trimmed, immutable .ogg — no live/end_ts logic. Its peaks
// are decoded from the clip file, keyed by clip_id, mirroring the simple
// silence-free path. On-disk existence is the cache (the file never changes).
#[utoipa::path(
    get,
    path = "/api/audio/clips/waveform/{guild_id}/{clip_id}",
    tag = "audio",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("clip_id" = String, Path, description = "Clip id"),
        ("silence" = Option<bool>, Query, description = "Serve the silence-free waveform when true"),
    ),
    responses(
        (status = 200, description = "Base64 waveform peaks"),
        (status = 202, description = "Waveform is being built from the audio"),
        (status = 400, description = "Invalid clip id", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 404, description = "Clip not found", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/clips/waveform/{guild_id}/{clip_id}")]
pub async fn get_clip_waveform_data(
    path: web::Path<(i64, String)>,
    query: web::Query<AudioQuery>,
    pool: web::Data<Pool<Postgres>>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, clip_id) = path.into_inner();
    if !is_valid_file_segment(&clip_id) {
        return Err(AppError::BadRequest("Invalid clip id".into()));
    }
    let token = token.ok_or(AppError::Unauthorized)?;

    let row = sqlx::query!(
        "SELECT saved_file_name, channel_id, recording_session_id
           FROM clips
          WHERE guild_id = $1 AND clip_id = $2 AND deleted_at IS NULL",
        guild_id,
        clip_id
    )
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or(AppError::ClipNotFound)?;
    if let Some(session_id) = row.recording_session_id {
        super::sessions::require_session_access(&pool, session_id, token.viewer()).await?;
    } else {
        let channel_id = row.channel_id.ok_or(AppError::ClipNotFound)?;
        require_channel_access(&pool, guild_id, channel_id, token.viewer()).await?;
    }

    let saved_file_name = row.saved_file_name.ok_or(AppError::ClipNotFound)?;
    let input_path = crate::media_archive::clip_local_path(&saved_file_name)?;
    // Prefix the cache/progress key so it never collides with recording stems
    // ({ts}-{user_id}) or the silence-free (_no_silence_) key.
    let cache_key = clip_waveform_key(&clip_id, &input_path);
    if let Some(frames) = read_peaks(&DataRoots::from_env().peaks_path(&cache_key)).await? {
        return Ok(waveform_response(&frames, false));
    }

    let request = MediaJobRequest::ClipWaveform {
        guild_id,
        clip_id: clip_id.clone(),
    };
    let key = format!("waveform-{cache_key}-{}", query.t.unwrap_or_default());
    let status = crate::media_jobs::enqueue_rebuild(
        pool.get_ref(),
        Some(guild_id),
        token.viewer(),
        &key,
        &cache_key,
        &request,
    )
    .await?;
    Ok(job_accepted(status))
}

pub(crate) async fn run_clip_waveform_job(
    attempt: &JobAttempt<'_>,
    guild_id: i64,
    clip_id: &str,
) -> Result<(Option<String>, Option<PathBuf>), AppError> {
    let saved_file_name = sqlx::query_scalar!(
        "SELECT saved_file_name FROM clips WHERE guild_id=$1 AND clip_id=$2 AND deleted_at IS NULL",
        guild_id,
        clip_id
    )
    .fetch_optional(attempt.pool)
    .await?
    .ok_or(AppError::ClipNotFound)?;
    let input_path =
        crate::media_archive::clip_local_path(&saved_file_name.ok_or(AppError::ClipNotFound)?)?;
    attempt
        .media
        .ensure_clip_local(attempt.pool, clip_id, &input_path)
        .await?;
    let peaks = DataRoots::from_env().peaks_path(&clip_waveform_key(clip_id, &input_path));
    let url = format!("/api/audio/clips/waveform/{guild_id}/{clip_id}");
    decode_and_publish(attempt, &[(input_path, peaks)], &url).await?;
    Ok((Some(url), None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn live_recordings_have_no_end_yet(
        pool: PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        sqlx::query!(
            "INSERT INTO audio_files (file_name, guild_id, channel_id, user_id, year, month, end_ts)
             VALUES ('live', 1, 2, 3, 2026, 10, NULL), ('ended', 1, 2, 3, 2026, 10, 5000)"
        )
        .execute(&pool)
        .await?;

        assert_eq!(recording_end_ts(&pool, "live").await?, None);
        assert_eq!(recording_end_ts(&pool, "ended").await?, Some(5000));
        assert!(matches!(
            recording_end_ts(&pool, "missing").await,
            Err(AppError::FileNotFound)
        ));
        Ok(())
    }
}
