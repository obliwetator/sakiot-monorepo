use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use sqlx::postgres::PgPoolOptions;
use std::error::Error;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use web_server::admin::cooldowns::{
    delete_user_override, get_guild_cooldown, list_user_overrides, set_guild_cooldown,
    set_user_override,
};
use web_server::audio::{
    download_audio, find_similar, get_audio, get_current_month_permission, get_waveform_data,
    remove_silence, HashMapContainer, WaveformProgressContainer,
};
use web_server::auth::{discord_login, get_token, logout, refresh_jwt, AccessKeys, AuthMiddleware};
use web_server::clips::{create_clip, delete, get_clip, get_clips, play_clip};
use web_server::config::{ACCESS_SECRET, CORS_ALLOWED_ORIGIN, DATABASE_URL, REFRESH_SECRET};
use web_server::dashboard;
use web_server::grpc::hello_world::greeter_server::GreeterServer;
use web_server::grpc::MyGreeter;
use web_server::stamps::get_stamps;
use web_server::user::{get_current_user, get_current_user_guilds};
use web_server::clips::hello_world::jammer_client::JammerClient;
use web_server::websocket::web_socket;

use std::collections::HashMap;
use tokio::sync::RwLock;
use tonic::transport::{Channel, Server};

async fn not_found() -> impl Responder {
    HttpResponse::NotFound().json("not found")
}

#[actix_web::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().ok();
    env_logger::init();

    let hashmap = web::Data::new(HashMapContainer(RwLock::new(HashMap::new())));
    let waveform_progress = web::Data::new(WaveformProgressContainer(RwLock::new(HashMap::new())));

    let grpc_channel = Channel::from_static("http://[::1]:50052").connect_lazy();
    let jammer_client = JammerClient::new(grpc_channel);

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(DATABASE_URL.as_str())
        .await?;

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .pretty()
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;

    let server: actix_web::dev::Server = HttpServer::new(move || {
        let logger = Logger::default();
        let keys = AccessKeys {
            access_encode: jsonwebtoken::EncodingKey::from_secret(ACCESS_SECRET.as_bytes()),
            refresh_encode: jsonwebtoken::EncodingKey::from_secret(REFRESH_SECRET.as_bytes()),
            access_decode: jsonwebtoken::DecodingKey::from_secret(ACCESS_SECRET.as_bytes()),
            refresh_decode: jsonwebtoken::DecodingKey::from_secret(REFRESH_SECRET.as_bytes()),
        };

        let api_scope = web::scope("/api")
            .wrap(AuthMiddleware)
            .service(discord_login)
            .service(refresh_jwt)
            .service(logout)
            .service(get_current_user)
            .service(get_current_user_guilds)
            .service(get_token)
            .service(find_similar)
            .service(get_current_month_permission)
            .service(remove_silence)
            .service(delete)
            .service(dashboard::dashboard_stream)
            .service(get_clips)
            .service(get_clip)
            .service(get_stamps)
            .service(play_clip)
            .service(create_clip)
            .service(get_audio)
            .service(get_waveform_data)
            .service(download_audio)
            .service(get_guild_cooldown)
            .service(set_guild_cooldown)
            .service(list_user_overrides)
            .service(set_user_override)
            .service(delete_user_override);
        App::new()
            .wrap(logger)
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(reqwest::Client::new()))
            .app_data(hashmap.clone())
            .app_data(waveform_progress.clone())
            .app_data(web::Data::new(jammer_client.clone()))
            .app_data(web::Data::new(keys))
            .service(web_socket)
            .service(api_scope)
            .default_service(web::route().to(not_found))
            .wrap(
                Cors::default()
                    .allowed_origin_fn(|origin, _req_head| {
                        let allowed = CORS_ALLOWED_ORIGIN.as_str();
                        if let Ok(origin_str) = origin.to_str() {
                            if origin_str == allowed {
                                return true;
                            }
                            if let Some(domain) = allowed.strip_prefix("https://") {
                                if origin_str.starts_with("https://") && origin_str.ends_with(&format!(".{}", domain)) {
                                    return true;
                                }
                            }
                            if let Some(domain) = allowed.strip_prefix("http://") {
                                if origin_str.starts_with("http://") && origin_str.ends_with(&format!(".{}", domain)) {
                                    return true;
                                }
                            }
                        }
                        false
                    })
                    .allow_any_method()
                    .allow_any_header()
                    .supports_credentials()
                    .max_age(3600),
            )
    })
    .bind(("127.0.0.1", 8900))?
    .run();

    let addr = "[::1]:50051".parse()?;
    let tonic = tokio::spawn(async move {
        let greeter = MyGreeter::default();

        info!("GreeterServer listening on {}", addr);

        Server::builder()
            .add_service(GreeterServer::new(greeter))
            .serve(addr)
            .await
    });

    let http = tokio::spawn(server);
    let _ = tokio::join!(http, tonic);
    Ok(())
}
