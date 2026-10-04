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

// ---- seeding ---------------------------------------------------------------

struct Member {
    id: i64,
    insider: bool,
}

struct Fragment {
    session_id: i64,
    audio_file_id: i64,
    user_id: i64,
    channel_id: i64,
    start_ms: i64,
    end_ms: i64,
    variant: u8,
    minutes: u32,
}

struct Clip {
    clip_id: String,
    fragment: usize,
    user_id: i64,
    offset_s: f32,
    created_ms: i64,
    variant: u8,
}

struct Stamp {
    fragment: usize,
    stamper: i64,
    at_ms: i64,
}

async fn seed(state: &LoadtestState, fixture: u32, spec: &FixtureSpec) -> Result<(), AppError> {
    let guild_id = ids::guild_id(fixture);
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM guilds WHERE id = $1)")
        .bind(guild_id)
        .fetch_one(&state.pool)
        .await?;
    if exists {
        if !spec.reset {
            return Ok(());
        }
        teardown(state, fixture).await?;
    }

    let sources = media::ensure_sources().await?;
    let mut rng = fastrand::Rng::with_seed(spec.seed ^ u64::from(fixture));

    let members: Vec<Member> = (0..spec.members)
        .map(|index| Member {
            id: ids::user_id(guild_id, index),
            insider: index % 3 == 0,
        })
        .collect();
    let channels: Vec<i64> = (0..spec.voice_channels)
        .map(|index| ids::channel_id(guild_id, index))
        .collect();
    let public = (spec.voice_channels - spec.restricted_channels) as usize;

    // Plan the history before allocating ids, so each table is one insert.
    let now = Utc::now();
    let today = Utc
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .ok_or(AppError::InternalError)?;
    let mut fragments = Vec::new();
    let mut sittings: Vec<std::ops::Range<usize>> = Vec::new();
    let evening_ms = 12 * 3_600_000 / i64::from(spec.sittings_per_day);
    for day in 1..=i64::from(spec.history_days) {
        for sitting in 0..i64::from(spec.sittings_per_day) {
            let start = today - Duration::days(day)
                + Duration::hours(8)
                + Duration::milliseconds(sitting * evening_ms + rng.i64(0..1_800_000));
            let restricted = public < channels.len() && rng.u8(0..100) < 15;
            let channel_id = if restricted {
                channels[rng.usize(public..channels.len())]
            } else {
                channels[rng.usize(0..public)]
            };
            let pool: Vec<i64> = members
                .iter()
                .filter(|member| !restricted || member.insider)
                .map(|member| member.id)
                .collect();
            let wanted = rng.u32(spec.participants_min..=spec.participants_max) as usize;
            let mut chosen = pool;
            rng.shuffle(&mut chosen);
            chosen.truncate(wanted.max(1));

            let first = fragments.len();
            for user_id in chosen {
                let minutes = pick_minutes(&mut rng);
                let variant = rng.u8(0..VARIANTS);
                let source = sources
                    .recording(variant, minutes)
                    .ok_or(AppError::InternalError)?;
                let start_ms = start.timestamp_millis() + rng.i64(0..180_000);
                fragments.push(Fragment {
                    session_id: 0,
                    audio_file_id: 0,
                    user_id,
                    channel_id,
                    start_ms,
                    end_ms: start_ms + source.duration_ms,
                    variant,
                    minutes,
                });
            }
            sittings.push(first..fragments.len());
        }
    }

    let mut clips = Vec::new();
    let mut stamps = Vec::new();
    for range in &sittings {
        let in_sitting: Vec<usize> = range.clone().collect();
        if in_sitting.is_empty() {
            continue;
        }
        for _ in 0..rng.u32(0..=spec.clips_per_sitting) {
            let fragment = in_sitting[rng.usize(0..in_sitting.len())];
            let clipper = fragments[in_sitting[rng.usize(0..in_sitting.len())]].user_id;
            let span_s =
                ((fragments[fragment].end_ms - fragments[fragment].start_ms) / 1000).max(30);
            let offset_s = rng.i64(0..span_s - 20) as f32;
            clips.push(Clip {
                clip_id: uuid::Uuid::new_v4().to_string(),
                fragment,
                user_id: clipper,
                offset_s,
                created_ms: fragments[fragment].start_ms + (offset_s as i64 + 30) * 1000,
                variant: rng.u8(0..VARIANTS),
            });
        }
        for _ in 0..rng.u32(0..=spec.stamps_per_sitting) {
            let fragment = in_sitting[rng.usize(0..in_sitting.len())];
            let stamper = fragments[in_sitting[rng.usize(0..in_sitting.len())]].user_id;
            let f = &fragments[fragment];
            stamps.push(Stamp {
                fragment,
                stamper,
                at_ms: rng.i64(f.start_ms..f.end_ms),
            });
        }
    }

    let session_ids = allocate(&state.pool, "recording_sessions_id_seq", fragments.len()).await?;
    let audio_ids = allocate(&state.pool, "audio_files_id_seq", fragments.len()).await?;
    for (index, fragment) in fragments.iter_mut().enumerate() {
        fragment.session_id = session_ids[index];
        fragment.audio_file_id = audio_ids[index];
    }

    link_media(&fragments, &clips, &sources, guild_id).await?;
    insert_all(
        state, spec, &sources, guild_id, &members, &channels, public, &fragments, &clips, &stamps,
    )
    .await?;
    tracing::info!(
        fixture,
        guild_id,
        sessions = fragments.len(),
        clips = clips.len(),
        stamps = stamps.len(),
        "load-test fixture seeded"
    );
    Ok(())
}

/// Recording lengths weighted towards an hour or so, as evening calls run.
fn pick_minutes(rng: &mut fastrand::Rng) -> u32 {
    const WEIGHTS: [u32; 6] = [1, 2, 3, 4, 3, 2];
    let total: u32 = WEIGHTS.iter().sum();
    let mut roll = rng.u32(0..total);
    for (minutes, weight) in DURATIONS_MINUTES.iter().zip(WEIGHTS) {
        if roll < weight {
            return *minutes;
        }
        roll -= weight;
    }
    DURATIONS_MINUTES[0]
}

async fn allocate(
    pool: &Pool<Postgres>,
    sequence: &str,
    count: usize,
) -> Result<Vec<i64>, AppError> {
    Ok(
        sqlx::query_scalar("SELECT nextval($1::regclass) FROM generate_series(1, $2)")
            .bind(sequence)
            .bind(count as i64)
            .fetch_all(pool)
            .await?,
    )
}

fn utc(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(Utc::now)
}

fn recording_key(guild_id: i64, fragment: &Fragment) -> sakiot_paths::RecordingKey {
    let start = utc(fragment.start_ms);
    sakiot_paths::RecordingKey::new(
        guild_id,
        fragment.channel_id,
        start.year(),
        start.month(),
        sakiot_paths::RecordingKey::stem_for(fragment.start_ms, fragment.user_id),
    )
}

fn clip_saved_name(clip: &Clip) -> String {
    let created = utc(clip.created_ms);
    format!(
        "{:04}/{:02}/{}.ogg",
        created.year(),
        created.month(),
        clip.clip_id
    )
}

async fn link_media(
    fragments: &[Fragment],
    clips: &[Clip],
    sources: &Sources,
    guild_id: i64,
) -> Result<(), AppError> {
    let roots = sakiot_paths::DataRoots::from_env();
    let mut links = Vec::with_capacity(fragments.len() + clips.len());
    for fragment in fragments {
        let source = sources
            .recording(fragment.variant, fragment.minutes)
            .ok_or(AppError::InternalError)?;
        let dest = recording_key(guild_id, fragment).recording_path(&roots.recordings_str());
        links.push((source.path.clone(), dest));
    }
    for clip in clips {
        let source = sources.clip(clip.variant).ok_or(AppError::InternalError)?;
        links.push((source.path.clone(), roots.clips.join(clip_saved_name(clip))));
    }
    tokio::task::spawn_blocking(move || {
        links
            .iter()
            .try_for_each(|(source, dest)| media::link(source, dest))
    })
    .await
    .map_err(|_| AppError::InternalError)??;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_all(
    state: &LoadtestState,
    spec: &FixtureSpec,
    sources: &Sources,
    guild_id: i64,
    members: &[Member],
    channels: &[i64],
    public: usize,
    fragments: &[Fragment],
    clips: &[Clip],
    stamps: &[Stamp],
) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let owner = members
        .first()
        .map(|member| member.id)
        .ok_or(AppError::InternalError)?;

    super::agent::register(&mut tx, &state.agent_id).await?;
    sqlx::query("INSERT INTO guilds (id, owner_id, name, icon) VALUES ($1, $2, $3, NULL)")
        .bind(guild_id)
        .bind(owner)
        .bind(format!(
            "Load test {}",
            ids::guild_of(guild_id).unwrap_or(0)
        ))
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO guild_projection_state
            (guild_id, owner_instance_id, generation, roster_complete_at, presence_synced_at)
         VALUES ($1, $2, 1, now(), now())",
    )
    .bind(guild_id)
    .bind(&state.agent_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO guilds_present (guild_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(guild_id)
        .execute(&mut *tx)
        .await?;

    // Roles: @everyone (id = guild id), Insiders, Moderators, then cosmetic.
    let mut role_ids = vec![guild_id];
    let mut role_names = vec!["@everyone".to_string()];
    let mut role_perms = vec![EVERYONE_PERMISSIONS];
    for index in 0..FUNCTIONAL_ROLES + spec.cosmetic_roles {
        role_ids.push(ids::role_id(guild_id, index));
        role_names.push(match index {
            INSIDERS => "Insiders".to_string(),
            MODERATORS => "Moderators".to_string(),
            _ => format!("Role {index}"),
        });
        role_perms.push(if index == MODERATORS { MANAGE_GUILD } else { 0 });
    }
    sqlx::query(
        "INSERT INTO roles (guild_id, role_id, permission, name, color)
         SELECT $1, * FROM UNNEST($2::bigint[], $3::bigint[], $4::text[], $5::bigint[])",
    )
    .bind(guild_id)
    .bind(&role_ids)
    .bind(&role_perms)
    .bind(&role_names)
    .bind(
        role_ids
            .iter()
            .map(|id| id % 0xFF_FFFF)
            .collect::<Vec<i64>>(),
    )
    .execute(&mut *tx)
    .await?;

    let channel_names: Vec<String> = (0..channels.len())
        .map(|index| {
            if index < public {
                format!("voice-{}", index + 1)
            } else {
                format!("insiders-{}", index + 1 - public)
            }
        })
        .collect();
    sqlx::query(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         SELECT channel_id, $1, $2, name FROM UNNEST($3::bigint[], $4::text[]) AS c(channel_id, name)",
    )
    .bind(guild_id)
    .bind(VOICE_CHANNEL)
    .bind(channels)
    .bind(&channel_names)
    .execute(&mut *tx)
    .await?;
    let restricted: Vec<i64> = channels[public..].to_vec();
    sqlx::query(
        "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny)
         SELECT channel_id, $2, 'role', 0, $3 FROM UNNEST($1::bigint[]) AS c(channel_id)
         UNION ALL
         SELECT channel_id, $4, 'role', $5, 0 FROM UNNEST($1::bigint[]) AS c(channel_id)",
    )
    .bind(&restricted)
    .bind(guild_id)
    .bind(VIEW_CHANNEL)
    .bind(ids::role_id(guild_id, INSIDERS))
    .bind(VIEW_CHANNEL | CONNECT)
    .execute(&mut *tx)
    .await?;

    let member_ids: Vec<i64> = members.iter().map(|member| member.id).collect();
    let usernames: Vec<String> = member_ids
        .iter()
        .enumerate()
        .map(|(index, _)| format!("lt-user-{index}"))
        .collect();
    sqlx::query(
        "INSERT INTO guild_members (guild_id, user_id, username, global_name)
         SELECT $1, user_id, username, 'Load Tester ' || username
           FROM UNNEST($2::bigint[], $3::text[]) AS m(user_id, username)",
    )
    .bind(guild_id)
    .bind(&member_ids)
    .bind(&usernames)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO user_names (user_id, username, global_name)
         SELECT user_id, username, 'Load Tester ' || username
           FROM UNNEST($1::bigint[], $2::text[]) AS m(user_id, username)
         ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(&member_ids)
    .bind(&usernames)
    .execute(&mut *tx)
    .await?;
    // The agent records names as it first sees them, before their
    // recordings; listings resolve names from this history. The kinds are
    // the agent's `UserNameEventType`; a database copied from staging has
    // them already, a fresh one does not.
    sqlx::query(
        "INSERT INTO user_name_event_types (id, name)
         VALUES (1, 'username'), (2, 'global_name'), (3, 'nickname')
         ON CONFLICT DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO user_name_history (user_id, guild_id, kind_id, value, observed_at)
         SELECT user_id, NULL, kind.id, CASE kind.id WHEN 1 THEN username ELSE 'Load Tester ' || username END,
                now() - make_interval(days => $3 + 1)
           FROM UNNEST($1::bigint[], $2::text[]) AS m(user_id, username)
          CROSS JOIN (VALUES (1), (2)) AS kind(id)",
    )
    .bind(&member_ids)
    .bind(&usernames)
    .bind(i32::try_from(spec.history_days).map_err(|_| AppError::InternalError)?)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO discord_auth_user (id, username, avatar)
         SELECT id, username, '' FROM UNNEST($1::bigint[], $2::text[]) AS m(id, username)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(&member_ids)
    .bind(&usernames)
    .execute(&mut *tx)
    .await?;

    let mut rng = fastrand::Rng::with_seed(spec.seed.wrapping_add(17));
    let mut grant_users = Vec::new();
    let mut grant_roles = Vec::new();
    for (index, member) in members.iter().enumerate() {
        let mut roles = Vec::new();
        if member.insider {
            roles.push(ids::role_id(guild_id, INSIDERS));
        }
        if (1..=MODERATOR_COUNT as usize).contains(&index) {
            roles.push(ids::role_id(guild_id, MODERATORS));
        }
        if spec.cosmetic_roles > 0 {
            for _ in 0..rng.u32(0..=2) {
                let role =
                    ids::role_id(guild_id, FUNCTIONAL_ROLES + rng.u32(0..spec.cosmetic_roles));
                if !roles.contains(&role) {
                    roles.push(role);
                }
            }
        }
        for role in roles {
            grant_users.push(member.id);
            grant_roles.push(role);
        }
    }
    sqlx::query("INSERT INTO user_roles (user_id, role_id) SELECT * FROM UNNEST($1::bigint[], $2::bigint[])")
        .bind(&grant_users)
        .bind(&grant_roles)
        .execute(&mut *tx)
        .await?;

    insert_history(&mut tx, sources, guild_id, fragments, clips, stamps).await?;
    tx.commit().await?;
    Ok(())
}

async fn insert_history(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    sources: &Sources,
    guild_id: i64,
    fragments: &[Fragment],
    clips: &[Clip],
    stamps: &[Stamp],
) -> Result<(), AppError> {
    let session_ids: Vec<i64> = fragments.iter().map(|f| f.session_id).collect();
    let audio_ids: Vec<i64> = fragments.iter().map(|f| f.audio_file_id).collect();
    let users: Vec<i64> = fragments.iter().map(|f| f.user_id).collect();
    let channels: Vec<i64> = fragments.iter().map(|f| f.channel_id).collect();
    let starts: Vec<i64> = fragments.iter().map(|f| f.start_ms).collect();
    let ends: Vec<i64> = fragments.iter().map(|f| f.end_ms).collect();
    let keys: Vec<sakiot_paths::RecordingKey> = fragments
        .iter()
        .map(|f| recording_key(guild_id, f))
        .collect();
    let stems: Vec<String> = keys.iter().map(|key| key.stem.clone()).collect();
    let years: Vec<i32> = keys.iter().map(|key| key.year).collect();
    let months: Vec<i32> = keys.iter().map(|key| key.month as i32).collect();

    // A finished sitting as the recorder leaves it: the member left, the
    // grace period ran out, and the session finalized at the leave time.
    sqlx::query(
        "INSERT INTO recording_sessions
            (id, guild_id, user_id, starting_channel_id, current_channel_id, state,
             started_at, ended_at, pause_started_at, end_reason, last_segment_index)
         SELECT id, $1, user_id, channel_id, channel_id, 'finalized',
                to_timestamp(start_ms / 1000.0), to_timestamp(end_ms / 1000.0),
                to_timestamp(end_ms / 1000.0), 'pending_grace_expired', 0
           FROM UNNEST($2::bigint[], $3::bigint[], $4::bigint[], $5::bigint[], $6::bigint[])
             AS s(id, user_id, channel_id, start_ms, end_ms)",
    )
    .bind(guild_id)
    .bind(&session_ids)
    .bind(&users)
    .bind(&channels)
    .bind(&starts)
    .bind(&ends)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "INSERT INTO audio_files
            (id, file_name, guild_id, channel_id, user_id, year, month, start_ts, end_ts,
             recording_session_id, segment_index, finalize_reason_id)
         SELECT id, file_name, $1, channel_id, user_id, year, month, start_ms, end_ms,
                session_id, 0, $2
           FROM UNNEST($3::bigint[], $4::text[], $5::bigint[], $6::bigint[], $7::int[],
                       $8::int[], $9::bigint[], $10::bigint[], $11::bigint[])
             AS a(id, file_name, channel_id, user_id, year, month, start_ms, end_ms, session_id)",
    )
    .bind(guild_id)
    .bind(FINALIZE_WRITER_CLOSE)
    .bind(&audio_ids)
    .bind(&stems)
    .bind(&channels)
    .bind(&users)
    .bind(&years)
    .bind(&months)
    .bind(&starts)
    .bind(&ends)
    .bind(&session_ids)
    .execute(&mut **tx)
    .await?;

    let mut event_sessions = Vec::new();
    let mut event_at = Vec::new();
    let mut event_types = Vec::new();
    let mut event_channels = Vec::new();
    let mut event_details = Vec::new();
    for (index, f) in fragments.iter().enumerate() {
        let opened = serde_json::json!({
            "audio_file_id": f.audio_file_id, "segment_index": 0, "file_name": stems[index],
        });
        let closed = serde_json::json!({
            "audio_file_id": f.audio_file_id, "segment_index": 0, "reason": "writer_close",
        });
        let left = serde_json::json!({
            "reason": "disconnect", "pending_deadline_ms": f.end_ms + PENDING_GRACE_MS,
            "absolute_cap_deadline_ms": null,
        });
        let expired = serde_json::json!({
            "ended_at_ms": f.end_ms, "end_reason": "pending_grace_expired",
        });
        for (at, kind, details) in [
            (f.start_ms, "session_start", serde_json::json!({})),
            (f.start_ms, "fragment_open", opened),
            (f.end_ms, "fragment_close", closed),
            (f.end_ms, "disconnect", left),
            (f.end_ms + PENDING_GRACE_MS, "timeout", expired),
        ] {
            event_sessions.push(f.session_id);
            event_at.push(at);
            event_types.push(kind.to_string());
            event_channels.push(f.channel_id);
            event_details.push(details.to_string());
        }
    }
    sqlx::query(
        "INSERT INTO recording_session_events
            (recording_session_id, occurred_at, event_type, channel_id, details)
         SELECT session_id, to_timestamp(at_ms / 1000.0), event_type, channel_id, details::jsonb
           FROM UNNEST($1::bigint[], $2::bigint[], $3::text[], $4::bigint[], $5::text[])
             AS e(session_id, at_ms, event_type, channel_id, details)",
    )
    .bind(&event_sessions)
    .bind(&event_at)
    .bind(&event_types)
    .bind(&event_channels)
    .bind(&event_details)
    .execute(&mut **tx)
    .await?;

    if !clips.is_empty() {
        let mut clip_ids = Vec::new();
        let mut lengths = Vec::new();
        let mut sizes = Vec::new();
        let mut clip_channels = Vec::new();
        let mut clip_users = Vec::new();
        let mut originals = Vec::new();
        let mut saved = Vec::new();
        let mut created = Vec::new();
        let mut names = Vec::new();
        let mut offsets = Vec::new();
        let mut clip_sessions = Vec::new();
        for (index, clip) in clips.iter().enumerate() {
            let source = sources.clip(clip.variant).ok_or(AppError::InternalError)?;
            let f = &fragments[clip.fragment];
            clip_ids.push(clip.clip_id.clone());
            lengths.push(source.duration_ms as f32 / 1000.0);
            sizes.push(source.bytes as i64);
            clip_channels.push(f.channel_id);
            clip_users.push(clip.user_id);
            originals.push(stems[clip.fragment].clone());
            saved.push(clip_saved_name(clip));
            created.push(clip.created_ms);
            names.push(format!("load test clip {}", index + 1));
            offsets.push(clip.offset_s);
            clip_sessions.push(f.session_id);
        }
        sqlx::query(
            "INSERT INTO clips
                (clip_id, length, size, channel_id, guild_id, user_id, original_file_name,
                 saved_file_name, created_at, name, start_time, recording_session_id)
             SELECT clip_id, length, size, channel_id, $1, user_id, original, saved,
                    to_timestamp(created_ms / 1000.0), name, start_time, session_id
               FROM UNNEST($2::text[], $3::real[], $4::bigint[], $5::bigint[], $6::bigint[],
                           $7::text[], $8::text[], $9::bigint[], $10::text[], $11::real[],
                           $12::bigint[])
                 AS c(clip_id, length, size, channel_id, user_id, original, saved,
                      created_ms, name, start_time, session_id)",
        )
        .bind(guild_id)
        .bind(&clip_ids)
        .bind(&lengths)
        .bind(&sizes)
        .bind(&clip_channels)
        .bind(&clip_users)
        .bind(&originals)
        .bind(&saved)
        .bind(&created)
        .bind(&names)
        .bind(&offsets)
        .bind(&clip_sessions)
        .execute(&mut **tx)
        .await?;
    }

    if !stamps.is_empty() {
        let f = |stamp: &Stamp| &fragments[stamp.fragment];
        sqlx::query(
            "INSERT INTO stamps
                (guild_id, channel_id, target_user_id, stamper_user_id, stamp_ts, offset_ms,
                 audio_file_id, recording_session_id)
             SELECT $1, channel_id, target, stamper, stamp_ts, 0, audio_file_id, session_id
               FROM UNNEST($2::bigint[], $3::bigint[], $4::bigint[], $5::bigint[],
                           $6::bigint[], $7::bigint[])
                 AS s(channel_id, target, stamper, stamp_ts, audio_file_id, session_id)",
        )
        .bind(guild_id)
        .bind(stamps.iter().map(|s| f(s).channel_id).collect::<Vec<_>>())
        .bind(stamps.iter().map(|s| f(s).user_id).collect::<Vec<_>>())
        .bind(stamps.iter().map(|s| s.stamper).collect::<Vec<_>>())
        .bind(stamps.iter().map(|s| s.at_ms).collect::<Vec<_>>())
        .bind(
            stamps
                .iter()
                .map(|s| f(s).audio_file_id)
                .collect::<Vec<_>>(),
        )
        .bind(stamps.iter().map(|s| f(s).session_id).collect::<Vec<_>>())
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

// ---- catalog ---------------------------------------------------------------

#[derive(Serialize)]
struct Catalog {
    state: &'static str,
    fixture: u32,
    guild_id: String,
    owner_id: String,
    channels: Vec<ChannelEntry>,
    /// Every member, insiders and moderators included.
    members: Vec<String>,
    insiders: Vec<String>,
    moderators: Vec<String>,
    sessions: Vec<SessionEntry>,
    clips: Vec<ClipEntry>,
    live: Vec<super::live::SimSummary>,
}

#[derive(Serialize)]
struct ChannelEntry {
    id: String,
    name: String,
    restricted: bool,
}

#[derive(Serialize)]
struct SessionEntry {
    id: String,
    user_id: String,
    channel_id: String,
    state: String,
    audio_file_id: String,
    file_name: String,
    year: i32,
    month: i32,
    start_ms: i64,
    end_ms: Option<i64>,
    restricted: bool,
}

#[derive(Serialize)]
struct ClipEntry {
    clip_id: String,
    channel_id: String,
    recording_session_id: Option<String>,
    restricted: bool,
}

async fn describe(state: &LoadtestState, fixture: u32) -> Result<Option<Catalog>, AppError> {
    let guild_id = ids::guild_id(fixture);
    let owner: Option<i64> = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = $1")
        .bind(guild_id)
        .fetch_optional(&state.pool)
        .await?;
    let Some(owner) = owner else {
        return Ok(None);
    };

    let channel_rows: Vec<(i64, Option<String>, bool)> = sqlx::query_as(
        "SELECT c.channel_id, c.name,
                EXISTS (SELECT 1 FROM channel_permissions p
                         WHERE p.channel_id = c.channel_id AND p.target_id = c.guild_id
                           AND p.kind = 'role' AND (p.deny & $2) <> 0)
           FROM channels c WHERE c.guild_id = $1 ORDER BY c.channel_id",
    )
    .bind(guild_id)
    .bind(VIEW_CHANNEL)
    .fetch_all(&state.pool)
    .await?;
    let restricted: HashSet<i64> = channel_rows
        .iter()
        .filter(|(_, _, restricted)| *restricted)
        .map(|(id, _, _)| *id)
        .collect();

    let members: Vec<i64> = sqlx::query_scalar(
        "SELECT user_id FROM guild_members WHERE guild_id = $1 ORDER BY user_id",
    )
    .bind(guild_id)
    .fetch_all(&state.pool)
    .await?;
    let with_role = |role: i64| {
        sqlx::query_scalar::<_, i64>(
            "SELECT user_id FROM user_roles WHERE role_id = $1 ORDER BY user_id",
        )
        .bind(role)
        .fetch_all(&state.pool)
    };
    let insiders = with_role(ids::role_id(guild_id, INSIDERS)).await?;
    let moderators = with_role(ids::role_id(guild_id, MODERATORS)).await?;

    type SessionRow = (
        i64,
        i64,
        String,
        i64,
        String,
        i64,
        i32,
        i32,
        Option<i64>,
        Option<i64>,
    );
    let session_rows: Vec<SessionRow> = sqlx::query_as(
        "SELECT rs.id, rs.user_id, rs.state, af.id, af.file_name, af.channel_id, af.year,
                af.month, af.start_ts, af.end_ts
           FROM recording_sessions rs
           JOIN audio_files af ON af.recording_session_id = rs.id
          WHERE rs.guild_id = $1 AND rs.deletion_requested_at IS NULL AND af.reaped IS FALSE
          ORDER BY rs.started_at DESC, af.segment_index",
    )
    .bind(guild_id)
    .fetch_all(&state.pool)
    .await?;
    let sessions = session_rows
        .into_iter()
        .map(
            |(
                id,
                user_id,
                state,
                audio_file_id,
                file_name,
                channel_id,
                year,
                month,
                start,
                end,
            )| {
                SessionEntry {
                    id: id.to_string(),
                    user_id: user_id.to_string(),
                    channel_id: channel_id.to_string(),
                    state,
                    audio_file_id: audio_file_id.to_string(),
                    file_name,
                    year,
                    month,
                    start_ms: start.unwrap_or_default(),
                    end_ms: end,
                    restricted: restricted.contains(&channel_id),
                }
            },
        )
        .collect();

    let clip_rows: Vec<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT clip_id, channel_id, recording_session_id FROM clips
          WHERE guild_id = $1 AND deleted_at IS NULL ORDER BY created_at DESC",
    )
    .bind(guild_id)
    .fetch_all(&state.pool)
    .await?;
    let clips = clip_rows
        .into_iter()
        .map(|(clip_id, channel_id, session_id)| ClipEntry {
            clip_id,
            channel_id: channel_id.unwrap_or_default().to_string(),
            recording_session_id: session_id.map(|id| id.to_string()),
            restricted: channel_id.is_some_and(|id| restricted.contains(&id)),
        })
        .collect();

    let strings = |ids: Vec<i64>| ids.into_iter().map(|id| id.to_string()).collect();
    Ok(Some(Catalog {
        state: "ready",
        fixture,
        guild_id: guild_id.to_string(),
        owner_id: owner.to_string(),
        channels: channel_rows
            .into_iter()
            .map(|(id, name, restricted)| ChannelEntry {
                id: id.to_string(),
                name: name.unwrap_or_default(),
                restricted,
            })
            .collect(),
        members: strings(members),
        insiders: strings(insiders),
        moderators: strings(moderators),
        sessions,
        clips,
        live: state.sims.summaries(guild_id),
    }))
}

// ---- teardown --------------------------------------------------------------

#[derive(Serialize, Default)]
struct TeardownReport {
    guild_id: String,
    rows: u64,
    files: u64,
}

async fn teardown(state: &LoadtestState, fixture: u32) -> Result<TeardownReport, AppError> {
    let guild_id = ids::guild_id(fixture);
    let (low, high) = ids::slot(guild_id);
    state.sims.stop_guild(guild_id).await;

    // Collect what names the derived files before the rows go.
    let stems: Vec<String> =
        sqlx::query_scalar("SELECT file_name FROM audio_files WHERE guild_id = $1")
            .bind(guild_id)
            .fetch_all(&state.pool)
            .await?;
    let session_ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM recording_sessions WHERE guild_id = $1")
            .bind(guild_id)
            .fetch_all(&state.pool)
            .await?;
    let clip_rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT clip_id, saved_file_name FROM clips WHERE guild_id = $1")
            .bind(guild_id)
            .fetch_all(&state.pool)
            .await?;
    let media_jobs: Vec<String> =
        sqlx::query_scalar("SELECT id FROM media_jobs WHERE guild_id = $1")
            .bind(guild_id)
            .fetch_all(&state.pool)
            .await?;
    let composition_jobs: Vec<String> =
        sqlx::query_scalar("SELECT id FROM composition_jobs WHERE guild_id = $1")
            .bind(guild_id)
            .fetch_all(&state.pool)
            .await?;

    let mut tx = state.pool.begin().await?;
    let mut rows = 0;
    let statements: [&str; 26] = [
        "DELETE FROM media_objects WHERE audio_file_id IN (SELECT id FROM audio_files WHERE guild_id = $1)
            OR clip_id IN (SELECT clip_id FROM clips WHERE guild_id = $1)",
        "DELETE FROM stamps WHERE guild_id = $1",
        "DELETE FROM clip_source_history WHERE target_clip_id IN (SELECT clip_id FROM clips WHERE guild_id = $1)
            OR source_clip_id IN (SELECT clip_id FROM clips WHERE guild_id = $1)",
        "DELETE FROM clips WHERE guild_id = $1",
        "DELETE FROM media_jobs WHERE guild_id = $1",
        "DELETE FROM composition_jobs WHERE guild_id = $1",
        "DELETE FROM recording_deletion_jobs WHERE guild_id = $1",
        "DELETE FROM audio_files WHERE guild_id = $1",
        "DELETE FROM recording_sessions WHERE guild_id = $1",
        "DELETE FROM voice_state_events WHERE guild_id = $1",
        "DELETE FROM voice_events WHERE guild_id = $1",
        "DELETE FROM voice_connection_events WHERE guild_id = $1",
        "DELETE FROM voice_session_leases WHERE guild_id = $1",
        "DELETE FROM jam_invocations WHERE guild_id = $1",
        "DELETE FROM user_jam_cooldown_overrides WHERE guild_id = $1",
        "DELETE FROM guild_jam_cooldowns WHERE guild_id = $1",
        "DELETE FROM guild_voice_settings WHERE guild_id = $1",
        "DELETE FROM guild_recording_policy WHERE guild_id = $1",
        "DELETE FROM recording_opt_outs WHERE guild_id = $1",
        "DELETE FROM user_guilds WHERE id = $1",
        "DELETE FROM user_nicknames WHERE guild_id = $1",
        "DELETE FROM user_name_history WHERE guild_id = $1",
        "DELETE FROM user_roles WHERE role_id IN (SELECT role_id FROM roles WHERE guild_id = $1)",
        // Cascades to guild_members and voice_presence.
        "DELETE FROM guild_projection_state WHERE guild_id = $1",
        "DELETE FROM guilds_present WHERE guild_id = $1",
        // Cascades to roles, channels and channel_permissions.
        "DELETE FROM guilds WHERE id = $1",
    ];
    for statement in statements {
        rows += sqlx::query(statement)
            .bind(guild_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    for statement in [
        "DELETE FROM user_name_history WHERE user_id >= $1 AND user_id < $2",
        "DELETE FROM user_names WHERE user_id >= $1 AND user_id < $2",
        "DELETE FROM discord_auth_user WHERE id >= $1 AND id < $2",
    ] {
        rows += sqlx::query(statement)
            .bind(low)
            .bind(high)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    tx.commit().await?;

    let files = purge_files(
        guild_id,
        &stems,
        &session_ids,
        &clip_rows,
        &media_jobs,
        &composition_jobs,
    )
    .await?;
    tracing::info!(fixture, guild_id, rows, files, "load-test fixture deleted");
    Ok(TeardownReport {
        guild_id: guild_id.to_string(),
        rows,
        files,
    })
}

/// Remove every file the fixture and the load against it produced: the
/// guild's recording and silence-free trees (originals, live HLS and mix
/// caches), waveforms, logical-session renders, clips, and job outputs.
async fn purge_files(
    guild_id: i64,
    stems: &[String],
    session_ids: &[i64],
    clips: &[(String, Option<String>)],
    media_jobs: &[String],
    composition_jobs: &[String],
) -> Result<u64, AppError> {
    let roots = sakiot_paths::DataRoots::from_env();
    let guild = guild_id.to_string();
    let stems: HashSet<String> = stems.iter().cloned().collect();
    let session_prefixes: Vec<String> = session_ids
        .iter()
        .flat_map(|id| {
            [
                format!("logical-session-{id}-"),
                format!("logical-session-{id}."),
            ]
        })
        .collect();
    let clip_prefixes: Vec<String> = clips.iter().map(|(id, _)| format!("clip-{id}-")).collect();
    let saved_clips: Vec<String> = clips
        .iter()
        .filter_map(|(_, saved)| saved.clone())
        .collect();
    let jobs: Vec<String> = media_jobs.iter().chain(composition_jobs).cloned().collect();
    let session_renders: Vec<String> = session_ids.iter().map(|id| format!("{id}-")).collect();

    tokio::task::spawn_blocking(move || -> std::io::Result<u64> {
        let mut removed = 0;
        for tree in [roots.recordings.join(&guild), roots.no_silence.join(&guild)] {
            if tree.exists() {
                removed += count_files(&tree);
                std::fs::remove_dir_all(&tree)?;
            }
        }
        removed += remove_matching(&roots.waveforms, |name| {
            let stem = name
                .strip_prefix(sakiot_paths::NO_SILENCE_PREFIX)
                .unwrap_or(name);
            stem.split_once('.')
                .is_some_and(|(stem, _)| stems.contains(stem))
                || session_prefixes
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
                || clip_prefixes.iter().any(|prefix| name.starts_with(prefix))
        })?;
        removed += remove_matching(&roots.no_silence.join("logical_sessions"), |name| {
            session_renders
                .iter()
                .any(|prefix| name.starts_with(prefix))
        })?;
        for saved in &saved_clips {
            if let Some(path) = sakiot_paths::safe_join(&roots.clips, Path::new(saved))
                && std::fs::remove_file(&path).is_ok()
            {
                removed += 1;
            }
        }
        for dir in [
            roots.recordings.join(".media-jobs"),
            roots.recordings.join(".composition-jobs"),
            roots.clips.join("compositions"),
        ] {
            removed += remove_matching(&dir, |name| {
                jobs.iter().any(|job| name.starts_with(job.as_str()))
            })?;
        }
        Ok(removed)
    })
    .await
    .map_err(|_| AppError::InternalError)?
    .map_err(AppError::from)
}

fn remove_matching(dir: &Path, matches: impl Fn(&str) -> bool) -> std::io::Result<u64> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if !matches(&name.to_string_lossy()) {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            removed += count_files(&path);
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn count_files(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| match entry.file_type() {
                    Ok(kind) if kind.is_dir() => count_files(&entry.path()),
                    _ => 1,
                })
                .sum()
        })
        .unwrap_or(0)
}

/// The guild ids of every seeded fixture, for the agent.
pub(super) async fn fixture_guilds(pool: &Pool<Postgres>) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM guilds WHERE id >= $1 ORDER BY id")
        .bind(crate::synthetic::SYNTHETIC_ID_FLOOR)
        .fetch_all(pool)
        .await
}
