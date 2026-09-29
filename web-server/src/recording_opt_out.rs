//! Self-service recording opt-out: a member stops (or resumes) the bot
//! recording their voice in one server. Recording is on by default; the bot
//! reads `recording_opt_outs` before opening a recording and re-checks open
//! ones every second, so a change applies without a restart. Earlier
//! recordings are kept. The same switch exists in Discord as `/recording`.

use actix_web::{
    HttpResponse, get, put,
    web::{self, ReqData},
};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres};

use crate::auth::{Access, Token};
use crate::errors::AppError;

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RecordingOptOut {
    /// True when the bot does not record the current user's voice in this
    /// server.
    pub opted_out: bool,
}

/// Only members may change their own recording preference for a server.
async fn require_membership(
    pool: &Pool<Postgres>,
    guild_id: i64,
    user_id: i64,
) -> Result<(), AppError> {
    let member = sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1 FROM user_guilds WHERE id = $1 AND user_id = $2
           ) AS "member!""#,
        guild_id,
        user_id
    )
    .fetch_one(pool)
    .await?;
    if member {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

#[utoipa::path(
    get,
    path = "/api/users/current/guilds/{guild_id}/recording-opt-out",
    tag = "user",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "Whether the bot skips recording the current user in this guild", body = RecordingOptOut),
        (status = 403, description = "Not a member of this guild", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/users/current/guilds/{guild_id}/recording-opt-out")]
pub async fn get_recording_opt_out(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    token: Option<ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let user_id = token.ok_or(AppError::Unauthorized)?.user_id;
    let guild_id = path.into_inner();
    require_membership(&pool, guild_id, user_id).await?;
    let opted_out = sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1 FROM recording_opt_outs WHERE guild_id = $1 AND user_id = $2
           ) AS "opted_out!""#,
        guild_id,
        user_id
    )
    .fetch_one(pool.get_ref())
    .await?;
    Ok(HttpResponse::Ok().json(RecordingOptOut { opted_out }))
}

#[utoipa::path(
    put,
    path = "/api/users/current/guilds/{guild_id}/recording-opt-out",
    tag = "user",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    request_body = RecordingOptOut,
    responses(
        (status = 200, description = "The stored recording preference", body = RecordingOptOut),
        (status = 403, description = "Not a member of this guild", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[put("/users/current/guilds/{guild_id}/recording-opt-out")]
pub async fn put_recording_opt_out(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    token: Option<ReqData<Token<Access>>>,
    body: web::Json<RecordingOptOut>,
) -> Result<HttpResponse, AppError> {
    let user_id = token.ok_or(AppError::Unauthorized)?.user_id;
    let guild_id = path.into_inner();
    require_membership(&pool, guild_id, user_id).await?;
    let mut tx = pool.begin().await?;
    // The bot takes the same lock while it checks the opt-out and opens a
    // recording fragment, so none can open after this commits.
    sqlx::query!("SELECT pg_advisory_xact_lock($1)", guild_id)
        .execute(&mut *tx)
        .await?;
    if body.opted_out {
        sqlx::query!(
            "INSERT INTO recording_opt_outs (guild_id, user_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
            guild_id,
            user_id
        )
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query!(
            "DELETE FROM recording_opt_outs WHERE guild_id = $1 AND user_id = $2",
            guild_id,
            user_id
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(HttpResponse::Ok().json(RecordingOptOut {
        opted_out: body.opted_out,
    }))
}
