use super::*;

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
        (status = 200, description = "Composed Ogg/Opus download", content_type = "audio/ogg"),
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
    path: web::Path<i64>,
    query: web::Query<SessionDownloadQuery>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    media: web::Data<MediaArchive>,
) -> Result<NamedFile, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let output = temporary_ogg_path("session-download");
    compose_session(
        &pool,
        &access,
        query.start,
        query.end,
        query.remove_silence.unwrap_or(false),
        &output,
        media.get_ref(),
    )
    .await?;
    schedule_temporary_cleanup(output.clone());
    Ok(NamedFile::open_async(output)
        .await
        .map_err(AppError::IoError)?
        .set_content_disposition(actix_web::http::header::ContentDisposition {
            disposition: actix_web::http::header::DispositionType::Attachment,
            parameters: vec![],
        }))
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
    path: web::Path<i64>,
    query: web::Query<SilenceRemovalQuery>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    progress: web::Data<WaveformProgressContainer>,
    media: web::Data<MediaArchive>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let output = session_silence_free_path(&access)?;
    let cache_key = silence_removal_progress_key(session_id);

    if let Some(value) = progress.0.read().await.get(&cache_key).copied()
        && value >= 0
    {
        return Ok(HttpResponse::Accepted()
            .json(silence_removal_response("processing", value.clamp(0, 99))));
    }
    let force = query.force.unwrap_or(false);
    if !force && tokio::fs::try_exists(&output).await? {
        progress.0.write().await.remove(&cache_key);
        return Ok(HttpResponse::Ok().json(silence_removal_response("ready", 100)));
    }
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    {
        let mut values = progress.0.write().await;
        if let Some(value) = values.get(&cache_key).copied()
            && value >= 0
        {
            return Ok(HttpResponse::Accepted()
                .json(silence_removal_response("processing", value.clamp(0, 99))));
        }
        values.insert(cache_key.clone(), 0);
    }

    if force
        && let Err(error) = tokio::fs::remove_file(&output).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        progress.0.write().await.remove(&cache_key);
        return Err(AppError::IoError(error));
    }
    let (silence_waveform_key, silence_waveform_output) = session_waveform_cache(session_id, true);
    if force {
        progress.0.write().await.remove(&silence_waveform_key);
        let _ = tokio::fs::remove_file(&silence_waveform_output).await;
    }

    let temporary = output.with_extension(format!("{}.tmp.ogg", uuid::Uuid::new_v4()));
    let pool_clone = pool.clone();
    let progress_clone = progress.clone();
    let media_clone = media.clone();
    tokio::spawn(async move {
        progress_clone.0.write().await.insert(cache_key.clone(), 1);
        let composition_progress = CompositionProgress {
            cache_key: cache_key.clone(),
            progress: progress_clone.clone(),
            completed: 99,
        };
        if let Err(error) = compose_session_with_progress(
            &pool_clone,
            &access,
            None,
            None,
            true,
            &temporary,
            composition_progress,
            media_clone.get_ref(),
        )
        .await
        {
            tracing::error!(session_id, "session silence removal failed: {}", error);
            let _ = tokio::fs::remove_file(temporary).await;
            progress_clone.0.write().await.insert(cache_key, -1);
            return;
        }
        if let Err(error) = tokio::fs::rename(&temporary, &output).await {
            tracing::error!(
                session_id,
                "persisting silence-free session failed: {}",
                error
            );
            let _ = tokio::fs::remove_file(temporary).await;
            progress_clone.0.write().await.insert(cache_key, -1);
            return;
        }
        let _ = tokio::fs::remove_file(silence_waveform_output).await;
        progress_clone.0.write().await.remove(&silence_waveform_key);
        progress_clone.0.write().await.remove(&cache_key);
    });

    Ok(HttpResponse::Accepted().json(silence_removal_response("processing", 0)))
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
