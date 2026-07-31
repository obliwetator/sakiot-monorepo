use actix_web::{HttpResponse, Responder, get, web};
use sakiot_paths::RecordingKey;
use serde::Serialize;
use serde_with::{As, DisplayFromStr};
use sqlx::{Pool, Postgres};

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::permissions::visible_channels_for_user;

type DisplayFromstr = As<DisplayFromStr>;

#[derive(Serialize, Debug, utoipa::ToSchema)]
pub struct StampInfo {
    id: i64,
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    guild_id: i64,
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    channel_id: i64,
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    target_user_id: i64,
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    stamper_user_id: i64,
    stamp_ts: i64,
    offset_ms: i32,
    audio_file_id: Option<i64>,
    note: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    target_name: Option<String>,
    stamper_name: Option<String>,
    channel_name: Option<String>,
    file_name: Option<String>,
    year: Option<i32>,
    month: Option<i32>,
    start_ts: Option<i64>,
    recording_session_id: Option<String>,
    segment_index: Option<i32>,
    session_started_at_ms: Option<i64>,
    session_fragment_count: Option<i64>,
    audio_url: Option<String>,
    waveform_url: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/stamps/{guild_id}",
    tag = "stamps",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "Recent stamps for guild", body = [StampInfo]),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "Missing guild or channel permission", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/stamps/{guild_id}")]
pub async fn get_stamps(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<impl Responder, AppError> {
    let guild_id = path.into_inner();
    let token = token.ok_or(AppError::Unauthorized)?;
    let permitted = visible_channels_for_user(&pool, guild_id, token.user_id).await?;
    if permitted.is_empty() {
        return Ok(HttpResponse::Ok().json(Vec::<StampInfo>::new()));
    }
    let permitted: Vec<i64> = permitted.into_iter().collect();

    #[derive(Debug, sqlx::FromRow)]
    struct Row {
        id: i64,
        guild_id: i64,
        channel_id: i64,
        target_user_id: i64,
        stamper_user_id: i64,
        stamp_ts: i64,
        offset_ms: i32,
        audio_file_id: Option<i64>,
        note: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
        target_name: Option<String>,
        stamper_name: Option<String>,
        channel_name: Option<String>,
        file_name: Option<String>,
        year: Option<i32>,
        month: Option<i32>,
        start_ts: Option<i64>,
        recording_session_id: Option<i64>,
        segment_index: Option<i32>,
        session_started_at_ms: Option<i64>,
        session_fragment_count: Option<i64>,
    }

    let rows = sqlx::query_as::<_, Row>(
        r#"
        SELECT s.id,
               s.guild_id,
               s.channel_id,
               s.target_user_id,
               s.stamper_user_id,
               s.stamp_ts,
               s.offset_ms,
               s.audio_file_id,
               s.note,
               s.created_at,
               COALESCE(tn.nickname, tu.global_name, tu.username) as target_name,
               COALESCE(sn.nickname, su.global_name, su.username) as stamper_name,
               c.name                   as channel_name,
               af.file_name             as file_name,
               af.year                  as year,
               af.month                 as month,
               af.start_ts              as start_ts,
               rs.id                    as recording_session_id,
               CASE WHEN rs.id IS NULL THEN NULL ELSE af.segment_index END
                                           as segment_index,
               (EXTRACT(EPOCH FROM rs.started_at) * 1000)::bigint
                                           as session_started_at_ms,
               CASE
                   WHEN rs.id IS NULL THEN NULL
                   ELSE (
                       SELECT COUNT(*)
                         FROM audio_files sibling
                        WHERE sibling.recording_session_id = rs.id
                   )
               END                         as session_fragment_count
        FROM stamps s
        LEFT JOIN user_names      tu ON tu.user_id = s.target_user_id
        LEFT JOIN user_nicknames  tn ON tn.user_id = s.target_user_id  AND tn.guild_id = s.guild_id
        LEFT JOIN user_names      su ON su.user_id = s.stamper_user_id
        LEFT JOIN user_nicknames  sn ON sn.user_id = s.stamper_user_id AND sn.guild_id = s.guild_id
        LEFT JOIN channels        c  ON c.channel_id = s.channel_id
        LEFT JOIN audio_files     af ON af.id = s.audio_file_id
        LEFT JOIN recording_sessions rs
          ON rs.id = COALESCE(s.recording_session_id, af.recording_session_id)
         AND rs.guild_id = s.guild_id
         AND rs.user_id = s.target_user_id
         AND NOT EXISTS (
                 SELECT 1
                   FROM audio_files restricted
                  WHERE restricted.recording_session_id = rs.id
                    AND NOT (restricted.channel_id = ANY($2))
             )
        WHERE s.guild_id = $1
          AND s.channel_id = ANY($2)
        ORDER BY s.stamp_ts DESC
        LIMIT 500
        "#,
    )
    .bind(guild_id)
    .bind(&permitted)
    .fetch_all(pool.get_ref())
    .await?;

    let enriched: Vec<StampInfo> = rows
        .into_iter()
        .map(|r| {
            let urls = match (&r.file_name, r.year, r.month) {
                (Some(stem), Some(y), Some(m)) if (1..=12).contains(&m) => {
                    let key = RecordingKey::new(r.guild_id, r.channel_id, y, m as u32, stem);
                    (Some(key.audio_url()), Some(key.waveform_url()))
                }
                _ => (None, None),
            };
            StampInfo {
                id: r.id,
                guild_id: r.guild_id,
                channel_id: r.channel_id,
                target_user_id: r.target_user_id,
                stamper_user_id: r.stamper_user_id,
                stamp_ts: r.stamp_ts,
                offset_ms: r.offset_ms,
                audio_file_id: r.audio_file_id,
                note: r.note,
                created_at: r.created_at,
                target_name: r.target_name,
                stamper_name: r.stamper_name,
                channel_name: r.channel_name,
                file_name: r.file_name,
                year: r.year,
                month: r.month,
                start_ts: r.start_ts,
                recording_session_id: r.recording_session_id.map(|id| id.to_string()),
                segment_index: r.segment_index,
                session_started_at_ms: r.session_started_at_ms,
                session_fragment_count: r.session_fragment_count,
                audio_url: urls.0,
                waveform_url: urls.1,
            }
        })
        .collect();

    Ok(HttpResponse::Ok().json(enriched))
}
