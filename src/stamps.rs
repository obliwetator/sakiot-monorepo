use actix_web::{get, web, HttpResponse, Responder};
use serde::Serialize;
use serde_with::{As, DisplayFromStr};
use sqlx::{Pool, Postgres};

use crate::errors::AppError;

type DisplayFromstr = As<DisplayFromStr>;

#[derive(Serialize, Debug)]
struct StampInfo {
    id: i64,
    #[serde(with = "DisplayFromstr")]
    guild_id: i64,
    #[serde(with = "DisplayFromstr")]
    channel_id: i64,
    #[serde(with = "DisplayFromstr")]
    target_user_id: i64,
    #[serde(with = "DisplayFromstr")]
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

#[get("/stamps/{guild_id}")]
pub async fn get_stamps(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
) -> Result<impl Responder, AppError> {
    let guild_id = path.into_inner();

    let rows = sqlx::query_as!(
        StampInfo,
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

    Ok(HttpResponse::Ok().json(rows))
}
