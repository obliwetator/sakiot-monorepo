use crate::auth::discord::parse_discord_response;
use crate::auth::{Access, AuthKind, BASE_URL, Token};
use crate::errors::AppError;
use actix_web::{
    HttpRequest, HttpResponse, Responder, get,
    web::{self, ReqData},
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_with::{As, DisplayFromStr};
use sqlx::{Pool, Postgres};

pub type DisplayFromstr = As<DisplayFromStr>;

#[derive(Debug, Serialize, Deserialize)]
pub struct User {
    #[serde(with = "DisplayFromstr")]
    pub id: i64,
    pub username: String,
    // Discord returns `null` for accounts without an uploaded avatar; the
    // frontend resolves an empty hash to the deterministic default avatar.
    pub avatar: Option<String>,
}

pub async fn get_user(
    client: web::Data<Client>,
    access_token: &str,
    pool: &web::Data<Pool<Postgres>>,
) -> Result<User, AppError> {
    let user = parse_discord_response(
        "users_me",
        client
            .get(format!("{}users/@me", BASE_URL))
            .bearer_auth(access_token),
        false,
    )
    .await?;
    insert_user_db(&user, pool).await?;
    Ok(user)
}

pub async fn insert_user_db(user: &User, pool: &web::Data<Pool<Postgres>>) -> Result<(), AppError> {
    sqlx::query!(
        "INSERT INTO discord_auth_user (id, username, avatar) VALUES ($1,$2,$3) \
         ON CONFLICT (id) DO UPDATE SET username=EXCLUDED.username, avatar=EXCLUDED.avatar",
        user.id,
        user.username,
        user.avatar.as_deref().unwrap_or_default()
    )
    .execute(pool.get_ref())
    .await?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UserDataForFrontEnd {
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub user_id: i64,
    pub username: String,
    pub avatar: String,
    pub is_dev: bool,
}

#[utoipa::path(
    get,
    path = "/api/users/current",
    tag = "user",
    responses(
        (status = 200, description = "Current authenticated user", body = UserDataForFrontEnd),
        (status = 401, description = "Missing or invalid access_token cookie"),
        (status = 403, description = "Token not attached to request context"),
    ),
    security(("access_token" = [])),
)]
#[get("/users/current")]
pub async fn get_current_user(
    _req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    cfg: web::Data<crate::config::Config>,
    token: Option<ReqData<Token<Access>>>,
) -> Result<impl Responder, AppError> {
    let token_data = token.ok_or_else(|| AppError::Forbidden)?;
    let dev_account_id = cfg.dev_account_id;
    let is_dev = token_data.user_id == dev_account_id
        && dev_account_id != 0
        && token_data.auth_kind == AuthKind::Dev;

    let result = sqlx::query!(
        "SELECT id, username, avatar FROM discord_auth_user WHERE id = $1",
        token_data.user_id
    )
    .fetch_one(pool.get_ref())
    .await?;

    let user_data = UserDataForFrontEnd {
        user_id: result.id,
        username: result.username,
        avatar: result.avatar,
        is_dev,
    };

    Ok(HttpResponse::Ok().json(user_data))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct GuildDataForFrontEnd {
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub id: i64,
    pub name: String,
    pub icon: Option<String>,
    /// What the viewer may do across the guild.
    pub capabilities: Vec<crate::permissions::Capability>,
}

#[utoipa::path(
    get,
    path = "/api/users/current/guilds",
    tag = "user",
    responses(
        (status = 200, description = "Guilds visible to current user", body = [GuildDataForFrontEnd]),
        (status = 403, description = "Token not attached to request context", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/users/current/guilds")]
pub async fn get_current_user_guilds(
    _req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    token: Option<ReqData<Token<Access>>>,
) -> Result<impl Responder, AppError> {
    let token_data = token.ok_or_else(|| AppError::Forbidden)?;
    let guilds = member_guilds(pool.get_ref(), token_data.viewer()).await?;
    Ok(HttpResponse::Ok().json(guilds))
}

struct CandidateGuild {
    id: i64,
    name: String,
    icon: Option<String>,
}

/// The guilds the bot is in that the viewer belongs to, with their names and
/// icons and what the viewer may do there.
///
/// The candidates are the guilds the viewer owns or is listed in, or for a
/// dev login those it is seeded in, under their seeded names. Membership then
/// decides, as for every other check: a guild whose roster is not complete
/// yet is left out until it is.
async fn member_guilds(
    pool: &Pool<Postgres>,
    viewer: crate::permissions::Viewer,
) -> Result<Vec<GuildDataForFrontEnd>, AppError> {
    let candidates = if viewer.dev {
        sqlx::query_as!(
            CandidateGuild,
            r#"SELECT ug.id, ug.name, ug.icon
                 FROM guilds_present gp
                 JOIN user_guilds ug ON ug.id = gp.guild_id
                WHERE ug.user_id = $1
                ORDER BY lower(ug.name), ug.id"#,
            viewer.user_id
        )
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as!(
            CandidateGuild,
            r#"SELECT g.id,
                      COALESCE(g.name, g.id::text) AS "name!",
                      g.icon
                 FROM guilds_present gp
                 JOIN guilds g ON g.id = gp.guild_id
                WHERE g.owner_id = $1
                   OR EXISTS (SELECT 1
                                FROM guild_members m
                               WHERE m.guild_id = g.id AND m.user_id = $1)
                ORDER BY lower(COALESCE(g.name, '')), g.id"#,
            viewer.user_id
        )
        .fetch_all(pool)
        .await?
    };
    let ids: Vec<i64> = candidates.iter().map(|guild| guild.id).collect();
    let mut access = crate::permissions::GuildAccess::load_many(pool, &ids, viewer).await?;
    let mut guilds = Vec::with_capacity(candidates.len());
    for guild in candidates {
        let Some(access) = access.remove(&guild.id) else {
            continue;
        };
        let access = match access.access() {
            Ok(access) => access,
            Err(AppError::MembershipUnavailable) => continue,
            Err(error) => return Err(error),
        };
        if access.membership == sakiot_permissions::Membership::Outsider {
            continue;
        }
        guilds.push(GuildDataForFrontEnd {
            id: guild.id,
            name: guild.name,
            icon: guild.icon,
            capabilities: crate::permissions::Capability::named(access.guild),
        });
    }
    Ok(guilds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn login_refreshes_minimal_profile(
        pool: sqlx::PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let pool = web::Data::new(pool);
        let mut user = User {
            id: 123,
            username: "old-name".into(),
            avatar: Some("old-avatar".into()),
        };
        insert_user_db(&user, &pool).await?;
        user.username = "new-name".into();
        user.avatar = None;
        insert_user_db(&user, &pool).await?;
        let (username, avatar): (String, String) =
            sqlx::query_as("SELECT username, avatar FROM discord_auth_user WHERE id=$1")
                .bind(user.id)
                .fetch_one(pool.get_ref())
                .await?;
        assert_eq!(username, "new-name");
        assert_eq!(avatar, "");
        sqlx::query(
            "UPDATE discord_auth_user SET email='old@example.invalid', flags=7 WHERE id=$1",
        )
        .bind(user.id)
        .execute(pool.get_ref())
        .await?;
        let (email, flags): (Option<String>, Option<i32>) =
            sqlx::query_as("SELECT email, flags FROM discord_auth_user WHERE id=$1")
                .bind(user.id)
                .fetch_one(pool.get_ref())
                .await?;
        assert_eq!((email, flags), (None, None));

        let current = UserDataForFrontEnd {
            user_id: user.id,
            username,
            avatar,
            is_dev: false,
        };
        let value = serde_json::to_value(current)?;
        assert_eq!(value.as_object().unwrap().len(), 4);
        assert!(value.get("email").is_none());
        Ok(())
    }
}
