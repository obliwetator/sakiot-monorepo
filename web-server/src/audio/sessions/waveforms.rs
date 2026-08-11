use super::*;

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/waveform",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Combined peaks; explicit gaps are zero-valued", body = SessionWaveformResponse),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Session not found", body = crate::errors::ApiError),
        (status = 500, description = "Waveform generation failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/sessions/{recording_session_id}/waveform")]
pub async fn get_session_waveform(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    progress: web::Data<WaveformProgressContainer>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    require_session_access(&pool, session_id, token.user_id).await?;
    let (cache_key, output) = session_waveform_cache(session_id, false);
    session_waveform_status(&cache_key, &output, &progress).await
}

#[utoipa::path(
    post,
    path = "/api/audio/sessions/{recording_session_id}/waveform/rebuild",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Combined waveform rebuild started or already running", body = SessionWaveformResponse),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Session not found", body = crate::errors::ApiError),
        (status = 500, description = "Waveform generation failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/audio/sessions/{recording_session_id}/waveform/rebuild")]
pub async fn rebuild_session_waveform(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    progress: web::Data<WaveformProgressContainer>,
    media: web::Data<MediaArchive>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let (cache_key, output) = session_waveform_cache(session_id, false);
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    {
        let mut map = progress.0.write().await;
        if let Some(value) = map.get(&cache_key).copied() {
            if value >= 0 {
                return Ok(HttpResponse::Ok().json(SessionWaveformResponse {
                    progress: value.min(99),
                    building: true,
                    data: None,
                }));
            }
            map.remove(&cache_key);
        }
        map.insert(cache_key.clone(), 0);
    }

    let composite =
        PathBuf::from(waveform_path()).join(format!("{cache_key}-{}.ogg", uuid::Uuid::new_v4()));
    let pool_clone = pool.clone();
    let progress_clone = progress.clone();
    let cache_key_clone = cache_key.clone();
    let media_clone = media.clone();
    tokio::spawn(async move {
        progress_clone
            .0
            .write()
            .await
            .insert(cache_key_clone.clone(), 1);
        let composition_progress = CompositionProgress {
            cache_key: cache_key_clone.clone(),
            progress: progress_clone.clone(),
            completed: 85,
        };
        if let Err(err) = compose_session_with_progress(
            &pool_clone,
            &access,
            None,
            None,
            false,
            &composite,
            composition_progress,
            media_clone.get_ref(),
        )
        .await
        {
            tracing::error!(session_id, "session waveform composition failed: {}", err);
            progress_clone.0.write().await.insert(cache_key_clone, -1);
            return;
        }
        let generation = crate::waveform::generate_peaks_background(
            composite.to_string_lossy().into_owned(),
            output.to_string_lossy().into_owned(),
            cache_key_clone.clone(),
            // Sessions run for hours and the dashboard zooms into them to cut
            // clips, so they are sampled by duration rather than to a fixed
            // 2500 points that would smear a six hour recording into a blob.
            crate::waveform::PeakDensity::PerSecond(SESSION_PEAKS_PER_SECOND),
            progress_clone.clone(),
            None,
            Some((85, 99)),
        )
        .await;
        let _ = tokio::fs::remove_file(composite).await;
        if let Err(err) = generation {
            tracing::error!(session_id, "session waveform generation failed: {}", err);
            progress_clone.0.write().await.insert(cache_key_clone, -1);
        }
    });

    Ok(HttpResponse::Ok().json(SessionWaveformResponse {
        progress: 0,
        building: true,
        data: None,
    }))
}

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/silence-free/waveform",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Silence-free session waveform status and peaks", body = SessionWaveformResponse),
        (status = 400, description = "Session is not finalized", body = crate::errors::ApiError),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Silence-free session has not been generated", body = crate::errors::ApiError),
        (status = 500, description = "Waveform generation failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/sessions/{recording_session_id}/silence-free/waveform")]
pub async fn get_session_silence_free_waveform(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    progress: web::Data<WaveformProgressContainer>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let source = session_silence_free_path(&access)?;
    if !tokio::fs::try_exists(&source).await? {
        return Err(AppError::FileNotFound);
    }
    let (cache_key, output) = session_waveform_cache(session_id, true);
    if waveform_is_stale(&output, &source).await {
        let _ = tokio::fs::remove_file(&output).await;
    }
    session_waveform_status(&cache_key, &output, &progress).await
}

#[utoipa::path(
    post,
    path = "/api/audio/sessions/{recording_session_id}/silence-free/waveform/rebuild",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Silence-free waveform rebuild started or already running", body = SessionWaveformResponse),
        (status = 400, description = "Session is not finalized", body = crate::errors::ApiError),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Silence-free session has not been generated", body = crate::errors::ApiError),
        (status = 500, description = "Waveform generation failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/audio/sessions/{recording_session_id}/silence-free/waveform/rebuild")]
pub async fn rebuild_session_silence_free_waveform(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    progress: web::Data<WaveformProgressContainer>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let source = session_silence_free_path(&access)?;
    if !tokio::fs::try_exists(&source).await? {
        return Err(AppError::FileNotFound);
    }
    let (cache_key, output) = session_waveform_cache(session_id, true);
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    {
        let mut map = progress.0.write().await;
        if let Some(value) = map.get(&cache_key).copied() {
            if value >= 0 {
                return Ok(HttpResponse::Ok().json(SessionWaveformResponse {
                    progress: value.min(99),
                    building: true,
                    data: None,
                }));
            }
            map.remove(&cache_key);
        }
        map.insert(cache_key.clone(), 0);
    }

    let progress_clone = progress.clone();
    let cache_key_clone = cache_key.clone();
    let source_version = tokio::fs::metadata(&source)
        .await
        .and_then(|metadata| metadata.modified())?;
    tokio::spawn(async move {
        let generation = crate::waveform::generate_peaks_background(
            source.to_string_lossy().into_owned(),
            output.to_string_lossy().into_owned(),
            cache_key_clone.clone(),
            crate::waveform::PeakDensity::PerSecond(SESSION_PEAKS_PER_SECOND),
            progress_clone.clone(),
            None,
            None,
        )
        .await;
        if let Err(err) = generation {
            tracing::error!(
                session_id,
                "silence-free session waveform generation failed: {}",
                err
            );
            progress_clone.0.write().await.insert(cache_key_clone, -1);
            return;
        }
        let current_version = tokio::fs::metadata(&source)
            .await
            .and_then(|metadata| metadata.modified());
        if current_version.as_ref().ok() != Some(&source_version) {
            let _ = tokio::fs::remove_file(output).await;
            progress_clone.0.write().await.remove(&cache_key_clone);
            tracing::warn!(
                session_id,
                "discarded silence-free waveform because its source changed"
            );
        }
    });

    Ok(HttpResponse::Ok().json(SessionWaveformResponse {
        progress: 0,
        building: true,
        data: None,
    }))
}

pub(super) async fn session_waveform_status(
    cache_key: &str,
    output: &Path,
    progress: &web::Data<WaveformProgressContainer>,
) -> Result<HttpResponse, AppError> {
    {
        let mut map = progress.0.write().await;
        if let Some(value) = map.get(cache_key).copied() {
            if value < 0 {
                map.remove(cache_key);
                return Err(AppError::InternalError);
            }
            return Ok(HttpResponse::Ok().json(SessionWaveformResponse {
                progress: value.min(99),
                building: true,
                data: None,
            }));
        }
    }

    if tokio::fs::try_exists(output).await.unwrap_or(false) {
        return waveform_file_response(output).await;
    }

    Ok(HttpResponse::Ok().json(SessionWaveformResponse {
        progress: 0,
        building: false,
        data: None,
    }))
}

pub(super) fn session_waveform_cache(session_id: i64, silence_free: bool) -> (String, PathBuf) {
    let suffix = if silence_free { "-silence-free" } else { "" };
    let cache_key = format!("logical-session-{session_id}{suffix}");
    let output = PathBuf::from(waveform_path()).join(format!("{cache_key}.dat"));
    (cache_key, output)
}

pub(super) async fn waveform_is_stale(waveform: &Path, source: &Path) -> bool {
    let Ok(waveform_modified) = tokio::fs::metadata(waveform)
        .await
        .and_then(|metadata| metadata.modified())
    else {
        return false;
    };
    let Ok(source_modified) = tokio::fs::metadata(source)
        .await
        .and_then(|metadata| metadata.modified())
    else {
        return false;
    };
    waveform_modified < source_modified
}

pub(super) async fn waveform_file_response(path: &Path) -> Result<HttpResponse, AppError> {
    let bytes = tokio::fs::read(path).await?;
    Ok(HttpResponse::Ok().json(SessionWaveformResponse {
        progress: 100,
        building: false,
        data: Some(BASE64_STANDARD.encode(bytes)),
    }))
}
