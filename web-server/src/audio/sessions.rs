use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use actix_files::NamedFile;
use actix_web::{HttpRequest, HttpResponse, Responder, get, http::header, post, web};
use base64::prelude::*;
use chrono::Datelike;
use sakiot_paths::RecordingKey;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres, Row};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::permissions::visible_channels_for_user;

use super::live::LiveContainer;
use super::paths::{clips_path, recording_path, waveform_path};
use super::types::{StartEnd, WaveformProgressContainer};

#[derive(Clone, Debug)]
pub(crate) struct SessionAccess {
    pub session_id: i64,
    pub guild_id: i64,
    pub user_id: i64,
    pub starting_channel_id: i64,
    pub state: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub pause_started_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct SessionSegmentDto {
    pub kind: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub channel_id: Option<String>,
    pub from_channel_id: Option<String>,
    pub to_channel_id: Option<String>,
    pub audio_file_id: Option<String>,
    pub file_name: Option<String>,
    pub segment_index: Option<i32>,
    pub reason: Option<String>,
    pub media_url: Option<String>,
    pub hls_playlist_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct SessionTimelineEventDto {
    pub source: String,
    pub event_type: String,
    pub offset_ms: i64,
    pub channel_id: Option<String>,
    pub previous_channel_id: Option<String>,
    pub details: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct SessionManifestDto {
    pub recording_session_id: String,
    pub guild_id: String,
    pub user_id: String,
    pub state: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub duration_ms: i64,
    pub starting_channel_id: String,
    pub current_channel_id: Option<String>,
    pub channel_journey: Vec<String>,
    pub segments: Vec<SessionSegmentDto>,
    pub events: Vec<SessionTimelineEventDto>,
}

#[derive(Clone, Debug)]
struct AudioFragment {
    id: i64,
    guild_id: i64,
    channel_id: i64,
    file_name: String,
    year: i32,
    month: i32,
    start_ms: i64,
    end_ms: Option<i64>,
    segment_index: Option<i32>,
    live: bool,
}

#[derive(Clone, Debug)]
struct Gap {
    start_ms: i64,
    end_ms: i64,
    reason: String,
    from_channel_id: Option<i64>,
    to_channel_id: Option<i64>,
}

pub(crate) async fn require_session_access(
    pool: &web::Data<Pool<Postgres>>,
    recording_session_id: i64,
    viewer_user_id: i64,
) -> Result<SessionAccess, AppError> {
    let row = sqlx::query(
        "SELECT id,
                guild_id,
                user_id,
                starting_channel_id,
                state,
                (EXTRACT(EPOCH FROM started_at) * 1000)::bigint AS started_at_ms,
                (EXTRACT(EPOCH FROM ended_at) * 1000)::bigint AS ended_at_ms,
                (EXTRACT(EPOCH FROM pause_started_at) * 1000)::bigint AS pause_started_at_ms
           FROM recording_sessions
          WHERE id = $1",
    )
    .bind(recording_session_id)
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or(AppError::FileNotFound)?;

    let access = SessionAccess {
        session_id: row.try_get("id")?,
        guild_id: row.try_get("guild_id")?,
        user_id: row.try_get("user_id")?,
        starting_channel_id: row.try_get("starting_channel_id")?,
        state: row.try_get("state")?,
        started_at_ms: row.try_get("started_at_ms")?,
        ended_at_ms: row.try_get("ended_at_ms")?,
        pause_started_at_ms: row.try_get("pause_started_at_ms")?,
    };

    let permitted = visible_channels_for_user(pool, access.guild_id, viewer_user_id).await?;
    let rows = sqlx::query(
        "SELECT DISTINCT channel_id
           FROM audio_files
          WHERE recording_session_id = $1",
    )
    .bind(recording_session_id)
    .fetch_all(pool.get_ref())
    .await?;
    let mut audible_channels: HashSet<i64> = rows
        .into_iter()
        .map(|row| row.get::<i64, _>("channel_id"))
        .collect();
    if audible_channels.is_empty() {
        audible_channels.insert(access.starting_channel_id);
    }
    if audible_channels
        .iter()
        .all(|channel_id| permitted.contains(channel_id))
    {
        Ok(access)
    } else {
        Err(AppError::Forbidden)
    }
}

/// Keeps stem-based routes compatible while applying logical-session
/// authorization whenever the physical file has a logical parent. A viewer
/// must never retrieve one permitted fragment from an otherwise forbidden
/// multi-channel session.
pub(crate) async fn require_recording_access(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    channel_id: i64,
    file_name: &str,
    viewer_user_id: i64,
) -> Result<(), AppError> {
    let stem = file_name.strip_suffix(".ogg").unwrap_or(file_name);
    let session_id = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT recording_session_id
           FROM audio_files
          WHERE guild_id = $1
            AND channel_id = $2
            AND (file_name = $3 OR file_name = $4)
          ORDER BY id DESC
          LIMIT 1",
    )
    .bind(guild_id)
    .bind(channel_id)
    .bind(file_name)
    .bind(stem)
    .fetch_optional(pool.get_ref())
    .await?
    .flatten();
    if let Some(session_id) = session_id {
        require_session_access(pool, session_id, viewer_user_id).await?;
    } else {
        crate::permissions::require_channel_access(pool, guild_id, channel_id, viewer_user_id)
            .await?;
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/manifest",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Logical recording manifest", body = SessionManifestDto),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Session not found", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/sessions/{recording_session_id}/manifest")]
pub async fn get_session_manifest(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    Ok(HttpResponse::Ok().json(build_manifest(&pool, access).await?))
}

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/events",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Unified logical timeline events", body = [SessionTimelineEventDto]),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Session not found", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/sessions/{recording_session_id}/events")]
pub async fn get_session_events(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let timeline_end_ms = timeline_end_ms(&access);
    Ok(HttpResponse::Ok().json(load_events(&pool, &access, timeline_end_ms).await?))
}

#[get("/audio/sessions/{recording_session_id}/segments/{audio_file_id}")]
pub async fn get_session_segment(
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<impl Responder, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let (session_id, audio_file_id) = path.into_inner();
    require_session_access(&pool, session_id, token.user_id).await?;
    let fragment = load_fragment(&pool, session_id, audio_file_id).await?;
    let path = fragment_path(&fragment);
    let file = NamedFile::open_async(path)
        .await
        .map_err(|_| AppError::FileNotFound)?;
    Ok(file.into_response(&req))
}

#[get("/audio/sessions/{recording_session_id}/live/{audio_file_id}/playlist.m3u8")]
pub async fn session_live_playlist(
    path: web::Path<(i64, i64)>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    container: web::Data<LiveContainer>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let (session_id, audio_file_id) = path.into_inner();
    require_session_access(&pool, session_id, token.user_id).await?;
    let fragment = load_fragment(&pool, session_id, audio_file_id).await?;
    let key = fragment_key(&fragment);
    let _ = super::live::ensure_job(container, pool, key.clone()).await?;
    let playlist = key.live_playlist_path(&recording_path());
    let body = tokio::fs::read(playlist)
        .await
        .map_err(|_| AppError::FileNotFound)?;
    let finalized = std::str::from_utf8(&body)
        .map(|contents| contents.contains("#EXT-X-ENDLIST"))
        .unwrap_or(false);
    Ok(HttpResponse::Ok()
        .content_type("application/vnd.apple.mpegurl")
        .insert_header((
            header::CACHE_CONTROL,
            if finalized {
                "public, max-age=300"
            } else {
                "no-cache"
            },
        ))
        .body(body))
}

#[get("/audio/sessions/{recording_session_id}/live/{audio_file_id}/{segment}")]
pub async fn session_live_segment(
    req: HttpRequest,
    path: web::Path<(i64, i64, String)>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<impl Responder, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let (session_id, audio_file_id, segment) = path.into_inner();
    validate_segment_name(&segment)?;
    require_session_access(&pool, session_id, token.user_id).await?;
    let fragment = load_fragment(&pool, session_id, audio_file_id).await?;
    let path = fragment_key(&fragment).live_segment_path(&recording_path(), &segment);
    let file = NamedFile::open_async(path)
        .await
        .map_err(|_| AppError::FileNotFound)?;
    let mut response = file.into_response(&req);
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static(if segment.starts_with("seg_") {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=3600"
        }),
    );
    Ok(response)
}

async fn build_manifest(
    pool: &web::Data<Pool<Postgres>>,
    access: SessionAccess,
) -> Result<SessionManifestDto, AppError> {
    let fragments = load_fragments(pool, access.session_id).await?;
    let gaps = load_gaps(pool, access.session_id).await?;
    let end_ms = timeline_end_ms(&access).max(
        fragments
            .iter()
            .filter_map(|fragment| fragment.end_ms)
            .max()
            .unwrap_or(access.started_at_ms),
    );
    let mut segments = Vec::with_capacity(fragments.len() + gaps.len());
    for fragment in &fragments {
        let fragment_end = fragment.end_ms.unwrap_or(end_ms.max(fragment.start_ms));
        segments.push(SessionSegmentDto {
            kind: if fragment.live {
                "active_hls".to_string()
            } else {
                "audio".to_string()
            },
            start_ms: fragment.start_ms.saturating_sub(access.started_at_ms),
            end_ms: fragment_end.saturating_sub(access.started_at_ms),
            channel_id: Some(fragment.channel_id.to_string()),
            from_channel_id: None,
            to_channel_id: None,
            audio_file_id: Some(fragment.id.to_string()),
            file_name: Some(fragment.file_name.clone()),
            segment_index: fragment.segment_index,
            reason: None,
            media_url: Some(format!(
                "/api/audio/sessions/{}/segments/{}",
                access.session_id, fragment.id
            )),
            hls_playlist_url: fragment.live.then(|| {
                format!(
                    "/api/audio/sessions/{}/live/{}/playlist.m3u8",
                    access.session_id, fragment.id
                )
            }),
        });
    }
    for gap in gaps {
        segments.push(SessionSegmentDto {
            kind: "silence".to_string(),
            start_ms: gap.start_ms.saturating_sub(access.started_at_ms),
            end_ms: gap.end_ms.saturating_sub(access.started_at_ms),
            channel_id: None,
            from_channel_id: gap.from_channel_id.map(|id| id.to_string()),
            to_channel_id: gap.to_channel_id.map(|id| id.to_string()),
            audio_file_id: None,
            file_name: None,
            segment_index: None,
            reason: Some(gap.reason),
            media_url: None,
            hls_playlist_url: None,
        });
    }
    segments.sort_by_key(|segment| (segment.start_ms, segment.end_ms));

    let mut channel_journey = vec![access.starting_channel_id.to_string()];
    for fragment in &fragments {
        let channel_id = fragment.channel_id.to_string();
        if channel_journey.last() != Some(&channel_id) {
            channel_journey.push(channel_id);
        }
    }
    let current_channel_id = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT current_channel_id FROM recording_sessions WHERE id = $1",
    )
    .bind(access.session_id)
    .fetch_optional(pool.get_ref())
    .await?
    .flatten();
    if let Some(current_channel_id) = current_channel_id {
        let current_channel_id = current_channel_id.to_string();
        if channel_journey.last() != Some(&current_channel_id) {
            channel_journey.push(current_channel_id);
        }
    }

    Ok(SessionManifestDto {
        recording_session_id: access.session_id.to_string(),
        guild_id: access.guild_id.to_string(),
        user_id: access.user_id.to_string(),
        state: access.state.clone(),
        started_at_ms: access.started_at_ms,
        ended_at_ms: access.ended_at_ms,
        duration_ms: end_ms.saturating_sub(access.started_at_ms),
        starting_channel_id: access.starting_channel_id.to_string(),
        current_channel_id: current_channel_id.map(|id| id.to_string()),
        channel_journey,
        events: load_events(pool, &access, end_ms).await?,
        segments,
    })
}

async fn load_fragments(
    pool: &web::Data<Pool<Postgres>>,
    session_id: i64,
) -> Result<Vec<AudioFragment>, AppError> {
    let rows = sqlx::query(
        "SELECT af.id,
                af.guild_id,
                af.channel_id,
                af.file_name,
                af.year,
                af.month,
                COALESCE(af.start_ts, 0) AS start_ms,
                af.end_ts AS end_ms,
                af.segment_index,
                (
                    af.end_ts IS NULL
                    AND af.reaped IS FALSE
                    AND EXISTS (
                        SELECT 1
                          FROM bot_instances bi
                         WHERE bi.instance_id = af.recording_owner_instance_id
                           AND af.recording_heartbeat_at > now() - interval '120 seconds'
                           AND bi.heartbeat_at > now() - interval '120 seconds'
                           AND bi.state <> 'stopped'
                    )
                ) AS live
           FROM audio_files af
          WHERE af.recording_session_id = $1
          ORDER BY af.segment_index NULLS LAST, af.start_ts, af.id",
    )
    .bind(session_id)
    .fetch_all(pool.get_ref())
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(AudioFragment {
                id: row.try_get("id")?,
                guild_id: row.try_get("guild_id")?,
                channel_id: row.try_get("channel_id")?,
                file_name: row.try_get("file_name")?,
                year: row.try_get("year")?,
                month: row.try_get("month")?,
                start_ms: row.try_get("start_ms")?,
                end_ms: row.try_get("end_ms")?,
                segment_index: row.try_get("segment_index")?,
                live: row.try_get::<Option<bool>, _>("live")?.unwrap_or(false),
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(AppError::DbError)
}

async fn load_fragment(
    pool: &web::Data<Pool<Postgres>>,
    session_id: i64,
    audio_file_id: i64,
) -> Result<AudioFragment, AppError> {
    load_fragments(pool, session_id)
        .await?
        .into_iter()
        .find(|fragment| fragment.id == audio_file_id)
        .ok_or(AppError::FileNotFound)
}

async fn load_gaps(
    pool: &web::Data<Pool<Postgres>>,
    session_id: i64,
) -> Result<Vec<Gap>, AppError> {
    let rows = sqlx::query(
        "SELECT (EXTRACT(EPOCH FROM started_at) * 1000)::bigint AS start_ms,
                (EXTRACT(EPOCH FROM ended_at) * 1000)::bigint AS end_ms,
                reason,
                from_channel_id,
                to_channel_id
           FROM recording_gaps
          WHERE recording_session_id = $1
          ORDER BY started_at, id",
    )
    .bind(session_id)
    .fetch_all(pool.get_ref())
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(Gap {
                start_ms: row.try_get("start_ms")?,
                end_ms: row.try_get("end_ms")?,
                reason: row.try_get("reason")?,
                from_channel_id: row.try_get("from_channel_id")?,
                to_channel_id: row.try_get("to_channel_id")?,
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(AppError::DbError)
}

async fn load_events(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    timeline_end_ms: i64,
) -> Result<Vec<SessionTimelineEventDto>, AppError> {
    let mut events = Vec::new();
    let session_rows = sqlx::query(
        "SELECT event_type,
                ((EXTRACT(EPOCH FROM occurred_at) * 1000)::bigint - $2) AS offset_ms,
                channel_id,
                previous_channel_id,
                details
           FROM recording_session_events
          WHERE recording_session_id = $1
          ORDER BY occurred_at, id",
    )
    .bind(access.session_id)
    .bind(access.started_at_ms)
    .fetch_all(pool.get_ref())
    .await?;
    for row in session_rows {
        events.push(SessionTimelineEventDto {
            source: "recording".to_string(),
            event_type: row.try_get("event_type")?,
            offset_ms: row.try_get("offset_ms")?,
            channel_id: row
                .try_get::<Option<i64>, _>("channel_id")?
                .map(|id| id.to_string()),
            previous_channel_id: row
                .try_get::<Option<i64>, _>("previous_channel_id")?
                .map(|id| id.to_string()),
            details: row.try_get("details")?,
        });
    }

    let voice_rows = sqlx::query(
        "SELECT t.name AS event_type,
                ((EXTRACT(EPOCH FROM v.occurred_at) * 1000)::bigint - $2) AS offset_ms,
                v.channel_id,
                v.previous_channel_id
           FROM voice_state_events v
           JOIN voice_state_event_types t ON t.id = v.event_type_id
          WHERE v.guild_id = $1
            AND v.user_id = $3
            AND v.occurred_at >= to_timestamp($2::double precision / 1000.0)
            AND v.occurred_at <= to_timestamp($4::double precision / 1000.0)
          ORDER BY v.occurred_at, v.id",
    )
    .bind(access.guild_id)
    .bind(access.started_at_ms)
    .bind(access.user_id)
    .bind(timeline_end_ms)
    .fetch_all(pool.get_ref())
    .await?;
    for row in voice_rows {
        events.push(SessionTimelineEventDto {
            source: "voice_state".to_string(),
            event_type: row.try_get("event_type")?,
            offset_ms: row.try_get("offset_ms")?,
            channel_id: row
                .try_get::<Option<i64>, _>("channel_id")?
                .map(|id| id.to_string()),
            previous_channel_id: row
                .try_get::<Option<i64>, _>("previous_channel_id")?
                .map(|id| id.to_string()),
            details: serde_json::json!({}),
        });
    }

    let connection_rows = sqlx::query(
        "SELECT outcome,
                trigger,
                owner_instance_id,
                release_id,
                ((EXTRACT(EPOCH FROM started_at) * 1000)::bigint - $2) AS started_offset_ms,
                ((EXTRACT(EPOCH FROM completed_at) * 1000)::bigint - $2) AS offset_ms,
                from_channel_id,
                to_channel_id,
                operation_id,
                error,
                fallback_outcome,
                fallback_error,
                population_snapshot
           FROM voice_connection_events
          WHERE guild_id = $1
            AND completed_at >= to_timestamp($2::double precision / 1000.0)
            AND started_at <= to_timestamp($3::double precision / 1000.0)
          ORDER BY completed_at, id",
    )
    .bind(access.guild_id)
    .bind(access.started_at_ms)
    .bind(timeline_end_ms)
    .fetch_all(pool.get_ref())
    .await?;
    for row in connection_rows {
        let trigger: String = row.try_get("trigger")?;
        let outcome: String = row.try_get("outcome")?;
        events.push(SessionTimelineEventDto {
            source: "voice_connection".to_string(),
            event_type: format!("{trigger}:{outcome}"),
            offset_ms: row.try_get("offset_ms")?,
            channel_id: row
                .try_get::<Option<i64>, _>("to_channel_id")?
                .map(|id| id.to_string()),
            previous_channel_id: row
                .try_get::<Option<i64>, _>("from_channel_id")?
                .map(|id| id.to_string()),
            details: serde_json::json!({
                "operation_id": row.try_get::<String, _>("operation_id")?,
                "owner_instance_id": row.try_get::<Option<String>, _>("owner_instance_id")?,
                "release_id": row.try_get::<Option<String>, _>("release_id")?,
                "started_offset_ms": row.try_get::<i64, _>("started_offset_ms")?,
                "error": row.try_get::<Option<String>, _>("error")?,
                "fallback_outcome": row.try_get::<Option<String>, _>("fallback_outcome")?,
                "fallback_error": row.try_get::<Option<String>, _>("fallback_error")?,
                "population_snapshot": row.try_get::<serde_json::Value, _>("population_snapshot")?,
            }),
        });
    }
    events.sort_by_key(|event| event.offset_ms);
    Ok(events)
}

fn timeline_end_ms(access: &SessionAccess) -> i64 {
    match access.state.as_str() {
        "finalized" => access.ended_at_ms.unwrap_or(access.started_at_ms),
        "pending" => access.pause_started_at_ms.unwrap_or(access.started_at_ms),
        _ => chrono::Utc::now().timestamp_millis(),
    }
}

fn fragment_key(fragment: &AudioFragment) -> RecordingKey {
    RecordingKey::new(
        fragment.guild_id,
        fragment.channel_id,
        fragment.year,
        fragment.month as u32,
        fragment.file_name.clone(),
    )
}

fn fragment_path(fragment: &AudioFragment) -> PathBuf {
    RecordingKey::new(
        fragment.guild_id,
        fragment.channel_id,
        fragment.year,
        fragment.month as u32,
        fragment.file_name.clone(),
    )
    .recording_path(&recording_path())
}

fn validate_segment_name(segment: &str) -> Result<(), AppError> {
    if segment.is_empty()
        || segment == "playlist.m3u8"
        || segment == "state"
        || segment.contains('/')
        || segment.contains("..")
        || segment.contains('\\')
    {
        Err(AppError::BadRequest("Invalid HLS segment name".into()))
    } else {
        Ok(())
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SessionDownloadQuery {
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub remove_silence: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SessionWaveformResponse {
    pub progress: i16,
    pub building: bool,
    pub data: Option<String>,
}

#[derive(Clone, Debug)]
enum PartKind {
    Audio { path: PathBuf, source_start_ms: i64 },
    Silence,
}

#[derive(Clone, Debug)]
struct TimelinePart {
    start_ms: i64,
    end_ms: i64,
    kind: PartKind,
}

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/download",
    tag = "audio",
    params(
        ("recording_session_id" = i64, Path, description = "Logical recording session id"),
        ("start" = Option<f64>, Query, description = "Range start in logical seconds"),
        ("end" = Option<f64>, Query, description = "Range end in logical seconds"),
        ("remove_silence" = Option<bool>, Query, description = "Run FFmpeg silence removal after composition"),
    ),
    responses(
        (status = 200, description = "Composed Ogg/Opus download", content_type = "audio/ogg"),
        (status = 400, description = "Invalid range", body = crate::errors::ApiError),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Session not found", body = crate::errors::ApiError),
        (status = 500, description = "Composition failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/sessions/{recording_session_id}/download")]
pub async fn download_session(
    path: web::Path<i64>,
    query: web::Query<SessionDownloadQuery>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<NamedFile, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let output = temporary_ogg_path("session-download");
    compose_session(
        &pool,
        &access,
        query.start,
        query.end,
        query.remove_silence.unwrap_or(false),
        &output,
    )
    .await?;
    schedule_temporary_cleanup(output.clone());
    Ok(NamedFile::open_async(output)
        .await
        .map_err(AppError::IoError)?
        .set_content_disposition(actix_web::http::header::ContentDisposition {
            disposition: actix_web::http::header::DispositionType::Attachment,
            parameters: vec![],
        }))
}

#[utoipa::path(
    post,
    path = "/api/audio/sessions/{recording_session_id}/remove-silence",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    request_body = StartEnd,
    responses(
        (status = 200, description = "Composed silence-free Ogg/Opus", content_type = "audio/ogg"),
        (status = 400, description = "Invalid range", body = crate::errors::ApiError),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 500, description = "Composition failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/audio/sessions/{recording_session_id}/remove-silence")]
pub async fn remove_session_silence(
    path: web::Path<i64>,
    range: web::Json<StartEnd>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<NamedFile, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let output = temporary_ogg_path("session-silence-free");
    compose_session(
        &pool,
        &access,
        range.start.map(f64::from),
        range.end.map(f64::from),
        true,
        &output,
    )
    .await?;
    schedule_temporary_cleanup(output.clone());
    Ok(NamedFile::open_async(output)
        .await
        .map_err(AppError::IoError)?
        .set_content_disposition(actix_web::http::header::ContentDisposition {
            disposition: actix_web::http::header::DispositionType::Attachment,
            parameters: vec![],
        }))
}

#[utoipa::path(
    post,
    path = "/api/audio/sessions/{recording_session_id}/clips",
    tag = "clips",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    request_body = StartEnd,
    responses(
        (status = 200, description = "Cross-fragment clip created", body = crate::clips::CreateClipResponse),
        (status = 400, description = "Clip must be 1-20 seconds", body = crate::errors::ApiError),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 500, description = "Composition failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/audio/sessions/{recording_session_id}/clips")]
pub async fn create_session_clip(
    path: web::Path<i64>,
    range: web::Json<StartEnd>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let start = f64::from(range.start.unwrap_or(0.0));
    let end = f64::from(range.end.unwrap_or(0.0));
    if !start.is_finite() || !end.is_finite() || !(1.0..=20.0).contains(&(end - start)) {
        return Err(AppError::BadRequest(
            "Clip duration must be between 1 and 20 seconds".into(),
        ));
    }

    let now = chrono::Utc::now();
    let clip_id = uuid::Uuid::new_v4().to_string();
    let target_dir = PathBuf::from(clips_path())
        .join(now.year().to_string())
        .join(format!("{:02}", now.month()));
    tokio::fs::create_dir_all(&target_dir).await?;
    let full_path = target_dir.join(format!("{clip_id}.ogg"));
    compose_session(&pool, &access, Some(start), Some(end), false, &full_path).await?;
    let size = tokio::fs::metadata(&full_path).await?.len() as i64;
    let saved_file_name = format!("{}/{:02}/{clip_id}.ogg", now.year(), now.month());
    let name = range
        .name
        .clone()
        .unwrap_or_else(|| format!("session-{session_id}"));

    let insert = sqlx::query(
        "INSERT INTO clips
            (clip_id, length, size, channel_id, guild_id, user_id,
             original_file_name, saved_file_name, name, start_time,
             recording_session_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&clip_id)
    .bind((end - start) as f32)
    .bind(size)
    .bind(access.starting_channel_id)
    .bind(access.guild_id)
    .bind(token.user_id)
    .bind(format!("session:{session_id}"))
    .bind(&saved_file_name)
    .bind(&name)
    .bind(start as f32)
    .bind(session_id)
    .execute(pool.get_ref())
    .await;
    if let Err(err) = insert {
        let _ = tokio::fs::remove_file(&full_path).await;
        return Err(AppError::DbError(err));
    }

    Ok(HttpResponse::Ok().json(crate::clips::CreateClipResponse {
        status: "success",
        file: saved_file_name,
        id: clip_id,
        name,
    }))
}

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/waveform",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Combined peaks; explicit gaps are zero-valued", body = SessionWaveformResponse),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Session not found", body = crate::errors::ApiError),
        (status = 500, description = "Waveform generation failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/sessions/{recording_session_id}/waveform")]
pub async fn get_session_waveform(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    progress: web::Data<WaveformProgressContainer>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    require_session_access(&pool, session_id, token.user_id).await?;
    let cache_key = format!("logical-session-{session_id}");
    let output = PathBuf::from(waveform_path()).join(format!("{cache_key}.dat"));

    {
        let mut map = progress.0.write().await;
        if let Some(value) = map.get(&cache_key).copied() {
            if value < 0 {
                map.remove(&cache_key);
                return Err(AppError::InternalError);
            }
            return Ok(HttpResponse::Ok().json(SessionWaveformResponse {
                progress: value.min(99),
                building: true,
                data: None,
            }));
        }
    }

    if tokio::fs::try_exists(&output).await.unwrap_or(false) {
        return waveform_file_response(&output).await;
    }

    Ok(HttpResponse::Ok().json(SessionWaveformResponse {
        progress: 0,
        building: false,
        data: None,
    }))
}

#[utoipa::path(
    post,
    path = "/api/audio/sessions/{recording_session_id}/waveform/rebuild",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Combined waveform rebuild started or already running", body = SessionWaveformResponse),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Session not found", body = crate::errors::ApiError),
        (status = 500, description = "Waveform generation failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/audio/sessions/{recording_session_id}/waveform/rebuild")]
pub async fn rebuild_session_waveform(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    progress: web::Data<WaveformProgressContainer>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let cache_key = format!("logical-session-{session_id}");
    let output = PathBuf::from(waveform_path()).join(format!("{cache_key}.dat"));
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    {
        let mut map = progress.0.write().await;
        if let Some(value) = map.get(&cache_key).copied() {
            if value >= 0 {
                return Ok(HttpResponse::Ok().json(SessionWaveformResponse {
                    progress: value.min(99),
                    building: true,
                    data: None,
                }));
            }
            map.remove(&cache_key);
        }
        map.insert(cache_key.clone(), 0);
    }

    let composite =
        PathBuf::from(waveform_path()).join(format!("{cache_key}-{}.ogg", uuid::Uuid::new_v4()));
    let pool_clone = pool.clone();
    let progress_clone = progress.clone();
    let cache_key_clone = cache_key.clone();
    tokio::spawn(async move {
        progress_clone
            .0
            .write()
            .await
            .insert(cache_key_clone.clone(), 1);
        let composition_progress = CompositionProgress {
            cache_key: cache_key_clone.clone(),
            progress: progress_clone.clone(),
            completed: 85,
        };
        if let Err(err) = compose_session_with_progress(
            &pool_clone,
            &access,
            None,
            None,
            false,
            &composite,
            composition_progress,
        )
        .await
        {
            tracing::error!(session_id, "session waveform composition failed: {}", err);
            progress_clone.0.write().await.insert(cache_key_clone, -1);
            return;
        }
        let generation = crate::waveform::generate_peaks_background(
            composite.to_string_lossy().into_owned(),
            output.to_string_lossy().into_owned(),
            cache_key_clone.clone(),
            None,
            progress_clone.clone(),
            None,
            Some((85, 99)),
        )
        .await;
        let _ = tokio::fs::remove_file(composite).await;
        if let Err(err) = generation {
            tracing::error!(session_id, "session waveform generation failed: {}", err);
            progress_clone.0.write().await.insert(cache_key_clone, -1);
        }
    });

    Ok(HttpResponse::Ok().json(SessionWaveformResponse {
        progress: 0,
        building: true,
        data: None,
    }))
}

async fn waveform_file_response(path: &Path) -> Result<HttpResponse, AppError> {
    let bytes = tokio::fs::read(path).await?;
    Ok(HttpResponse::Ok().json(SessionWaveformResponse {
        progress: 100,
        building: false,
        data: Some(BASE64_STANDARD.encode(bytes)),
    }))
}

async fn compose_session(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    range_start_seconds: Option<f64>,
    range_end_seconds: Option<f64>,
    remove_silence: bool,
    output: &Path,
) -> Result<(), AppError> {
    compose_session_inner(
        pool,
        access,
        range_start_seconds,
        range_end_seconds,
        remove_silence,
        output,
        None,
    )
    .await
}

#[derive(Clone)]
struct CompositionProgress {
    cache_key: String,
    progress: web::Data<WaveformProgressContainer>,
    completed: i16,
}

async fn compose_session_with_progress(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    range_start_seconds: Option<f64>,
    range_end_seconds: Option<f64>,
    remove_silence: bool,
    output: &Path,
    progress: CompositionProgress,
) -> Result<(), AppError> {
    compose_session_inner(
        pool,
        access,
        range_start_seconds,
        range_end_seconds,
        remove_silence,
        output,
        Some(progress),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn compose_session_inner(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    range_start_seconds: Option<f64>,
    range_end_seconds: Option<f64>,
    remove_silence: bool,
    output: &Path,
    progress: Option<CompositionProgress>,
) -> Result<(), AppError> {
    let parts = timeline_parts(pool, access).await?;
    let timeline_end = timeline_end_ms(access);
    let duration_ms = timeline_end.saturating_sub(access.started_at_ms);
    let start_ms = seconds_to_ms(range_start_seconds.unwrap_or(0.0))?;
    let end_ms = seconds_to_ms(range_end_seconds.unwrap_or(duration_ms as f64 / 1_000.0))?;
    if start_ms < 0 || end_ms <= start_ms || end_ms > duration_ms {
        return Err(AppError::BadRequest(
            "Invalid logical recording range".into(),
        ));
    }
    let selected_start = access.started_at_ms.saturating_add(start_ms);
    let selected_end = access.started_at_ms.saturating_add(end_ms.min(duration_ms));
    let selected: Vec<TimelinePart> = parts
        .into_iter()
        .filter_map(|part| {
            let start = part.start_ms.max(selected_start);
            let end = part.end_ms.min(selected_end);
            (end > start).then_some(TimelinePart {
                start_ms: start,
                end_ms: end,
                kind: part.kind,
            })
        })
        .collect();
    if selected.is_empty() {
        return Err(AppError::BadRequest(
            "Selected range contains no timeline".into(),
        ));
    }
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .arg("-y")
        .args(["-hide_banner", "-loglevel", "error"]);
    for part in &selected {
        let duration = milliseconds_as_seconds(part.end_ms.saturating_sub(part.start_ms));
        match &part.kind {
            PartKind::Audio {
                path,
                source_start_ms,
            } => {
                command
                    .args([
                        "-ss",
                        &milliseconds_as_seconds(part.start_ms.saturating_sub(*source_start_ms)),
                        "-t",
                        &duration,
                        "-i",
                    ])
                    .arg(path);
            }
            PartKind::Silence => {
                command.args([
                    "-f",
                    "lavfi",
                    "-t",
                    &duration,
                    "-i",
                    "anullsrc=channel_layout=mono:sample_rate=48000",
                ]);
            }
        }
    }

    let mut filter = String::new();
    for index in 0..selected.len() {
        filter.push_str(&format!(
            "[{index}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=mono[a{index}];"
        ));
    }
    for index in 0..selected.len() {
        filter.push_str(&format!("[a{index}]"));
    }
    filter.push_str(&format!("concat=n={}:v=0:a=1[joined]", selected.len()));
    let output_label = if remove_silence {
        filter.push_str(
            ";[joined]silenceremove=stop_periods=-1:stop_duration=1:stop_threshold=-40dB[out]",
        );
        "[out]"
    } else {
        "[joined]"
    };
    command
        .args(["-filter_complex", &filter, "-map", output_label])
        .args(["-c:a", "libopus", "-b:a", "96k"])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    if progress.is_some() {
        command.args(["-progress", "pipe:2", "-nostats"]);
    }

    let mut child = command.spawn().map_err(AppError::IoError)?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::FfmpegError("FFmpeg stderr pipe unavailable".into()))?;
    let mut lines = BufReader::new(stderr).lines();
    let mut error_output = Vec::new();
    while let Some(line) = lines.next_line().await.map_err(AppError::IoError)? {
        if let Some(progress) = &progress
            && let Some(value) = composition_progress_percent(
                &line,
                selected_end.saturating_sub(selected_start),
                progress.completed,
            )
        {
            let mut values = progress.progress.0.write().await;
            let current = values.entry(progress.cache_key.clone()).or_insert(0);
            if *current >= 0 && value > *current {
                *current = value;
            }
        } else if !is_ffmpeg_progress_line(&line) && error_output.len() < 4_096 {
            let remaining = 4_096 - error_output.len();
            error_output.extend_from_slice(&line.as_bytes()[..line.len().min(remaining)]);
            error_output.push(b'\n');
        }
    }

    let status = child.wait().await.map_err(AppError::IoError)?;
    if !status.success() {
        return Err(AppError::FfmpegError(
            String::from_utf8_lossy(&error_output).into_owned(),
        ));
    }
    if let Some(progress) = progress {
        progress
            .progress
            .0
            .write()
            .await
            .insert(progress.cache_key, progress.completed);
    }
    Ok(())
}

fn composition_progress_percent(line: &str, duration_ms: i64, completed: i16) -> Option<i16> {
    let elapsed_us = line.strip_prefix("out_time_us=")?.parse::<u64>().ok()?;
    let duration_ms = u64::try_from(duration_ms).ok()?.max(1);
    let completed = u64::try_from(completed).ok()?.clamp(2, 99);
    let elapsed_ms = elapsed_us / 1_000;
    let progress = elapsed_ms
        .saturating_mul(completed)
        .checked_div(duration_ms)?
        .clamp(1, completed - 1);
    i16::try_from(progress).ok()
}

fn is_ffmpeg_progress_line(line: &str) -> bool {
    matches!(
        line.split_once('=').map(|(key, _)| key),
        Some(
            "bitrate"
                | "drop_frames"
                | "dup_frames"
                | "fps"
                | "frame"
                | "out_time"
                | "out_time_ms"
                | "out_time_us"
                | "progress"
                | "speed"
                | "stream_0_0_q"
                | "total_size"
        )
    )
}

async fn timeline_parts(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
) -> Result<Vec<TimelinePart>, AppError> {
    let timeline_end = timeline_end_ms(access);
    let mut raw = Vec::new();
    for fragment in load_fragments(pool, access.session_id).await? {
        let end_ms = fragment.end_ms.unwrap_or(timeline_end).min(timeline_end);
        if end_ms > fragment.start_ms {
            raw.push(TimelinePart {
                start_ms: fragment.start_ms,
                end_ms,
                kind: PartKind::Audio {
                    path: fragment_path(&fragment),
                    source_start_ms: fragment.start_ms,
                },
            });
        }
    }
    for gap in load_gaps(pool, access.session_id).await? {
        if gap.end_ms > gap.start_ms {
            raw.push(TimelinePart {
                start_ms: gap.start_ms,
                end_ms: gap.end_ms,
                kind: PartKind::Silence,
            });
        }
    }
    raw.sort_by_key(|part| (part.start_ms, part.end_ms));

    let mut complete = Vec::new();
    let mut cursor = access.started_at_ms;
    for part in raw {
        if part.end_ms <= cursor || part.start_ms >= timeline_end {
            continue;
        }
        if part.start_ms > cursor {
            complete.push(TimelinePart {
                start_ms: cursor,
                end_ms: part.start_ms.min(timeline_end),
                kind: PartKind::Silence,
            });
        }
        let start_ms = part.start_ms.max(cursor);
        let end_ms = part.end_ms.min(timeline_end);
        if end_ms > start_ms {
            complete.push(TimelinePart {
                start_ms,
                end_ms,
                kind: part.kind,
            });
            cursor = end_ms;
        }
    }
    if cursor < timeline_end {
        complete.push(TimelinePart {
            start_ms: cursor,
            end_ms: timeline_end,
            kind: PartKind::Silence,
        });
    }
    Ok(complete)
}

fn seconds_to_ms(seconds: f64) -> Result<i64, AppError> {
    if !seconds.is_finite() || seconds < 0.0 || seconds > i64::MAX as f64 / 1_000.0 {
        return Err(AppError::BadRequest("Invalid range timestamp".into()));
    }
    Ok((seconds * 1_000.0).round() as i64)
}

fn milliseconds_as_seconds(milliseconds: i64) -> String {
    format!("{:.3}", milliseconds.max(0) as f64 / 1_000.0)
}

fn temporary_ogg_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}.ogg", uuid::Uuid::new_v4()))
}

fn schedule_temporary_cleanup(path: PathBuf) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10 * 60)).await;
        let _ = tokio::fs::remove_file(path).await;
    });
}

#[cfg(test)]
mod tests {
    use super::{
        SessionAccess, composition_progress_percent, timeline_end_ms, validate_segment_name,
    };

    fn access(state: &str) -> SessionAccess {
        SessionAccess {
            session_id: 1,
            guild_id: 2,
            user_id: 3,
            starting_channel_id: 4,
            state: state.to_string(),
            started_at_ms: 1_000,
            ended_at_ms: Some(9_000),
            pause_started_at_ms: Some(5_000),
        }
    }

    #[test]
    fn pending_timeline_stops_at_original_pause() {
        assert_eq!(timeline_end_ms(&access("pending")), 5_000);
        assert_eq!(timeline_end_ms(&access("finalized")), 9_000);
    }

    #[test]
    fn hls_segment_validation_blocks_traversal_and_reserved_routes() {
        assert!(validate_segment_name("seg_00001.m4s").is_ok());
        assert!(validate_segment_name("../secret").is_err());
        assert!(validate_segment_name("playlist.m3u8").is_err());
    }

    #[test]
    fn maps_ffmpeg_output_time_into_composition_progress() {
        assert_eq!(
            composition_progress_percent("out_time_us=5000000", 10_000, 85),
            Some(42)
        );
        assert_eq!(
            composition_progress_percent("out_time_us=10000000", 10_000, 85),
            Some(84)
        );
        assert_eq!(
            composition_progress_percent("progress=end", 10_000, 85),
            None
        );
    }
}
