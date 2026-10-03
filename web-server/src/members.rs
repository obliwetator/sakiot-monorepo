use actix_web::{HttpRequest, HttpResponse, get, web};
use serde::{Deserialize, Serialize};
use serde_with::{As, DisplayFromStr};
use sqlx::{Pool, Postgres};

use crate::errors::AppError;
use crate::permissions::{
    begin_snapshot, get_combined_perm_for_role, require_guild_manager, role_access_for_preview,
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
    /// Discord never lists `@everyone` among a member's roles, so its count
    /// comes from the member roster: null until a complete roster exists.
    pub member_count: Option<i64>,
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

    let mut snapshot = begin_snapshot(&pool).await?;
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
    .fetch_all(&mut *snapshot)
    .await?;
    let everyone_count = complete_roster_size(&mut snapshot, guild_id).await?;
    snapshot.commit().await?;

    let roles: Vec<GuildRole> = rows
        .into_iter()
        .map(|r| GuildRole {
            role_id: r.role_id,
            name: r.name,
            permission: r.permission,
            member_count: if r.role_id == guild_id {
                everyone_count
            } else {
                Some(r.member_count.unwrap_or(0))
            },
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

    let belongs_to_guild = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM roles WHERE role_id = $1 AND guild_id = $2) AS "exists!""#,
        role_id,
        guild_id
    )
    .fetch_one(pool.get_ref())
    .await?;
    if !belongs_to_guild {
        return Err(AppError::RoleNotFound);
    }

    // `@everyone` holds every member, so it lists the roster.
    let members: Vec<RoleMember> = if role_id == guild_id {
        sqlx::query!(
            r#"SELECT user_id,
                      COALESCE(nickname, global_name, username) AS "name!"
                 FROM guild_members
                WHERE guild_id = $1
                ORDER BY lower(COALESCE(nickname, global_name, username)), user_id"#,
            guild_id
        )
        .fetch_all(pool.get_ref())
        .await?
        .into_iter()
        .map(|r| RoleMember {
            user_id: r.user_id,
            name: Some(r.name),
        })
        .collect()
    } else {
        sqlx::query!(
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
        .await?
        .into_iter()
        .map(|r| RoleMember {
            user_id: r.user_id,
            name: r.name,
        })
        .collect()
    };

    Ok(HttpResponse::Ok().json(members))
}

#[derive(Serialize, Debug, utoipa::ToSchema)]
pub struct RoleChannel {
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub channel_id: i64,
    pub name: String,
    pub can_view: bool,
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
    let manager = crate::permissions::Viewer::of_request(&req)?;

    // Lists the role's access for every voice channel the manager can view
    // and join themselves, including ones the role cannot see at all; a
    // preview never shows more than the manager's own view.
    let access = role_access_for_preview(&pool, guild_id, manager, role_id).await?;
    let permission = get_combined_perm_for_role(&pool, guild_id, role_id).await?;
    let all_ids: Vec<i64> = access.keys().copied().collect();
    let rows = sqlx::query!(
        "SELECT channel_id, name FROM channels WHERE channel_id = ANY($1)",
        &all_ids
    )
    .fetch_all(pool.get_ref())
    .await?;

    let mut channels: Vec<RoleChannel> = rows
        .into_iter()
        .filter_map(|r| {
            let a = access.get(&r.channel_id)?;
            Some(RoleChannel {
                channel_id: r.channel_id,
                name: r.name.unwrap_or_default(),
                can_view: a.viewable,
                can_join: a.joinable,
            })
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

/// The roster's size once a complete roster has been written (a full member
/// list from Discord); `None` before. A complete roster stays the answer
/// while the bot is away: it is the last complete count, not a partial one.
async fn complete_roster_size(
    conn: &mut sqlx::PgConnection,
    guild_id: i64,
) -> Result<Option<i64>, AppError> {
    Ok(sqlx::query_scalar!(
        r#"SELECT (SELECT COUNT(*) FROM guild_members WHERE guild_id = s.guild_id) AS "count!"
             FROM guild_projection_state s
            WHERE s.guild_id = $1 AND s.roster_complete_at IS NOT NULL"#,
        guild_id
    )
    .fetch_optional(&mut *conn)
    .await?)
}

const MEMBER_PAGE_DEFAULT: i64 = 20;
const MEMBER_PAGE_MAX: i64 = 50;
/// Searching narrows a picker; nobody pages this deep.
const MEMBER_OFFSET_MAX: i64 = 1_000;

#[derive(Deserialize, Debug, utoipa::IntoParams)]
pub struct MemberSearchQuery {
    /// Part of a nickname, display name or username, or the start of a user
    /// id. Omitted: everyone, by name.
    pub q: Option<String>,
    /// 1 to 50; 20 when omitted.
    pub limit: Option<i64>,
    /// 0 to 1000; from `next_offset` of the previous page.
    pub offset: Option<i64>,
}

#[derive(Serialize, Debug, PartialEq, Eq, utoipa::ToSchema)]
pub struct GuildMember {
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub user_id: i64,
    /// Nickname, display name or username, whichever is set first.
    pub name: String,
    pub username: String,
    pub is_bot: bool,
}

#[derive(Serialize, Debug, PartialEq, Eq, utoipa::ToSchema)]
pub struct MemberPage {
    /// Whether the roster has ever been complete. While false, members the
    /// bot has not seen yet are missing; enter their id by hand.
    pub complete: bool,
    pub members: Vec<GuildMember>,
    pub next_offset: Option<i64>,
}

/// `value` as a literal inside an ILIKE pattern.
fn like_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

#[utoipa::path(
    get,
    path = "/api/admin/guilds/{guild_id}/members",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id"), MemberSearchQuery),
    responses(
        (status = 200, description = "Matching guild members, by name", body = MemberPage),
        (status = 400, description = "Invalid limit or offset", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "User cannot manage this guild", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/admin/guilds/{guild_id}/members")]
pub async fn search_guild_members(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    query: web::Query<MemberSearchQuery>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    require_guild_manager(&req, &pool, guild_id).await?;

    let limit = query.limit.unwrap_or(MEMBER_PAGE_DEFAULT);
    if !(1..=MEMBER_PAGE_MAX).contains(&limit) {
        return Err(AppError::InvalidParam("limit".into()));
    }
    let offset = query.offset.unwrap_or(0);
    if !(0..=MEMBER_OFFSET_MAX).contains(&offset) {
        return Err(AppError::InvalidParam("offset".into()));
    }
    let search = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|search| !search.is_empty());
    let id_prefix = search
        .filter(|search| search.chars().all(|character| character.is_ascii_digit()))
        .map(|digits| format!("{digits}%"));
    let pattern = search.map(|search| format!("%{}%", like_escape(search)));

    let mut snapshot = begin_snapshot(&pool).await?;
    let complete = complete_roster_size(&mut snapshot, guild_id)
        .await?
        .is_some();
    let mut members: Vec<GuildMember> = sqlx::query!(
        r#"SELECT user_id,
                  COALESCE(nickname, global_name, username) AS "name!",
                  username,
                  is_bot
             FROM guild_members
            WHERE guild_id = $1
              AND ($2::text IS NULL
                   OR nickname ILIKE $2 OR global_name ILIKE $2 OR username ILIKE $2
                   OR ($3::text IS NOT NULL AND user_id::text LIKE $3))
            ORDER BY lower(COALESCE(nickname, global_name, username)), user_id
            LIMIT $4 OFFSET $5"#,
        guild_id,
        pattern,
        id_prefix,
        limit + 1,
        offset
    )
    .fetch_all(&mut *snapshot)
    .await?
    .into_iter()
    .map(|row| GuildMember {
        user_id: row.user_id,
        name: row.name,
        username: row.username,
        is_bot: row.is_bot,
    })
    .collect();
    snapshot.commit().await?;

    let more = i64::try_from(members.len()).unwrap_or(i64::MAX) > limit;
    members.truncate(usize::try_from(limit).unwrap_or_default());
    let next_offset = (more && offset + limit <= MEMBER_OFFSET_MAX).then_some(offset + limit);
    Ok(HttpResponse::Ok().json(MemberPage {
        complete,
        members,
        next_offset,
    }))
}

#[cfg(test)]
mod tests {
    use super::like_escape;

    #[test]
    fn search_text_is_matched_literally() {
        assert_eq!(like_escape("50%_off\\"), "50\\%\\_off\\\\");
        assert_eq!(like_escape("plain"), "plain");
    }
}
