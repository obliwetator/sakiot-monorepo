//! Listening consent: a member decides who may hear the recordings of their
//! voice in one server. Shared by default, which means whoever the server's
//! permissions let listen; private means only themselves, whatever anyone
//! holds, the owner included (`Access::allows_item`). It applies to earlier
//! recordings, clips and stamps too, and a former member may change it as
//! well. The same switch exists in Discord as `/recording private` and
//! `/recording shared`.

use actix_web::{
    HttpResponse, get, put,
    web::{self, ReqData},
};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres};

use crate::auth::{Access, Token};
use crate::errors::AppError;

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RecordingPrivacy {
    /// True when nobody but the current user may hear the recordings of
    /// their voice in this server.
    pub private: bool,
}

#[utoipa::path(
    get,
    path = "/api/users/current/guilds/{guild_id}/recording-privacy",
    tag = "user",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "Who may hear the current user's recordings in this guild", body = RecordingPrivacy),
        (status = 403, description = "Neither a member nor a former member with recordings here", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/users/current/guilds/{guild_id}/recording-privacy")]
pub async fn get_recording_privacy(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    token: Option<ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let viewer = token.viewer();
    let guild_id = path.into_inner();
    crate::permissions::GuildAccess::load(pool.get_ref(), guild_id, viewer, None)
        .await?
        .require_member_or_former()?;
    let private = sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1 FROM recording_private WHERE guild_id = $1 AND user_id = $2
           ) AS "private!""#,
        guild_id,
        viewer.user_id
    )
    .fetch_one(pool.get_ref())
    .await?;
    Ok(HttpResponse::Ok().json(RecordingPrivacy { private }))
}

#[utoipa::path(
    put,
    path = "/api/users/current/guilds/{guild_id}/recording-privacy",
    tag = "user",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    request_body = RecordingPrivacy,
    responses(
        (status = 200, description = "The stored setting", body = RecordingPrivacy),
        (status = 403, description = "Neither a member nor a former member with recordings here", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[put("/users/current/guilds/{guild_id}/recording-privacy")]
pub async fn put_recording_privacy(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    token: Option<ReqData<Token<Access>>>,
    body: web::Json<RecordingPrivacy>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let viewer = token.viewer();
    let guild_id = path.into_inner();
    // Never gated beyond belonging: it is the speaker's own control.
    crate::permissions::GuildAccess::load(pool.get_ref(), guild_id, viewer, None)
        .await?
        .require_member_or_former()?;
    if body.private {
        sqlx::query!(
            "INSERT INTO recording_private (guild_id, user_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
            guild_id,
            viewer.user_id
        )
        .execute(pool.get_ref())
        .await?;
    } else {
        sqlx::query!(
            "DELETE FROM recording_private WHERE guild_id = $1 AND user_id = $2",
            guild_id,
            viewer.user_id
        )
        .execute(pool.get_ref())
        .await?;
    }
    Ok(HttpResponse::Ok().json(RecordingPrivacy {
        private: body.private,
    }))
}
