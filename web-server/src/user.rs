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
    /// Whether this server serves `/api/realtime`. Clients must not open a
    /// socket when it is false (or absent, from an older server) and keep
    /// polling instead.
    pub realtime_enabled: bool,
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
        realtime_enabled: cfg.realtime_enabled,
    };

    Ok(HttpResponse::Ok().json(user_data))
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GuildDataForFrontEnd {
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub id: i64,
    pub name: String,
    pub icon: Option<String>,
    pub owner: bool,
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "268435456")]
    pub permissions: i64,
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
    cfg: web::Data<crate::config::Config>,
    token: Option<ReqData<Token<Access>>>,
) -> Result<impl Responder, AppError> {
    let token_data = token.ok_or_else(|| AppError::Forbidden)?;
    let dev_account_id = cfg.dev_account_id;

    let result = if token_data.user_id == dev_account_id
        && dev_account_id != 0
        && token_data.auth_kind == AuthKind::Dev
    {
        sqlx::query_as!(
            GuildDataForFrontEnd,
            "
            SELECT DISTINCT ON (guilds_present.guild_id)
            user_guilds.id as \"id!\",
            user_guilds.name as \"name!\",
            user_guilds.icon as \"icon\",
            true as \"owner!\",
            user_guilds.permissions as \"permissions!\"
            FROM guilds_present
            JOIN user_guilds ON user_guilds.id = guilds_present.guild_id;
            "
        )
        .fetch_all(pool.get_ref())
        .await?
    } else {
        member_guilds(pool.get_ref(), crate::permissions::Viewer::of(&token_data)).await?
    };

    Ok(HttpResponse::Ok().json(result))
}

/// The guilds a Discord login belongs to, as the bot sees them: guilds it is
/// in where the viewer owns the guild or is on a complete roster, with live
/// names and icons and the viewer's calculated permissions. A guild whose
/// roster is not complete yet is left out until it is.
async fn member_guilds(
    pool: &Pool<Postgres>,
    viewer: crate::permissions::Viewer,
) -> Result<Vec<GuildDataForFrontEnd>, AppError> {
    let mut snapshot = crate::permissions::begin_snapshot(pool).await?;
    let rows = sqlx::query!(
        r#"SELECT g.id,
                  COALESCE(g.name, g.id::text) AS "name!",
                  g.icon,
                  g.owner_id = $1 AS "owner!"
             FROM guilds_present gp
             JOIN guilds g ON g.id = gp.guild_id
            WHERE g.owner_id = $1
               OR EXISTS (
                      SELECT 1
                        FROM guild_projection_state s
                        JOIN guild_members m ON m.guild_id = s.guild_id
                       WHERE s.guild_id = g.id
                         AND s.roster_complete_at IS NOT NULL
                         AND m.user_id = $1
                  )
            ORDER BY lower(COALESCE(g.name, '')), g.id"#,
        viewer.user_id
    )
    .fetch_all(&mut *snapshot)
    .await?;
    let mut guilds = Vec::with_capacity(rows.len());
    for row in rows {
        let permissions =
            crate::permissions::combined_perm_for_user(&mut snapshot, row.id, viewer).await?;
        guilds.push(GuildDataForFrontEnd {
            id: row.id,
            name: row.name,
            icon: row.icon,
            owner: row.owner,
            permissions: permissions.bits(),
        });
    }
    snapshot.commit().await?;
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
            realtime_enabled: false,
        };
        let value = serde_json::to_value(current)?;
        assert_eq!(value.as_object().unwrap().len(), 5);
        assert!(value.get("email").is_none());
        Ok(())
    }
}
