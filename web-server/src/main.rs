use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{App, HttpResponse, HttpServer, Responder, web};
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
use web_server::admin::voice_settings::{
    delete_voice_settings, get_voice_settings, put_voice_settings,
};
use web_server::audio::{
    LiveContainer, SessionMixContainer, SilenceJobContainer, WaveformProgressContainer,
    create_session_clip, download_audio, download_session, generate_session_channel_mix, get_audio,
    get_clip_waveform_data, get_current_month_permission, get_live_stems, get_recording_events,
    get_session_channel_mix, get_session_channel_mix_media, get_session_events,
    get_session_manifest, get_session_segment, get_session_silence_free,
    get_session_silence_free_waveform, get_session_silence_removal_status, get_session_waveform,
    get_waveform_data, live_playlist, live_segment, live_state,
    rebuild_session_silence_free_waveform, rebuild_session_waveform, remove_session_silence,
    remove_silence, session_live_playlist, session_live_segment, spawn_hls_reaper,
};
#[cfg(feature = "dev-login")]
use web_server::auth::dev_login;
use web_server::auth::{
    AccessKeys, AuthMiddleware, discord_login, logout, oauth_start, refresh_jwt,
};
use web_server::clip_editor::{compose_clip, compose_clip_status};
use web_server::clips::{create_clip, delete, get_clip, get_clips, play_clip, rename_clip};
use web_server::config::Config;
use web_server::fbi_agent_registry::{
    AgentGrpcRegistry, get_agent_grpc_endpoints, register_agent_grpc_endpoints,
};
use web_server::health::healthz;
use web_server::media_archive::{
    MediaArchive, run_media_command, spawn_archive_worker, spawn_local_cleanup,
};
use web_server::members::{get_guild_roles, get_role_members, get_role_view};
use web_server::stamps::get_stamps;
use web_server::user::{get_current_user, get_current_user_guilds};

use std::collections::HashMap;
use tokio::sync::RwLock;

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

fn is_cors_origin_allowed(
    origin: &str,
    exact: &str,
    oauth_opener_origins: &[String],
    subdomain: Option<&(&'static str, String)>,
) -> bool {
    if origin == exact || oauth_opener_origins.iter().any(|allowed| origin == allowed) {
        return true;
    }

    subdomain.is_some_and(|(scheme, suffix)| origin.starts_with(scheme) && origin.ends_with(suffix))
}

#[actix_web::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().ok();
    env_logger::init();

    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == "media")
    {
        let telemetry_port = std::env::var("PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(8900);
        init_telemetry(telemetry_port);
        run_media_command(&arguments[1..])
            .await
            .map_err(|error| -> Box<dyn Error> { error })?;
        return Ok(());
    }

    let cfg = Config::from_env()?;
    let media_archive = MediaArchive::from_env().await?;
    init_telemetry(cfg.port);

    // Periodically delete stale per-recording HLS caches (dead after live).
    spawn_hls_reaper();

    let silence_jobs = web::Data::new(SilenceJobContainer::default());
    let waveform_progress = web::Data::new(WaveformProgressContainer(RwLock::new(HashMap::new())));
    let live_container = web::Data::new(LiveContainer::default());
    let session_mix_container = web::Data::new(SessionMixContainer::default());
    let agent_grpc_registry = web::Data::new(AgentGrpcRegistry::new(&cfg.grpc_address));

    let pool = PgPoolOptions::new()
        .max_connections(cfg.db_max_connections)
        .connect(&cfg.database_url)
        .await?;

    spawn_archive_worker(pool.clone(), media_archive.clone());
    spawn_local_cleanup(pool.clone(), media_archive.clone());

    let keys = web::Data::new(AccessKeys {
        access_encode: jsonwebtoken::EncodingKey::from_secret(cfg.access_secret.as_bytes()),
        refresh_encode: jsonwebtoken::EncodingKey::from_secret(cfg.refresh_secret.as_bytes()),
        access_decode: jsonwebtoken::DecodingKey::from_secret(cfg.access_secret.as_bytes()),
        refresh_decode: jsonwebtoken::DecodingKey::from_secret(cfg.refresh_secret.as_bytes()),
    });

    let cors_subdomain = cors_subdomain_pattern(&cfg.cors_allowed_origin);
    let cors_exact = cfg.cors_allowed_origin.clone();
    let cors_oauth_openers = cfg.oauth_allowed_opener_origins.clone();
    let host = cfg.host.clone();
    let port = cfg.port;
    let cfg_data = web::Data::new(cfg);

    let server = HttpServer::new(move || {
        let cors_exact = cors_exact.clone();
        let cors_sub = cors_subdomain.clone();
        let cors_oauth_openers = cors_oauth_openers.clone();
        let cors = Cors::default()
            .allowed_origin_fn(move |origin, _req_head| {
                let Ok(origin_str) = origin.to_str() else {
                    return false;
                };
                is_cors_origin_allowed(
                    origin_str,
                    &cors_exact,
                    &cors_oauth_openers,
                    cors_sub.as_ref(),
                )
            })
            .allow_any_method()
            .allow_any_header()
            // Media element streaming needs these readable from JS / browser
            // internals; not safelisted by default under CORS.
            .expose_headers([
                "Content-Length",
                "Content-Range",
                "Content-Disposition",
                "ETag",
                "Accept-Ranges",
                "X-CSRF-Token",
            ])
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
            .service(get_clips)
            .service(compose_clip)
            // Before get_clip: its {clip_id:.*} is greedy and would otherwise
            // match /audio/clips/{guild_id}/compose/{clip_id} too.
            .service(compose_clip_status)
            // Before get_clip: its {clip_id:.*} is greedy and would otherwise
            // match /audio/clips/waveform/... too.
            .service(get_clip_waveform_data)
            .service(rename_clip)
            .service(get_clip)
            .service(get_stamps)
            .service(play_clip)
            .service(create_clip)
            .service(get_session_manifest)
            .service(get_session_events)
            .service(get_session_channel_mix)
            .service(generate_session_channel_mix)
            .service(get_session_channel_mix_media)
            .service(get_session_waveform)
            .service(rebuild_session_waveform)
            .service(download_session)
            .service(create_session_clip)
            .service(get_session_silence_removal_status)
            .service(remove_session_silence)
            .service(get_session_silence_free)
            .service(get_session_silence_free_waveform)
            .service(rebuild_session_silence_free_waveform)
            .service(session_live_playlist)
            .service(session_live_segment)
            .service(get_session_segment)
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
            .service(delete_user_override)
            .service(get_guild_roles)
            .service(get_role_members)
            .service(get_role_view);
        let api_scope = api_scope
            .service(get_voice_settings)
            .service(put_voice_settings)
            .service(delete_voice_settings);

        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(reqwest::Client::new()))
            .app_data(web::Data::new(media_archive.clone()))
            .app_data(silence_jobs.clone())
            .app_data(waveform_progress.clone())
            .app_data(live_container.clone())
            .app_data(session_mix_container.clone())
            .app_data(agent_grpc_registry.clone())
            .app_data(keys.clone())
            .app_data(cfg_data.clone())
            .service(healthz)
            .service(api_scope)
            .service(register_agent_grpc_endpoints)
            .service(get_agent_grpc_endpoints)
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

#[cfg(test)]
mod tests {
    use super::{cors_subdomain_pattern, is_cors_origin_allowed};

    #[test]
    fn cors_allows_exact_oauth_opener_origin() {
        let exact = "https://debug.patrykstyla.com";
        let openers = vec!["https://staging.patrykstyla.com".to_string()];
        let subdomain = cors_subdomain_pattern(exact);

        assert!(is_cors_origin_allowed(
            "https://staging.patrykstyla.com",
            exact,
            &openers,
            subdomain.as_ref(),
        ));
        assert!(!is_cors_origin_allowed(
            "https://evil.example",
            exact,
            &openers,
            subdomain.as_ref(),
        ));
    }
}
