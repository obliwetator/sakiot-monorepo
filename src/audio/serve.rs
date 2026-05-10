use actix_files::NamedFile;
use actix_web::{
    get,
    http::header::{ContentDisposition, DispositionType},
    web, HttpRequest, Responder,
};
use serde::Deserialize;
use tracing::info;

use crate::errors::AppError;

use std::path::PathBuf;

use sakiot_paths::RecordingKey;

use super::paths::{NO_SILENCE_PREFIX, NO_SILENCE_RECORDING_PATH, RECORDING_PATH};

/// Legacy dirs use unpadded month (`2026/4`). New writes pad (`2026/04`).
/// Return padded path first, then fall back to unpadded.
fn candidates(
    root: &str,
    guild_id: i64,
    channel_id: i64,
    year: i32,
    month: u32,
    leaf: &str,
) -> [PathBuf; 2] {
    let padded = RecordingKey::new(guild_id, channel_id, year, month, "")
        .recording_dir(root)
        .join(leaf);
    let unpadded = PathBuf::from(root.trim_end_matches('/'))
        .join(format!("{}/{}/{}/{}", guild_id, channel_id, year, month))
        .join(leaf);
    [padded, unpadded]
}

#[derive(Deserialize, Debug)]
pub struct AudioQuery {
    pub silence: Option<bool>,
}

#[get("/audio/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
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
            NO_SILENCE_RECORDING_PATH,
            format!("{}{}", NO_SILENCE_PREFIX, file_name),
        )
    } else {
        (RECORDING_PATH, file_name.clone())
    };

    for path in candidates(root, guild_id as i64, channel_id, year, month as u32, &leaf) {
        if let Ok(f) = NamedFile::open_async(&path).await {
            return Ok(f.into_response(&req));
        }
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
            NO_SILENCE_RECORDING_PATH,
            format!("{}{}", NO_SILENCE_PREFIX, file_name_from_url),
        )
    } else {
        (RECORDING_PATH, file_name_from_url.clone())
    };

    let file = candidates(root, guild_id, channel_id, year, month as u32, &leaf)
        .into_iter()
        .find_map(|p| {
            info!("download try: {} is_silence: {:?}", p.display(), is_silence);
            actix_files::NamedFile::open(&p).ok()
        })
        .ok_or(AppError::FileNotFound)?;

    Ok(file
        .use_last_modified(true)
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![],
        }))
}
