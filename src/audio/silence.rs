use std::process::Stdio;

use actix_web::{get, web, HttpRequest, HttpResponse};
use sqlx::{Pool, Postgres};
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::errors::AppError;

use super::paths::{no_silence_recording_path, recording_path, NO_SILENCE_PREFIX};
use super::types::HashMapContainer;
use super::util::{file_exists, get_file_path_root, handle_idempotency_key};

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct RemoveSilenceResponse {
    pub url: String,
    pub message: &'static str,
}

#[utoipa::path(
    get,
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
        (status = 200, description = "Silence removal file exists or processing started", body = RemoveSilenceResponse),
        (status = 202, description = "Existing processing request completed", body = RemoveSilenceResponse),
        (status = 400, description = "Missing idempotency key", body = crate::errors::ApiError),
        (status = 503, description = "Concurrent processing state unavailable", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/remove_silence/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
pub async fn remove_silence(
    req: HttpRequest,
    path: web::Path<(i64, i64, i32, i32, String)>,
    hashmap: web::Data<HashMapContainer>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let path = path.into_inner();

    let file_path: String = get_file_path_root(&recording_path(), &path);
    let no_silence_file_path = get_file_path_root(&no_silence_recording_path(), &path);
    let file_no_silence =
        no_silence_file_path.to_owned() + "/" + NO_SILENCE_PREFIX + path.4.as_str() + ".ogg";

    let _idempotency_key = handle_idempotency_key(&req)?;

    info!("File name: {}", path.4);

    if hashmap.0.read().await.contains_key(&path.4) {
        info!("Already processing");
        let mut rec = {
            let lock = hashmap.0.read().await;
            match lock.get(&path.4) {
                Some(sender) => sender.subscribe(),
                None => {
                    return Err(AppError::ServiceUnavailable("retry".into()));
                }
            }
        };
        match rec.recv().await {
            Ok(value) => {
                if value == 0 {
                    info!("received value");
                }
            }
            Err(e) => {
                error!("broadcast recv error: {:?}", e);
                return Err(AppError::InternalError);
            }
        }

        Ok(HttpResponse::Accepted().json(RemoveSilenceResponse {
            url: file_no_silence,
            message: " Success",
        }))
    } else if file_exists(&(no_silence_file_path.to_owned() + "/" + &path.4 + ".ogg")).await {
        info!("file already exists");
        Ok(HttpResponse::Ok().json(RemoveSilenceResponse {
            url: file_no_silence,
            message: "File already exists",
        }))
    } else {
        info!("Creating new file");
        let (tx, _) = broadcast::channel::<i32>(10);
        {
            hashmap
                .0
                .write()
                .await
                .insert(path.4.to_owned(), tx.clone());
        }
        if let Err(err) = tokio::fs::create_dir_all(&no_silence_file_path).await {
            error!("create_dir_all failed: {}", err);
            hashmap.0.write().await.remove(&path.4);
            return Err(AppError::IoError(err));
        }

        let file: String = file_path.to_owned() + "/" + path.4.as_str() + ".ogg";
        let file_no_silence_clone = file_no_silence.to_owned();

        info!("NO SILENCE FILE PATH: {}", &file_no_silence);

        let hashmap_clone = hashmap.clone();
        tokio::spawn(async move {
            let file_name: String = path.4.clone();
            let mut command = tokio::process::Command::new("ffmpeg");
            command
                .args(["-i", &file])
                .args([
                    "-af",
                    "silenceremove=stop_periods=-1:stop_duration=1:stop_threshold=-40dB",
                ])
                .arg(file_no_silence_clone)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            let _output = match command.output().await {
                Ok(result) => result,
                Err(err) => {
                    error!("ffmpeg spawn failed: {}", err);
                    hashmap_clone.0.write().await.remove(&path.4);
                    return;
                }
            };

            if let Err(e) = sqlx::query!(
                "UPDATE public.audio_files
				SET silence=true
				WHERE file_name=$1;",
                file_name
            )
            .execute(pool.get_ref())
            .await
            {
                error!("audio_files UPDATE failed: {:?}", e);
                hashmap_clone.0.write().await.remove(&path.4);
                return;
            }

            match tx.send(0) {
                Ok(_) => {
                    info!("Value sent success");
                }
                Err(_) => {
                    warn!("There were no receivers to receive the value.")
                }
            }
            {
                hashmap_clone.0.write().await.remove(&path.4);
            }
        });

        Ok(HttpResponse::Ok().json(RemoveSilenceResponse {
            url: file_no_silence,
            message: "Request Accepted",
        }))
    }
}
