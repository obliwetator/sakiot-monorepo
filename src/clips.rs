use actix_web::{
    delete, get,
    http::header::{ContentDisposition, DispositionType},
    post, web, HttpMessage, HttpRequest, HttpResponse,
};

use serde::{Deserialize, Serialize};

use serde_repr::{Deserialize_repr, Serialize_repr};
use serde_with::{As, DisplayFromStr};
use sqlx::{Pool, Postgres};
use tracing::{error, info};

use crate::proto::jammer::jam_response::JamResponseEnum;
use crate::proto::jammer::JamData;
use crate::{
    audio::{clips_path, recording_path},
    auth::{Access, Token},
    errors::AppError,
    fbi_agent_registry::AgentGrpcRegistry,
    grpc_client,
};
use serde_json::json;

type DisplayFromstr = As<DisplayFromStr>;

fn is_valid_recording_file_name(file_name: &str) -> bool {
    !file_name.is_empty()
        && !file_name.contains("..")
        && !file_name.contains('/')
        && !file_name.contains('\\')
        && !file_name.contains('\'')
        && !file_name.contains('"')
        && !file_name.chars().any(char::is_control)
}

#[derive(Serialize, Debug, utoipa::ToSchema)]
pub struct ClipInfo {
    clip_id: String,
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    user_id: i64,
    name: Option<String>,
    original_file_name: Option<String>,
    saved_file_name: Option<String>,
    length: Option<f32>,
    size: Option<i64>,
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    guild_id: i64,
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    channel_id: i64,
    start_time: f32,
}

#[get("/audio/clips/{guild_id}/{clip_id:.*}")]
pub async fn get_clip(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, String)>,
) -> Result<HttpResponse, AppError> {
    use actix_files::NamedFile;

    let (guild_id, clip_id) = path.into_inner();

    let row = sqlx::query!(
        "SELECT saved_file_name FROM clips WHERE guild_id = $1 AND clip_id = $2 AND deleted_at IS NULL",
        guild_id,
        clip_id
    )
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or(AppError::ClipNotFound)?;

    let saved_file_name = row.saved_file_name.unwrap_or_default();
    let full_path = format!("{}{}", clips_path(), saved_file_name);

    let file = NamedFile::open_async(&full_path)
        .await
        .map_err(|_| AppError::FileNotFound)?;

    let mut res = file
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
    Ok(res)
}

#[utoipa::path(
    get,
    path = "/api/audio/clips/{guild_id}",
    tag = "clips",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "Guild clips", body = [ClipInfo]),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
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
        WHERE guild_id = $1 AND deleted_at IS NULL
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
    Unknown,
}

#[post("/jamit")]
pub async fn play_clip(
    req: HttpRequest,
    info: web::Json<JamItBody>,
    registry: web::Data<AgentGrpcRegistry>,
) -> Result<HttpResponse, AppError> {
    let user_id = req
        .extensions()
        .get::<Token<Access>>()
        .map(|t| t.user_id)
        .ok_or(AppError::Unauthorized)?;

    let active_address = registry.active_address();
    let (grpc_address, mut client) = grpc_client::connect_jammer(active_address.clone())
        .await
        .map_err(|e| {
            grpc_client::record_failure("jammer_connect");
            error!(
                grpc_address = %active_address,
                "Failed to connect to Jammer gRPC service: {}",
                e,
            );
            AppError::GrpcError(e.to_string())
        })?;

    let request = tonic::Request::new(JamData {
        clip_name: info.clip_name.clone(),
        guild_id: info.guild_id,
        user_id,
    });

    let response = client.jam_it(request).await.map_err(|e| {
        grpc_client::record_failure("jammer_jam_it");
        error!(
            grpc_address = %grpc_address,
            "Failed to jam_it via GRPC: {}",
            e,
        );
        AppError::GrpcError(e.to_string())
    })?;

    info!("GRPC response: {:#?}", response);

    let jam_response = response.into_inner();
    let remaining = jam_response.cooldown_remaining_seconds;

    Ok(match jam_response.resp() {
        JamResponseEnum::Ok => HttpResponse::Ok().json(json!({"code" : "0"})),
        JamResponseEnum::NotPresent => HttpResponse::Ok().json(json!({"code" : 1})),
        JamResponseEnum::Unknown => HttpResponse::Ok().json(json!({"code" : 2})),
        JamResponseEnum::Cooldown => HttpResponse::TooManyRequests()
            .json(json!({"code": 3, "cooldown_remaining_seconds": remaining})),
    })
}

use crate::audio::StartEnd;
use chrono::Datelike;
use std::process::Stdio;

#[derive(Serialize, utoipa::ToSchema)]
pub struct CreateClipResponse {
    pub status: &'static str,
    pub file: String,
    pub id: String,
    pub name: String,
}

async fn crop_ffmpeg(
    start: f32,
    end: f32,
    file_path: &str,
    target_path: &str,
) -> Result<tokio::process::Child, AppError> {
    let duration = end - start;
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .arg("-y")
        // seek to
        .args(["-ss", &start.to_string()])
        // input
        .args(["-i", file_path])
        // length
        .args(["-t", &duration.to_string()])
        // copy the codec
        .args(["-c:a", "copy"])
        // output file
        .arg(target_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    command
        .spawn()
        .map_err(|e| AppError::FfmpegError(e.to_string()))
}

#[utoipa::path(
    post,
    path = "/api/audio/clips/create/{guild_id}/{channel_id}/{year}/{month}/{file_name}",
    tag = "clips",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("channel_id" = i64, Path, description = "Discord channel id"),
        ("year" = i32, Path, description = "Recording year"),
        ("month" = i32, Path, description = "Recording month"),
        ("file_name" = String, Path, description = "Recording file stem"),
    ),
    request_body = StartEnd,
    responses(
        (status = 200, description = "Clip created", body = CreateClipResponse),
        (status = 400, description = "Invalid clip request", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/audio/clips/create/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
pub async fn create_clip(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, i64, i32, i32, String)>,
    clip_duration: web::Json<StartEnd>,
) -> Result<HttpResponse, AppError> {
    let user_id = req
        .extensions()
        .get::<Token<Access>>()
        .map(|t| t.user_id)
        .ok_or(AppError::Unauthorized)?;
    let (guild_id, channel_id, year, month, file_name_from_url) = path.into_inner();
    if !is_valid_recording_file_name(&file_name_from_url) {
        return Err(AppError::BadRequest("Invalid file name".into()));
    }
    let src_path = {
        let dir = crate::audio::util::get_file_path_root(
            &recording_path(),
            &(
                guild_id,
                channel_id,
                year,
                month,
                file_name_from_url.clone(),
            ),
        );
        format!("{}/{}.ogg", dir, file_name_from_url)
    };

    let start = clip_duration.start.unwrap_or(0.0);
    let end = clip_duration.end.unwrap_or(0.0);

    let length = end - start;
    if !(1.0..=20.0).contains(&length) {
        return Err(AppError::BadRequest(
            "Clip duration must be between 1 and 20 seconds".into(),
        ));
    }

    let clip_name = if let Some(ref name) = clip_duration.name {
        name.clone()
    } else {
        file_name_from_url.clone()
    };

    let now = chrono::Utc::now();
    let c_year = now.year();
    let c_month = now.month();

    let clip_id = uuid::Uuid::new_v4().to_string();

    let target_dir = format!("{}{}/{:02}", clips_path(), c_year, c_month);
    let saved_file_name = format!("{}/{:02}/{}.ogg", c_year, c_month, clip_id);
    let full_save_path = format!("{}/{}.ogg", target_dir, clip_id);

    tokio::fs::create_dir_all(&target_dir).await?;

    let child = crop_ffmpeg(start, end, src_path.as_str(), &full_save_path).await?;
    let output = child
        .wait_with_output()
        .await
        .map_err(|e| AppError::FfmpegError(e.to_string()))?;

    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr);
        error!("FFMPEG error: {}", err_msg);
        return Err(AppError::FfmpegError(err_msg.to_string()));
    }

    let size = tokio::fs::metadata(&full_save_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0) as i64;
    let length = end - start;

    sqlx::query!(
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
    .map_err(|e| {
        error!("Database error inserting clip: {:?}", e);
        AppError::InternalError
    })?;

    Ok(HttpResponse::Ok().json(CreateClipResponse {
        status: "success",
        file: saved_file_name,
        id: clip_id,
        name: clip_name,
    }))
}

#[utoipa::path(
    delete,
    path = "/api/audio/clips/{guild_id}/{clip_id}",
    tag = "clips",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("clip_id" = String, Path, description = "Clip id"),
    ),
    responses(
        (status = 200, description = "Clip deleted"),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 404, description = "Clip not found", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[delete("/audio/clips/{guild_id}/{clip_id}")]
pub async fn delete(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, String)>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, clip_id) = path.into_inner();

    let result = sqlx::query!(
        r#"
        UPDATE clips
        SET deleted_at = NOW()
        WHERE guild_id = $1 AND clip_id = $2 AND deleted_at IS NULL
        "#,
        guild_id,
        clip_id
    )
    .execute(pool.get_ref())
    .await?;

    if result.rows_affected() != 1 {
        return Err(AppError::ClipNotFound);
    }

    info!("clip soft-deleted: guild={} clip={}", guild_id, clip_id);

    Ok(HttpResponse::Ok().finish())
}

#[cfg(test)]
mod tests {
    use super::is_valid_recording_file_name;

    #[test]
    fn validates_recording_file_name_for_clip_creation() {
        assert!(is_valid_recording_file_name("1712345678-123456789"));
        assert!(!is_valid_recording_file_name(""));
        assert!(!is_valid_recording_file_name("../secret"));
        assert!(!is_valid_recording_file_name("dir/file"));
        assert!(!is_valid_recording_file_name("dir\\file"));
        assert!(!is_valid_recording_file_name("bad'name"));
        assert!(!is_valid_recording_file_name("bad\"name"));
        assert!(!is_valid_recording_file_name("bad\nname"));
    }
}
