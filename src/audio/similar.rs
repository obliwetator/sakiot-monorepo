use std::process::Stdio;

use actix_web::{get, web, HttpRequest, HttpResponse};
use tracing::{error, info, warn};

use crate::errors::AppError;

use super::paths::RECORDING_PATH;

#[get("/find/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
pub async fn find_similar(
    _req: HttpRequest,
    path: web::Path<(u64, String, i32, i32, String)>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, channel_id, year, month, file_name) = path.into_inner();

    let file_path = format!(
        "{}{}/{}/{}/{}",
        RECORDING_PATH, guild_id, channel_id, year, month
    );
    let files = std::fs::read_dir(&file_path)?;

    for file in files {
        let entry = match file {
            Ok(e) => e,
            Err(e) => {
                warn!("read_dir entry error: {}", e);
                continue;
            }
        };
        let fname = entry.file_name();
        let file_n = fname.to_string_lossy();
        let start = std::time::Instant::now();
        let mut command = tokio::process::Command::new("ffprobe");
        command
            .arg("-show_entries")
            .arg("format=duration")
            .args(["-of", "default=noprint_wrappers=1:nokey=1"])
            .arg(format!("{}/{}", file_path, file_n))
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .stdout(Stdio::piped());

        let output = match command.output().await {
            Ok(o) => o,
            Err(e) => {
                error!("ffprobe failed: {}", e);
                continue;
            }
        };

        let duration = start.elapsed();
        info!("Time elapsed in ffprobe is: {:?}", duration);
        info!("Out: {}", String::from_utf8_lossy(&output.stdout));
    }

    let (_time, user_id) = file_name
        .split_once('-')
        .ok_or_else(|| AppError::InvalidParam("file_name".into()))?;

    info!("1: {}, 2: {}", user_id, _time);

    Ok(HttpResponse::Ok().finish())
}
