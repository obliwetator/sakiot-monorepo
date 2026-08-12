use actix_web::{HttpRequest, HttpResponse, get, web};
use serde::Serialize;
use serde_with::{As, DisplayFromStr};
use sqlx::{Pool, Postgres};

use crate::errors::AppError;
use crate::permissions::{
    get_channel_access_for_role, get_combined_perm_for_role, require_guild_manager,
};

type DisplayFromstr = As<DisplayFromStr>;

#[derive(Serialize, Debug, utoipa::ToSchema)]
pub struct GuildRole {
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub role_id: i64,
    pub name: String,
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "268435456")]
    pub permission: i64,
    pub member_count: i64,
    #[schema(example = 16711680)]
    pub color: i64,
    #[schema(example = 65280)]
    pub color_secondary: Option<i64>,
    #[schema(example = 255)]
    pub color_tertiary: Option<i64>,
}

#[derive(Serialize, Debug, utoipa::ToSchema)]
pub struct RoleMember {
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub user_id: i64,
    pub name: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/admin/guilds/{guild_id}/roles",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "Guild roles with member counts", body = [GuildRole]),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "User cannot manage this guild", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/admin/guilds/{guild_id}/roles")]
pub async fn get_guild_roles(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    require_guild_manager(&req, &pool, guild_id).await?;

    let rows = sqlx::query!(
        "SELECT r.role_id,
                r.name,
                r.permission,
                r.color,
                r.color_secondary,
                r.color_tertiary,
                COALESCE(COUNT(ur.user_id), 0)::bigint AS member_count
         FROM roles r
         LEFT JOIN user_roles ur ON ur.role_id = r.role_id
         WHERE r.guild_id = $1
         GROUP BY r.role_id, r.name, r.permission, r.color, r.color_secondary, r.color_tertiary
         ORDER BY r.role_id = $1, r.role_id",
        guild_id
    )
    .fetch_all(pool.get_ref())
    .await?;

    let roles: Vec<GuildRole> = rows
        .into_iter()
        .map(|r| GuildRole {
            role_id: r.role_id,
            name: r.name,
            permission: r.permission,
            member_count: r.member_count.unwrap_or(0),
            color: r.color,
            color_secondary: r.color_secondary,
            color_tertiary: r.color_tertiary,
        })
        .collect();

    Ok(HttpResponse::Ok().json(roles))
}

#[utoipa::path(
    get,
    path = "/api/admin/guilds/{guild_id}/roles/{role_id}/members",
    tag = "admin",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("role_id" = i64, Path, description = "Discord role id"),
    ),
    responses(
        (status = 200, description = "Members holding the role", body = [RoleMember]),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "User cannot manage this guild", body = crate::errors::ApiError),
        (status = 404, description = "Role does not exist in this guild", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/admin/guilds/{guild_id}/roles/{role_id}/members")]
pub async fn get_role_members(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, role_id) = path.into_inner();
    require_guild_manager(&req, &pool, guild_id).await?;

    let belongs_to_guild = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM roles WHERE role_id = $1 AND guild_id = $2)",
    )
    .bind(role_id)
    .bind(guild_id)
    .fetch_one(pool.get_ref())
    .await?;
    if !belongs_to_guild {
        return Err(AppError::RoleNotFound);
    }

    let rows = sqlx::query!(
        "SELECT ur.user_id,
                COALESCE(nn.nickname, un.global_name, un.username) AS name
         FROM user_roles ur
         LEFT JOIN user_names     un ON un.user_id = ur.user_id
         LEFT JOIN user_nicknames nn ON nn.user_id = ur.user_id AND nn.guild_id = $1
         WHERE ur.role_id = $2
         ORDER BY name NULLS LAST, ur.user_id",
        guild_id,
        role_id
    )
    .fetch_all(pool.get_ref())
    .await?;

    let members: Vec<RoleMember> = rows
        .into_iter()
        .map(|r| RoleMember {
            user_id: r.user_id,
            name: r.name,
        })
        .collect();

    Ok(HttpResponse::Ok().json(members))
}

#[derive(Serialize, Debug, utoipa::ToSchema)]
pub struct RoleChannel {
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub channel_id: i64,
    pub name: String,
    pub can_join: bool,
}

#[derive(Serialize, Debug, utoipa::ToSchema)]
pub struct RoleView {
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "1024")]
    pub permission: i64,
    pub can_manage_guild: bool,
    pub channels: Vec<RoleChannel>,
}

#[utoipa::path(
    get,
    path = "/api/admin/guilds/{guild_id}/roles/{role_id}/channels",
    tag = "admin",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("role_id" = i64, Path, description = "Discord role id"),
    ),
    responses(
        (status = 200, description = "What a member with only this role can see", body = RoleView),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "User cannot manage this guild", body = crate::errors::ApiError),
        (status = 404, description = "Role does not exist in this guild", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/admin/guilds/{guild_id}/roles/{role_id}/channels")]
pub async fn get_role_view(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, role_id) = path.into_inner();
    require_guild_manager(&req, &pool, guild_id).await?;

    let belongs_to_guild = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM roles WHERE role_id = $1 AND guild_id = $2)",
    )
    .bind(role_id)
    .bind(guild_id)
    .fetch_one(pool.get_ref())
    .await?;
    if !belongs_to_guild {
        return Err(AppError::RoleNotFound);
    }

    let permission = get_combined_perm_for_role(&pool, guild_id, role_id).await?;
    let access = get_channel_access_for_role(&pool, guild_id, role_id).await?;
    let visible: Vec<i64> = access
        .iter()
        .filter(|a| a.viewable)
        .map(|a| a.channel_id)
        .collect();
    let can_join: std::collections::HashSet<i64> = access
        .into_iter()
        .filter(|a| a.joinable)
        .map(|a| a.channel_id)
        .collect();
    let rows = sqlx::query!(
        "SELECT channel_id, name FROM channels WHERE channel_id = ANY($1)",
        &visible
    )
    .fetch_all(pool.get_ref())
    .await?;

    let mut channels: Vec<RoleChannel> = rows
        .into_iter()
        .map(|r| RoleChannel {
            channel_id: r.channel_id,
            name: r.name.unwrap_or_default(),
            can_join: can_join.contains(&r.channel_id),
        })
        .collect();
    channels.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(HttpResponse::Ok().json(RoleView {
        permission: permission.bits(),
        can_manage_guild: permission.intersects(
            crate::permissions::Permissions::ADMINISTRATOR
                | crate::permissions::Permissions::MANAGE_GUILD,
        ),
        channels,
    }))
}
