use actix_web::{
    get,
    http::header::{ContentDisposition, DispositionType},
    post, web, HttpMessage, HttpRequest, HttpResponse, Responder,
};

use serde::{Deserialize, Serialize};

use serde_repr::{Deserialize_repr, Serialize_repr};
use serde_with::{As, DisplayFromStr};
use sqlx::{Pool, Postgres};
use tracing::{error, info};

use hello_world::jammer_client::JammerClient;
use hello_world::JamData;

pub mod hello_world {
    #![allow(non_snake_case)]
    tonic::include_proto!("helloworld");
}

use crate::{
    audio::CLIPS_PATH,
    auth::{Access, Token},
    clips::hello_world::jam_response::JamResponseEnum,
    errors::{ApiResponse, AppError},
};
use serde_json::json;

type DisplayFromstr = As<DisplayFromStr>;

#[derive(Serialize, Debug)]
struct ClipInfo {
    clip_id: String,
    #[serde(with = "DisplayFromstr")]
    user_id: i64,
    name: Option<String>,
    original_file_name: Option<String>,
    saved_file_name: Option<String>,
    length: Option<f32>,
    size: Option<i64>,
    #[serde(with = "DisplayFromstr")]
    guild_id: i64,
    #[serde(with = "DisplayFromstr")]
    channel_id: i64,
    start_time: f32,
}

#[get("/audio/clips/{guild_id}/{clip_id:.*}")]
pub async fn get_clip(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, String)>,
) -> impl Responder {
    use actix_files::NamedFile;

    let path = path.into_inner();
    let guild_id = path.0;
    let clip_id = path.1;

    let row = sqlx::query!(
        "SELECT saved_file_name FROM clips WHERE guild_id = $1 AND clip_id = $2",
        guild_id,
        clip_id
    )
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);

    if let Some(record) = row {
        let saved_file_name = record.saved_file_name.unwrap_or_default();
        let full_path = format!("{}{}", CLIPS_PATH, saved_file_name);
        info!("clips path: {}", full_path);
        match NamedFile::open_async(&full_path).await {
            Ok(ok) => {
                let mut res = ok
                    .use_last_modified(true)
                    .set_content_disposition(ContentDisposition {
                        disposition: DispositionType::Inline,
                        parameters: vec![],
                    })
                    .into_response(&req);
                res.headers_mut().insert(
                    actix_web::http::header::CONTENT_TYPE,
                    actix_web::http::header::HeaderValue::from_static("audio/ogg"),
                );
                res
            }
            Err(_) => HttpResponse::NotFound().body("File not found"),
        }
    } else {
        HttpResponse::NotFound().body("File not found in database")
    }
}

#[get("/audio/clips/{guild_id}")]
pub async fn get_clips(
    _req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();

    let result = sqlx::query_as!(
        ClipInfo,
        r#"
        SELECT clip_id,
        user_id as "user_id!",
        name,
        original_file_name,
        saved_file_name,
        length,
        size,
        guild_id as "guild_id!",
        channel_id as "channel_id!",
        start_time
        FROM clips
        WHERE guild_id = $1
        "#,
        guild_id
    )
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(result))
}
#[derive(Deserialize, PartialEq, Debug)]
pub struct JamItBody {
    #[serde(with = "DisplayFromstr")]
    guild_id: i64,
    clip_name: String,
}

#[derive(Serialize_repr, Deserialize_repr, PartialEq, Debug)]
#[repr(u8)]
#[serde(tag = "code")]
pub enum JamItResponse {
    OK,
    NotPresentInChannel,
    Unkown,
}

#[derive(Deserialize, PartialEq, Debug)]
pub struct A {
    code: JamItResponse,
}

// TODO: Get GRPC Client
#[post("/jamit")]
pub async fn play_clip(info: web::Json<JamItBody>) -> Result<HttpResponse, AppError> {
    info!("Received jamit request: {:?}", info);

    let mut client = JammerClient::connect("http://[::1]:50052")
        .await
        .map_err(|e| {
            error!("Failed to connect to GRPC JammerClient: {}", e);
            AppError::GrpcError(e.to_string())
        })?;

    info!(
        "Connected to GRPC server. Sending request for clip '{}' and guild '{}'",
        info.clip_name, info.guild_id
    );

    let request = tonic::Request::new(JamData {
        clip_name: info.clip_name.clone(),
        guild_id: info.guild_id,
    });

    let response = client.jam_it(request).await.map_err(|e| {
        error!("Failed to jam_it via GRPC: {}", e);
        AppError::GrpcError(e.to_string())
    })?;

    info!("GRPC response: {:#?}", response);

    let jam_response = response.into_inner();

    Ok(match jam_response.resp() {
        JamResponseEnum::Ok => HttpResponse::Ok().json(json!({"code" : "0"})),
        JamResponseEnum::NotPressent => HttpResponse::Ok().json(json!({"code" : 1})),
        JamResponseEnum::Unkown => HttpResponse::Ok().json(json!({"code" : 2})),
    })
}

use crate::audio::StartEnd;
use chrono::Datelike;
use std::process::Stdio;

async fn crop_ffmpeg(
    start: f32,
    end: f32,
    file_path: &str,
) -> Result<tokio::process::Child, AppError> {
    let duration = end - start;
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        // seek to
        .args(["-ss", &start.to_string()])
        // input
        .args(["-i", file_path])
        // length
        .args(["-t", &duration.to_string()])
        // copy the codec
        .args(["-c", "copy"])
        // since we pipe the output we have to tell ffmpeg whats its gonna be
        .args(["-f", "ogg"])
        // output to pipe
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    command
        .spawn()
        .map_err(|e| AppError::FfmpegError(e.to_string()))
}

#[post("audio/clips/create/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
pub async fn create_clip(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, i64, i32, i32, String)>,
    clip_duration: web::Json<StartEnd>,
) -> Result<HttpResponse, AppError> {
    info!("creating clip with duration: {:?}", clip_duration);
    let user_id = req
        .extensions()
        .get::<Token<Access>>()
        .map(|t| t.id)
        .ok_or(AppError::Unauthorized)?;
    let (guild_id, channel_id, year, month, file_name_from_url) = path.into_inner();
    let file_name_without_guild_id = format!("{}/{}/{}", year, month, file_name_from_url);

    let src_path = format!(
        "{}{}/{}/{}.ogg",
        crate::audio::RECORDING_PATH,
        guild_id,
        channel_id,
        &file_name_without_guild_id
    );

    let start = clip_duration.start.unwrap_or(0.0);
    let end = clip_duration.end.unwrap_or(0.0);

    let length = end - start;
    if !(1.0..=20.0).contains(&length) {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({"status": "error", "message": "Clip duration must be between 1 and 20 seconds"})));
    }

    let child = crop_ffmpeg(start, end, src_path.as_str()).await?;
    let output = child
        .wait_with_output()
        .await
        .map_err(|e| AppError::FfmpegError(e.to_string()))?;
    let bytes = output.stdout.clone();

    let clip_name = if let Some(ref name) = clip_duration.name {
        name.clone()
    } else {
        file_name_from_url.clone()
    };

    let now = chrono::Utc::now();
    let c_year = now.year();
    let c_month = now.month();

    let mut clip_id = uuid::Uuid::new_v4().to_string();

    let target_dir = format!("{}{}/{:02}", CLIPS_PATH, c_year, c_month);
    let mut saved_file_name = format!("{}/{:02}/{}.ogg", c_year, c_month, clip_id);
    let mut full_save_path = format!("{}/{}.ogg", target_dir, clip_id);
    info!("saving clip to: {}", full_save_path);

    std::fs::create_dir_all(&target_dir)?;

    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .arg("-y")
        .args(["-i", "-"])
        .args(["-c:a", "copy"])
        .args(["-f", "ogg"])
        .arg(&full_save_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|e| AppError::FfmpegError(e.to_string()))?;

    use tokio::io::AsyncWriteExt;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| AppError::FfmpegError("ffmpeg stdin missing".into()))?;
    tokio::spawn(async move {
        if let Err(e) = stdin.write_all(&bytes).await {
            error!("ffmpeg stdin write failed: {}", e);
        }
    });

    child
        .wait_with_output()
        .await
        .map_err(|e| AppError::FfmpegError(e.to_string()))?;

    let size = std::fs::metadata(&full_save_path)
        .map(|m| m.len())
        .unwrap_or(0) as i64;
    let length = end - start;

    let mut retries = 3;
    while retries > 0 {
        match sqlx::query!(
            "INSERT INTO clips (clip_id, length, size, channel_id, guild_id, user_id, original_file_name, saved_file_name, name, start_time) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            clip_id,
            length as f32,
            size,
            channel_id,
            guild_id,
            user_id,
            file_name_from_url,
            saved_file_name,
            clip_name,
            start as f32
        )
        .execute(pool.get_ref())
        .await
        {
            Ok(_) => break,
            Err(e) => {
                if let sqlx::Error::Database(ref db_err) = e {
                    if db_err.code().as_deref() == Some("23505") { // unique violation
                        retries -= 1;
                        if retries == 0 {
                            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({"status": "error", "message": "Failed to generate unique clip ID"})));
                        }

                        let new_clip_id = uuid::Uuid::new_v4().to_string();
                        let new_saved_file_name = format!("{}/{:02}/{}.ogg", c_year, c_month, new_clip_id);
                        let new_full_save_path = format!("{}/{}.ogg", target_dir, new_clip_id);

                        if let Err(err) = std::fs::rename(&full_save_path, &new_full_save_path) {
                            error!("Failed to rename file after collision: {:?}", err);
                            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({"status": "error", "message": "Failed to rename clip"})));
                        }

                        clip_id = new_clip_id;
                        saved_file_name = new_saved_file_name;
                        full_save_path = new_full_save_path;
                        continue;
                    }
                }
                error!("Database error inserting clip: {:?}", e);
                return Ok(HttpResponse::InternalServerError().json(serde_json::json!({"status": "error", "message": "Database error"})));
            }
        }
    }

    Ok(HttpResponse::Ok().json(serde_json::json!({"status": "success", "file": saved_file_name, "id": clip_id, "name": clip_name})))
}

#[post("audio/clips/delete/{guild_id}")]
pub async fn delete(
    clip_id: String,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
) -> impl Responder {
    let guild_id = path.into_inner();

    let row = sqlx::query!(
        "SELECT saved_file_name FROM clips WHERE guild_id = $1 AND clip_id = $2",
        guild_id,
        clip_id
    )
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);

    let saved_file_name = if let Some(record) = row {
        record.saved_file_name
    } else {
        return HttpResponse::NotFound().json(ApiResponse::FILE_NOT_FOUND());
    };

    let result = match sqlx::query!(
        r#"
        DELETE FROM clips
        WHERE guild_id = $1 AND clip_id = $2
        "#,
        guild_id,
        clip_id
    )
    .execute(pool.get_ref())
    .await
    {
        Ok(ok) => ok,
        Err(_) => return HttpResponse::Ok().json(ApiResponse::FILE_ALREADY_DELETED()),
    };

    if result.rows_affected() == 1 {
        if let Some(sfn) = saved_file_name {
            let res = std::fs::remove_file(format!("{}{}", CLIPS_PATH, sfn));
            match res {
                Ok(_) => {
                    info!("file deleted");
                    HttpResponse::Ok().json(ApiResponse::OK())
                }
                Err(_err) => {
                    error!("file cannot be deleted");
                    error!("{:?}", _err.kind());
                    HttpResponse::NotFound().json(())
                }
            }
        } else {
            HttpResponse::Ok().json(ApiResponse::OK())
        }
    } else {
        HttpResponse::NotFound().json(ApiResponse::FILE_NOT_FOUND())
    }
}
