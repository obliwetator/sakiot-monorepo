use std::collections::{HashMap, HashSet};
use std::fs::{self, ReadDir};
use std::path::Path;
use std::process::Stdio;

use actix_files::NamedFile;
use actix_web::{
    get,
    http::header::{ContentDisposition, DispositionType},
    web, HttpRequest, HttpResponse, Responder,
};

use serde::Deserialize;
use serde_json::json;
use sqlx::{Pool, Postgres};

use base64::prelude::*;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::waveform::generate_peaks_background;

pub const RECORDING_PATH: &str = "./voice_recordings/";
pub const NO_SILENCE_RECORDING_PATH: &str = "./no_silence_voice_recordings/";
pub const CLIPS_PATH: &str = "./clips/";
pub const NO_SILENCE_PREFIX: &str = "_no_silence_";
pub const WAVEFORM_PATH: &str = "./waveform_data/";

#[derive(serde::Serialize, Debug)]
pub struct Directories {
    pub year: i32,
    pub months: Option<Months>,
}

#[derive(serde::Serialize, Debug)]
pub struct Channels {
    pub channel_id: String,
    pub dirs: Vec<Directories>,
}

#[derive(serde::Serialize, Debug)]
pub struct File {
    pub file: String,
    pub comment: Option<String>,
}

pub type Months = HashMap<i32, Option<Vec<File>>>;

#[derive(serde::Deserialize, Debug)]
pub struct StartEnd {
    pub start: Option<f32>,
    pub end: Option<f32>,
    pub name: Option<String>,
}

#[derive(Debug)]
pub struct HashMapContainer(
    pub tokio::sync::RwLock<HashMap<String, tokio::sync::broadcast::Sender<i32>>>,
);

#[derive(Debug)]
pub struct WaveformProgressContainer(pub tokio::sync::RwLock<HashMap<String, i16>>);

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
                comment: None,
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
                // we have the channel is the hashset. User can access this channel
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

pub async fn get_months(path: web::Path<String>) -> Result<Vec<Directories>, HttpResponse> {
    let guild_id = path.into_inner();

    let years = match std::fs::read_dir(format!("{}{}", RECORDING_PATH, guild_id)) {
        Ok(ok) => ok,
        Err(err) => {
            tracing::error!("{}", err);
            return Err(HttpResponse::NotFound()
                .body("files does not exist or are innacessible to you 1\n"));
        }
    };

    let mut dirs_vec = Vec::new();

    // Get all the year(s) for this guild
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

            info!("{}", year_as_int);

            let months = match std::fs::read_dir(format!(
                "{}{}/{}",
                RECORDING_PATH, guild_id, year_as_int
            )) {
                Ok(ok) => ok,
                Err(err) => {
                    tracing::error!("{}", err);
                    return Err(HttpResponse::NotFound()
                        .body("files does not exist or are innacessible to you 2\n"));
                }
            };

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
                        "{}{}/{}",
                        RECORDING_PATH, guild_id, year_as_int
                    )) {
                        Ok(ok) => ok,
                        Err(err) => {
                            tracing::error!("{}", err);
                            return Err(HttpResponse::NotFound()
                                .body("files does not exist or are innacessible to you 3\n"));
                        }
                    };

                    for entry in entries {
                        if let Ok(entry) = entry {
                            let file_name_str = match entry.file_name().into_string() {
                                Ok(s) => s,
                                Err(_) => continue,
                            };
                            let file_name = File {
                                file: file_name_str,
                                comment: None,
                            };
                            if let Some(months_map) = dirs.months.as_mut() {
                                if let Some(Some(files)) = months_map.get_mut(&month_as_int) {
                                    files.push(file_name);
                                }
                            }
                        } else {
                            tracing::error!("error for file");
                        }
                    }
                } else {
                    tracing::error!("error for month")
                }
            }
            dirs_vec.push(dirs);
        } else {
            tracing::error!("error for year");
        }
    }

    Ok(dirs_vec)
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

#[get("/audio/waveform/{guild_id}/{channel_id}/{year}/{month}/{file}")]
async fn get_waveform_data(
    _req: HttpRequest,
    path: web::Path<(i64, i64, i32, i32, String)>,
    progress_map: web::Data<WaveformProgressContainer>,
) -> impl Responder {
    let path = path.into_inner();
    let base_path_recording: String = get_file_path_root(RECORDING_PATH, &path);
    let file_path = format!("{}/{}.ogg", base_path_recording, path.4);
    let output = format!("{}{}.dat", WAVEFORM_PATH, path.4);
    info!("File path: {}", output);
    let file_name = path.4.clone();

    info!("Received poll request for waveform data: {}", file_path);

    // 1. Check if the output file already exists on disk
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

    // 2. Check if it's currently generating in the hashmap
    if let Some(&pct) = progress_map.0.read().await.get(&file_name) {
        if pct == -1 {
            // Remove the error state so user can retry
            progress_map.0.write().await.remove(&file_name);
            return HttpResponse::InternalServerError().json(json!({
                "error": "Failed to generate waveform"
            }));
        }
        return HttpResponse::Ok().json(json!({ "progress": pct }));
    }

    // 3. It's not on disk and not generating, so start generating!
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

pub fn get_file_path_root(base_path: &str, path: &(i64, i64, i32, i32, String)) -> String {
    let guild_id = &path.0;
    let channel_id = &path.1;
    let year = &path.2;
    let month = &path.3;

    let file_path = format!(
        "{}{}/{}/{}/{}",
        base_path, guild_id, channel_id, year, month
    );

    file_path
}

fn file_exists(path: &str) -> bool {
    Path::new(path).try_exists().unwrap_or(false)
}

fn handle_idempotency_key(req: &HttpRequest) -> Result<String, ()> {
    let header = match req.headers().get("Idempotency-Key") {
        Some(ok) => ok,
        None => {
            error!("Idempotency key is missing");
            return Err(());
        }
    };

    let res = match header.to_str() {
        Ok(ok) => ok.to_owned(),
        Err(_) => {
            error!("No value in Idempotency header");
            return Err(());
        }
    };

    Ok(res)
}

#[get("/remove_silence/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
async fn remove_silence(
    req: HttpRequest,
    path: web::Path<(i64, i64, i32, i32, String)>,
    hashmap: web::Data<HashMapContainer>,
    pool: web::Data<Pool<Postgres>>,
) -> impl Responder {
    let path = path.into_inner();

    let file_path: String = get_file_path_root(RECORDING_PATH, &path);
    let no_silence_file_path = get_file_path_root(NO_SILENCE_RECORDING_PATH, &path);
    let file_no_silence =
        no_silence_file_path.to_owned() + "/" + NO_SILENCE_PREFIX + path.4.as_str() + ".ogg";

    let idemonpotency = match handle_idempotency_key(&req) {
        Ok(ok) => ok,
        Err(_) => return HttpResponse::BadRequest().finish(),
    };

    info!("File name: {}", path.4);

    // We have they key in the hashmap. We are processing the request
    if hashmap.0.read().await.contains_key(&path.4) {
        info!("Already processing");
        let mut rec = {
            let lock = hashmap.0.read().await;
            match lock.get(&path.4) {
                Some(sender) => sender.subscribe(),
                None => {
                    // Race: entry removed between contains_key and get. Treat as fresh.
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
    } else {
        // It's the first time we receive the request
        // ---OR---
        // We have already processed this request and the file already exists on the server

        // Check if file exists before we try to process it.
        if file_exists(&(no_silence_file_path.to_owned() + "/" + &path.4 + ".ogg")) {
            info!("file already exists");
            let json = json!({"url":file_no_silence,"message":"File already exists"});
            return HttpResponse::Ok().json(json);
        } else {
            info!("Creating new file");
            let (tx, _) = broadcast::channel::<i32>(10);
            // File no present and its the first time we receive a request for this file
            {
                hashmap
                    .0
                    .write()
                    .await
                    .insert(path.4.to_owned(), tx.clone());
            }
            // Create the directory for the file
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
                        // Value received
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
            return HttpResponse::Ok().json(json);
        }
    }

    // if file_exists(&(no_silence_file_path.to_owned() + "/" + &path.4 + ".ogg")) {
    //     // That file was already created
    //     let file_no_silence = no_silence_file_path + path.4.as_str() + ".ogg";
    //     info!("Audio with removed silence already exists");
    //     let json = json!({"url":file_no_silence,"message":"File already exists"});
    //     HttpResponse::Conflict().json(json)
    // } else {

    //     hashmap.0.write().await.remove(&idemonpotency.to_owned());

    // }
}

#[get("/find/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
async fn find_similar(
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

#[derive(Deserialize, Debug)]
struct AudioQuery {
    silence: Option<bool>,
}
#[get("/audio/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
async fn get_audio(
    req: HttpRequest,
    path: web::Path<(u64, String, i32, i32, String)>,
    query_param: web::Query<AudioQuery>,
) -> impl Responder {
    use actix_files::NamedFile;
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

    let res = match NamedFile::open_async(path).await {
        Ok(ok) => ok.into_response(&req),
        Err(_) => return HttpResponse::NotFound().finish(),
    };

    res
}

#[get("/download/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
async fn download_audio(
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
