use actix_files::NamedFile;
use actix_web::{
    get,
    http::header::{ContentDisposition, DispositionType},
    route, web, HttpRequest, Responder,
};
use serde::Deserialize;
use tracing::info;

use crate::errors::AppError;

use sakiot_paths::RecordingKey;

use super::paths::{no_silence_recording_path, recording_path, NO_SILENCE_PREFIX};

#[derive(Deserialize, Debug)]
pub struct AudioQuery {
    pub silence: Option<bool>,
}

#[route(
    "/audio/{guild_id}/{channel_id}/{year}/{month}/{file_name}",
    method = "GET",
    method = "HEAD"
)]
pub async fn get_audio(
    req: HttpRequest,
    path: web::Path<(u64, i64, i32, i32, String)>,
    query_param: web::Query<AudioQuery>,
) -> Result<impl Responder, AppError> {
    let (guild_id, channel_id, year, month, file_name) = path.into_inner();

    if file_name.contains("..") || file_name.contains('/') || file_name.contains('\\') {
        return Err(AppError::BadRequest("Invalid file name".into()));
    }

    let (root, leaf) = if query_param.silence.is_some() {
        (
            no_silence_recording_path(),
            format!("{}{}", NO_SILENCE_PREFIX, file_name),
        )
    } else {
        (recording_path(), file_name.clone())
    };

    let path = RecordingKey::new(guild_id as i64, channel_id, year, month as u32, "")
        .recording_dir(&root)
        .join(leaf);

    if let Ok(f) = NamedFile::open_async(&path).await {
        return Ok(f.into_response(&req));
    }
    Err(AppError::FileNotFound)
}

#[get("/download/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
pub async fn download_audio(
    _req: HttpRequest,
    path: web::Path<(i64, i64, i32, i32, String)>,
    is_silence: web::Query<AudioQuery>,
) -> Result<NamedFile, AppError> {
    let (guild_id, channel_id, year, month, file_name_from_url) = path.into_inner();

    if file_name_from_url.contains("..")
        || file_name_from_url.contains('/')
        || file_name_from_url.contains('\\')
    {
        return Err(AppError::BadRequest("Invalid file name".to_string()));
    }

    let (root, leaf) = if is_silence.silence.is_some() {
        (
            no_silence_recording_path(),
            format!("{}{}", NO_SILENCE_PREFIX, file_name_from_url),
        )
    } else {
        (recording_path(), file_name_from_url.clone())
    };

    let path = RecordingKey::new(guild_id, channel_id, year, month as u32, "")
        .recording_dir(&root)
        .join(leaf);

    info!(
        "download try: {} is_silence: {:?}",
        path.display(),
        is_silence
    );
    let file = actix_files::NamedFile::open_async(&path)
        .await
        .map_err(|_| AppError::FileNotFound)?;

    Ok(file
        .use_last_modified(true)
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![],
        }))
}
