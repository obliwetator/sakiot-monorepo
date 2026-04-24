use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use sqlx::postgres::PgPoolOptions;
use std::error::Error;
use web_server::http_metrics::HttpMetrics;
use web_server::telemetry::init_telemetry;

use web_server::admin::cooldowns::{
    delete_user_override, get_guild_cooldown, list_user_overrides, set_guild_cooldown,
    set_user_override,
};
use web_server::audio::{
    download_audio, get_audio, get_current_month_permission, get_waveform_data, remove_silence,
    HashMapContainer, WaveformProgressContainer,
};
use web_server::auth::{
    dev_login, discord_login, get_token, logout, refresh_jwt, AccessKeys, AuthMiddleware,
};
use web_server::clips::hello_world::jammer_client::JammerClient;
use web_server::clips::{create_clip, delete, get_clip, get_clips, play_clip};
use web_server::config::{
    ACCESS_SECRET, CORS_ALLOWED_ORIGIN, DATABASE_URL, DB_MAX_CONNECTIONS, GRPC_ADDRESS, HOST, PORT,
    REFRESH_SECRET,
};
use web_server::dashboard;
use web_server::stamps::get_stamps;
use web_server::user::{get_current_user, get_current_user_guilds};

use std::collections::HashMap;
use tokio::sync::RwLock;
use tonic::transport::Channel;

async fn not_found() -> impl Responder {
    let html = include_str!("../404.html");

    HttpResponse::NotFound()
        .content_type("text/html; charset=utf-8")
        .body(html)
}

// (scheme_prefix, suffix-including-dot) for subdomain match, or None if exact-only.
fn cors_subdomain_pattern() -> Option<(&'static str, String)> {
    let allowed = CORS_ALLOWED_ORIGIN.as_str();
    for scheme in ["https://", "http://"] {
        if let Some(domain) = allowed.strip_prefix(scheme) {
            return Some((scheme, format!(".{domain}")));
        }
    }
    None
}

#[actix_web::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().ok();
    env_logger::init();

    init_telemetry();

    let hashmap = web::Data::new(HashMapContainer(RwLock::new(HashMap::new())));
    let waveform_progress = web::Data::new(WaveformProgressContainer(RwLock::new(HashMap::new())));

    let grpc_channel = Channel::from_shared(GRPC_ADDRESS.to_string())?.connect_lazy();
    let jammer_client = JammerClient::new(grpc_channel);

    let pool = PgPoolOptions::new()
        .max_connections(*DB_MAX_CONNECTIONS)
        .connect(DATABASE_URL.as_str())
        .await?;

    let keys = web::Data::new(AccessKeys {
        access_encode: jsonwebtoken::EncodingKey::from_secret(ACCESS_SECRET.as_bytes()),
        refresh_encode: jsonwebtoken::EncodingKey::from_secret(REFRESH_SECRET.as_bytes()),
        access_decode: jsonwebtoken::DecodingKey::from_secret(ACCESS_SECRET.as_bytes()),
        refresh_decode: jsonwebtoken::DecodingKey::from_secret(REFRESH_SECRET.as_bytes()),
    });

    let cors_subdomain = cors_subdomain_pattern();

    let server = HttpServer::new(move || {
        let cors_exact = CORS_ALLOWED_ORIGIN.as_str();
        let cors_sub = cors_subdomain.clone();
        let cors = Cors::default()
            .allowed_origin_fn(move |origin, _req_head| {
                let Ok(origin_str) = origin.to_str() else {
                    return false;
                };
                if origin_str == cors_exact {
                    return true;
                }
                if let Some((scheme, suffix)) = &cors_sub {
                    return origin_str.starts_with(scheme) && origin_str.ends_with(suffix);
                }
                false
            })
            .allow_any_method()
            .allow_any_header()
            .supports_credentials()
            .max_age(3600);

        let api_scope = web::scope("/api")
            .wrap(AuthMiddleware)
            .service(discord_login)
            .service(dev_login)
            .service(refresh_jwt)
            .service(logout)
            .service(get_current_user)
            .service(get_current_user_guilds)
            .service(get_token)
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
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(reqwest::Client::new()))
            .app_data(hashmap.clone())
            .app_data(waveform_progress.clone())
            .app_data(web::Data::new(jammer_client.clone()))
            .app_data(keys.clone())
            .service(api_scope)
            .default_service(web::route().to(not_found))
            // Wraps execute outermost-first on request (reverse registration order).
            // Request flow:  Cors -> Logger -> HttpMetrics -> AuthMiddleware -> handler
            // Response flow: handler -> AuthMiddleware -> HttpMetrics -> Logger -> Cors
            // Cors outermost: short-circuits preflights before logging/metrics;
            // applies headers to every response (including 404/5xx).
            // Logger above metrics: records final status after all middleware runs.
            // HttpMetrics innermost at app level: measures handler+auth latency only.
            .wrap(HttpMetrics)
            .wrap(Logger::default())
            .wrap(cors)
    })
    .bind((HOST.as_str(), *PORT))?
    .run();

    server.await?;
    Ok(())
}
