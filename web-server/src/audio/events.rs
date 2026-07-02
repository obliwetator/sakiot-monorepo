use actix_web::{HttpResponse, get, web};
use serde::Serialize;
use serde_with::{DisplayFromStr, serde_as};
use sqlx::{Pool, Postgres};

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::permissions::require_channel_access;

fn validate_stem(s: &str) -> Result<(), AppError> {
    if s.is_empty() || s.contains('/') || s.contains("..") || s.contains('\\') || s.contains('\'') {
        return Err(AppError::BadRequest("Invalid stem".into()));
    }
    Ok(())
}

#[serde_as]
#[derive(Serialize, utoipa::ToSchema)]
pub struct VoiceEventDto {
    pub offset_ms: i64,
    #[serde_as(as = "DisplayFromStr")]
    #[schema(value_type = String)]
    pub user_id: i64,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[schema(value_type = Option<String>)]
    pub channel_id: Option<i64>,
    pub event_type: String,
}

/// Voice state events (mute/deafen/etc.) overlapping the recording window for
/// `{stem}`. Frontend uses `offset_ms` against the audio element's currentTime
/// to render markers / current-state badges on the timeline.
///
/// Bounds: [start_ts, COALESCE(end_ts, now())]. start_ts/end_ts are stored as
/// epoch-ms BIGINTs on `audio_files`; voice_state_events.occurred_at is
/// TIMESTAMPTZ — converted via `to_timestamp(ms / 1000.0)` for the range scan.
#[utoipa::path(
    get,
    path = "/api/audio/events/{guild_id}/{channel_id}/{year}/{month}/{stem}",
    tag = "audio",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("channel_id" = i64, Path, description = "Discord channel id"),
        ("year" = i32, Path, description = "Recording year"),
        ("month" = u32, Path, description = "Recording month"),
        ("stem" = String, Path, description = "Recording file stem"),
        ("user_id" = Option<i64>, Query, description = "Optional user id filter"),
    ),
    responses(
        (status = 200, description = "Voice events for recording", body = [VoiceEventDto]),
        (status = 400, description = "Invalid request", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "Missing channel permission", body = crate::errors::ApiError),
        (status = 404, description = "Recording not found", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/events/{guild_id}/{channel_id}/{year}/{month}/{stem}")]
pub async fn get_recording_events(
    path: web::Path<(i64, i64, i32, u32, String)>,
    query: web::Query<EventsQuery>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let (guild_id, channel_id, _year, _month, stem) = path.into_inner();
    validate_stem(&stem)?;

    require_channel_access(&pool, guild_id, channel_id, token.user_id).await?;

    let row = sqlx::query!(
        "SELECT start_ts, end_ts FROM audio_files WHERE file_name = $1",
        stem
    )
    .fetch_optional(pool.get_ref())
    .await?;
    let row = row.ok_or(AppError::FileNotFound)?;
    let start_ts = row.start_ts.ok_or(AppError::FileNotFound)?;
    let end_ts = row.end_ts;

    let user_filter = query.user_id;

    let events = sqlx::query!(
        r#"SELECT
              v.user_id,
              v.channel_id,
              t.name AS "event_type!",
              ((EXTRACT(EPOCH FROM v.occurred_at) * 1000)::bigint - $1::bigint) AS "offset_ms!"
           FROM voice_state_events v
           JOIN voice_state_event_types t ON t.id = v.event_type_id
           WHERE v.guild_id = $2
             AND v.occurred_at >= to_timestamp(($1::bigint)::double precision / 1000.0)
             AND v.occurred_at <= COALESCE(
                 to_timestamp(($3::bigint)::double precision / 1000.0),
                 now()
             )
             AND ($4::bigint IS NULL OR v.user_id = $4)
           ORDER BY v.occurred_at ASC"#,
        start_ts,
        guild_id,
        end_ts,
        user_filter
    )
    .fetch_all(pool.get_ref())
    .await?;

    let dto: Vec<VoiceEventDto> = events
        .into_iter()
        .map(|r| VoiceEventDto {
            offset_ms: r.offset_ms,
            user_id: r.user_id,
            channel_id: r.channel_id,
            event_type: r.event_type,
        })
        .collect();

    Ok(HttpResponse::Ok().json(dto))
}

#[derive(serde::Deserialize)]
pub struct EventsQuery {
    pub user_id: Option<i64>,
}
