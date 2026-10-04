//! Simulated calls: what the recorder writes while members talk in a
//! channel.
//!
//! Each participant joins at a staggered time, gets an active recording
//! session, a growing fragment and a voice presence row, exactly as the
//! recorder's `create_fragment` leaves them. Their Ogg/Opus file then grows
//! in real time: every 500 ms the pages of a pre-encoded source that have
//! "happened" are appended, so readers see a valid file up to its last
//! complete page, as with the recorder. The synthetic agent heartbeats the
//! fragments. At the end each participant leaves the way a disconnect plays
//! out: the fragment closes, the session finalizes, presence goes.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use actix_web::{HttpRequest, HttpResponse, delete, get, post, web};
use chrono::{Datelike, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres};
use tokio::sync::watch;

use super::ogg::Page;
use super::{LoadtestState, authorize, ids, media};
use crate::config::Config;
use crate::errors::AppError;

const TICK: Duration = Duration::from_millis(500);
const FINALIZE_WRITER_CLOSE: i32 = 1;
const FINALIZE_ZOMBIE_REAPED: i32 = 3;
const INSIDERS_ROLE: u32 = 0;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct StartRequest {
    fixture: u32,
    /// Index into the fixture's channels; the last ones are restricted.
    channel: u32,
    participants: u32,
    duration_seconds: u64,
    /// Participants join evenly spread over this window.
    join_spread_seconds: u64,
    seed: u64,
}

impl Default for StartRequest {
    fn default() -> Self {
        Self {
            fixture: 1,
            channel: 0,
            participants: 10,
            duration_seconds: 1800,
            join_spread_seconds: 30,
            seed: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SimSummary {
    sim_id: u64,
    guild_id: String,
    channel_id: String,
    started_ms: i64,
    ends_ms: i64,
    participants: Vec<ParticipantSummary>,
}

#[derive(Debug, Clone, Serialize)]
struct ParticipantSummary {
    user_id: String,
    joins_ms: i64,
    /// Filled in once the participant has joined.
    recording_session_id: Option<String>,
    audio_file_id: Option<String>,
    file_name: Option<String>,
    year: Option<i32>,
    month: Option<u32>,
}

struct SimEntry {
    guild_id: i64,
    summary: SimSummary,
    live_audio_ids: Vec<i64>,
    stop: watch::Sender<bool>,
    task: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Default)]
pub(super) struct Sims {
    entries: parking_lot::Mutex<HashMap<u64, SimEntry>>,
    next_id: AtomicU64,
}

impl Sims {
    pub fn is_empty(&self) -> bool {
        self.entries.lock().is_empty()
    }

    /// Fragments being written right now, for the agent's heartbeat.
    pub fn audio_file_ids(&self) -> Vec<i64> {
        self.entries
            .lock()
            .values()
            .flat_map(|entry| entry.live_audio_ids.iter().copied())
            .collect()
    }

    pub fn summaries(&self, guild_id: i64) -> Vec<SimSummary> {
        let mut summaries: Vec<SimSummary> = self
            .entries
            .lock()
            .values()
            .filter(|entry| entry.guild_id == guild_id)
            .map(|entry| entry.summary.clone())
            .collect();
        summaries.sort_by_key(|summary| summary.sim_id);
        summaries
    }

    fn busy_users(&self, guild_id: i64) -> Vec<i64> {
        self.entries
            .lock()
            .values()
            .filter(|entry| entry.guild_id == guild_id)
            .flat_map(|entry| entry.summary.participants.iter())
            .filter_map(|participant| participant.user_id.parse().ok())
            .collect()
    }

    /// Stop one simulation and wait until every participant has left.
    async fn stop(&self, sim_id: u64) -> bool {
        let task = {
            let mut entries = self.entries.lock();
            let Some(entry) = entries.get_mut(&sim_id) else {
                return false;
            };
            let _ = entry.stop.send(true);
            entry.task.take()
        };
        if let Some(task) = task {
            let _ = task.await;
        }
        true
    }

    pub async fn stop_guild(&self, guild_id: i64) {
        let ids: Vec<u64> = self
            .entries
            .lock()
            .iter()
            .filter(|(_, entry)| entry.guild_id == guild_id)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.stop(id).await;
        }
    }
}

/// Start a simulated call in a fixture channel. Returns at once; the
/// participants join over `join_spread_seconds`.
#[post("/live")]
pub async fn start_call(
    req: HttpRequest,
    body: Option<web::Json<StartRequest>>,
    cfg: web::Data<Config>,
    state: web::Data<LoadtestState>,
) -> Result<HttpResponse, AppError> {
    authorize(&req, &cfg)?;
    let request = body.map(web::Json::into_inner).unwrap_or_default();
    if !(1..=ids::MAX_GUILD).contains(&request.fixture)
        || !(1..=200).contains(&request.participants)
        || !(10..=86_400).contains(&request.duration_seconds)
        || request.join_spread_seconds > request.duration_seconds / 2
    {
        return Err(AppError::BadRequest(
            "need fixture 1..=999, participants 1..=200, duration_seconds 10..=86400 \
             and join_spread_seconds <= duration_seconds / 2"
                .into(),
        ));
    }
    let guild_id = ids::guild_id(request.fixture);
    let channel_id = ids::channel_id(guild_id, request.channel);
    let restricted: Option<bool> = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM channel_permissions p
                         WHERE p.channel_id = c.channel_id AND p.target_id = c.guild_id
                           AND p.kind = 'role' AND (p.deny & 1024) <> 0)
           FROM channels c WHERE c.channel_id = $1 AND c.guild_id = $2",
    )
    .bind(channel_id)
    .bind(guild_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(restricted) = restricted else {
        return Err(AppError::NotFound);
    };

    // A member is in one channel at a time: skip anyone already in a call.
    let busy = state.sims.busy_users(guild_id);
    let mut candidates: Vec<i64> = sqlx::query_scalar(
        "SELECT m.user_id FROM guild_members m
          WHERE m.guild_id = $1 AND NOT (m.user_id = ANY($2))
            AND ($3 = false OR EXISTS (SELECT 1 FROM user_roles r
                                        WHERE r.user_id = m.user_id AND r.role_id = $4))
            AND NOT EXISTS (SELECT 1 FROM recording_sessions s
                             WHERE s.guild_id = m.guild_id AND s.user_id = m.user_id
                               AND s.state <> 'finalized')",
    )
    .bind(guild_id)
    .bind(&busy)
    .bind(restricted)
    .bind(ids::role_id(guild_id, INSIDERS_ROLE))
    .fetch_all(&state.pool)
    .await?;
    let mut rng = fastrand::Rng::with_seed(request.seed ^ Utc::now().timestamp_millis() as u64);
    rng.shuffle(&mut candidates);
    candidates.truncate(request.participants as usize);
    if candidates.is_empty() {
        return Err(AppError::Conflict(
            "no free members to join this channel".into(),
        ));
    }

    let sources = media::ensure_sources().await?;
    let longest_ms = (0..media::VARIANTS)
        .filter_map(|variant| sources.longest(variant))
        .map(|source| source.duration_ms)
        .min()
        .unwrap_or(0);
    let now_ms = Utc::now().timestamp_millis();
    let ends_ms = now_ms + (request.duration_seconds as i64 * 1000);
    let spread_ms = request.join_spread_seconds as i64 * 1000;
    let count = candidates.len() as i64;
    let mut plan = Vec::with_capacity(candidates.len());
    for (index, user_id) in candidates.iter().enumerate() {
        let joins_ms = now_ms + spread_ms * index as i64 / count;
        if ends_ms - joins_ms > longest_ms {
            return Err(AppError::BadRequest(format!(
                "a participant would talk longer than the {} s audio source",
                longest_ms / 1000
            )));
        }
        let variant = rng.u8(0..media::VARIANTS);
        let source = sources.longest(variant).ok_or(AppError::InternalError)?;
        plan.push(Planned {
            user_id: *user_id,
            joins_ms,
            source: source.path.clone(),
        });
    }

    let sim_id = state.sims.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let summary = SimSummary {
        sim_id,
        guild_id: guild_id.to_string(),
        channel_id: channel_id.to_string(),
        started_ms: now_ms,
        ends_ms,
        participants: plan
            .iter()
            .map(|planned| ParticipantSummary {
                user_id: planned.user_id.to_string(),
                joins_ms: planned.joins_ms,
                recording_session_id: None,
                audio_file_id: None,
                file_name: None,
                year: None,
                month: None,
            })
            .collect(),
    };
    let (stop, stopped) = watch::channel(false);
    let shared = state.clone().into_inner();
    let task = tokio::spawn(run(
        shared, sim_id, guild_id, channel_id, ends_ms, plan, stopped,
    ));
    state.sims.entries.lock().insert(
        sim_id,
        SimEntry {
            guild_id,
            summary: summary.clone(),
            live_audio_ids: Vec::new(),
            stop,
            task: Some(task),
        },
    );
    tracing::info!(
        sim_id,
        guild_id,
        channel_id,
        participants = count,
        "load-test call started"
    );
    Ok(HttpResponse::Accepted().json(summary))
}

#[get("/live")]
pub async fn list_calls(
    req: HttpRequest,
    cfg: web::Data<Config>,
    state: web::Data<LoadtestState>,
) -> Result<HttpResponse, AppError> {
    authorize(&req, &cfg)?;
    let summaries: Vec<SimSummary> = state
        .sims
        .entries
        .lock()
        .values()
        .map(|entry| entry.summary.clone())
        .collect();
    Ok(HttpResponse::Ok().json(summaries))
}

/// End a simulated call now. Waits until every participant has left.
#[delete("/live/{sim_id}")]
pub async fn stop_call(
    req: HttpRequest,
    path: web::Path<u64>,
    cfg: web::Data<Config>,
    state: web::Data<LoadtestState>,
) -> Result<HttpResponse, AppError> {
    authorize(&req, &cfg)?;
    if state.sims.stop(path.into_inner()).await {
        Ok(HttpResponse::NoContent().finish())
    } else {
        Err(AppError::NotFound)
    }
}

// ---- the call ---------------------------------------------------------------

struct Planned {
    user_id: i64,
    joins_ms: i64,
    source: PathBuf,
}

/// One participant's writer, moved into the blocking pool for each tick.
///
/// It opens its files only while appending. The agent records in its own
/// process, so a writer that kept them open would spend two of the web
/// server's file descriptors per participant that the real server never
/// spends.
struct Writer {
    user_id: i64,
    session_id: i64,
    audio_file_id: i64,
    start_ms: i64,
    pages: Arc<Vec<Page>>,
    next_page: usize,
    source: PathBuf,
    dest: PathBuf,
}

impl Writer {
    /// Append every page that has "happened" by now.
    fn advance(&mut self, now_ms: i64) -> std::io::Result<()> {
        let elapsed = now_ms - self.start_ms;
        let mut end = self.next_page;
        while end < self.pages.len() && self.pages[end].end_ms <= elapsed {
            end += 1;
        }
        if end == self.next_page {
            return Ok(());
        }
        let first = self.pages[self.next_page];
        let last = self.pages[end - 1];
        let len = (last.offset + last.len - first.offset) as usize;
        let mut buffer = vec![0_u8; len];
        let mut source = std::fs::File::open(&self.source)?;
        source.seek(SeekFrom::Start(first.offset))?;
        source.read_exact(&mut buffer)?;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&self.dest)?
            .write_all(&buffer)?;
        self.next_page = end;
        Ok(())
    }

    fn written_ms(&self) -> i64 {
        self.next_page
            .checked_sub(1)
            .map(|index| self.pages[index].end_ms)
            .unwrap_or(0)
    }
}

/// What leaving needs, kept outside the writers so a call can always be
/// closed out even if a write tick panics.
#[derive(Clone, Copy)]
struct Seat {
    user_id: i64,
    session_id: i64,
    audio_file_id: i64,
    start_ms: i64,
    written_ms: i64,
}

async fn run(
    state: Arc<LoadtestState>,
    sim_id: u64,
    guild_id: i64,
    channel_id: i64,
    ends_ms: i64,
    mut plan: Vec<Planned>,
    mut stopped: watch::Receiver<bool>,
) {
    let mut indexes: HashMap<PathBuf, Arc<Vec<Page>>> = HashMap::new();
    let mut writers: Vec<Writer> = Vec::new();
    let mut seats: Vec<Seat> = Vec::new();
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    plan.sort_by_key(|planned| std::cmp::Reverse(planned.joins_ms));

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = stopped.changed() => break,
        }
        let now_ms = Utc::now().timestamp_millis();
        if now_ms >= ends_ms {
            break;
        }
        while plan
            .last()
            .is_some_and(|planned| planned.joins_ms <= now_ms)
        {
            let Some(planned) = plan.pop() else { break };
            match join(&state, &mut indexes, guild_id, channel_id, &planned).await {
                Ok((writer, participant)) => {
                    let mut entries = state.sims.entries.lock();
                    if let Some(entry) = entries.get_mut(&sim_id) {
                        entry.live_audio_ids.push(writer.audio_file_id);
                        if let Some(slot) = entry
                            .summary
                            .participants
                            .iter_mut()
                            .find(|p| p.user_id == participant.user_id)
                        {
                            *slot = participant;
                        }
                    }
                    seats.push(Seat {
                        user_id: writer.user_id,
                        session_id: writer.session_id,
                        audio_file_id: writer.audio_file_id,
                        start_ms: writer.start_ms,
                        written_ms: 0,
                    });
                    writers.push(writer);
                }
                Err(error) => {
                    tracing::warn!(
                        ?error,
                        user_id = planned.user_id,
                        "load-test participant failed to join"
                    );
                }
            }
        }
        writers = match tokio::task::spawn_blocking(move || {
            for writer in &mut writers {
                if let Err(error) = writer.advance(now_ms) {
                    tracing::warn!(
                        ?error,
                        audio_file_id = writer.audio_file_id,
                        "load-test write failed"
                    );
                }
            }
            writers
        })
        .await
        {
            Ok(writers) => writers,
            Err(error) => {
                tracing::error!(
                    ?error,
                    sim_id,
                    "load-test write tick panicked; ending the call"
                );
                break;
            }
        };
        for (seat, writer) in seats.iter_mut().zip(&writers) {
            seat.written_ms = writer.written_ms();
        }
    }

    let now_ms = Utc::now().timestamp_millis();
    for seat in &seats {
        if let Err(error) = leave(&state, guild_id, channel_id, seat, now_ms).await {
            tracing::warn!(
                ?error,
                user_id = seat.user_id,
                "load-test participant failed to leave"
            );
        }
    }
    state.sims.entries.lock().remove(&sim_id);
    tracing::info!(sim_id, participants = seats.len(), "load-test call ended");
}

async fn join(
    state: &LoadtestState,
    indexes: &mut HashMap<PathBuf, Arc<Vec<Page>>>,
    guild_id: i64,
    channel_id: i64,
    planned: &Planned,
) -> Result<(Writer, ParticipantSummary), AppError> {
    let pages = match indexes.get(&planned.source) {
        Some(pages) => pages.clone(),
        None => {
            let data = tokio::fs::read(&planned.source).await?;
            let pages = Arc::new(super::ogg::index(&data)?);
            indexes.insert(planned.source.clone(), pages.clone());
            pages
        }
    };

    // The agent flushes every Ogg page, so a live file holds audio almost as
    // soon as its row exists. The file starts with the headers and the first
    // audio page, and the recording started that page's length ago.
    let first_audio = *pages.get(2).ok_or(AppError::InternalError)?;
    let start_ms = Utc::now().timestamp_millis() - first_audio.end_ms;
    let started = Utc
        .timestamp_millis_opt(start_ms)
        .single()
        .ok_or(AppError::InternalError)?;
    let stem = sakiot_paths::RecordingKey::stem_for(start_ms, planned.user_id);
    let key = sakiot_paths::RecordingKey::new(
        guild_id,
        channel_id,
        started.year(),
        started.month(),
        stem.clone(),
    );
    let path = key.recording_path(&sakiot_paths::DataRoots::from_env().recordings_str());

    let source = planned.source.clone();
    let dest = path.clone();
    let header_len = first_audio.offset + first_audio.len;
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut header = vec![0_u8; header_len as usize];
        std::fs::File::open(&source)?.read_exact(&mut header)?;
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?
            .write_all(&header)
    })
    .await
    .map_err(|_| AppError::InternalError)??;

    let mut tx = state.pool.begin().await?;
    let session_id: i64 = sqlx::query_scalar(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, current_channel_id, state, started_at,
             owner_instance_id, last_segment_index)
         VALUES ($1, $2, $3, $3, 'active', to_timestamp($4 / 1000.0), $5, 0)
         RETURNING id",
    )
    .bind(guild_id)
    .bind(planned.user_id)
    .bind(channel_id)
    .bind(start_ms)
    .bind(&state.agent_id)
    .fetch_one(&mut *tx)
    .await?;
    let audio_file_id: i64 = sqlx::query_scalar(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month, start_ts, end_ts,
             recording_owner_instance_id, recording_heartbeat_at, recording_session_id,
             segment_index)
         VALUES ($1, $2, $3, $4, $5, $6, $7, NULL, $8, now(), $9, 0)
         RETURNING id",
    )
    .bind(&stem)
    .bind(guild_id)
    .bind(channel_id)
    .bind(planned.user_id)
    .bind(started.year())
    .bind(started.month() as i32)
    .bind(start_ms)
    .bind(&state.agent_id)
    .bind(session_id)
    .fetch_one(&mut *tx)
    .await?;
    insert_events(
        &mut tx,
        session_id,
        channel_id,
        &[
            (start_ms, "session_start", serde_json::json!({})),
            (
                start_ms,
                "fragment_open",
                serde_json::json!({"audio_file_id": audio_file_id, "segment_index": 0, "file_name": stem}),
            ),
        ],
    )
    .await?;
    sqlx::query(
        "INSERT INTO voice_presence (guild_id, user_id, channel_id) VALUES ($1, $2, $3)
         ON CONFLICT (guild_id, user_id) DO UPDATE SET channel_id = EXCLUDED.channel_id",
    )
    .bind(guild_id)
    .bind(planned.user_id)
    .bind(channel_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok((
        Writer {
            user_id: planned.user_id,
            session_id,
            audio_file_id,
            start_ms,
            pages,
            next_page: 3,
            source: planned.source.clone(),
            dest,
        },
        ParticipantSummary {
            user_id: planned.user_id.to_string(),
            joins_ms: start_ms,
            recording_session_id: Some(session_id.to_string()),
            audio_file_id: Some(audio_file_id.to_string()),
            file_name: Some(stem),
            year: Some(started.year()),
            month: Some(started.month()),
        },
    ))
}

async fn leave(
    state: &LoadtestState,
    guild_id: i64,
    channel_id: i64,
    writer: &Seat,
    now_ms: i64,
) -> Result<(), AppError> {
    let end_ms = writer.start_ms + writer.written_ms;
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE audio_files
            SET end_ts = $2, recording_heartbeat_at = NULL, finalize_reason_id = $3
          WHERE id = $1 AND end_ts IS NULL",
    )
    .bind(writer.audio_file_id)
    .bind(end_ms)
    .bind(FINALIZE_WRITER_CLOSE)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE recording_sessions
            SET state = 'finalized', ended_at = to_timestamp($2 / 1000.0),
                pause_started_at = to_timestamp($2 / 1000.0), owner_instance_id = NULL,
                end_reason = 'pending_grace_expired', updated_at = now()
          WHERE id = $1 AND state <> 'finalized'",
    )
    .bind(writer.session_id)
    .bind(end_ms)
    .execute(&mut *tx)
    .await?;
    insert_events(
        &mut tx,
        writer.session_id,
        channel_id,
        &[
            (
                end_ms,
                "fragment_close",
                serde_json::json!({"audio_file_id": writer.audio_file_id, "segment_index": 0, "reason": "writer_close"}),
            ),
            (end_ms, "disconnect", serde_json::json!({"reason": "disconnect"})),
            (
                now_ms.max(end_ms),
                "timeout",
                serde_json::json!({"ended_at_ms": end_ms, "end_reason": "pending_grace_expired"}),
            ),
        ],
    )
    .await?;
    sqlx::query("DELETE FROM voice_presence WHERE guild_id = $1 AND user_id = $2")
        .bind(guild_id)
        .bind(writer.user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn insert_events(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    session_id: i64,
    channel_id: i64,
    events: &[(i64, &str, serde_json::Value)],
) -> Result<(), sqlx::Error> {
    for (at_ms, event_type, details) in events {
        sqlx::query(
            "INSERT INTO recording_session_events
                (recording_session_id, occurred_at, event_type, channel_id, details)
             VALUES ($1, to_timestamp($2 / 1000.0), $3, $4, $5::jsonb)",
        )
        .bind(session_id)
        .bind(*at_ms)
        .bind(*event_type)
        .bind(channel_id)
        .bind(details.to_string())
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Close recordings a previous process left open: each ends at its last
/// heartbeat, as the agent's reaper would close them.
pub(super) async fn close_orphans(
    pool: &Pool<Postgres>,
    agent_id: &str,
    live: &[i64],
) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let closed = sqlx::query(
        "UPDATE audio_files
            SET end_ts = GREATEST(start_ts,
                    (EXTRACT(EPOCH FROM COALESCE(recording_heartbeat_at, now())) * 1000)::bigint),
                recording_heartbeat_at = NULL,
                finalize_reason_id = $3
          WHERE recording_owner_instance_id = $1 AND end_ts IS NULL AND NOT (id = ANY($2))",
    )
    .bind(agent_id)
    .bind(live)
    .bind(FINALIZE_ZOMBIE_REAPED)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    sqlx::query(
        "UPDATE recording_sessions rs
            SET state = 'finalized',
                ended_at = GREATEST(rs.started_at, COALESCE(
                    (SELECT to_timestamp(MAX(af.end_ts) / 1000.0) FROM audio_files af
                      WHERE af.recording_session_id = rs.id), rs.started_at)),
                owner_instance_id = NULL, end_reason = 'owner_lost', updated_at = now()
          WHERE rs.owner_instance_id = $1 AND rs.state <> 'finalized'
            AND NOT EXISTS (SELECT 1 FROM audio_files af
                             WHERE af.recording_session_id = rs.id AND af.id = ANY($2))",
    )
    .bind(agent_id)
    .bind(live)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM voice_presence vp
          WHERE vp.guild_id >= $1
            AND NOT EXISTS (SELECT 1 FROM audio_files af
                             WHERE af.id = ANY($2) AND af.user_id = vp.user_id)",
    )
    .bind(crate::synthetic::SYNTHETIC_ID_FLOOR)
    .bind(live)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(closed)
}
