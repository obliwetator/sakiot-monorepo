use std::{fs, path::Path, process::Stdio};

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

use crate::{
    waveform::generate_peaks_background, HashMapContainer, WaveformProgressContainer,
    NO_SILENCE_PREFIX, NO_SILENCE_RECORDING_PATH, RECORDING_PATH, WAVEFORM_PATH,
};

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

async fn _get_file(path: web::Path<(i64, i64, i32, i32, String)>) -> NamedFile {
    let (guild_id, channel_id, year, month, file_name_from_url) = path.into_inner();

    match NamedFile::open(format!(
        "{}{}/{}/{}/{}/{}",
        RECORDING_PATH, guild_id, channel_id, year, month, file_name_from_url
    )) {
        Ok(ok) => ok,
        Err(err) => {
            panic!("{err}")
        }
    }
}

fn get_file_path_root(base_path: &str, path: &(i64, i64, i32, i32, String)) -> String {
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
    let rs = match Path::new(path).try_exists() {
        Ok(ok) => ok,
        Err(err) => {
            panic!("{err}")
        }
    };

    rs
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
        let lock = hashmap.0.read().await;
        let sender = lock.get(&path.4).unwrap();
        let mut rec = sender.subscribe();
        let value = rec.recv().await.unwrap();

        if value == 0 {
            info!("received value");
            // placeholder
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
            // Crate the directory for the file
            let res = fs::create_dir_all(&no_silence_file_path);
            {
                match res {
                    Ok(_) => (),
                    Err(err) => {
                        // Something went very wrong when making the dir
                        hashmap.0.write().await.remove(&idemonpotency);
                        panic!("{err}")
                    }
                }
            }

            let file: String = file_path.to_owned() + "/" + path.4.as_str() + ".ogg";

            let file_no_silence_clone = file_no_silence.to_owned();

            info!("NO SILENCE FILE PATH: {}", &file_no_silence);

            let hashmap_clone = hashmap.clone();
            tokio::spawn(async move {
                let file_name: String = path.4.clone();
                let command = match std::process::Command::new("ffmpeg")
                    .args(["-i", &file])
                    .args([
                        "-af",
                        "silenceremove=stop_periods=-1:stop_duration=1:stop_threshold=-40dB",
                    ])
                    .arg(file_no_silence_clone)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                {
                    Ok(result) => result,
                    Err(err) => {
                        // Something when very wrong when spawing the process
                        hashmap_clone.0.write().await.remove(&path.4);
                        panic!("error: {}", err);
                    }
                };

                let _output = command.wait_with_output().unwrap();
                // info!("Err: {}", String::from_utf8(output.stderr).unwrap());
                // info!("Status: {}", output.status);
                // info!("Out: {}", String::from_utf8(output.stdout).unwrap());

                sqlx::query!(
                    "UPDATE public.audio_files
				SET silence=true
				WHERE file_name=$1;",
                    file_name
                )
                .execute(pool.get_ref())
                .await
                .unwrap();

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
) -> impl Responder {
    let (guild_id, channel_id, year, month, file_name) = path.into_inner();

    let file_path = format!(
        "{}{}/{}/{}/{}",
        RECORDING_PATH, guild_id, channel_id, year, month
    );
    let files = match std::fs::read_dir(&file_path) {
        Ok(ok) => ok,
        Err(err) => {
            panic!("cannot read files {}", err);
        }
    };

    for file in files {
        let file_name = file.unwrap().file_name();
        let file_n = file_name.to_string_lossy();
        // file_n.rsplit_once('/');
        let start = std::time::Instant::now();
        let command = std::process::Command::new("ffprobe")
            .arg("-show_entries")
            .arg("format=duration")
            .args(["-of", "default=noprint_wrappers=1:nokey=1"])
            .arg(format!("{}/{}", file_path, file_n))
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        let output = command.wait_with_output().unwrap();

        let duration = start.elapsed();

        info!("Time elapsed in ffprobe is: {:?}", duration);

        info!("Out: {}", String::from_utf8(output.stdout).unwrap());
        // info!("ERR: {}", String::from_utf8(output.stderr).unwrap());
    }

    let (_time, user_id) = file_name.split_once('-').expect("expected valid string");

    info!("1: {}, 2: {}", user_id, _time);

    // info!("{:#?}", files);
    return "";
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
) -> impl Responder {
    let (guild_id, channel_id, year, month, file_name_from_url) = path.into_inner();

    let file_name_without_guild_id = format!("{}/{}/{}", year, month, file_name_from_url);
    let temp_file = format!(
        "{}/{}/{}{}",
        year, month, NO_SILENCE_PREFIX, file_name_from_url
    );

    // Download full file
    info!(
        "file_path: {:#?} is silence recording? {:#?}",
        format!(
            "{}{}/{}/{}",
            RECORDING_PATH, guild_id, channel_id, &file_name_without_guild_id
        ),
        is_silence
    );

    let file = if is_silence.silence.is_some() {
        actix_files::NamedFile::open(
            format!(
                "{}{}/{}/{}",
                NO_SILENCE_RECORDING_PATH, guild_id, channel_id, &temp_file
            )
            .as_str(),
        )
        .unwrap()
    } else {
        actix_files::NamedFile::open(
            format!(
                "{}{}/{}/{}",
                RECORDING_PATH, guild_id, channel_id, &file_name_without_guild_id
            )
            .as_str(),
        )
        .unwrap()
    };

    file.use_last_modified(true)
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![],
        })
}
