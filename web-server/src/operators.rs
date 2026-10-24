//! The bot's operators: the people who run this Sakiot deployment. They are
//! listed in `operators`, which only `web_server operators` on the server
//! changes, and they alone set what each guild is allowed
//! (`guild_limits`), which nobody in a guild can change.

use std::error::Error;

use actix_web::{HttpResponse, get, put, web};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres, postgres::PgPoolOptions};

use crate::admin::keep_allowances::MAX_KEPT;
use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::snowflake_serde::SnowflakeAsStr;

pub async fn is_operator<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    user_id: i64,
) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM operators WHERE user_id = $1) AS "operator!""#,
        user_id
    )
    .fetch_one(executor)
    .await?)
}

/// The operator making the request.
async fn require_operator(
    pool: &Pool<Postgres>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<i64, AppError> {
    let user_id = token.ok_or(AppError::Unauthorized)?.user_id;
    if is_operator(pool, user_id).await? {
        Ok(user_id)
    } else {
        Err(AppError::Forbidden)
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct OperatorGuild {
    #[serde(with = "SnowflakeAsStr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub guild_id: i64,
    pub name: Option<String>,
    pub icon: Option<String>,
    /// The bot is in it.
    pub present: bool,
    /// How many recordings it may keep.
    pub kept_recordings_cap: i32,
    /// How many it keeps.
    pub kept_recordings: i64,
    /// After how many days its retention removes a recording; absent when
    /// it never does.
    pub retention_days: Option<i32>,
}

#[utoipa::path(
    get,
    path = "/api/operator/guilds",
    tag = "operator",
    responses(
        (status = 200, description = "Every guild the bot is in, has limits for or holds recordings of", body = [OperatorGuild]),
        (status = 403, description = "Not an operator", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/operator/guilds")]
pub async fn get_operator_guilds(
    pool: web::Data<Pool<Postgres>>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    require_operator(pool.get_ref(), token).await?;
    // Seeded guilds of a dev login (staging, previews) may be unknown to
    // the bot there: their names come from the seeding.
    let guilds = sqlx::query_as!(
        OperatorGuild,
        r#"WITH known AS (
               SELECT guild_id FROM guilds_present
               UNION SELECT guild_id FROM guild_limits
               UNION SELECT DISTINCT guild_id FROM recording_sessions
           )
           SELECT k.guild_id AS "guild_id!",
                  COALESCE(g.name, (SELECT ug.name FROM user_guilds ug
                                     WHERE ug.id = k.guild_id
                                     ORDER BY ug.user_id LIMIT 1)) AS name,
                  COALESCE(g.icon, (SELECT ug.icon FROM user_guilds ug
                                     WHERE ug.id = k.guild_id
                                     ORDER BY ug.user_id LIMIT 1)) AS icon,
                  EXISTS (SELECT 1 FROM guilds_present gp WHERE gp.guild_id = k.guild_id)
                      AS "present!",
                  COALESCE(l.kept_recordings, 0) AS "kept_recordings_cap!",
                  (SELECT count(DISTINCT recording_session_id)
                     FROM kept_recordings kr
                    WHERE kr.guild_id = k.guild_id) AS "kept_recordings!",
                  p.retention_days
             FROM known k
             LEFT JOIN guilds g ON g.id = k.guild_id
             LEFT JOIN guild_limits l ON l.guild_id = k.guild_id
             LEFT JOIN guild_recording_policy p ON p.guild_id = k.guild_id
            ORDER BY 4 DESC, lower(COALESCE(g.name, '')), k.guild_id"#
    )
    .fetch_all(pool.get_ref())
    .await?;
    Ok(HttpResponse::Ok().json(guilds))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct GuildLimits {
    /// How many recordings the guild may keep past retention.
    pub kept_recordings: i32,
}

#[utoipa::path(
    put,
    path = "/api/operator/guilds/{guild_id}/limits",
    tag = "operator",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    request_body = GuildLimits,
    responses(
        (status = 204, description = "Limits set. Lowering the cap below what the guild keeps keeps those, and stops new keeps until it is under the cap again"),
        (status = 400, description = "Invalid limit", body = crate::errors::ApiError),
        (status = 403, description = "Not an operator", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[put("/operator/guilds/{guild_id}/limits")]
pub async fn put_guild_limits(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    body: web::Json<GuildLimits>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let user_id = require_operator(pool.get_ref(), token).await?;
    let guild_id = path.into_inner();
    if !(0..=MAX_KEPT).contains(&body.kept_recordings) {
        return Err(AppError::BadRequest(format!(
            "A guild may keep between 0 and {MAX_KEPT} recordings."
        )));
    }
    sqlx::query!(
        "INSERT INTO guild_limits (guild_id, kept_recordings, updated_by) VALUES ($1, $2, $3)
         ON CONFLICT (guild_id) DO UPDATE SET kept_recordings = EXCLUDED.kept_recordings,
                                              updated_by = EXCLUDED.updated_by,
                                              updated_at = now()",
        guild_id,
        body.kept_recordings,
        user_id
    )
    .execute(pool.get_ref())
    .await?;
    Ok(HttpResponse::NoContent().finish())
}

type CliResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

const USAGE: &str = "usage: web_server operators list | operators add <discord user id> | operators remove <discord user id>";

/// `web_server operators ...`: who may set guilds' limits.
pub async fn run_operators_command(arguments: &[String]) -> CliResult<()> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("DATABASE_URL")?)
        .await?;
    match arguments {
        [command] if command == "list" => {
            let operators = sqlx::query!(
                r#"SELECT o.user_id, o.added_at, un.username AS "username?"
                     FROM operators o
                     LEFT JOIN user_names un ON un.user_id = o.user_id
                    ORDER BY o.added_at, o.user_id"#
            )
            .fetch_all(&pool)
            .await?;
            if operators.is_empty() {
                output("no operators");
            }
            for operator in operators {
                output(&format!(
                    "{}\t{}\tadded {}",
                    operator.user_id,
                    operator.username.as_deref().unwrap_or("-"),
                    operator.added_at.to_rfc3339()
                ));
            }
        }
        [command, user_id] if command == "add" => {
            let user_id = parse_user_id(user_id)?;
            sqlx::query!(
                "INSERT INTO operators (user_id) VALUES ($1) ON CONFLICT DO NOTHING",
                user_id
            )
            .execute(&pool)
            .await?;
            output(&format!("{user_id} is an operator"));
        }
        [command, user_id] if command == "remove" => {
            let user_id = parse_user_id(user_id)?;
            sqlx::query!("DELETE FROM operators WHERE user_id = $1", user_id)
                .execute(&pool)
                .await?;
            output(&format!("{user_id} is not an operator"));
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

fn parse_user_id(value: &str) -> CliResult<i64> {
    match value.parse::<i64>() {
        Ok(user_id) if user_id > 0 => Ok(user_id),
        _ => Err(format!("not a Discord user id: {value}\n{USAGE}").into()),
    }
}

#[expect(clippy::print_stdout, reason = "operator CLI writes requested results")]
fn output(message: &str) {
    println!("{message}");
}
