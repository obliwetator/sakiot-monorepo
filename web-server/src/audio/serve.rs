use actix_files::NamedFile;
use actix_web::{
    HttpRequest, Responder,
    http::header::{ContentDisposition, DispositionType},
    route, web,
};
use serde::Deserialize;
use sqlx::{Pool, Postgres};
use tracing::info;

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::media_archive::{MediaArchive, RemoteDisposition};

use sakiot_paths::RecordingKey;

use super::paths::{NO_SILENCE_PREFIX, no_silence_recording_path, recording_path};

fn audio_leaf(file_name: &str, silence_free: bool) -> String {
    let file_name = if file_name.ends_with(".ogg") {
        file_name.to_owned()
    } else {
        format!("{file_name}.ogg")
    };
    if silence_free {
        format!("{NO_SILENCE_PREFIX}{file_name}")
    } else {
        file_name
    }
}

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
    path: web::Path<(i64, i64, i32, i32, String)>,
    query_param: web::Query<AudioQuery>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    media: web::Data<MediaArchive>,
) -> Result<impl Responder, AppError> {
    let (guild_id, channel_id, year, month, file_name) = path.into_inner();

    if file_name.contains("..") || file_name.contains('/') || file_name.contains('\\') {
        return Err(AppError::BadRequest("Invalid file name".into()));
    }

    let token = token.ok_or(AppError::Unauthorized)?;
    super::sessions::require_recording_access(
        &pool,
        guild_id,
        channel_id,
        year,
        month,
        &file_name,
        token.user_id,
    )
    .await?;

    let silence_free = query_param.silence.is_some();
    let root = if silence_free {
        no_silence_recording_path()
    } else {
        recording_path()
    };
    let leaf = audio_leaf(&file_name, silence_free);

    let path = RecordingKey::new(guild_id, channel_id, year, month as u32, "")
        .recording_dir(&root)
        .join(leaf);

    if let Ok(f) = NamedFile::open_async(&path).await {
        return Ok(f.into_response(&req));
    }
    if query_param.silence.is_some() {
        return Err(AppError::FileNotFound);
    }
    let audio_file_id = crate::media_archive::recording_id(
        pool.get_ref(),
        guild_id,
        channel_id,
        year,
        month,
        &file_name,
    )
    .await?
    .ok_or(AppError::FileNotFound)?;
    media
        .serve_recording(
            &req,
            pool.get_ref(),
            audio_file_id,
            RemoteDisposition::Inline,
        )
        .await
}

#[route(
    "/download/{guild_id}/{channel_id}/{year}/{month}/{file_name}",
    method = "GET",
    method = "HEAD"
)]
pub async fn download_audio(
    req: HttpRequest,
    path: web::Path<(i64, i64, i32, i32, String)>,
    is_silence: web::Query<AudioQuery>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    media: web::Data<MediaArchive>,
) -> Result<actix_web::HttpResponse, AppError> {
    let (guild_id, channel_id, year, month, file_name_from_url) = path.into_inner();

    if file_name_from_url.contains("..")
        || file_name_from_url.contains('/')
        || file_name_from_url.contains('\\')
    {
        return Err(AppError::BadRequest("Invalid file name".to_string()));
    }

    let token = token.ok_or(AppError::Unauthorized)?;
    super::sessions::require_recording_access(
        &pool,
        guild_id,
        channel_id,
        year,
        month,
        &file_name_from_url,
        token.user_id,
    )
    .await?;

    let silence_free = is_silence.silence.is_some();
    let root = if silence_free {
        no_silence_recording_path()
    } else {
        recording_path()
    };
    let leaf = audio_leaf(&file_name_from_url, silence_free);

    let path = RecordingKey::new(guild_id, channel_id, year, month as u32, "")
        .recording_dir(&root)
        .join(leaf);

    info!(
        "download try: {} is_silence: {:?}",
        path.display(),
        is_silence
    );
    if let Ok(file) = actix_files::NamedFile::open_async(&path).await {
        return Ok(file
            .use_last_modified(true)
            .set_content_disposition(ContentDisposition {
                disposition: DispositionType::Attachment,
                parameters: vec![],
            })
            .into_response(&req));
    }
    if is_silence.silence.is_some() {
        return Err(AppError::FileNotFound);
    }
    let audio_file_id = crate::media_archive::recording_id(
        pool.get_ref(),
        guild_id,
        channel_id,
        year,
        month,
        &file_name_from_url,
    )
    .await?
    .ok_or(AppError::FileNotFound)?;
    media
        .serve_recording(
            &req,
            pool.get_ref(),
            audio_file_id,
            RemoteDisposition::Attachment,
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::audio_leaf;

    #[test]
    fn audio_leaf_adds_ogg_to_a_recording_stem() {
        assert_eq!(
            audio_leaf("1786473460682-183931044829986817", false),
            "1786473460682-183931044829986817.ogg"
        );
    }

    #[test]
    fn audio_leaf_preserves_an_existing_extension() {
        assert_eq!(audio_leaf("recording.ogg", false), "recording.ogg");
    }

    #[test]
    fn audio_leaf_adds_the_silence_free_prefix_after_normalizing() {
        assert_eq!(audio_leaf("recording", true), "_no_silence_recording.ogg");
    }
}
