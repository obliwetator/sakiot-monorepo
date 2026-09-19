use super::*;
use crate::media_jobs::{MediaJobRequest, MediaJobStatus};

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/download",
    tag = "audio",
    params(
        ("recording_session_id" = i64, Path, description = "Logical recording session id"),
        ("start" = Option<f64>, Query, description = "Range start in logical seconds"),
        ("end" = Option<f64>, Query, description = "Range end in logical seconds"),
        ("remove_silence" = Option<bool>, Query, description = "Run FFmpeg silence removal after composition"),
    ),
    responses(
        (status = 202, description = "Download composition durably queued", body = MediaJobStatus),
        (status = 400, description = "Invalid range", body = crate::errors::ApiError),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Session not found", body = crate::errors::ApiError),
        (status = 500, description = "Composition failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/sessions/{recording_session_id}/download")]
pub async fn download_session(
    request: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<SessionDownloadQuery>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let job_request = MediaJobRequest::SessionDownload {
        session_id,
        start: query.start,
        end: query.end,
        remove_silence: query.remove_silence.unwrap_or(false),
    };
    // GET cannot require a custom idempotency header from a navigation. A
    // canonical key still deduplicates retries and repeated clicks.
    let default_key = format!(
        "download-{session_id}-{:016x}-{:016x}-{}",
        query.start.unwrap_or(-1.0).to_bits(),
        query.end.unwrap_or(-1.0).to_bits(),
        u8::from(query.remove_silence.unwrap_or(false))
    );
    let idempotency_key = request
        .headers()
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or(&default_key)
        .to_owned();
    let resource_key = format!(
        "session-download:{}:{}:{idempotency_key}",
        token.user_id, session_id
    );
    let status = crate::media_jobs::enqueue(
        pool.get_ref(),
        Some(access.guild_id),
        token.user_id,
        &idempotency_key,
        &resource_key,
        &job_request,
    )
    .await?;
    let location = format!("/api/media-jobs/{}", status.id);
    Ok(HttpResponse::Accepted()
        .insert_header((actix_web::http::header::LOCATION, location))
        .json(status))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_download_job(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    user_id: i64,
    session_id: i64,
    start: Option<f64>,
    end: Option<f64>,
    remove_silence: bool,
    job_id: &str,
    attempt_token: &str,
) -> Result<(Option<String>, Option<PathBuf>), AppError> {
    let pool_data = web::Data::new(pool.clone());
    let access = require_session_access(&pool_data, session_id, user_id).await?;
    let output_dir = PathBuf::from(recording_path()).join(".media-jobs");
    tokio::fs::create_dir_all(&output_dir).await?;
    let output = output_dir.join(format!("{job_id}-{attempt_token}.ogg"));
    let progress = web::Data::new(WaveformProgressContainer(tokio::sync::RwLock::new(
        std::collections::HashMap::new(),
    )));
    let composition_progress = CompositionProgress {
        cache_key: job_id.to_owned(),
        progress: progress.clone(),
        completed: 99,
    };
    if let Err(error) = crate::media_jobs::track_progress(
        pool,
        job_id,
        attempt_token,
        "composing",
        &progress,
        job_id,
        compose_session_with_progress(
            &pool_data,
            &access,
            start,
            end,
            remove_silence,
            &output,
            composition_progress,
            media,
        ),
    )
    .await
    {
        let _ = tokio::fs::remove_file(&output).await;
        return Err(error);
    }
    Ok((
        Some(format!("/api/media-jobs/{job_id}/result")),
        Some(output),
    ))
}

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/silence-free",
    tag = "audio",
    params(
        ("recording_session_id" = i64, Path, description = "Logical recording session id"),
        ("download" = Option<bool>, Query, description = "Download instead of inline playback"),
    ),
    responses(
        (status = 200, description = "Cached silence-free Ogg/Opus", content_type = "audio/ogg"),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Silence-free session has not been generated", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[route(
    "/audio/sessions/{recording_session_id}/silence-free",
    method = "GET",
    method = "HEAD"
)]
pub async fn get_session_silence_free(
    path: web::Path<i64>,
    query: web::Query<SilenceFreeSessionQuery>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<NamedFile, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let output = session_silence_free_path(&access)?;
    let disposition = if query.download.unwrap_or(false) {
        actix_web::http::header::DispositionType::Attachment
    } else {
        actix_web::http::header::DispositionType::Inline
    };
    let file = NamedFile::open_async(output).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::FileNotFound
        } else {
            AppError::IoError(error)
        }
    })?;
    Ok(
        file.set_content_disposition(actix_web::http::header::ContentDisposition {
            disposition,
            parameters: vec![],
        }),
    )
}

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/remove-silence",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Current silence-removal status", body = SilenceFreeSessionResponse),
        (status = 400, description = "Session is not finalized", body = crate::errors::ApiError),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/sessions/{recording_session_id}/remove-silence")]
pub async fn get_session_silence_removal_status(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    progress: web::Data<WaveformProgressContainer>,
) -> Result<web::Json<SilenceFreeSessionResponse>, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let output = session_silence_free_path(&access)?;
    let cache_key = silence_removal_progress_key(session_id);

    if let Some(job) =
        crate::media_jobs::active_for_resource(&pool, token.user_id, "session_silence", &cache_key)
            .await?
    {
        return Ok(web::Json(silence_removal_response(
            "processing",
            job.progress,
        )));
    }

    let value = progress.0.read().await.get(&cache_key).copied();
    if let Some(value) = value {
        return Ok(web::Json(if value < 0 {
            silence_removal_response("failed", 0)
        } else {
            silence_removal_response("processing", value.clamp(0, 99))
        }));
    }
    if tokio::fs::try_exists(output).await? {
        return Ok(web::Json(silence_removal_response("ready", 100)));
    }
    if crate::media_jobs::latest_failed_for_resource(
        &pool,
        token.user_id,
        "session_silence",
        &cache_key,
    )
    .await?
    .is_some()
    {
        return Ok(web::Json(silence_removal_response("failed", 0)));
    }
    Ok(web::Json(silence_removal_response("idle", 0)))
}

#[utoipa::path(
    post,
    path = "/api/audio/sessions/{recording_session_id}/remove-silence",
    tag = "audio",
    params(
        ("recording_session_id" = i64, Path, description = "Logical recording session id"),
        ("force" = Option<bool>, Query, description = "Replace an existing silence-free session"),
    ),
    responses(
        (status = 200, description = "Silence-free session is ready", body = SilenceFreeSessionResponse),
        (status = 202, description = "Silence removal started or is already running", body = SilenceFreeSessionResponse),
        (status = 400, description = "Session is not finalized", body = crate::errors::ApiError),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 500, description = "Composition failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/audio/sessions/{recording_session_id}/remove-silence")]
pub async fn remove_session_silence(
    request: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<SilenceRemovalQuery>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let output = session_silence_free_path(&access)?;
    let cache_key = silence_removal_progress_key(session_id);
    let force = query.force.unwrap_or(false);
    if !force && tokio::fs::try_exists(&output).await? {
        return Ok(HttpResponse::Ok().json(silence_removal_response("ready", 100)));
    }
    let job_request = MediaJobRequest::SessionSilence { session_id };
    let key = request
        .headers()
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let status = crate::media_jobs::enqueue(
        pool.get_ref(),
        Some(access.guild_id),
        token.user_id,
        &key,
        &cache_key,
        &job_request,
    )
    .await?;
    Ok(HttpResponse::Accepted()
        .insert_header((
            actix_web::http::header::LOCATION,
            format!("/api/media-jobs/{}", status.id),
        ))
        .json(status))
}

pub(crate) async fn run_session_silence_job(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    user_id: i64,
    session_id: i64,
    job_id: &str,
    attempt_token: &str,
) -> Result<(Option<String>, Option<PathBuf>), AppError> {
    let pool_data = web::Data::new(pool.clone());
    let access = require_session_access(&pool_data, session_id, user_id).await?;
    let output = session_silence_free_path(&access)?;
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let temporary = output.with_extension(format!("{job_id}.{attempt_token}.tmp.ogg"));
    let cache_key = silence_removal_progress_key(session_id);
    let progress = web::Data::new(WaveformProgressContainer(tokio::sync::RwLock::new(
        std::collections::HashMap::new(),
    )));
    let composition_progress = CompositionProgress {
        cache_key,
        progress: progress.clone(),
        completed: 99,
    };
    if let Err(error) = crate::media_jobs::track_progress(
        pool,
        job_id,
        attempt_token,
        "removing_silence",
        &progress,
        &composition_progress.cache_key,
        compose_session_with_progress(
            &pool_data,
            &access,
            None,
            None,
            true,
            &temporary,
            composition_progress.clone(),
            media,
        ),
    )
    .await
    {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(error);
    }
    let tx = crate::media_jobs::begin_publication(pool, job_id, attempt_token).await?;
    tokio::fs::rename(&temporary, &output).await?;
    let (_, silence_waveform_output) = session_waveform_cache(session_id, true);
    let _ = tokio::fs::remove_file(silence_waveform_output).await;
    let url = format!("/api/audio/sessions/{session_id}/silence-free");
    crate::media_jobs::complete_publication(tx, job_id, attempt_token, &url, None).await?;
    Ok((Some(url), None))
}

pub(super) fn silence_removal_progress_key(session_id: i64) -> String {
    format!("logical-session-{session_id}-silence-removal")
}

pub(super) fn silence_removal_response(status: &str, progress: i16) -> SilenceFreeSessionResponse {
    SilenceFreeSessionResponse {
        status: status.to_owned(),
        progress,
    }
}
