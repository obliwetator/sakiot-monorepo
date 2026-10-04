//! Load-test harness: synthetic guilds, users, recording history and live
//! recordings for the k6 suite, served under `/api/loadtest`.
//!
//! It exists only with the `loadtest` feature, which `dev-login` turns on,
//! so local, staging and preview builds have it and production builds do
//! not. Every route also requires this environment's `DEV_LOGIN_SECRET` in
//! `X-Dev-Login-Secret`, as dev login does.
//!
//! - `fixtures`: seeds one synthetic guild (roles, channels with permission
//!   overwrites, a member roster, weeks of finalized recordings with audio,
//!   clips and stamps), lists it for the load generator, and deletes it.
//! - `login`: signs in as one of a fixture's members. Tokens are Discord
//!   logins, so requests take the production authorization path (roster
//!   membership, roles, overwrites), not the dev-login shortcut.
//! - `live`: plays the recorder. It writes growing Ogg/Opus files in real
//!   time with the recorder's rows and heartbeats, so live listing, manifest
//!   polling, live HLS and realtime events behave as in a real call.
//! - `agent`: heartbeats as a bot instance that owns the fixture guilds, so
//!   presence and liveness checks see a running agent.
//!
//! Everything it creates uses ids in the synthetic range
//! (`crate::synthetic`), and it deletes only rows and files in that range.
//! SQL here is runtime-checked (`sqlx::query`) like the other seeding tools:
//! the `.sqlx` metadata is prepared without this feature.

mod agent;
mod fixtures;
mod ids;
mod live;
mod login;
mod media;
mod ogg;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use actix_web::{HttpRequest, web};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres};
use subtle::ConstantTimeEq;

use crate::config::Config;
use crate::errors::AppError;

pub struct LoadtestState {
    /// Separate from the request pool, so the harness's own writes never
    /// take connections the measured requests need.
    pool: Pool<Postgres>,
    agent_id: String,
    seeding: parking_lot::Mutex<HashMap<u32, fixtures::SeedStatus>>,
    sims: live::Sims,
}

/// Build the harness state and start the synthetic agent. Called once at
/// startup.
pub fn init(cfg: &Config) -> Result<web::Data<LoadtestState>, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect_lazy(&cfg.database_url)?;
    let state = Arc::new(LoadtestState {
        pool,
        agent_id: format!("loadtest-agent-{}", cfg.port),
        seeding: parking_lot::Mutex::new(HashMap::new()),
        sims: live::Sims::default(),
    });
    tokio::spawn(agent::run(state.clone()));
    Ok(web::Data::from(state))
}

/// The `/loadtest` routes, carrying their own state. Mount it inside `/api`.
pub fn scope(state: web::Data<LoadtestState>) -> actix_web::Scope {
    web::scope("/loadtest")
        .app_data(state)
        .service(login::login)
        .service(fixtures::create)
        .service(fixtures::catalog)
        .service(fixtures::remove)
        .service(live::start_call)
        .service(live::list_calls)
        .service(live::stop_call)
}

/// The same gate as dev login: the environment's secret, compared in
/// constant time.
fn authorize(req: &HttpRequest, cfg: &Config) -> Result<(), AppError> {
    let expected = cfg.dev_login_secret.as_deref().ok_or(AppError::Forbidden)?;
    let provided = req
        .headers()
        .get("X-Dev-Login-Secret")
        .and_then(|value| value.to_str().ok())
        .ok_or(AppError::Forbidden)?;
    if expected.as_bytes().ct_eq(provided.as_bytes()).unwrap_u8() != 1 {
        return Err(AppError::Forbidden);
    }
    Ok(())
}
