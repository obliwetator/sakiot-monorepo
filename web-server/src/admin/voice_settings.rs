use actix_web::{HttpRequest, HttpResponse, delete, get, put, web};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres};

use crate::errors::AppError;
use crate::permissions::require_guild_manager;

pub const DEFAULT_PENDING_CAP_SECONDS: i32 = 6 * 60 * 60;
pub const MIN_PENDING_CAP_SECONDS: i32 = 60;

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct GuildVoiceSettings {
    pub pending_cap_seconds: i32,
    pub is_default: bool,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct GuildVoiceSettingsBody {
    pub pending_cap_seconds: i32,
}

#[utoipa::path(
    get,
    path = "/api/admin/guilds/{guild_id}/voice-settings",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "Effective guild voice settings", body = GuildVoiceSettings),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "Manage Guild required", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/admin/guilds/{guild_id}/voice-settings")]
pub async fn get_voice_settings(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    require_guild_manager(&req, &pool, guild_id).await?;
    let configured = sqlx::query_scalar::<_, i32>(
        "SELECT pending_cap_seconds FROM guild_voice_settings WHERE guild_id = $1",
    )
    .bind(guild_id)
    .fetch_optional(pool.get_ref())
    .await?;
    Ok(HttpResponse::Ok().json(GuildVoiceSettings {
        pending_cap_seconds: configured.unwrap_or(DEFAULT_PENDING_CAP_SECONDS),
        is_default: configured.is_none(),
    }))
}

#[utoipa::path(
    put,
    path = "/api/admin/guilds/{guild_id}/voice-settings",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    request_body = GuildVoiceSettingsBody,
    responses(
        (status = 200, description = "Updated guild voice settings", body = GuildVoiceSettings),
        (status = 400, description = "Pending cap below 60 seconds", body = crate::errors::ApiError),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "Manage Guild required", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[put("/admin/guilds/{guild_id}/voice-settings")]
pub async fn put_voice_settings(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    body: web::Json<GuildVoiceSettingsBody>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    let user_id = require_guild_manager(&req, &pool, guild_id).await?;
    if body.pending_cap_seconds < MIN_PENDING_CAP_SECONDS {
        return Err(AppError::BadRequest(format!(
            "pending_cap_seconds must be at least {MIN_PENDING_CAP_SECONDS}"
        )));
    }

    sqlx::query(
        "INSERT INTO guild_voice_settings
            (guild_id, pending_cap_seconds, updated_by, updated_at)
         VALUES ($1, $2, $3, now())
         ON CONFLICT (guild_id) DO UPDATE
            SET pending_cap_seconds = EXCLUDED.pending_cap_seconds,
                updated_by = EXCLUDED.updated_by,
                updated_at = now()",
    )
    .bind(guild_id)
    .bind(body.pending_cap_seconds)
    .bind(user_id)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(GuildVoiceSettings {
        pending_cap_seconds: body.pending_cap_seconds,
        is_default: false,
    }))
}

#[utoipa::path(
    delete,
    path = "/api/admin/guilds/{guild_id}/voice-settings",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "Default guild voice settings restored", body = GuildVoiceSettings),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "Manage Guild required", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[delete("/admin/guilds/{guild_id}/voice-settings")]
pub async fn delete_voice_settings(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    require_guild_manager(&req, &pool, guild_id).await?;
    sqlx::query("DELETE FROM guild_voice_settings WHERE guild_id = $1")
        .bind(guild_id)
        .execute(pool.get_ref())
        .await?;
    Ok(HttpResponse::Ok().json(GuildVoiceSettings {
        pending_cap_seconds: DEFAULT_PENDING_CAP_SECONDS,
        is_default: true,
    }))
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_PENDING_CAP_SECONDS, MIN_PENDING_CAP_SECONDS};

    #[test]
    fn defaults_and_minimum_match_recording_policy() {
        assert_eq!(DEFAULT_PENDING_CAP_SECONDS, 21_600);
        assert_eq!(MIN_PENDING_CAP_SECONDS, 60);
    }
}
