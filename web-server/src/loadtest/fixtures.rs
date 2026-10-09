//! Seed, describe and delete one synthetic guild.
//!
//! A fixture guild looks like a real one to every read path: the bot's
//! roster and role cache (`guild_members`, `roles`, `user_roles`,
//! `channel_permissions`), a complete projection owned by the synthetic
//! agent, and finalized recording sessions whose rows, events and audio
//! files match what the recorder writes. Restricted channels deny
//! `@everyone` and admit an "Insiders" role, so permission filtering has real
//! work to do.

use std::collections::HashSet;
use std::path::Path;

use actix_web::{HttpRequest, HttpResponse, delete, get, post, web};
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres};

use super::media::{self, DURATIONS_MINUTES, Sources, VARIANTS};
use super::{LoadtestState, authorize, ids};
use crate::config::Config;
use crate::errors::AppError;

mod describe;
mod insert;
mod plan;
mod purge;

use describe::*;
use insert::*;
use plan::*;
use purge::*;

const VIEW_CHANNEL: i64 = 1 << 10;
const CONNECT: i64 = 1 << 20;
const MANAGE_GUILD: i64 = 1 << 5;
/// What `@everyone` usually holds in a voice community: see and join voice,
/// talk, stream, chat.
const EVERYONE_PERMISSIONS: i64 = VIEW_CHANNEL
    | CONNECT
    | (1 << 6)
    | (1 << 9)
    | (1 << 11)
    | (1 << 16)
    | (1 << 21)
    | (1 << 25)
    | (1 << 26);
const VOICE_CHANNEL: i32 = 2;
/// The role that may see restricted channels.
const INSIDERS: u32 = 0;
/// The role holding MANAGE_GUILD.
const MODERATORS: u32 = 1;
const FUNCTIONAL_ROLES: u32 = 2;
const MODERATOR_COUNT: u32 = 3;
const FINALIZE_WRITER_CLOSE: i32 = 1;
const PENDING_GRACE_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FixtureSpec {
    members: u32,
    cosmetic_roles: u32,
    voice_channels: u32,
    restricted_channels: u32,
    history_days: u32,
    sittings_per_day: u32,
    participants_min: u32,
    participants_max: u32,
    clips_per_sitting: u32,
    stamps_per_sitting: u32,
    seed: u64,
    /// Delete an existing fixture with this number first.
    reset: bool,
}

impl Default for FixtureSpec {
    fn default() -> Self {
        Self {
            members: 150,
            cosmetic_roles: 6,
            voice_channels: 4,
            restricted_channels: 1,
            history_days: 30,
            sittings_per_day: 2,
            participants_min: 3,
            participants_max: 12,
            clips_per_sitting: 3,
            stamps_per_sitting: 4,
            seed: 1,
            reset: false,
        }
    }
}

impl FixtureSpec {
    fn validate(&self) -> Result<(), AppError> {
        let checks: [(bool, &str); 8] = [
            (
                (10..=50_000).contains(&self.members),
                "members must be 10..=50000",
            ),
            (self.cosmetic_roles <= 200, "cosmetic_roles must be <= 200"),
            (
                (1..=50).contains(&self.voice_channels),
                "voice_channels must be 1..=50",
            ),
            (
                self.restricted_channels < self.voice_channels,
                "restricted_channels must leave a public channel",
            ),
            (self.history_days <= 365, "history_days must be <= 365"),
            (
                (1..=12).contains(&self.sittings_per_day),
                "sittings_per_day must be 1..=12",
            ),
            (
                (1..=100).contains(&self.participants_min)
                    && self.participants_min <= self.participants_max
                    && self.participants_max <= 100,
                "participants must satisfy 1 <= min <= max <= 100",
            ),
            (
                self.clips_per_sitting <= 50 && self.stamps_per_sitting <= 50,
                "clips_per_sitting and stamps_per_sitting must be <= 50",
            ),
        ];
        match checks.iter().find(|(ok, _)| !ok) {
            Some((_, message)) => Err(AppError::BadRequest((*message).into())),
            None => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(super) enum SeedStatus {
    Seeding { started_at_ms: i64 },
    Failed { error: String },
}

fn fixture_number(guild: u32) -> Result<u32, AppError> {
    if (1..=ids::MAX_GUILD).contains(&guild) {
        Ok(guild)
    } else {
        Err(AppError::InvalidParam("fixture".into()))
    }
}

/// Seed fixture guild `{fixture}` in the background (it can take a minute
/// the first time, while the audio sources are generated). Poll the catalog
/// until it reports `ready`.
#[post("/fixtures/{fixture}")]
pub async fn create(
    req: HttpRequest,
    path: web::Path<u32>,
    spec: Option<web::Json<FixtureSpec>>,
    cfg: web::Data<Config>,
    state: web::Data<LoadtestState>,
) -> Result<HttpResponse, AppError> {
    authorize(&req, &cfg)?;
    let fixture = fixture_number(path.into_inner())?;
    let spec = spec.map(web::Json::into_inner).unwrap_or_default();
    spec.validate()?;

    {
        let mut seeding = state.seeding.lock();
        if matches!(seeding.get(&fixture), Some(SeedStatus::Seeding { .. })) {
            return Err(AppError::Conflict(
                "this fixture is already being seeded".into(),
            ));
        }
        seeding.insert(
            fixture,
            SeedStatus::Seeding {
                started_at_ms: Utc::now().timestamp_millis(),
            },
        );
    }

    let state = state.into_inner();
    tokio::spawn(async move {
        let result = seed(&state, fixture, &spec).await;
        let mut seeding = state.seeding.lock();
        match result {
            Ok(()) => {
                seeding.remove(&fixture);
            }
            Err(error) => {
                tracing::error!(?error, fixture, "load-test fixture seeding failed");
                seeding.insert(
                    fixture,
                    SeedStatus::Failed {
                        error: error.to_string(),
                    },
                );
            }
        }
    });
    Ok(HttpResponse::Accepted().json(serde_json::json!({ "state": "seeding" })))
}

/// Everything a load generator needs to address the fixture: ids of the
/// guild, channels, members by role, sessions with their files, clips and
/// running live recordings.
#[get("/fixtures/{fixture}")]
pub async fn catalog(
    req: HttpRequest,
    path: web::Path<u32>,
    cfg: web::Data<Config>,
    state: web::Data<LoadtestState>,
) -> Result<HttpResponse, AppError> {
    authorize(&req, &cfg)?;
    let fixture = fixture_number(path.into_inner())?;
    if let Some(status) = state.seeding.lock().get(&fixture).cloned() {
        return Ok(HttpResponse::Ok().json(status));
    }
    match describe(&state, fixture).await? {
        Some(catalog) => Ok(HttpResponse::Ok().json(catalog)),
        None => Err(AppError::NotFound),
    }
}

/// Delete the fixture guild: its live recordings, every row in its id slot
/// and every file derived from them.
#[delete("/fixtures/{fixture}")]
pub async fn remove(
    req: HttpRequest,
    path: web::Path<u32>,
    cfg: web::Data<Config>,
    state: web::Data<LoadtestState>,
) -> Result<HttpResponse, AppError> {
    authorize(&req, &cfg)?;
    let fixture = fixture_number(path.into_inner())?;
    if matches!(
        state.seeding.lock().get(&fixture),
        Some(SeedStatus::Seeding { .. })
    ) {
        return Err(AppError::Conflict("this fixture is being seeded".into()));
    }
    let report = teardown(&state, fixture).await?;
    state.seeding.lock().remove(&fixture);
    Ok(HttpResponse::Ok().json(report))
}

/// The guild ids of every seeded fixture, for the agent.
pub(super) async fn fixture_guilds(pool: &Pool<Postgres>) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM guilds WHERE id >= $1 ORDER BY id")
        .bind(crate::synthetic::SYNTHETIC_ID_FLOOR)
        .fetch_all(pool)
        .await
}
