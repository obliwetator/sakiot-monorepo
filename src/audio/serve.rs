use actix_files::NamedFile;
use actix_web::{
    get,
    http::header::{ContentDisposition, DispositionType},
    web, HttpRequest, HttpResponse, Responder,
};
use serde::Deserialize;
use tracing::info;

use crate::errors::AppError;

use super::paths::{NO_SILENCE_PREFIX, NO_SILENCE_RECORDING_PATH, RECORDING_PATH};

#[derive(Deserialize, Debug)]
pub struct AudioQuery {
    pub silence: Option<bool>,
}

#[get("/audio/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
pub async fn get_audio(
    req: HttpRequest,
    path: web::Path<(u64, String, i32, i32, String)>,
    query_param: web::Query<AudioQuery>,
) -> impl Responder {
    let (guild_id, channel_id, year, month, file_name) = path.into_inner();

    let path = {
        if let Some(value) = query_param.silence {
            if value {
                format!(
                    "{}{}/{}/{}/{}/{}{}",
                    NO_SILENCE_RECORDING_PATH,
                    guild_id,
                    channel_id,
                    year,
                    month,
                    NO_SILENCE_PREFIX,
                    file_name
                )
            } else {
                format!(
                    "{}{}/{}/{}/{}/{}",
                    RECORDING_PATH, guild_id, channel_id, year, month, file_name
                )
            }
        } else {
            format!(
                "{}{}/{}/{}/{}/{}",
                RECORDING_PATH, guild_id, channel_id, year, month, file_name
            )
        }
    };

    info!("File path: {}", path);

    match NamedFile::open_async(path).await {
        Ok(ok) => ok.into_response(&req),
        Err(_) => HttpResponse::NotFound().finish(),
    }
}

#[get("/download/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
pub async fn download_audio(
    _req: HttpRequest,
    path: web::Path<(i64, i64, i32, i32, String)>,
    is_silence: web::Query<AudioQuery>,
) -> Result<NamedFile, AppError> {
    let (guild_id, channel_id, year, month, file_name_from_url) = path.into_inner();

    let file_name_without_guild_id = format!("{}/{}/{}", year, month, file_name_from_url);
    let temp_file = format!(
        "{}/{}/{}{}",
        year, month, NO_SILENCE_PREFIX, file_name_from_url
    );

    info!(
        "file_path: {:#?} is silence recording? {:#?}",
        format!(
            "{}{}/{}/{}",
            RECORDING_PATH, guild_id, channel_id, &file_name_without_guild_id
        ),
        is_silence
    );

    let full_path = if is_silence.silence.is_some() {
        format!(
            "{}{}/{}/{}",
            NO_SILENCE_RECORDING_PATH, guild_id, channel_id, &temp_file
        )
    } else {
        format!(
            "{}{}/{}/{}",
            RECORDING_PATH, guild_id, channel_id, &file_name_without_guild_id
        )
    };

    let file = actix_files::NamedFile::open(&full_path).map_err(|_| AppError::NotFound)?;

    Ok(file
        .use_last_modified(true)
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![],
        }))
}
