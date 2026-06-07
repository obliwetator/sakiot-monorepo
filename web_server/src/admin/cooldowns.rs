use actix_web::{delete, get, put, web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres};

use crate::errors::AppError;
use crate::permissions::require_guild_manager;

#[derive(Serialize, utoipa::ToSchema)]
pub struct GuildCooldown {
    pub cooldown_seconds: i32,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CooldownBody {
    pub cooldown_seconds: i32,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct UserOverride {
    pub user_id: i64,
    pub cooldown_seconds: i32,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

fn validate_seconds(secs: i32) -> Result<(), AppError> {
    if secs < 0 {
        Err(AppError::BadRequest(
            "cooldown_seconds must be >= 0".to_string(),
        ))
    } else {
        Ok(())
    }
}

#[utoipa::path(
    get,
    path = "/api/admin/guilds/{guild_id}/cooldown",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "Guild cooldown", body = GuildCooldown),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "User cannot manage this guild", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[get("/admin/guilds/{guild_id}/cooldown")]
pub async fn get_guild_cooldown(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    require_guild_manager(&req, &pool, guild_id).await?;

    let row = sqlx::query!(
        "SELECT cooldown_seconds FROM guild_jam_cooldowns WHERE guild_id = $1",
        guild_id
    )
    .fetch_optional(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(GuildCooldown {
        cooldown_seconds: row.map(|r| r.cooldown_seconds).unwrap_or(0),
    }))
}

#[utoipa::path(
    put,
    path = "/api/admin/guilds/{guild_id}/cooldown",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    request_body = CooldownBody,
    responses(
        (status = 204, description = "Guild cooldown updated"),
        (status = 400, description = "Invalid cooldown", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "User cannot manage this guild", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[put("/admin/guilds/{guild_id}/cooldown")]
pub async fn set_guild_cooldown(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    body: web::Json<CooldownBody>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    require_guild_manager(&req, &pool, guild_id).await?;
    validate_seconds(body.cooldown_seconds)?;

    sqlx::query!(
        "INSERT INTO guild_jam_cooldowns (guild_id, cooldown_seconds, updated_at)
         VALUES ($1, $2, NOW())
         ON CONFLICT (guild_id) DO UPDATE
         SET cooldown_seconds = EXCLUDED.cooldown_seconds, updated_at = NOW()",
        guild_id,
        body.cooldown_seconds
    )
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::NoContent().finish())
}

#[utoipa::path(
    get,
    path = "/api/admin/guilds/{guild_id}/cooldown/overrides",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "User cooldown overrides", body = [UserOverride]),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "User cannot manage this guild", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/admin/guilds/{guild_id}/cooldown/overrides")]
pub async fn list_user_overrides(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    require_guild_manager(&req, &pool, guild_id).await?;

    let rows = sqlx::query!(
        "SELECT user_id, cooldown_seconds, updated_at
         FROM user_jam_cooldown_overrides
         WHERE guild_id = $1
         ORDER BY updated_at DESC",
        guild_id
    )
    .fetch_all(pool.get_ref())
    .await?;

    let out: Vec<UserOverride> = rows
        .into_iter()
        .map(|r| UserOverride {
            user_id: r.user_id,
            cooldown_seconds: r.cooldown_seconds,
            updated_at: r.updated_at,
        })
        .collect();

    Ok(HttpResponse::Ok().json(out))
}

#[utoipa::path(
    put,
    path = "/api/admin/guilds/{guild_id}/cooldown/overrides/{user_id}",
    tag = "admin",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("user_id" = i64, Path, description = "Discord user id"),
    ),
    request_body = CooldownBody,
    responses(
        (status = 204, description = "User cooldown override updated"),
        (status = 400, description = "Invalid cooldown", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "User cannot manage this guild", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[put("/admin/guilds/{guild_id}/cooldown/overrides/{user_id}")]
pub async fn set_user_override(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, i64)>,
    body: web::Json<CooldownBody>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, user_id) = path.into_inner();
    require_guild_manager(&req, &pool, guild_id).await?;
    validate_seconds(body.cooldown_seconds)?;

    sqlx::query!(
        "INSERT INTO user_jam_cooldown_overrides (guild_id, user_id, cooldown_seconds, updated_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (guild_id, user_id) DO UPDATE
         SET cooldown_seconds = EXCLUDED.cooldown_seconds, updated_at = NOW()",
        guild_id,
        user_id,
        body.cooldown_seconds
    )
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::NoContent().finish())
}

#[utoipa::path(
    delete,
    path = "/api/admin/guilds/{guild_id}/cooldown/overrides/{user_id}",
    tag = "admin",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("user_id" = i64, Path, description = "Discord user id"),
    ),
    responses(
        (status = 204, description = "User cooldown override deleted"),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "User cannot manage this guild", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[delete("/admin/guilds/{guild_id}/cooldown/overrides/{user_id}")]
pub async fn delete_user_override(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, user_id) = path.into_inner();
    require_guild_manager(&req, &pool, guild_id).await?;

    sqlx::query!(
        "DELETE FROM user_jam_cooldown_overrides WHERE guild_id = $1 AND user_id = $2",
        guild_id,
        user_id
    )
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::NoContent().finish())
}
