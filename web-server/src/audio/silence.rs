use actix_web::{HttpRequest, HttpResponse, post, web};
use sqlx::{Pool, Postgres};
use tracing::info;

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::media_archive::MediaArchive;
use crate::media_jobs::{MediaJobRequest, MediaJobStatus};

use super::paths::{NO_SILENCE_PREFIX, no_silence_recording_path, recording_path};
use super::util::{
    file_exists, get_file_path_root, handle_idempotency_key, is_stale, is_valid_file_segment,
};

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct RemoveSilenceResponse {
    pub url: String,
    pub message: &'static str,
}

/// Kept as a zero-sized compatibility app-data type for integration tests and
/// overlapping releases. Durable silence work now lives in `media_jobs`.
#[derive(Debug, Default)]
pub struct SilenceJobContainer {
    _compat: (),
}

fn recording_fingerprint(path: &(i64, i64, i32, i32, String)) -> String {
    format!(
        "{}/{}/{:04}/{:02}/{}",
        path.0, path.1, path.2, path.3, path.4
    )
}

#[utoipa::path(
    post,
    path = "/api/remove_silence/{guild_id}/{channel_id}/{year}/{month}/{file_name}",
    tag = "audio",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("channel_id" = i64, Path, description = "Discord channel id"),
        ("year" = i32, Path, description = "Recording year"),
        ("month" = i32, Path, description = "Recording month"),
        ("file_name" = String, Path, description = "Recording file stem"),
        ("Idempotency-Key" = String, Header, description = "Idempotency key for processing request"),
    ),
    responses(
        (status = 200, description = "Silence-free file already exists", body = RemoveSilenceResponse),
        (status = 202, description = "Silence removal durably queued", body = MediaJobStatus),
        (status = 400, description = "Invalid file name or missing idempotency key", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "Missing channel permission", body = crate::errors::ApiError),
        (status = 409, description = "Idempotency key reused for another request", body = crate::errors::ApiError),
        (status = 503, description = "Concurrent processing state unavailable", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[post("/remove_silence/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
pub async fn remove_silence(
    req: HttpRequest,
    path: web::Path<(i64, i64, i32, i32, String)>,
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
        token.user_id,
    )
    .await?;

    let file_path: String = get_file_path_root(&recording_path(), &path);
    let no_silence_file_path = get_file_path_root(&no_silence_recording_path(), &path);
    let file_no_silence =
        no_silence_file_path.to_owned() + "/" + NO_SILENCE_PREFIX + path.4.as_str() + ".ogg";
    let idempotency_key = handle_idempotency_key(&req)?;
    let fingerprint = recording_fingerprint(&path);

    info!("File name: {}", path.4);

    // Source recording and the (correctly prefixed) cached output. The cache is
    // only valid when it's newer than the source — a file produced from an
    // earlier, shorter version of the recording (e.g. while still live) is
    // treated as stale, so the user can refresh it as the recording grows.
    let source_file = format!("{}/{}.ogg", file_path, path.4);
    let cached_fresh =
        file_exists(&file_no_silence).await && !is_stale(&source_file, &file_no_silence).await;

    if cached_fresh {
        info!("silence-free file already exists");
        return Ok(HttpResponse::Ok().json(RemoveSilenceResponse {
            url: file_no_silence,
            message: "File already exists",
        }));
    }
    let job_request = MediaJobRequest::RecordingSilence {
        guild_id: path.0,
        channel_id: path.1,
        year: path.2,
        month: path.3,
        file_name: path.4,
    };
    let status = crate::media_jobs::enqueue(
        pool.get_ref(),
        Some(path.0),
        token.user_id,
        &idempotency_key,
        &fingerprint,
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_recording_silence_job(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    guild_id: i64,
    channel_id: i64,
    year: i32,
    month: i32,
    file_name: &str,
    job_id: &str,
    attempt_token: &str,
) -> Result<(Option<String>, Option<std::path::PathBuf>), AppError> {
    let tuple = (guild_id, channel_id, year, month, file_name.to_owned());
    let source_dir = get_file_path_root(&recording_path(), &tuple);
    let output_dir = get_file_path_root(&no_silence_recording_path(), &tuple);
    let source = format!("{source_dir}/{file_name}.ogg");
    let output =
        std::path::PathBuf::from(format!("{output_dir}/{NO_SILENCE_PREFIX}{file_name}.ogg"));
    let audio_file_id =
        crate::media_archive::recording_id(pool, guild_id, channel_id, year, month, file_name)
            .await?
            .ok_or(AppError::FileNotFound)?;
    media
        .ensure_recording_local(pool, audio_file_id, std::path::Path::new(&source))
        .await?;
    tokio::fs::create_dir_all(&output_dir).await?;
    let temporary = output.with_extension(format!("{job_id}.{attempt_token}.tmp.ogg"));
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .arg("-y")
        .args(["-i", &source])
        .args([
            "-af",
            "silenceremove=stop_periods=-1:stop_duration=1:stop_threshold=-40dB",
        ])
        .arg(&temporary);
    if let Err(error) = crate::ffmpeg::run_ffmpeg(command).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(error);
    }
    let mut tx = crate::media_jobs::begin_publication(pool, job_id, attempt_token).await?;
    tokio::fs::rename(&temporary, &output).await?;
    if let Err(error) = sqlx::query!(
        "UPDATE audio_files SET silence=true WHERE file_name=$1",
        file_name
    )
    .execute(&mut *tx)
    .await
    {
        return Err(error.into());
    }
    let url = format!("/api/audio/{guild_id}/{channel_id}/{year}/{month}/{file_name}?silence=true");
    crate::media_jobs::complete_publication(tx, job_id, attempt_token, &url, None).await?;
    Ok((Some(url), None))
}
