use actix_web::{HttpRequest, HttpResponse, get, web};
use base64::prelude::*;
use serde_json::json;
use sqlx::{Pool, Postgres};
use std::time::Duration;

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::media_archive::MediaArchive;
use crate::media_jobs::MediaJobRequest;
use crate::permissions::require_channel_access;
use crate::waveform::{PeakDensity, generate_peaks_background};

use super::paths::{NO_SILENCE_PREFIX, no_silence_recording_path, recording_path, waveform_path};
use super::serve::AudioQuery;
use super::types::WaveformProgressContainer;
use super::util::{file_exists, get_file_path_root, is_stale, is_valid_file_segment};

const LIVE_WAVEFORM_MIN_REFRESH: Duration = Duration::from_secs(5);

async fn waveform_response(output: &str) -> Result<HttpResponse, AppError> {
    waveform_response_with_progress(output, 100).await
}

async fn waveform_response_with_progress(
    output: &str,
    progress: i16,
) -> Result<HttpResponse, AppError> {
    let file_content = tokio::fs::read(output).await?;
    let base64_content = BASE64_STANDARD.encode(file_content);
    Ok(HttpResponse::Ok().json(json!({
        "progress": progress,
        "data": base64_content
    })))
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
        (status = 200, description = "Base64 waveform peaks"),
        (status = 202, description = "Waveform is still being generated"),
        (status = 400, description = "Invalid file name", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 404, description = "Recording not found", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/waveform/{guild_id}/{channel_id}/{year}/{month}/{file}")]
pub async fn get_waveform_data(
    _req: HttpRequest,
    path: web::Path<(i64, i64, i32, i32, String)>,
    query: web::Query<AudioQuery>,
    _progress_map: web::Data<WaveformProgressContainer>,
    pool: web::Data<Pool<Postgres>>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let path = path.into_inner();
    if !is_valid_file_segment(&path.4) {
        return Err(AppError::BadRequest("Invalid file name".into()));
    }
    let token = token.ok_or(AppError::Unauthorized)?;
    super::sessions::require_recording_access(
        &pool,
        path.0,
        path.1,
        path.2,
        path.3,
        &path.4,
        crate::permissions::Viewer::of(&token),
    )
    .await?;

    // Silence-free version is a separate static file: distinct input,
    // distinct cache/progress key. No DB cache marker — the file is final
    // once produced, so on-disk existence is the cache.
    if query.wants_silence_free() {
        let base = get_file_path_root(&no_silence_recording_path(), &path);
        let input_file = format!("{base}/{NO_SILENCE_PREFIX}{}.ogg", path.4);
        let output = format!("{}{}{}.dat", waveform_path(), NO_SILENCE_PREFIX, path.4);
        if file_exists(&output).await && !is_stale(&input_file, &output).await {
            return waveform_response(&output).await;
        }
        if !file_exists(&input_file).await {
            return Err(AppError::FileNotFound);
        }
        let version = format!("final-{}", query.t.unwrap_or_default());
        return enqueue_recording_waveform(
            &pool,
            crate::permissions::Viewer::of(&token),
            &path,
            true,
            &version,
        )
        .await;
    }

    let output = format!("{}{}.dat", waveform_path(), path.4);
    let file_name = path.4.clone();

    let row = sqlx::query!(
        "SELECT end_ts, waveform_end_ts FROM audio_files WHERE file_name = $1",
        file_name
    )
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or(AppError::FileNotFound)?;
    let end_ts = row.end_ts;
    let waveform_end_ts = row.waveform_end_ts;
    let has_final_cache =
        end_ts.is_some() && waveform_end_ts == end_ts && file_exists(&output).await;

    if has_final_cache {
        return waveform_response(&output).await;
    }

    // A live recording keeps serving its last complete atomic snapshot while
    // a refresh is in flight. Do not launch a new audiowaveform process more
    // often than the snapshot interval, even when several track rows poll at
    // once.
    if end_ts.is_none()
        && file_exists(&output).await
        && tokio::fs::metadata(&output)
            .await
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age < LIVE_WAVEFORM_MIN_REFRESH)
    {
        return waveform_response_with_progress(&output, 100).await;
    }

    let version = end_ts.map_or_else(
        || format!("live-{}", chrono::Utc::now().timestamp() / 5),
        |value| value.to_string(),
    );
    let version = format!("{version}-{}", query.t.unwrap_or_default());
    enqueue_recording_waveform(
        &pool,
        crate::permissions::Viewer::of(&token),
        &path,
        false,
        &version,
    )
    .await
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
    let resource = format!(
        "recording-waveform:{}/{}/{}/{}/{}:{variant}",
        path.0, path.1, path.2, path.3, path.4
    );
    let key = format!("waveform-{variant}-{}-{version}", path.4);
    let status =
        crate::media_jobs::enqueue(pool, Some(path.0), requester, &key, &resource, &request)
            .await?;
    Ok(HttpResponse::Accepted()
        .insert_header((
            actix_web::http::header::LOCATION,
            format!("/api/media-jobs/{}", status.id),
        ))
        .json(status))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_recording_waveform_job(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    guild_id: i64,
    channel_id: i64,
    year: i32,
    month: i32,
    file_name: &str,
    silence_free: bool,
    job_id: &str,
    attempt_token: &str,
) -> Result<(Option<String>, Option<std::path::PathBuf>), AppError> {
    // None while the recording is live: the job builds a snapshot and leaves
    // waveform_end_ts unset, so the next request refreshes it.
    let end_ts = if silence_free {
        None
    } else {
        recording_end_ts(pool, file_name).await?
    };
    let path = (guild_id, channel_id, year, month, file_name.to_owned());
    let (input, cache_key) = if silence_free {
        let base = get_file_path_root(&no_silence_recording_path(), &path);
        (
            format!("{base}/{NO_SILENCE_PREFIX}{file_name}.ogg"),
            format!("{NO_SILENCE_PREFIX}{file_name}"),
        )
    } else {
        let base = get_file_path_root(&recording_path(), &path);
        let input = format!("{base}/{file_name}.ogg");
        let audio_file_id =
            crate::media_archive::recording_id(pool, guild_id, channel_id, year, month, file_name)
                .await?
                .ok_or(AppError::FileNotFound)?;
        media
            .ensure_recording_local(pool, audio_file_id, std::path::Path::new(&input))
            .await?;
        (input, file_name.to_owned())
    };
    if !file_exists(&input).await {
        return Err(AppError::FileNotFound);
    }
    let output = std::path::PathBuf::from(format!("{}{cache_key}.dat", waveform_path()));
    let attempt_output = output.with_extension(format!("{job_id}.{attempt_token}.tmp.dat"));
    let progress = web::Data::new(WaveformProgressContainer(tokio::sync::RwLock::new(
        std::collections::HashMap::new(),
    )));
    crate::media_jobs::track_progress(
        pool,
        job_id,
        attempt_token,
        "waveform",
        &progress,
        &cache_key,
        async {
            generate_peaks_background(
                input,
                attempt_output.to_string_lossy().into_owned(),
                cache_key.clone(),
                PeakDensity::DEFAULT,
                progress.clone(),
                None,
                None,
            )
            .await
            .map_err(|error| AppError::IoError(std::io::Error::other(error.to_string())))
        },
    )
    .await?;
    let mut tx = crate::media_jobs::begin_publication(pool, job_id, attempt_token).await?;
    tokio::fs::rename(&attempt_output, &output).await?;
    if let Some(end_ts) = end_ts {
        sqlx::query!(
            "UPDATE audio_files SET waveform_end_ts=$2 WHERE file_name=$1 AND end_ts=$2",
            file_name,
            end_ts
        )
        .execute(&mut *tx)
        .await?;
    }
    let url = format!(
        "/api/audio/waveform/{guild_id}/{channel_id}/{year}/{month}/{file_name}{}",
        if silence_free { "?silence=true" } else { "" }
    );
    crate::media_jobs::complete_publication(tx, job_id, attempt_token, &url, None).await?;
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

// A clip is its own trimmed, immutable .ogg — no live/end_ts logic. Generate
// peaks straight from the clip file, keyed by clip_id, mirroring the simple
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
        (status = 202, description = "Waveform is still being generated"),
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
    _progress_map: web::Data<WaveformProgressContainer>,
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
        super::sessions::require_session_access(
            &pool,
            session_id,
            crate::permissions::Viewer::of(&token),
        )
        .await?;
    } else {
        let channel_id = row.channel_id.ok_or(AppError::ClipNotFound)?;
        require_channel_access(
            &pool,
            guild_id,
            channel_id,
            crate::permissions::Viewer::of(&token),
        )
        .await?;
    }

    let saved_file_name = row.saved_file_name.ok_or(AppError::ClipNotFound)?;
    let input_path = crate::media_archive::clip_local_path(&saved_file_name)?;
    // Prefix the cache/progress key so it never collides with recording stems
    // ({ts}-{user_id}) or the silence-free (_no_silence_) key.
    let cache_key = clip_waveform_key(&clip_id, &input_path);
    let output = format!("{}{}.dat", waveform_path(), cache_key);

    if file_exists(&output).await {
        return waveform_response(&output).await;
    }

    let request = MediaJobRequest::ClipWaveform {
        guild_id,
        clip_id: clip_id.clone(),
    };
    let key = format!("waveform-{cache_key}-{}", query.t.unwrap_or_default());
    let status = crate::media_jobs::enqueue(
        pool.get_ref(),
        Some(guild_id),
        crate::permissions::Viewer::of(&token),
        &key,
        &cache_key,
        &request,
    )
    .await?;
    Ok(HttpResponse::Accepted()
        .insert_header((
            actix_web::http::header::LOCATION,
            format!("/api/media-jobs/{}", status.id),
        ))
        .json(status))
}

pub(crate) async fn run_clip_waveform_job(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    guild_id: i64,
    clip_id: &str,
    job_id: &str,
    attempt_token: &str,
) -> Result<(Option<String>, Option<std::path::PathBuf>), AppError> {
    let saved_file_name = sqlx::query_scalar!(
        "SELECT saved_file_name FROM clips WHERE guild_id=$1 AND clip_id=$2 AND deleted_at IS NULL",
        guild_id,
        clip_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::ClipNotFound)?;
    let input_path =
        crate::media_archive::clip_local_path(&saved_file_name.ok_or(AppError::ClipNotFound)?)?;
    media.ensure_clip_local(pool, clip_id, &input_path).await?;
    let cache_key = clip_waveform_key(clip_id, &input_path);
    let output = std::path::PathBuf::from(format!("{}{cache_key}.dat", waveform_path()));
    let attempt_output = output.with_extension(format!("{job_id}.{attempt_token}.tmp.dat"));
    let progress = web::Data::new(WaveformProgressContainer(tokio::sync::RwLock::new(
        std::collections::HashMap::new(),
    )));
    crate::media_jobs::track_progress(
        pool,
        job_id,
        attempt_token,
        "waveform",
        &progress,
        &cache_key,
        async {
            generate_peaks_background(
                input_path.to_string_lossy().into_owned(),
                attempt_output.to_string_lossy().into_owned(),
                cache_key.clone(),
                PeakDensity::DEFAULT,
                progress.clone(),
                None,
                None,
            )
            .await
            .map_err(|error| AppError::IoError(std::io::Error::other(error.to_string())))
        },
    )
    .await?;
    let tx = crate::media_jobs::begin_publication(pool, job_id, attempt_token).await?;
    tokio::fs::rename(&attempt_output, &output).await?;
    let url = format!("/api/audio/clips/waveform/{guild_id}/{clip_id}");
    crate::media_jobs::complete_publication(tx, job_id, attempt_token, &url, None).await?;
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
