use actix_web::{get, web, HttpRequest, HttpResponse};
use base64::prelude::*;
use serde_json::json;
use sqlx::{Pool, Postgres};
use tracing::{error, info};

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::permissions::require_channel_access;
use crate::waveform::generate_peaks_background;

use super::paths::{no_silence_recording_path, recording_path, waveform_path, NO_SILENCE_PREFIX};
use super::serve::AudioQuery;
use super::types::WaveformProgressContainer;
use super::util::{file_exists, get_file_path_root, is_stale};

const LIVE_WAVEFORM_READY: i16 = 100;
const FINAL_WAVEFORM_WRITTEN: i16 = 101;

async fn silence_free_waveform(
    path: &(i64, i64, i32, i32, String),
    progress_map: &web::Data<WaveformProgressContainer>,
) -> Result<HttpResponse, AppError> {
    let base = get_file_path_root(&no_silence_recording_path(), path);
    let input_file = format!("{}/{}{}.ogg", base, NO_SILENCE_PREFIX, path.4);
    // Prefix the cache/progress key so it never collides with the normal one.
    let cache_key = format!("{}{}", NO_SILENCE_PREFIX, path.4);
    let output = format!("{}{}.dat", waveform_path(), cache_key);

    // Serve the cache only if it's newer than the silence-free audio it was
    // built from; a regenerated (e.g. post-live) source invalidates it.
    if file_exists(&output).await && !is_stale(&input_file, &output).await {
        return waveform_response(&output).await;
    }

    // Claim the generation slot (or report an in-flight one) under one lock.
    {
        let mut progress = progress_map.0.write().await;
        if let Some(&pct) = progress.get(&cache_key) {
            if pct == -1 {
                progress.remove(&cache_key);
                return Err(AppError::InternalError);
            }
            return Ok(HttpResponse::Ok().json(json!({ "progress": pct.clamp(0, 99) })));
        }
        if !file_exists(&input_file).await {
            return Err(AppError::FileNotFound);
        }
        progress.insert(cache_key.clone(), 0);
    }

    let progress_map_clone = progress_map.clone();
    tokio::spawn(async move {
        if let Err(e) = generate_peaks_background(
            input_file,
            output,
            cache_key,
            None,
            progress_map_clone,
            None,
        )
        .await
        {
            error!("Error generating silence-free peaks: {:?}", e);
        }
    });

    Ok(HttpResponse::Ok().json(json!({ "progress": 0 })))
}

async fn waveform_response(output: &str) -> Result<HttpResponse, AppError> {
    let file_content = tokio::fs::read(output).await?;
    let base64_content = BASE64_STANDARD.encode(file_content);
    Ok(HttpResponse::Ok().json(json!({
        "progress": 100,
        "data": base64_content
    })))
}

#[get("/audio/waveform/{guild_id}/{channel_id}/{year}/{month}/{file}")]
pub async fn get_waveform_data(
    _req: HttpRequest,
    path: web::Path<(i64, i64, i32, i32, String)>,
    query: web::Query<AudioQuery>,
    progress_map: web::Data<WaveformProgressContainer>,
    pool: web::Data<Pool<Postgres>>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let path = path.into_inner();
    let token = token.ok_or(AppError::Unauthorized)?;
    require_channel_access(&pool, path.0, path.1, token.user_id).await?;

    // Silence-free version is a separate static file: distinct input,
    // distinct cache/progress key. No DB cache marker — the file is final
    // once produced, so on-disk existence is the cache.
    if query.silence.is_some() {
        return silence_free_waveform(&path, &progress_map).await;
    }

    let base_path_recording: String = get_file_path_root(&recording_path(), &path);
    let file_path = format!("{}/{}.ogg", base_path_recording, path.4);
    let output = format!("{}{}.dat", waveform_path(), path.4);
    let file_name = path.4.clone();

    let row = sqlx::query!(
        "SELECT end_ts, waveform_end_ts FROM audio_files WHERE file_name = $1",
        file_name
    )
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or(AppError::FileNotFound)?;
    let end_ts = row.end_ts;
    let waveform_end_ts = row.waveform_end_ts;
    let has_final_cache =
        end_ts.is_some() && waveform_end_ts == end_ts && file_exists(&output).await;

    if has_final_cache {
        return waveform_response(&output).await;
    }

    let pct = {
        let progress = progress_map.0.read().await;
        progress.get(&file_name).copied()
    };
    if let Some(pct) = pct {
        if pct == -1 {
            progress_map.0.write().await.remove(&file_name);
            return Err(AppError::InternalError);
        }
        if pct == FINAL_WAVEFORM_WRITTEN {
            return Ok(HttpResponse::Ok().json(json!({ "progress": 99 })));
        }
        if pct == LIVE_WAVEFORM_READY {
            if file_exists(&output).await {
                if end_ts.is_none() {
                    let response = waveform_response(&output).await;
                    progress_map.0.write().await.remove(&file_name);
                    return response;
                }
                progress_map.0.write().await.remove(&file_name);
            } else {
                progress_map.0.write().await.remove(&file_name);
            }
        } else {
            return Ok(HttpResponse::Ok().json(json!({ "progress": pct })));
        }
    }

    {
        let mut progress = progress_map.0.write().await;
        if let Some(&pct) = progress.get(&file_name) {
            if pct == -1 {
                progress.remove(&file_name);
                return Err(AppError::InternalError);
            }
            if pct == FINAL_WAVEFORM_WRITTEN {
                return Ok(HttpResponse::Ok().json(json!({ "progress": 99 })));
            }
            if pct != LIVE_WAVEFORM_READY || end_ts.is_none() {
                return Ok(HttpResponse::Ok().json(json!({ "progress": pct })));
            }
            progress.remove(&file_name);
        }
        progress.insert(file_name.clone(), 0);
    }

    let progress_map_clone = progress_map.clone();
    let pool_clone = pool.clone();
    let generation_file_name = file_name.clone();
    tokio::spawn(async move {
        if let Err(e) = generate_peaks_background(
            file_path,
            output,
            file_name.clone(),
            None,
            progress_map_clone.clone(),
            Some(FINAL_WAVEFORM_WRITTEN),
        )
        .await
        {
            error!("Error generating peaks: {:?}", e);
            return;
        }

        let current_end_ts = match sqlx::query!(
            "SELECT end_ts FROM audio_files WHERE file_name = $1",
            generation_file_name
        )
        .fetch_optional(pool_clone.get_ref())
        .await
        {
            Ok(Some(row)) => row.end_ts,
            Ok(None) => {
                progress_map_clone.0.write().await.insert(file_name, -1);
                return;
            }
            Err(e) => {
                error!("Error loading waveform cache state: {:?}", e);
                progress_map_clone.0.write().await.insert(file_name, -1);
                return;
            }
        };

        let Some(current_end_ts) = current_end_ts else {
            progress_map_clone
                .0
                .write()
                .await
                .insert(file_name, LIVE_WAVEFORM_READY);
            return;
        };

        match sqlx::query!(
            "UPDATE audio_files SET waveform_end_ts = $2 WHERE file_name = $1 AND end_ts = $2",
            generation_file_name,
            current_end_ts
        )
        .execute(pool_clone.get_ref())
        .await
        {
            Ok(result) if result.rows_affected() > 0 => {
                progress_map_clone.0.write().await.remove(&file_name);
            }
            Ok(_) => {
                progress_map_clone.0.write().await.remove(&file_name);
                info!(
                    "Skipped waveform cache marker update because end_ts changed for {}",
                    file_name
                );
            }
            Err(e) => {
                error!("Error updating waveform cache marker: {:?}", e);
                progress_map_clone.0.write().await.insert(file_name, -1);
            }
        }
    });

    Ok(HttpResponse::Ok().json(json!({ "progress": 0 })))
}
