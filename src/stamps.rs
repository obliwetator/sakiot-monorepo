use actix_web::{get, web, HttpResponse, Responder};
use sakiot_paths::RecordingKey;
use serde::Serialize;
use serde_with::{As, DisplayFromStr};
use sqlx::{Pool, Postgres};

use crate::errors::AppError;

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
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/stamps/{guild_id}")]
pub async fn get_stamps(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
) -> Result<impl Responder, AppError> {
    let guild_id = path.into_inner();

    #[derive(Debug)]
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
    }

    let rows = sqlx::query_as!(
        Row,
        r#"
        SELECT s.id                     as "id!",
               s.guild_id               as "guild_id!",
               s.channel_id             as "channel_id!",
               s.target_user_id         as "target_user_id!",
               s.stamper_user_id        as "stamper_user_id!",
               s.stamp_ts               as "stamp_ts!",
               s.offset_ms              as "offset_ms!",
               s.audio_file_id,
               s.note,
               s.created_at             as "created_at!",
               COALESCE(tn.nickname, tu.global_name, tu.username) as target_name,
               COALESCE(sn.nickname, su.global_name, su.username) as stamper_name,
               c.name                   as channel_name,
               af.file_name             as file_name,
               af.year                  as year,
               af.month                 as month,
               af.start_ts              as start_ts
        FROM stamps s
        LEFT JOIN user_names      tu ON tu.user_id = s.target_user_id
        LEFT JOIN user_nicknames  tn ON tn.user_id = s.target_user_id  AND tn.guild_id = s.guild_id
        LEFT JOIN user_names      su ON su.user_id = s.stamper_user_id
        LEFT JOIN user_nicknames  sn ON sn.user_id = s.stamper_user_id AND sn.guild_id = s.guild_id
        LEFT JOIN channels        c  ON c.channel_id = s.channel_id
        LEFT JOIN audio_files     af ON af.id = s.audio_file_id
        WHERE s.guild_id = $1
        ORDER BY s.stamp_ts DESC
        LIMIT 500
        "#,
        guild_id
    )
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
                audio_url: urls.0,
                waveform_url: urls.1,
            }
        })
        .collect();

    Ok(HttpResponse::Ok().json(enriched))
}
