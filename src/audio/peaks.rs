use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use base64::prelude::*;
use serde_json::json;
use tracing::{error, info};

use crate::waveform::generate_peaks_background;

use super::paths::{RECORDING_PATH, WAVEFORM_PATH};
use super::types::WaveformProgressContainer;
use super::util::{file_exists, resolve_existing_dir};

#[get("/audio/waveform/{guild_id}/{channel_id}/{year}/{month}/{file}")]
pub async fn get_waveform_data(
    _req: HttpRequest,
    path: web::Path<(i64, i64, i32, i32, String)>,
    progress_map: web::Data<WaveformProgressContainer>,
) -> impl Responder {
    let path = path.into_inner();
    let base_path_recording: String = resolve_existing_dir(RECORDING_PATH, &path);
    let file_path = format!("{}/{}.ogg", base_path_recording, path.4);
    let output = format!("{}{}.dat", WAVEFORM_PATH, path.4);
    info!("File path: {}", output);
    let file_name = path.4.clone();

    info!("Received poll request for waveform data: {}", file_path);

    if file_exists(&output) {
        match tokio::fs::read(&output).await {
            Ok(file_content) => {
                let base64_content = BASE64_STANDARD.encode(file_content);
                return HttpResponse::Ok().json(json!({
                    "progress": 100,
                    "data": base64_content
                }));
            }
            Err(e) => {
                return HttpResponse::InternalServerError().json(json!({
                    "error": format!("Failed to read existing waveform file: {}", e)
                }));
            }
        }
    }

    if let Some(&pct) = progress_map.0.read().await.get(&file_name) {
        if pct == -1 {
            progress_map.0.write().await.remove(&file_name);
            return HttpResponse::InternalServerError().json(json!({
                "error": "Failed to generate waveform"
            }));
        }
        return HttpResponse::Ok().json(json!({ "progress": pct }));
    }

    progress_map.0.write().await.insert(file_name.clone(), 0);

    let progress_map_clone = progress_map.clone();
    tokio::spawn(async move {
        if let Err(e) =
            generate_peaks_background(file_path, output, file_name, None, progress_map_clone).await
        {
            error!("Error generating peaks: {:?}", e);
        }
    });

    HttpResponse::Ok().json(json!({ "progress": 0 }))
}
