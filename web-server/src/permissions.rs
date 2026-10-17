//! Authorization for guild content. The rules live in `sakiot_permissions`;
//! this module loads their facts from the cache tables the bot keeps
//! (`facts`) and answers through `GuildAccess`, which a request loads once,
//! in one statement, and asks everything.

use sakiot_permissions::Capabilities;
use sqlx::{Pool, Postgres};

use crate::auth::{Access, Token};
use crate::errors::AppError;

mod facts;
mod guild_access;

pub use guild_access::GuildAccess;
pub use sakiot_permissions::{Permissions, Viewer};

/// The viewer of a request the auth middleware let through.
pub fn request_viewer(req: &actix_web::HttpRequest) -> Result<Viewer, AppError> {
    use actix_web::HttpMessage;
    req.extensions()
        .get::<Token<Access>>()
        .map(Token::viewer)
        .ok_or(AppError::Unauthorized)
}

#[derive(Debug, serde::Deserialize)]
pub struct AsRoleQuery {
    pub as_role: Option<i64>,
}

/// Fails unless the request's viewer holds the guild-wide `capability`;
/// returns the viewer.
pub async fn require_capability(
    req: &actix_web::HttpRequest,
    pool: &Pool<Postgres>,
    guild_id: i64,
    capability: Capabilities,
) -> Result<Viewer, AppError> {
    let viewer = request_viewer(req)?;
    GuildAccess::load(pool, guild_id, viewer, None)
        .await?
        .require(capability)?;
    Ok(viewer)
}
