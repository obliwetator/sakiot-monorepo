use actix_web::{HttpRequest, HttpResponse, get, put, web};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres, Row};

use crate::{errors::AppError, permissions::require_guild_manager};

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct GuildRecordingPolicy {
    pub retention_days: Option<i32>,
    pub excluded_channel_ids: Vec<String>,
    pub channels: Vec<VoiceChannel>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct VoiceChannel {
    pub id: String,
    pub name: String,
}

async fn voice_channels(
    pool: &Pool<Postgres>,
    guild_id: i64,
) -> Result<Vec<VoiceChannel>, AppError> {
    let rows = sqlx::query("SELECT channel_id,name FROM channels WHERE guild_id=$1 AND type IN (2,13) ORDER BY name,channel_id")
        .bind(guild_id).fetch_all(pool).await?;
    rows.into_iter()
        .map(|row| {
            Ok(VoiceChannel {
                id: row.try_get::<i64, _>("channel_id")?.to_string(),
                name: row
                    .try_get::<Option<String>, _>("name")?
                    .unwrap_or_else(|| "Unnamed voice channel".into()),
            })
        })
        .collect()
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct GuildRecordingPolicyBody {
    /// Null disables automatic deletion. Existing recordings are untouched by default.
    pub retention_days: Option<i32>,
    pub excluded_channel_ids: Vec<String>,
}

fn validate(body: &GuildRecordingPolicyBody) -> Result<Vec<i64>, AppError> {
    if body
        .retention_days
        .is_some_and(|days| !(1..=3650).contains(&days))
    {
        return Err(AppError::BadRequest(
            "retention_days must be 1–3650 or null".into(),
        ));
    }
    if body.excluded_channel_ids.len() > 100 {
        return Err(AppError::BadRequest(
            "No more than 100 channels may be excluded".into(),
        ));
    }
    let mut ids = Vec::with_capacity(body.excluded_channel_ids.len());
    for value in &body.excluded_channel_ids {
        let id = value
            .parse::<i64>()
            .map_err(|_| AppError::BadRequest("Invalid channel id".into()))?;
        if id <= 0 || ids.contains(&id) {
            return Err(AppError::BadRequest(
                "Channel ids must be positive and unique".into(),
            ));
        }
        ids.push(id);
    }
    Ok(ids)
}

#[utoipa::path(
    get,
    path = "/api/admin/guilds/{guild_id}/recording-policy",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "Guild recording policy", body = GuildRecordingPolicy),
        (status = 403, description = "Manage Guild required", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/admin/guilds/{guild_id}/recording-policy")]
pub async fn get_recording_policy(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    require_guild_manager(&req, &pool, guild_id).await?;
    let channels = voice_channels(&pool, guild_id).await?;
    let row = sqlx::query(
        "SELECT retention_days, excluded_channel_ids FROM guild_recording_policy WHERE guild_id=$1",
    )
    .bind(guild_id)
    .fetch_optional(pool.get_ref())
    .await?;
    let policy = match row {
        Some(row) => GuildRecordingPolicy {
            retention_days: row.try_get("retention_days")?,
            excluded_channel_ids: row
                .try_get::<Vec<i64>, _>("excluded_channel_ids")?
                .into_iter()
                .map(|id| id.to_string())
                .collect(),
            channels: channels.clone(),
            is_default: false,
        },
        None => GuildRecordingPolicy {
            retention_days: None,
            excluded_channel_ids: Vec::new(),
            channels,
            is_default: true,
        },
    };
    Ok(HttpResponse::Ok().json(policy))
}

#[utoipa::path(
    put,
    path = "/api/admin/guilds/{guild_id}/recording-policy",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    request_body = GuildRecordingPolicyBody,
    responses(
        (status = 200, description = "Updated recording policy", body = GuildRecordingPolicy),
        (status = 400, description = "Invalid retention or channel policy", body = crate::errors::ApiError),
        (status = 403, description = "Manage Guild required", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[put("/admin/guilds/{guild_id}/recording-policy")]
pub async fn put_recording_policy(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    body: web::Json<GuildRecordingPolicyBody>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    let user_id = require_guild_manager(&req, &pool, guild_id).await?;
    let ids = validate(&body)?;
    let mut tx = pool.begin().await?;
    // The bot takes the same lock before opening a recording fragment. A
    // policy update and a new fragment therefore cannot cross unnoticed.
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(guild_id)
        .execute(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM channels WHERE guild_id=$1 AND channel_id=ANY($2) AND type IN (2,13)",
    )
    .bind(guild_id)
    .bind(&ids)
    .fetch_one(&mut *tx)
    .await?;
    if count != ids.len() as i64 {
        return Err(AppError::BadRequest(
            "Excluded channels must be voice channels in this server".into(),
        ));
    }
    sqlx::query("INSERT INTO guild_recording_policy (guild_id,retention_days,excluded_channel_ids,updated_by) VALUES ($1,$2,$3,$4) ON CONFLICT (guild_id) DO UPDATE SET retention_days=EXCLUDED.retention_days,excluded_channel_ids=EXCLUDED.excluded_channel_ids,updated_by=EXCLUDED.updated_by,updated_at=now()")
        .bind(guild_id).bind(body.retention_days).bind(&ids).bind(user_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(HttpResponse::Ok().json(GuildRecordingPolicy {
        retention_days: body.retention_days,
        excluded_channel_ids: ids.into_iter().map(|id| id.to_string()).collect(),
        channels: voice_channels(&pool, guild_id).await?,
        is_default: false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_retention_and_duplicate_channel_ids() {
        assert!(
            validate(&GuildRecordingPolicyBody {
                retention_days: Some(0),
                excluded_channel_ids: vec![]
            })
            .is_err()
        );
        assert!(
            validate(&GuildRecordingPolicyBody {
                retention_days: None,
                excluded_channel_ids: vec!["1".into(), "1".into()]
            })
            .is_err()
        );
        assert_eq!(
            validate(&GuildRecordingPolicyBody {
                retention_days: Some(30),
                excluded_channel_ids: vec!["1".into()]
            })
            .unwrap(),
            vec![1]
        );
    }
}
