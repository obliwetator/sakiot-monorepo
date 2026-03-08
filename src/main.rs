mod audio;
mod auth;
mod clips;
mod dashboard;
mod errors;
mod grpc;
mod permissions;
mod user;
mod waveform;
mod websocket;

use actix_web::body::EitherBody;
use actix_web::middleware::Logger;
use actix_web::web::ReqData;
use audio::{download_audio, find_similar, get_audio, get_waveform_data, remove_silence};
use auth::{discord_login, get_token, refresh_jwt, ACCESS_SECRET, REFRESH_SECRET};
use clips::{delete, get_clip, get_clips, play_clip};
use jsonwebtoken::{DecodingKey, EncodingKey};
use permissions::get_everyone_permission_for_guild;
use sqlx::Postgres;

use tokio::sync::broadcast::Sender;
use tokio::sync::RwLock;
use websocket::web_socket;

use std::collections::{HashMap, HashSet};
use std::fs::ReadDir;

use std::process::Stdio;
use tonic::transport::Server;
use user::{get_current_user, get_current_user_guilds};

use actix_cors::Cors;
use actix_web::{get, web, App, HttpMessage, HttpResponse, HttpServer, Responder};
use serde::{Deserialize, Serialize};
use sqlx::pool::Pool;
use sqlx::postgres::PgPoolOptions;
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Serialize, Debug)]
pub struct Directories {
    year: i32,
    months: Option<Months>,
}

#[derive(Serialize, Debug)]
pub struct Channels {
    channel_id: String,
    dirs: Vec<Directories>,
}

#[derive(Serialize, Debug)]
struct File {
    file: String,
    comment: Option<String>,
}

#[derive(Debug)]
enum DBErrors {
    Unknown,
    UniqueViolation,
}

type Months = HashMap<String, Option<Vec<File>>>;
pub const RECORDING_PATH: &str = "/home/tulipan/projects/FBI-agent/voice_recordings/";
pub const NO_SILENCE_RECORDING_PATH: &str =
    "/home/tulipan/projects/FBI-agent/no_silence_voice_recordings/";
pub const CLIPS_PATH: &str = "/home/tulipan/projects/FBI-agent/clips/";
pub const NO_SILENCE_PREFIX: &str = "_no_silence_";
pub const WAVEFORM_PATH: &str = "/home/tulipan/projects/FBI-agent/waveform_data/";

#[inline]
async fn for_entry(entries: ReadDir, _channel: i64, dirs: &mut Directories, month_as_string: &str) {
    for entry in entries {
        if let Ok(entry) = entry {
            let file_name = File {
                file: entry.file_name().to_str().unwrap().to_owned(),
                comment: None,
            };
            dirs.months
                .as_mut()
                .unwrap()
                .get_mut(month_as_string)
                .unwrap()
                .as_mut()
                .unwrap()
                .push(file_name);
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

async fn for_channel_ids(
    guild_id: String,
    dirs_vec: &mut Vec<Channels>,
    channel_hashset: HashSet<i64>,
) -> Option<Result<Vec<Channels>, HttpResponse>> {
    let channel_ids = match std::fs::read_dir(format!("{}{}", RECORDING_PATH, guild_id)) {
        Ok(ok) => ok,
        Err(err) => {
            error!("{}", err);
            return Some(Err(HttpResponse::NotFound()
                .body("files does not exist or are innacessible to you 1\n")));
        }
    };

    for channel_id in channel_ids {
        if let Ok(entry) = channel_id {
            let channel = entry
                .file_name()
                .to_str()
                .unwrap()
                .to_owned()
                .parse::<i64>()
                .unwrap();

            if channel_hashset.contains(&channel) {
                // we have the channel is the hashset. User can access this channel
                let years = match std::fs::read_dir(format!(
                    "{}{}/{}",
                    RECORDING_PATH, guild_id, channel
                )) {
                    Ok(ok) => ok,
                    Err(err) => {
                        error!("{}", err);
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

    // info!("{:#?}", dirs_vec);

    None
}

#[inline]
async fn for_years(
    years: ReadDir,
    guild_id: &String,
    channel: i64,
    dirs_vec: &mut Channels,
) -> Option<Result<Vec<Channels>, HttpResponse>> {
    for year in years {
        if let Ok(entry) = year {
            let year_as_int = entry
                .file_name()
                .to_str()
                .unwrap()
                .to_owned()
                .parse::<i32>()
                .unwrap();

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
                    error!("{}", err);
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
async fn for_months(
    months: ReadDir,
    dirs: &mut Directories,
    guild_id: &String,
    channel: i64,
    year_as_int: i32,
) -> Option<Result<Vec<Channels>, HttpResponse>> {
    for month in months {
        if let Ok(entry) = month {
            let month_as_string = entry.file_name().to_str().unwrap().to_owned();

            dirs.months
                .as_mut()
                .unwrap()
                .insert(month_as_string.to_owned(), Some(vec![]));

            let entries = match std::fs::read_dir(format!(
                "{}{}/{}/{}/{}",
                RECORDING_PATH, guild_id, channel, year_as_int, &month_as_string
            )) {
                Ok(ok) => ok,
                Err(err) => {
                    error!("{}", err);
                    return Some(Err(HttpResponse::NotFound()
                        .body("files does not exist or are innacessible to you 3\n")));
                }
            };

            for_entry(entries, channel, dirs, &month_as_string).await;
        } else {
            info!("error for month")
        }
    }
    None
}

pub async fn get_months(path: web::Path<String>) -> Result<Vec<Directories>, HttpResponse> {
    let guild_id = path.into_inner();

    let years = match std::fs::read_dir(format!(
        "/home/tulipan/projects/FBI-agent/voice_recordings/{}",
        guild_id
    )) {
        Ok(ok) => ok,
        Err(err) => {
            error!("{}", err);
            return Err(HttpResponse::NotFound()
                .body("files does not exist or are innacessible to you 1\n"));
        }
    };

    let mut dirs_vec = Vec::new();

    // Get all the year(s) for this guild
    for year in years {
        if let Ok(entry) = year {
            let year_as_int = entry
                .file_name()
                .to_str()
                .unwrap()
                .to_owned()
                .parse::<i32>()
                .unwrap();

            let mut dirs = Directories {
                year: year_as_int,
                months: Some(HashMap::new()),
            };

            info!("{}", year_as_int);

            let months = match std::fs::read_dir(format!(
                "/home/tulipan/projects/FBI-agent/voice_recordings/{}/{}",
                guild_id, year_as_int
            )) {
                Ok(ok) => ok,
                Err(err) => {
                    error!("{}", err);
                    return Err(HttpResponse::NotFound()
                        .body("files does not exist or are innacessible to you 2\n"));
                }
            };

            for month in months {
                if let Ok(entry) = month {
                    let month_as_string = entry.file_name().to_str().unwrap().to_owned();

                    dirs.months
                        .as_mut()
                        .unwrap()
                        .insert(month_as_string.to_owned(), Some(vec![]));

                    let entries = match std::fs::read_dir(format!(
                        "/home/tulipan/projects/FBI-agent/voice_recordings/{}/{}/{}",
                        guild_id, year_as_int, &month_as_string
                    )) {
                        Ok(ok) => ok,
                        Err(err) => {
                            error!("{}", err);
                            return Err(HttpResponse::NotFound()
                                .body("files does not exist or are innacessible to you 3\n"));
                        }
                    };

                    for entry in entries {
                        if let Ok(entry) = entry {
                            let file_name = File {
                                file: entry.file_name().to_str().unwrap().to_owned(),
                                comment: None,
                            };
                            dirs.months
                                .as_mut()
                                .unwrap()
                                .get_mut(&month_as_string)
                                .unwrap()
                                .as_mut()
                                .unwrap()
                                .push(file_name);
                        } else {
                            error!("error for file");
                        }
                    }
                } else {
                    error!("error for month")
                }
            }
            dirs_vec.push(dirs);
        } else {
            error!("error for year");
        }
    }

    Ok(dirs_vec)
}

#[get("/current/{guild_id}")]
async fn get_current_month_permission(
    path: web::Path<String>,
    token: Option<ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> impl Responder {
    let token = match token {
        Some(ok) => ok,
        None => {
            // TODO: Proper message
            panic!("Token should be present")
        }
    };

    let guild_id = path.into_inner();
    let guild_id_as_int = guild_id.parse::<i64>().unwrap();

    let permission_hashset =
        get_available_channels_for_user(&pool, guild_id_as_int, token.id).await;

    // Check which channel the user is allow to view/connect

    // for (ch, perm) in permission_hashmap.iter() {
    //     let res = perm & Permissions::CONNECT.bits() == Permissions::CONNECT.bits();

    //     info!("Can connect to channel_id :{} => {}", ch, res);
    // }

    // info!("perm_HASHMAP: {:#?}", permission_hashmap);

    let result = get_channels_dir(guild_id, permission_hashset).await;

    let resp = match result {
        Ok(dirs_vec) => HttpResponse::Ok().json(dirs_vec),
        Err(err) => err,
    };

    resp
}

// Different channels can have different permissions for roles AND specific users
// We go over every channel - role/user combination
// TODO: return early if admin
// TODO: check if we can return early between each check
#[get("/download_the_clip")]
async fn download_the_clip() -> Result<actix_files::NamedFile, Error> {
    Ok(actix_files::NamedFile::open("/home/tulipan/clips.tar.gz")?)
}

pub async fn get_available_channels_for_user(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    user_id: i64,
) -> HashSet<i64> {
    // [0] = allow, [1] = deny
    let mut perm_hash: HashMap<i64, [i64; 2]> = HashMap::new();
    let mut allowed_channels: HashSet<i64> = HashSet::new();
    let mut denied_channels: HashSet<i64> = HashSet::new();

    let everyone_permission = get_everyone_permission_for_guild(pool, guild_id).await;
    let combined_permission = get_combined_perm_for_user(pool, guild_id, user_id).await;

    // This is the highest non-specific permission for user
    let total_permission = everyone_permission | combined_permission;

    get_user_channel_overrides_for_user_id(user_id, guild_id, pool, &mut perm_hash).await;

    // Is admin
    if (total_permission & Permissions::ADMINISTRATOR.bits()) == Permissions::ADMINISTRATOR.bits() {
        perm_hash.retain(|ch_id, _| {
            allowed_channels.insert(*ch_id);

            false
        });
    }

    perm_hash.retain(|ch_id, perm_vec| {
        let allow = (perm_vec[0] & Permissions::CONNECT.bits()) == Permissions::CONNECT.bits();
        let deny = (perm_vec[1] & Permissions::CONNECT.bits()) == Permissions::CONNECT.bits();

        if allow {
            allowed_channels.insert(*ch_id);
        }
        if deny {
            denied_channels.insert(*ch_id);
        }

        !allow && !deny
    });

    perms_for_roles_for_channel(pool, user_id, guild_id, &mut perm_hash).await;

    perm_hash.retain(|ch_id, perm_vec| {
        let allow = (perm_vec[0] & Permissions::CONNECT.bits()) == Permissions::CONNECT.bits();
        let deny = (perm_vec[1] & Permissions::CONNECT.bits()) == Permissions::CONNECT.bits();

        // If both allow and deny are true the allow value overrides the deny
        if (allow == true) && (deny == true) {
            allowed_channels.insert(*ch_id);
            return !allow && !deny;
        }

        if allow {
            allowed_channels.insert(*ch_id);
        }
        if deny {
            denied_channels.insert(*ch_id);
        }

        !allow && !deny
    });

    get_everyone_permission_for_each_channel(pool, guild_id, &mut perm_hash).await;

    perm_hash.retain(|ch_id, perm_vec| {
        let allow = (perm_vec[0] & Permissions::CONNECT.bits()) == Permissions::CONNECT.bits();
        let deny = (perm_vec[1] & Permissions::CONNECT.bits()) == Permissions::CONNECT.bits();

        if allow {
            allowed_channels.insert(*ch_id);
        }
        if deny {
            denied_channels.insert(*ch_id);
        }

        !allow && !deny
    });

    // Final most general check. Check @everyone
    perm_hash.retain(|ch_id, perm_vec| {
        let allow = ((perm_vec[0] | total_permission) & Permissions::CONNECT.bits())
            == Permissions::CONNECT.bits();

        if allow {
            allowed_channels.insert(*ch_id);
        }

        !allow
    });

    allowed_channels
}

async fn get_user_channel_overrides_for_user_id(
    user_id: i64,
    guild_id: i64,
    pool: &web::Data<Pool<Postgres>>,
    perm_hash: &mut HashMap<i64, [i64; 2]>,
) {
    let specfic_perm_for_channel = match sqlx::query!(
        "SELECT allow, deny, channel_id as \"channel_id!\", name as 
			\"name!\" FROM get_user_channel_overriders_for_user_id($1, $2)",
        user_id,
        guild_id
    )
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(ok) => ok,
        Err(_) => {
            // TODO:
            panic!("Error ")
        }
    };

    // member specific override
    for specific_perm in specfic_perm_for_channel {
        perm_hash.insert(
            specific_perm.channel_id,
            [
                specific_perm.allow.unwrap_or(0),
                specific_perm.deny.unwrap_or(0),
            ],
        );
    }
}

#[get("/current/{guild_id}")]
async fn perm_calc(
    _path: web::Path<String>,
    _token: Option<ReqData<Token<Access>>>,
    _pool: web::Data<Pool<Postgres>>,
) -> impl Responder {
    HttpResponse::Ok()
}

#[derive(Deserialize, Debug)]
struct StartEnd {
    start: Option<f32>,
    end: Option<f32>,
    name: Option<String>,
}

async fn not_found() -> impl Responder {
    HttpResponse::NotFound().json("not found")
}

pub struct AccessKeys {
    pub access_encode: EncodingKey,
    pub refresh_encode: EncodingKey,
    pub access_decode: DecodingKey,
    pub refresh_decode: DecodingKey,
}

#[derive(Debug)]
// struct HashMapContainer(pub Arc<Mutex<HashMap<String, tokio::sync::broadcast::Receiver<f32>>>>);

struct HashMapContainer(pub RwLock<HashMap<String, Sender<i32>>>);

#[actix_web::main]
async fn main() {
    // std::env::set_var("RUST_LOG", "debug");
    // std::env::set_var("RUST_BACKTRACE", "1");
    env_logger::init();

    let hashmap = web::Data::new(HashMapContainer(RwLock::new(HashMap::new())));
    // Clone here, this one will be owned by the first closure = hashmap;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect("postgres://postgres:okcpli4t94@localhost/sakiot_rouvas")
        .await
        .expect("cannot connect to database");

    let subscriber = FmtSubscriber::builder()
        // .with_thread_names(true)
        // .with_file(true)
        // .with_target(true)
        // .with_line_number(true)
        // all spans/events with a level higher than TRACE (e.g, debug, info, warn, etc.)
        // will be written to stdout.
        .with_max_level(Level::INFO)
        .pretty()
        // completes the builder.
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    // let res = test_endpoint(pool.clone()).await;

    // let mut builder =
    //     openssl::ssl::SslAcceptor::mozilla_intermediate(openssl::ssl::SslMethod::tls()).unwrap();
    // builder
    //     .set_private_key_file("key.pem", openssl::ssl::SslFiletype::PEM)
    //     .unwrap();
    // builder.set_certificate_chain_file("cert.pem").unwrap();

    let server: actix_web::dev::Server = HttpServer::new(move || {
        let logger = Logger::default();
        // Create the singing keys once. reuse them for every encode/decode
        let keys = AccessKeys {
            access_encode: EncodingKey::from_secret(ACCESS_SECRET.as_bytes()),
            refresh_encode: EncodingKey::from_secret(REFRESH_SECRET.as_bytes()),
            access_decode: DecodingKey::from_secret(ACCESS_SECRET.as_bytes()),
            refresh_decode: DecodingKey::from_secret(REFRESH_SECRET.as_bytes()),
        };

        let api_scope = web::scope("/api")
            .wrap(AuthMiddleware)
            .service(discord_login)
            .service(refresh_jwt)
            .service(get_current_user)
            .service(get_current_user_guilds)
            .service(get_token)
            .service(find_similar)
            .service(get_current_month_permission)
            .service(perm_calc)
            .service(remove_silence)
            .service(delete)
            .service(dashboard::dashboard_stream);
        App::new()
            .wrap(logger)
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(reqwest::Client::new()))
            .app_data(hashmap.clone())
            .app_data(web::Data::new(keys))
            .service(web_socket)
            .service(api_scope)
            .service(get_audio)
            .service(download_audio)
            .service(get_clips)
            .service(get_clip)
            .service(play_clip)
            .service(get_waveform_data)
            .service(download_the_clip)
            .default_service(web::route().to(not_found))
            .wrap(
                Cors::permissive(), // Cors::default()
                                    //     .allow_any_origin()
                                    //     .allow_any_header()
                                    //     .allow_any_method(),
            )
    })
    .bind(("127.0.0.1", 8900))
    .unwrap()
    .run();

    let _tonic = tokio::spawn(async move {
        let addr = "[::1]:50051".parse().unwrap();
        let greeter = MyGreeter::default();

        info!("GreeterServer listening on {}", addr);

        Server::builder()
            .add_service(GreeterServer::new(greeter))
            .serve(addr)
            .await
    });

    let _c = tokio::spawn(async move { server.await });
    let _res = tokio::join!(_c, _tonic);
}

use std::future::{ready, Ready};

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error,
};
use futures_util::future::LocalBoxFuture;

use crate::auth::{Access, Refresh, Token};
use crate::grpc::hello_world::greeter_server::GreeterServer;
use crate::grpc::MyGreeter;
use crate::permissions::{
    get_combined_perm_for_user, get_everyone_permission_for_each_channel,
    perms_for_roles_for_channel, Permissions,
};

// There are two steps in middleware processing.
// 1. Middleware initialization, middleware factory gets called with
//    next service in chain as parameter.
// 2. Middleware's call method gets called with normal request.
pub struct AuthMiddleware;

// Middleware factory is `Transform` trait
// `S` - type of the next service
// `B` - type of response's body
impl<S, B> Transform<S, ServiceRequest> for AuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = SayHiMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(SayHiMiddleware { service }))
    }
}

pub struct SayHiMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for SayHiMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        info!("PATH: {:#?}", req.path());
        if req.path() == "/api/discord_login"
            || req.path() == "/api/refresh"
            || req.path() == "/api/dashboard/stream"
        {
            // Dont validate the token if user is trying to login, refresh, or connecting to websocket
            let res = self.service.call(req);

            Box::pin(async move {
                // forwarded responses map to "left" body
                res.await.map(ServiceResponse::map_into_left_body)
            })
        } else {
            let headers = req.headers();
            let cookie = match headers.get("cookie") {
                Some(cookie) => cookie,
                None => {
                    warn!(
                        "Unauthorized access attempt to  middleware {}: missing cookie",
                        req.path()
                    );
                    let (request, _pl) = req.into_parts();

                    let response = HttpResponse::Unauthorized().finish().map_into_right_body();
                    return Box::pin(async { Ok(ServiceResponse::new(request, response)) });
                }
            };

            let keys = req.app_data::<web::Data<AccessKeys>>().unwrap();

            let (access_token, refresh_token) = get_access_and_refresh_tokens(cookie);

            let decoded_access = match Token::<Access>::decode(access_token, keys) {
                Ok(token) => token,
                Err(_) => {
                    warn!(
                        "Unauthorized access attempt to middleware {}: invalid or expired token",
                        req.path()
                    );
                    let (request, _pl) = req.into_parts();
                    let response = HttpResponse::Unauthorized().finish().map_into_right_body();
                    return Box::pin(async { Ok(ServiceResponse::new(request, response)) });
                }
            };
            info!("Decoded access token: {:#?}", decoded_access.exp);
            let _decoded_refresh = Token::<Refresh>::decode(refresh_token, keys);

            req.extensions_mut().insert(decoded_access);
            let res = self.service.call(req);

            Box::pin(async move {
                // forwarded responses map to "left" body
                res.await.map(ServiceResponse::map_into_left_body)
            })
        }
    }
}

pub fn get_access_and_refresh_tokens(cookie: &reqwest::header::HeaderValue) -> (&str, &str) {
    let tokens = cookie.to_str().unwrap();
    let access_refresh: Vec<&str> = tokens.split(';').collect();

    let access: Vec<&str> = access_refresh[0].split('=').collect();
    let access_token = access[1];

    let refresh: Vec<&str> = access_refresh[1].split('=').collect();
    let refresh_token = refresh[1];

    (access_token, refresh_token)
}

#[allow(dead_code)]
pub async fn test_endpoint(pool: Pool<Postgres>) {
    let guilds = match std::fs::read_dir(format!("{}", RECORDING_PATH)) {
        Ok(ok) => ok,
        Err(err) => {
            error!("cannot read dir: {}", err);
            panic!()
        }
    };

    for guild in guilds {
        if let Ok(entry) = guild {
            let guild_id = entry
                .file_name()
                .to_str()
                .unwrap()
                .to_owned()
                .parse::<i64>()
                .unwrap();

            let channel_ids = match std::fs::read_dir(format!("{}{}", RECORDING_PATH, guild_id)) {
                Ok(ok) => ok,
                Err(err) => {
                    error!("{}", err);
                    panic!();
                }
            };

            for channel_id in channel_ids {
                if let Ok(entry) = channel_id {
                    let channel = entry
                        .file_name()
                        .to_str()
                        .unwrap()
                        .to_owned()
                        .parse::<i64>()
                        .unwrap();

                    let years = match std::fs::read_dir(format!(
                        "{}{}/{}",
                        RECORDING_PATH, guild_id, channel
                    )) {
                        Ok(ok) => ok,
                        Err(err) => {
                            error!("{}", err);
                            panic!();
                        }
                    };

                    for year in years {
                        if let Ok(entry) = year {
                            let year_as_int = entry
                                .file_name()
                                .to_str()
                                .unwrap()
                                .to_owned()
                                .parse::<i32>()
                                .unwrap();

                            let months = match std::fs::read_dir(format!(
                                "{}{}/{}/{}",
                                RECORDING_PATH, guild_id, channel, year_as_int
                            )) {
                                Ok(ok) => ok,
                                Err(err) => {
                                    error!("{}", err);
                                    panic!();
                                }
                            };

                            for month in months {
                                if let Ok(entry) = month {
                                    let month_as_string =
                                        entry.file_name().to_str().unwrap().to_owned();
                                    let month_as_number = match month_as_string.as_str() {
                                        "January" => 1,
                                        "February" => 2,
                                        "March" => 3,
                                        "April" => 4,
                                        "May" => 5,
                                        "June" => 6,
                                        "July" => 7,
                                        "August" => 8,
                                        "September" => 9,
                                        "October" => 10,
                                        "November" => 11,
                                        "December" => 12,
                                        _ => 13,
                                    };

                                    let entries = match std::fs::read_dir(format!(
                                        "{}{}/{}/{}/{}",
                                        RECORDING_PATH,
                                        guild_id,
                                        channel,
                                        year_as_int,
                                        &month_as_string
                                    )) {
                                        Ok(ok) => ok,
                                        Err(err) => {
                                            error!("{}", err);
                                            panic!();
                                        }
                                    };

                                    for entry in entries {
                                        if let Ok(entry) = entry {
                                            let file_name =
                                                entry.file_name().to_str().unwrap().to_owned();
                                            let (time, user_id_and_name) = file_name
                                                .split_once('-')
                                                .expect("expected valid string");
                                            let time_as_int = time.parse::<i64>().unwrap();
                                            let (user_id, _) = user_id_and_name
                                                .split_once('-')
                                                .expect("expected valid string");
                                            let user_id_as_int = user_id.parse::<i64>().unwrap();

                                            let file_path = format!(
                                                "{}{}/{}/{}/{}",
                                                RECORDING_PATH,
                                                guild_id,
                                                channel,
                                                year_as_int,
                                                month_as_string
                                            );

                                            let command = std::process::Command::new("ffprobe")
                                                .arg("-show_entries")
                                                .arg("format=duration")
                                                .args(["-of", "default=noprint_wrappers=1:nokey=1"])
                                                .arg(format!("{}/{}", file_path, file_name))
                                                .stderr(Stdio::null())
                                                .stdin(Stdio::null())
                                                .stdout(Stdio::piped())
                                                .spawn()
                                                .unwrap();

                                            let output = command.wait_with_output().unwrap();
                                            // let stderr = String::from_utf8(output.stderr).unwrap();
                                            let stdout = String::from_utf8(output.stdout).unwrap();
                                            // info!("STD ERR: {}", stderr);
                                            info!("STD OUT: {}", stdout);
                                            // If a file is corrupted its duration will be undefined. Set it to 0 and deal with it later
                                            // Duration is in seconds. Convert to ms
                                            let duration_in_ms =
                                                (stdout.trim().parse::<f64>().unwrap_or(0.0)
                                                    * 1000.0)
                                                    as i64;

                                            let end_ts = time_as_int + duration_in_ms;

                                            match sqlx::query!(
                                                "INSERT INTO public.audio_files(file_name, guild_id, channel_id, user_id, year, month, start_ts, end_ts) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT DO NOTHING;",
                                                file_name,
                                                guild_id,
                                                channel,
                                                user_id_as_int,
                                                year_as_int,
                                                month_as_number,
                                                time_as_int,
                                                end_ts
                                            )
                                            .execute(&pool)
                                            .await
                                            {
                                                Ok(ok) => ok,
                                                Err(err) => {
                                                    error!("{}",err);
                                                    panic!()
                                                }
                                            };
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
