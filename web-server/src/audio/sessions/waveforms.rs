use super::*;
use crate::media_jobs::JobAttempt;

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
    let access =
        require_session_access(&pool, session_id, crate::permissions::Viewer::of(&token)).await?;
    let (cache_key, output) = session_waveform_cache(session_id, false);
    if let Some(job) =
        crate::media_jobs::active_for_resource(&pool, token.user_id, "session_waveform", &cache_key)
            .await?
    {
        return Ok(building_response(job));
    }
    // A live session's waveform is built only when asked for, since each
    // build reads the whole session so far. Once the session has finished,
    // one that does not cover all of it is rebuilt without being asked.
    if let Some(ended_at_ms) = finished_at_ms(&access)
        && modified_ms(&output)
            .await
            .is_none_or(|built_from| built_from < ended_at_ms)
    {
        return build_finished_session_waveform(
            &pool,
            crate::permissions::Viewer::of(&token),
            access.guild_id,
            session_id,
            false,
            ended_at_ms,
        )
        .await;
    }
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
    request: HttpRequest,
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access =
        require_session_access(&pool, session_id, crate::permissions::Viewer::of(&token)).await?;
    enqueue_session_waveform(
        &request,
        &pool,
        crate::permissions::Viewer::of(&token),
        access.guild_id,
        session_id,
        false,
    )
    .await
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
    let access =
        require_session_access(&pool, session_id, crate::permissions::Viewer::of(&token)).await?;
    let source = session_silence_free_path(&access)?;
    if !tokio::fs::try_exists(&source).await? {
        return Err(AppError::FileNotFound);
    }
    let (cache_key, output) = session_waveform_cache(session_id, true);
    if waveform_is_stale(&output, &source).await {
        let _ = tokio::fs::remove_file(&output).await;
    }
    if let Some(job) =
        crate::media_jobs::active_for_resource(&pool, token.user_id, "session_waveform", &cache_key)
            .await?
    {
        return Ok(building_response(job));
    }
    // Silence-free audio exists only once the session has finished.
    if !tokio::fs::try_exists(&output).await.unwrap_or(false) {
        return build_finished_session_waveform(
            &pool,
            crate::permissions::Viewer::of(&token),
            access.guild_id,
            session_id,
            true,
            modified_ms(&source).await.unwrap_or_default(),
        )
        .await;
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
    request: HttpRequest,
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access =
        require_session_access(&pool, session_id, crate::permissions::Viewer::of(&token)).await?;
    let source = session_silence_free_path(&access)?;
    if !tokio::fs::try_exists(&source).await? {
        return Err(AppError::FileNotFound);
    }
    enqueue_session_waveform(
        &request,
        &pool,
        crate::permissions::Viewer::of(&token),
        access.guild_id,
        session_id,
        true,
    )
    .await
}

async fn enqueue_session_waveform(
    http_request: &HttpRequest,
    pool: &web::Data<Pool<Postgres>>,
    requester: crate::permissions::Viewer,
    guild_id: i64,
    session_id: i64,
    silence_free: bool,
) -> Result<HttpResponse, AppError> {
    let (resource, _) = session_waveform_cache(session_id, silence_free);
    let request = crate::media_jobs::MediaJobRequest::SessionWaveform {
        session_id,
        silence_free,
    };
    let key = http_request
        .headers()
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let status =
        crate::media_jobs::enqueue(pool, Some(guild_id), requester, &key, &resource, &request)
            .await?;
    Ok(HttpResponse::Accepted()
        .insert_header((
            actix_web::http::header::LOCATION,
            format!("/api/media-jobs/{}", status.id),
        ))
        .json(status))
}

fn building_response(job: crate::media_jobs::MediaJobStatus) -> HttpResponse {
    HttpResponse::Ok().json(SessionWaveformResponse {
        progress: job.progress,
        building: true,
        data: None,
        job_id: Some(job.id),
        error: None,
    })
}

/// When a finalized session ended; `None` while it can still grow.
fn finished_at_ms(access: &SessionAccess) -> Option<i64> {
    (access.state == "finalized").then(|| access.ended_at_ms.unwrap_or(access.started_at_ms))
}

/// A file's modification time in Unix milliseconds. Session waveform jobs
/// set it to when they started reading the session.
async fn modified_ms(path: &Path) -> Option<i64> {
    let modified = tokio::fs::metadata(path).await.ok()?.modified().ok()?;
    let since_epoch = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(since_epoch.as_millis()).ok()
}

/// Starts a finished session's waveform without being asked. The job key
/// names what the waveform covers, so repeated requests share one job and a
/// failed build is reported instead of retried on every request.
async fn build_finished_session_waveform(
    pool: &web::Data<Pool<Postgres>>,
    requester: crate::permissions::Viewer,
    guild_id: i64,
    session_id: i64,
    silence_free: bool,
    covers: i64,
) -> Result<HttpResponse, AppError> {
    let (resource, _) = session_waveform_cache(session_id, silence_free);
    let request = crate::media_jobs::MediaJobRequest::SessionWaveform {
        session_id,
        silence_free,
    };
    let key = format!("{resource}-{covers}");
    let mut job =
        crate::media_jobs::enqueue(pool, Some(guild_id), requester, &key, &resource, &request)
            .await?;
    if job.status == "ready" {
        // That build finished, but its waveform has since gone.
        let key = format!("{key}-{}", chrono::Utc::now().timestamp_millis());
        job =
            crate::media_jobs::enqueue(pool, Some(guild_id), requester, &key, &resource, &request)
                .await?;
    }
    if job.status == "failed" {
        return Ok(HttpResponse::Ok().json(SessionWaveformResponse {
            progress: 0,
            building: false,
            data: None,
            job_id: None,
            error: Some(
                job.error
                    .unwrap_or_else(|| "The waveform could not be built.".to_owned()),
            ),
        }));
    }
    Ok(building_response(job))
}

pub(crate) async fn run_session_waveform_job(
    attempt: &JobAttempt<'_>,
    session_id: i64,
    silence_free: bool,
) -> Result<(Option<String>, Option<PathBuf>), AppError> {
    let JobAttempt {
        pool,
        media,
        requester,
        id: job_id,
        token: attempt_token,
    } = *attempt;
    let pool_data = web::Data::new(pool.clone());
    let access = require_session_access(&pool_data, session_id, requester).await?;
    let (cache_key, output) = session_waveform_cache(session_id, silence_free);
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let attempt_output = output.with_extension(format!("{job_id}.{attempt_token}.tmp.dat"));
    let progress = web::Data::new(WaveformProgressContainer(tokio::sync::RwLock::new(
        std::collections::HashMap::new(),
    )));
    // Audio recorded after this is not in the waveform.
    let read_from = std::time::SystemTime::now();
    let input = if silence_free {
        let source = session_silence_free_path(&access)?;
        if !tokio::fs::try_exists(&source).await? {
            return Err(AppError::FileNotFound);
        }
        source
    } else {
        let composite =
            PathBuf::from(waveform_path()).join(format!("{cache_key}-{attempt_token}.ogg"));
        let composition_progress = CompositionProgress {
            cache_key: cache_key.clone(),
            progress: progress.clone(),
            completed: 85,
        };
        if let Err(error) = crate::media_jobs::track_progress(
            pool,
            job_id,
            attempt_token,
            "composing",
            &progress,
            &cache_key,
            compose_session_inner(
                &pool_data,
                &access,
                CompositionRequest::default(),
                &composite,
                Some(composition_progress),
                media,
            ),
        )
        .await
        {
            let _ = tokio::fs::remove_file(&composite).await;
            return Err(error);
        }
        composite
    };
    let generation = crate::media_jobs::track_progress(
        pool,
        job_id,
        attempt_token,
        "waveform",
        &progress,
        &cache_key,
        async {
            crate::waveform::generate_peaks_background(
                input.to_string_lossy().into_owned(),
                attempt_output.to_string_lossy().into_owned(),
                cache_key.clone(),
                crate::waveform::PeakDensity::PerSecond(SESSION_PEAKS_PER_SECOND),
                progress.clone(),
                None,
                Some((85, 99)),
            )
            .await
            .map_err(|error| AppError::IoError(std::io::Error::other(error.to_string())))
        },
    )
    .await;
    if !silence_free {
        let _ = tokio::fs::remove_file(&input).await;
    }
    generation?;
    // Stamped with when reading started, so a waveform built while the
    // session was live reads as older than its end and is rebuilt.
    tokio::fs::File::options()
        .write(true)
        .open(&attempt_output)
        .await?
        .into_std()
        .await
        .set_modified(read_from)?;
    let tx = crate::media_jobs::begin_publication(pool, job_id, attempt_token).await?;
    tokio::fs::rename(&attempt_output, &output).await?;
    let route = if silence_free {
        format!("/api/audio/sessions/{session_id}/silence-free/waveform")
    } else {
        format!("/api/audio/sessions/{session_id}/waveform")
    };
    crate::media_jobs::complete_publication(tx, job_id, attempt_token, &route, None).await?;
    Ok((Some(route), None))
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
                job_id: None,
                error: None,
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
        job_id: None,
        error: None,
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
        job_id: None,
        error: None,
    }))
}
