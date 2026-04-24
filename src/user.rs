use crate::auth::{Access, Token, BASE_URL};
use crate::errors::AppError;
use actix_web::{
    get,
    web::{self, ReqData},
    HttpRequest, HttpResponse, Responder,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_with::{As, DisplayFromStr};
use sqlx::{Pool, Postgres};

pub type DisplayFromstr = As<DisplayFromStr>;

#[derive(Debug, Serialize, Deserialize)]
pub struct UserGuilds {
    #[serde(with = "DisplayFromstr")]
    pub id: i64,
    pub name: String,
    pub icon: Option<String>,
    pub owner: bool,
    #[serde(with = "DisplayFromstr")]
    pub permissions: i64,
    pub features: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct User {
    #[serde(with = "DisplayFromstr")]
    pub id: i64,
    pub username: String,
    pub discriminator: String,
    pub avatar: String,
    pub bot: Option<bool>,
    pub system: Option<bool>,
    pub mfa_enabled: Option<bool>,
    pub banner: Option<String>,
    pub accent_color: Option<i32>,
    pub locale: Option<String>,
    pub verified: Option<bool>,
    pub email: Option<String>,
    pub flags: Option<i32>,
    pub premium_type: Option<i32>,
    pub public_flags: Option<i32>,
}

pub async fn get_user(
    client: web::Data<Client>,
    access_token: &str,
    pool: &web::Data<Pool<Postgres>>,
) -> Result<User, AppError> {
    let result = client
        .get(format!("{}users/@me", BASE_URL))
        .bearer_auth(access_token)
        .send()
        .await?;

    let user = result.json::<User>().await?;
    insert_user_db(&user, pool).await?;
    Ok(user)
}

pub async fn insert_user_db(user: &User, pool: &web::Data<Pool<Postgres>>) -> Result<(), AppError> {
    // this thing will not format
    // TODO: Update logic
    sqlx::query!(
		"INSERT INTO discord_auth_user (id, username, discriminator, avatar, bot, system, mfa_enabled, banner, accent_color, locale, verified, email, flags, premium_type, public_flags) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15) ON CONFLICT DO NOTHING",
		user.id,
		user.username,
		user.discriminator,
		user.avatar,
		user.bot,
		user.system,
		user.mfa_enabled,
		user.banner,
		user.accent_color,
		user.locale,
		user.verified,
		user.email,
		user.flags,
		user.premium_type,
		user.public_flags
	)
	.execute(pool.get_ref()).await?;
    Ok(())
}

pub async fn insert_user_guilds_db(
    user_guilds: &[UserGuilds],
    pool: &web::Data<Pool<Postgres>>,
    user_id: i64,
) -> Result<(), AppError> {
    // MAYBE: Better to do it in batch?
    futures_util::future::join_all(user_guilds.iter().map(|guild| {
        sqlx::query!(
            "INSERT INTO user_guilds (id, user_id, name, icon, owner, permissions, features) 
            VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT DO NOTHING",
            guild.id,
            user_id,
            guild.name,
            guild.icon,
            guild.owner,
            guild.permissions,
            &guild.features
        )
        .execute(pool.get_ref())
    }))
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;

    Ok(())
}

pub async fn get_user_guilds(
    client: web::Data<Client>,
    access_token: &str,
    user_id: i64,
    pool: &web::Data<Pool<Postgres>>,
) -> Result<Vec<UserGuilds>, AppError> {
    let result = client
        .get(format!("{}users/@me/guilds", BASE_URL))
        .bearer_auth(access_token)
        .send()
        .await?;

    let user_guilds = result.json::<Vec<UserGuilds>>().await?;
    insert_user_guilds_db(&user_guilds, pool, user_id).await?;
    Ok(user_guilds)
}

#[derive(Debug, Serialize, Deserialize)]
struct UserDataForFrontEnd {
    #[serde(with = "DisplayFromstr")]
    pub id: i64,
    pub username: String,
    pub avatar: String,
    pub email: Option<String>,
    pub flags: Option<i32>,
    pub public_flags: Option<i32>,
    pub is_dev: bool,
}

#[get("/users/@me")]
pub async fn get_current_user(
    _req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    token: Option<ReqData<Token<Access>>>,
) -> Result<impl Responder, AppError> {
    let token_data = token.ok_or_else(|| AppError::Forbidden)?;
    let dev_account_id = crate::config::DEV_ACCOUNT_ID.parse::<i64>().unwrap_or(0);
    let is_dev =
        token_data.id == dev_account_id && dev_account_id != 0 && token_data.token == "dev_access";

    let result = sqlx::query!(
        "
    	SELECT id,
        username,
    	avatar,
    	email,
    	flags,
    	public_flags
    	FROM discord_auth_user
    	WHERE id = $1
    	",
        token_data.id
    )
    .fetch_one(pool.get_ref())
    .await?;

    let user_data = UserDataForFrontEnd {
        id: result.id,
        username: result.username,
        avatar: result.avatar,
        email: result.email,
        flags: result.flags,
        public_flags: result.public_flags,
        is_dev,
    };

    Ok(HttpResponse::Ok().json(user_data))
}

#[derive(Debug, Serialize, Deserialize)]
struct GuildDataForFrontEnd {
    #[serde(with = "DisplayFromstr")]
    pub id: i64,
    pub name: String,
    pub icon: Option<String>,
    pub owner: bool,
    #[serde(with = "DisplayFromstr")]
    pub permissions: i64,
}

#[get("/users/@me/guilds")]
pub async fn get_current_user_guilds(
    _req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    token: Option<ReqData<Token<Access>>>,
) -> Result<impl Responder, AppError> {
    let token_data = token.ok_or_else(|| AppError::Forbidden)?;
    let dev_account_id = crate::config::DEV_ACCOUNT_ID.parse::<i64>().unwrap_or(0);

    let result = if token_data.id == dev_account_id
        && dev_account_id != 0
        && token_data.token == "dev_access"
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
        sqlx::query_as!(
            GuildDataForFrontEnd,
            "
            SELECT id,
            name,
            icon,
            owner,
            permissions 
            FROM guilds_present 
            JOIN user_guilds ON user_guilds.id = guilds_present.guild_id
            AND user_guilds.user_id = $1;
            ",
            token_data.id
        )
        .fetch_all(pool.get_ref())
        .await?
    };

    Ok(HttpResponse::Ok().json(result))
}
