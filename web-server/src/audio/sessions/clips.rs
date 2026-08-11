use super::*;

#[utoipa::path(
    post,
    path = "/api/audio/sessions/{recording_session_id}/clips",
    tag = "clips",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    request_body = StartEnd,
    responses(
        (status = 200, description = "Cross-fragment clip created", body = crate::clips::CreateClipResponse),
        (status = 400, description = "Clip must be 1-20 seconds", body = crate::errors::ApiError),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 500, description = "Composition failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/audio/sessions/{recording_session_id}/clips")]
pub async fn create_session_clip(
    path: web::Path<i64>,
    range: web::Json<StartEnd>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    media: web::Data<MediaArchive>,
    progress: web::Data<WaveformProgressContainer>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let start = f64::from(range.start.unwrap_or(0.0));
    let end = f64::from(range.end.unwrap_or(0.0));
    let silence_free = range.silence_free.unwrap_or(false);
    if !start.is_finite()
        || !end.is_finite()
        || start < 0.0
        || !(1.0..=20.0).contains(&(end - start))
    {
        return Err(AppError::BadRequest(
            "Clip duration must be between 1 and 20 seconds".into(),
        ));
    }

    let now = chrono::Utc::now();
    let clip_id = uuid::Uuid::new_v4().to_string();
    let target_dir = PathBuf::from(clips_path())
        .join(now.year().to_string())
        .join(format!("{:02}", now.month()));
    tokio::fs::create_dir_all(&target_dir).await?;
    let full_path = target_dir.join(format!("{clip_id}.ogg"));
    if silence_free {
        let source = session_silence_free_path(&access)?;
        crop_silence_free_session(&source, start, end, &full_path).await?;
    } else {
        compose_session(
            &pool,
            &access,
            Some(start),
            Some(end),
            false,
            &full_path,
            media.get_ref(),
        )
        .await?;
    }
    let size = tokio::fs::metadata(&full_path).await?.len() as i64;
    let saved_file_name = format!("{}/{:02}/{clip_id}.ogg", now.year(), now.month());
    let name = range
        .name
        .clone()
        .unwrap_or_else(|| format!("session-{session_id}"));

    let insert = sqlx::query(
        "INSERT INTO clips
            (clip_id, length, size, channel_id, guild_id, user_id,
             original_file_name, saved_file_name, name, start_time,
             recording_session_id, silence_free)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(&clip_id)
    .bind((end - start) as f32)
    .bind(size)
    .bind(access.starting_channel_id)
    .bind(access.guild_id)
    .bind(token.user_id)
    .bind(if silence_free {
        format!("session-silence-free:{session_id}")
    } else {
        format!("session:{session_id}")
    })
    .bind(&saved_file_name)
    .bind(&name)
    .bind(start as f32)
    .bind(session_id)
    .bind(silence_free)
    .execute(pool.get_ref())
    .await;
    if let Err(err) = insert {
        let _ = tokio::fs::remove_file(&full_path).await;
        return Err(AppError::DbError(err));
    }

    super::super::peaks::spawn_clip_waveform(clip_id.clone(), full_path, progress);

    Ok(HttpResponse::Ok().json(crate::clips::CreateClipResponse {
        status: "success",
        file: saved_file_name,
        id: clip_id,
        name,
    }))
}

pub(super) async fn crop_silence_free_session(
    source: &Path,
    start: f64,
    end: f64,
    output: &Path,
) -> Result<(), AppError> {
    if !tokio::fs::try_exists(source).await? {
        return Err(AppError::FileNotFound);
    }
    let probe = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(source)
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
    let duration = String::from_utf8_lossy(&probe.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .ok_or_else(|| AppError::FfmpegError("ffprobe returned no audio duration".into()))?;
    if end > duration + 0.02 {
        return Err(AppError::BadRequest(
            "Clip range exceeds the silence-free session duration".into(),
        ));
    }

    let result = tokio::process::Command::new("ffmpeg")
        .arg("-y")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &start.to_string(),
        ])
        .arg("-i")
        .arg(source)
        .args(["-t", &(end - start).to_string()])
        .args(["-map", "0:a:0", "-c:a", "libopus", "-b:a", "96k"])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
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
    if !result.status.success() {
        let _ = tokio::fs::remove_file(output).await;
        return Err(AppError::FfmpegError(
            String::from_utf8_lossy(&result.stderr).into_owned(),
        ));
    }
    Ok(())
}
