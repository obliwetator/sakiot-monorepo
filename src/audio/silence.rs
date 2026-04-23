use std::fs;
use std::process::Stdio;

use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use serde_json::json;
use sqlx::{Pool, Postgres};
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use super::paths::{NO_SILENCE_PREFIX, NO_SILENCE_RECORDING_PATH, RECORDING_PATH};
use super::types::HashMapContainer;
use super::util::{file_exists, get_file_path_root, handle_idempotency_key, resolve_existing_dir};

#[get("/remove_silence/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
pub async fn remove_silence(
    req: HttpRequest,
    path: web::Path<(i64, i64, i32, i32, String)>,
    hashmap: web::Data<HashMapContainer>,
    pool: web::Data<Pool<Postgres>>,
) -> impl Responder {
    let path = path.into_inner();

    let file_path: String = resolve_existing_dir(RECORDING_PATH, &path);
    let no_silence_file_path = get_file_path_root(NO_SILENCE_RECORDING_PATH, &path);
    let file_no_silence =
        no_silence_file_path.to_owned() + "/" + NO_SILENCE_PREFIX + path.4.as_str() + ".ogg";

    let _idemonpotency = match handle_idempotency_key(&req) {
        Ok(ok) => ok,
        Err(_) => return HttpResponse::BadRequest().finish(),
    };

    info!("File name: {}", path.4);

    if hashmap.0.read().await.contains_key(&path.4) {
        info!("Already processing");
        let mut rec = {
            let lock = hashmap.0.read().await;
            match lock.get(&path.4) {
                Some(sender) => sender.subscribe(),
                None => {
                    return HttpResponse::ServiceUnavailable().json(json!({"message": "retry"}));
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
                return HttpResponse::InternalServerError()
                    .json(json!({"message": "worker terminated"}));
            }
        }

        let json = json!({"url":file_no_silence,"message":" Success"});
        return HttpResponse::Accepted().json(json);
    } else if file_exists(&(no_silence_file_path.to_owned() + "/" + &path.4 + ".ogg")) {
        info!("file already exists");
        let json = json!({"url":file_no_silence,"message":"File already exists"});
        return HttpResponse::Ok().json(json);
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
        if let Err(err) = fs::create_dir_all(&no_silence_file_path) {
            error!("create_dir_all failed: {}", err);
            hashmap.0.write().await.remove(&path.4);
            return HttpResponse::InternalServerError()
                .json(json!({"message": "failed to create output dir"}));
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

        let json = json!({"url": file_no_silence,"message":"Request Accepted"});
        HttpResponse::Ok().json(json)
    }
}
