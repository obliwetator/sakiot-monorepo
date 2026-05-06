use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use sqlx::postgres::PgPoolOptions;
use std::error::Error;
use web_server::http_metrics::HttpMetrics;
use web_server::telemetry::init_telemetry;

use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};
use web_server::openapi::ApiDoc;

use web_server::admin::cooldowns::{
    delete_user_override, get_guild_cooldown, list_user_overrides, set_guild_cooldown,
    set_user_override,
};
use web_server::audio::{
    download_audio, get_audio, get_current_month_permission, get_live_stems,
    get_recording_events, get_waveform_data, live_playlist, live_segment, live_state,
    remove_silence, HashMapContainer, LiveContainer, WaveformProgressContainer,
};
use web_server::auth::{
    discord_login, logout, oauth_start, refresh_jwt, AccessKeys, AuthMiddleware,
};
#[cfg(feature = "dev-login")]
use web_server::auth::dev_login;
use web_server::proto::jammer::jammer_client::JammerClient;
use web_server::clips::{create_clip, delete, get_clip, get_clips, play_clip};
use web_server::config::Config;
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
fn cors_subdomain_pattern(allowed: &str) -> Option<(&'static str, String)> {
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

    let cfg = Config::from_env()?;

    let hashmap = web::Data::new(HashMapContainer(RwLock::new(HashMap::new())));
    let waveform_progress = web::Data::new(WaveformProgressContainer(RwLock::new(HashMap::new())));
    let live_container = web::Data::new(LiveContainer::default());

    let grpc_channel = Channel::from_shared(cfg.grpc_address.clone())?.connect_lazy();
    let jammer_client = JammerClient::new(grpc_channel);

    let pool = PgPoolOptions::new()
        .max_connections(cfg.db_max_connections)
        .connect(&cfg.database_url)
        .await?;

    let keys = web::Data::new(AccessKeys {
        access_encode: jsonwebtoken::EncodingKey::from_secret(cfg.access_secret.as_bytes()),
        refresh_encode: jsonwebtoken::EncodingKey::from_secret(cfg.refresh_secret.as_bytes()),
        access_decode: jsonwebtoken::DecodingKey::from_secret(cfg.access_secret.as_bytes()),
        refresh_decode: jsonwebtoken::DecodingKey::from_secret(cfg.refresh_secret.as_bytes()),
    });

    let cors_subdomain = cors_subdomain_pattern(&cfg.cors_allowed_origin);
    let cors_exact = cfg.cors_allowed_origin.clone();
    let host = cfg.host.clone();
    let port = cfg.port;
    let cfg_data = web::Data::new(cfg);

    let server = HttpServer::new(move || {
        let cors_exact = cors_exact.clone();
        let cors_sub = cors_subdomain.clone();
        let cors = Cors::default()
            .allowed_origin_fn(move |origin, _req_head| {
                let Ok(origin_str) = origin.to_str() else {
                    return false;
                };
                if origin_str == cors_exact.as_str() {
                    return true;
                }
                if let Some((scheme, suffix)) = &cors_sub {
                    return origin_str.starts_with(scheme) && origin_str.ends_with(suffix);
                }
                false
            })
            .allow_any_method()
            .allow_any_header()
            // Media element streaming needs these readable from JS / browser
            // internals; not safelisted by default under CORS.
            .expose_headers(["Content-Length", "Content-Range", "Accept-Ranges"])
            .supports_credentials()
            .max_age(3600);

        let api_scope = web::scope("/api")
            .wrap(AuthMiddleware)
            .service(discord_login)
            .service(oauth_start);
        #[cfg(feature = "dev-login")]
        let api_scope = api_scope.service(dev_login);
        let api_scope = api_scope
            .service(refresh_jwt)
            .service(logout)
            .service(get_current_user)
            .service(get_current_user_guilds)
            .service(get_live_stems)
            .service(get_current_month_permission)
            .service(remove_silence)
            .service(delete)
            .service(dashboard::dashboard_stream)
            .service(get_clips)
            .service(get_clip)
            .service(get_stamps)
            .service(play_clip)
            .service(create_clip)
            // Live HLS routes — register before get_audio to avoid pattern fallback churn.
            .service(live_playlist)
            .service(live_state)
            .service(live_segment)
            .service(get_audio)
            .service(get_recording_events)
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
            .app_data(live_container.clone())
            .app_data(web::Data::new(jammer_client.clone()))
            .app_data(keys.clone())
            .app_data(cfg_data.clone())
            .service(api_scope)
            .service(Scalar::with_url("/scalar", ApiDoc::openapi()))
            .route(
                "/api-doc/openapi.json",
                web::get().to(|| async { HttpResponse::Ok().json(ApiDoc::openapi()) }),
            )
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
    .bind((host.as_str(), port))?
    .run();

    server.await?;
    Ok(())
}
