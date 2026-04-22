use std::collections::{HashMap, HashSet};
use std::fs::ReadDir;

use actix_web::{get, web, HttpResponse};
use tracing::info;

use crate::auth::{Access, Token};
use crate::errors::AppError;

use super::paths::RECORDING_PATH;
use super::types::{Channels, Directories, File};

#[inline]
pub async fn for_entry(entries: ReadDir, _channel: i64, dirs: &mut Directories, month_as_int: i32) {
    for entry in entries {
        if let Ok(entry) = entry {
            let file_name_str = match entry.file_name().into_string() {
                Ok(s) => s,
                Err(_) => continue,
            };
            let file_name = File {
                file: file_name_str,
            };
            if let Some(months) = dirs.months.as_mut() {
                if let Some(Some(files)) = months.get_mut(&month_as_int) {
                    files.push(file_name);
                }
            }
        } else {
            info!("error for file");
        }
    }
}

pub async fn get_channels_dir(
    guild_id: String,
    channel_hashset: HashSet<i64>,
) -> Result<Vec<Channels>, HttpResponse> {
    let mut dirs_vec = Vec::new();

    if let Some(value) = for_channel_ids(guild_id, &mut dirs_vec, channel_hashset).await {
        return value;
    }

    Ok(dirs_vec)
}

pub async fn for_channel_ids(
    guild_id: String,
    dirs_vec: &mut Vec<Channels>,
    channel_hashset: HashSet<i64>,
) -> Option<Result<Vec<Channels>, HttpResponse>> {
    let channel_ids = match std::fs::read_dir(format!("{}{}", RECORDING_PATH, guild_id)) {
        Ok(ok) => ok,
        Err(err) => {
            tracing::error!("{}", err);
            return Some(Err(HttpResponse::NotFound()
                .body("files does not exist or are innacessible to you 1\n")));
        }
    };

    for channel_id in channel_ids {
        if let Ok(entry) = channel_id {
            let channel = match entry.file_name().into_string() {
                Ok(s) => match s.parse::<i64>() {
                    Ok(num) => num,
                    Err(_) => continue,
                },
                Err(_) => continue,
            };

            if channel_hashset.contains(&channel) {
                let years = match std::fs::read_dir(format!(
                    "{}{}/{}",
                    RECORDING_PATH, guild_id, channel
                )) {
                    Ok(ok) => ok,
                    Err(err) => {
                        tracing::error!("{}", err);
                        return Some(Err(HttpResponse::NotFound()
                            .body("files does not exist or are innacessible to you 2\n")));
                    }
                };

                let mut channels = Channels {
                    channel_id: channel.to_string(),
                    dirs: Vec::new(),
                };

                if let Some(value) = for_years(years, &guild_id, channel, &mut channels).await {
                    return Some(value);
                }

                dirs_vec.push(channels);
            }
        }
    }

    None
}

#[inline]
pub async fn for_years(
    years: ReadDir,
    guild_id: &String,
    channel: i64,
    dirs_vec: &mut Channels,
) -> Option<Result<Vec<Channels>, HttpResponse>> {
    for year in years {
        if let Ok(entry) = year {
            let year_as_int = match entry.file_name().into_string() {
                Ok(s) => match s.parse::<i32>() {
                    Ok(num) => num,
                    Err(_) => continue,
                },
                Err(_) => continue,
            };

            let mut dirs = Directories {
                year: year_as_int,
                months: Some(HashMap::new()),
            };

            let months = match std::fs::read_dir(format!(
                "{}{}/{}/{}",
                RECORDING_PATH, guild_id, channel, year_as_int
            )) {
                Ok(ok) => ok,
                Err(err) => {
                    tracing::error!("{}", err);
                    return Some(Err(HttpResponse::NotFound()
                        .body("files does not exist or are innacessible to you 2\n")));
                }
            };

            if let Some(value) = for_months(months, &mut dirs, guild_id, channel, year_as_int).await
            {
                return Some(value);
            }

            dirs_vec.dirs.push(dirs);
        }
    }
    None
}

#[inline]
pub async fn for_months(
    months: ReadDir,
    dirs: &mut Directories,
    guild_id: &String,
    channel: i64,
    year_as_int: i32,
) -> Option<Result<Vec<Channels>, HttpResponse>> {
    for month in months {
        if let Ok(entry) = month {
            let month_as_string = match entry.file_name().into_string() {
                Ok(s) => s,
                Err(_) => continue,
            };
            let month_as_int = month_as_string.parse::<i32>().unwrap_or(0);

            if let Some(months_map) = dirs.months.as_mut() {
                months_map.insert(month_as_int, Some(vec![]));
            }

            let entries = match std::fs::read_dir(format!(
                "{}{}/{}/{}/{}",
                RECORDING_PATH, guild_id, channel, year_as_int, &month_as_string
            )) {
                Ok(ok) => ok,
                Err(err) => {
                    tracing::error!("{}", err);
                    return Some(Err(HttpResponse::NotFound()
                        .body("files does not exist or are innacessible to you 3\n")));
                }
            };

            for_entry(entries, channel, dirs, month_as_int).await;
        } else {
            info!("error for month")
        }
    }
    None
}

#[get("/current/{guild_id}")]
pub async fn get_current_month_permission(
    path: web::Path<String>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<sqlx::Pool<sqlx::Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;

    let guild_id = path.into_inner();
    let guild_id_as_int = guild_id
        .parse::<i64>()
        .map_err(|_| AppError::InvalidParam("guild_id".into()))?;

    let permission_hashset =
        crate::permissions::get_available_channels_for_user(&pool, guild_id_as_int, token.id)
            .await?;

    let result = get_channels_dir(guild_id, permission_hashset).await;

    let resp = match result {
        Ok(dirs_vec) => HttpResponse::Ok().json(dirs_vec),
        Err(err) => err,
    };

    Ok(resp)
}
