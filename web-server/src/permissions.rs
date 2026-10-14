//! Authorization for guild content. The rules live in `sakiot_permissions`;
//! this module loads their facts from the cache tables the bot keeps
//! (`facts`) and answers through `GuildAccess`, one statement per check.
//!
//! The functions below keep the signatures and answers of the checks they
//! replaced (`legacy`), so callers move to `GuildAccess` one at a time.

use std::collections::{HashMap, HashSet};

use actix_web::web;
use sakiot_permissions::Capabilities;
use sqlx::{PgConnection, Pool, Postgres};

use crate::auth::{Access, AuthKind, Token};
use crate::errors::AppError;

mod facts;
mod guild_access;
#[doc(hidden)]
pub mod legacy;

pub use guild_access::GuildAccess;
pub use sakiot_permissions::Permissions;

/// Begin a read-only transaction that sees one consistent snapshot. A
/// permission check is a single statement and needs none of its own; callers
/// that combine one with their own reads, such as session access, run them
/// all in one snapshot.
pub async fn begin_snapshot(
    pool: &Pool<Postgres>,
) -> Result<sqlx::Transaction<'static, Postgres>, AppError> {
    Ok(pool
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .await?)
}

/// Who is asking. A dev login exists only in local, staging and preview
/// builds (`dev-login` feature), where guilds are seeded rather than seen by
/// the bot; membership follows different rules there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewer {
    pub user_id: i64,
    pub dev: bool,
}

impl Viewer {
    pub fn of(token: &Token<Access>) -> Self {
        Self {
            user_id: token.user_id,
            dev: token.auth_kind == AuthKind::Dev,
        }
    }

    /// A Discord login.
    pub fn discord(user_id: i64) -> Self {
        Self {
            user_id,
            dev: false,
        }
    }

    /// The viewer of a request the auth middleware let through.
    pub fn of_request(req: &actix_web::HttpRequest) -> Result<Self, AppError> {
        use actix_web::HttpMessage;
        req.extensions()
            .get::<Token<Access>>()
            .map(Self::of)
            .ok_or(AppError::Unauthorized)
    }
}

impl From<Viewer> for sakiot_permissions::Viewer {
    fn from(viewer: Viewer) -> Self {
        Self {
            user_id: viewer.user_id,
            dev: viewer.dev,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct AsRoleQuery {
    pub as_role: Option<i64>,
}

/// What one realtime subscription may receive: the channels whose sessions
/// its recording tree lists, the channels whose clips and stamps it lists,
/// the channels whose voice presence it shows, and whether the viewer
/// manages the guild.
///
/// For a normal view the tree and media sets are the viewer's own listening
/// channels. For a manager's role preview they follow the HTTP listings: the
/// tree lists the manager's own channels (annotated for the role), while
/// clips and stamps list the channels the role could listen in. Presence
/// follows `presence_channels`, as the voice-presence endpoint does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionAccess {
    pub tree_channels: HashSet<i64>,
    pub media_channels: HashSet<i64>,
    pub presence_channels: HashSet<i64>,
    pub manager: bool,
}

/// A role preview's access to one of the manager's channels: `viewable`
/// shows its presence, `joinable` its recordings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleChannelAccess {
    pub channel_id: i64,
    pub viewable: bool,
    pub joinable: bool,
}

/// The viewer's Discord permissions at guild level: everything for the
/// owner, nothing for an outsider.
pub async fn get_combined_perm_for_user(
    pool: &Pool<Postgres>,
    guild_id: i64,
    viewer: Viewer,
) -> Result<Permissions, AppError> {
    GuildAccess::load(pool, guild_id, viewer, None)
        .await?
        .discord_permissions()
}

/// `get_combined_perm_for_user` on the caller's connection.
pub async fn combined_perm_for_user(
    conn: &mut PgConnection,
    guild_id: i64,
    viewer: Viewer,
) -> Result<Permissions, AppError> {
    GuildAccess::load(conn, guild_id, viewer, None)
        .await?
        .discord_permissions()
}

/// The channels a listing shows: the viewer's own listening channels, or,
/// when previewing a role, the channels that role could listen in, clipped
/// to the viewer's own. A foreign role is a 404 rather than a silently empty
/// preview.
pub async fn listing_channels_for(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    viewer: Viewer,
    as_role: Option<i64>,
) -> Result<HashSet<i64>, AppError> {
    let access = GuildAccess::load(pool.get_ref(), guild_id, viewer, as_role).await?;
    match access.role_preview()? {
        None => access.channels(Capabilities::LISTEN_OTHERS),
        Some(preview) => Ok(preview
            .access
            .channels_with(Capabilities::LISTEN_OTHERS)
            .collect()),
    }
}

pub async fn subscription_access(
    pool: &Pool<Postgres>,
    guild_id: i64,
    viewer: Viewer,
    as_role: Option<i64>,
) -> Result<SubscriptionAccess, AppError> {
    let access = GuildAccess::load(pool, guild_id, viewer, as_role).await?;
    let manager = access.manages()?;
    if as_role.is_some() && !manager {
        return Err(AppError::Forbidden);
    }
    Ok(match access.role_preview()? {
        None => {
            let channels = access.channels(Capabilities::LISTEN_OTHERS)?;
            SubscriptionAccess {
                tree_channels: channels.clone(),
                media_channels: channels,
                presence_channels: access.channels(Capabilities::VIEW_PRESENCE)?,
                manager,
            }
        }
        Some(preview) => SubscriptionAccess {
            media_channels: preview
                .access
                .channels_with(Capabilities::LISTEN_OTHERS)
                .collect(),
            presence_channels: preview
                .access
                .channels_with(Capabilities::VIEW_PRESENCE)
                .collect(),
            tree_channels: preview.listed.into_iter().collect(),
            manager,
        },
    })
}

/// Role impersonation (view-as-role) is manager-only: someone who cannot
/// already manage the guild must not preview what one of its roles can see.
pub async fn require_role_preview(
    req: &actix_web::HttpRequest,
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    as_role: Option<i64>,
) -> Result<(), AppError> {
    if as_role.is_some() {
        require_guild_manager(req, pool, guild_id).await?;
    }
    Ok(())
}

/// Fails unless the viewer belongs to the guild.
pub async fn require_guild_member(
    pool: &Pool<Postgres>,
    guild_id: i64,
    viewer: Viewer,
) -> Result<(), AppError> {
    GuildAccess::load(pool, guild_id, viewer, None)
        .await?
        .require_member()
        .map(|_| ())
}

pub async fn require_channel_access(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    channel_id: i64,
    viewer: Viewer,
) -> Result<(), AppError> {
    if visible_channels_for_user(pool, guild_id, viewer)
        .await?
        .contains(&channel_id)
    {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

/// Fails unless the viewer manages the guild; returns their user id.
pub async fn require_guild_manager(
    req: &actix_web::HttpRequest,
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
) -> Result<i64, AppError> {
    let viewer = Viewer::of_request(req)?;
    if GuildAccess::load(pool.get_ref(), guild_id, viewer, None)
        .await?
        .manages()?
    {
        Ok(viewer.user_id)
    } else {
        Err(AppError::Forbidden)
    }
}

/// The voice and stage channels whose occupants a member may see, read on
/// `conn`. As in Discord's channel list, VIEW_CHANNEL is enough: seeing who
/// is in a channel does not need CONNECT. A non-member is forbidden.
///
/// With `as_role` (the caller must already have passed
/// `require_role_preview`), the channels that role could view, clipped to
/// the manager's own presence view.
pub async fn presence_channels(
    conn: &mut PgConnection,
    guild_id: i64,
    viewer: Viewer,
    as_role: Option<i64>,
) -> Result<HashSet<i64>, AppError> {
    let access = GuildAccess::load(conn, guild_id, viewer, as_role).await?;
    let own = access.channels(Capabilities::VIEW_PRESENCE)?;
    Ok(match access.role_preview()? {
        None => own,
        Some(preview) => preview
            .access
            .channels_with(Capabilities::VIEW_PRESENCE)
            .collect(),
    })
}

/// The channels a member can listen in. A non-member is forbidden.
pub async fn visible_channels_for_user(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    viewer: Viewer,
) -> Result<HashSet<i64>, AppError> {
    GuildAccess::load(pool.get_ref(), guild_id, viewer, None)
        .await?
        .channels(Capabilities::LISTEN_OTHERS)
}

/// `visible_channels_for_user` on the caller's connection, so it can be
/// combined with the caller's own reads in one snapshot.
pub async fn visible_channels(
    conn: &mut PgConnection,
    guild_id: i64,
    viewer: Viewer,
) -> Result<HashSet<i64>, AppError> {
    GuildAccess::load(conn, guild_id, viewer, None)
        .await?
        .channels(Capabilities::LISTEN_OTHERS)
}

/// The Discord permissions a member would hold if their only role were
/// `role_id`: the role itself plus `@everyone`. Discord's "view server as
/// role".
pub async fn get_combined_perm_for_role(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    role_id: i64,
) -> Result<Permissions, AppError> {
    let facts = facts::load(pool.get_ref(), guild_id, &[], &[role_id]).await?;
    Ok(facts.role_permissions(role_id).base)
}

/// The role preview's access to each channel the manager can listen in,
/// keyed by channel. A role that is not the guild's is `RoleNotFound`.
///
/// SECURITY: a preview never shows more than the manager's own normal view
/// (`sakiot_permissions::RolePreview`).
pub async fn role_access_for_preview(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    manager: Viewer,
    role_id: i64,
) -> Result<HashMap<i64, RoleChannelAccess>, AppError> {
    let access = GuildAccess::load(pool.get_ref(), guild_id, manager, Some(role_id)).await?;
    let Some(preview) = access.role_preview()? else {
        return Ok(HashMap::new());
    };
    Ok(preview
        .listed
        .iter()
        .map(|&channel_id| {
            let access = RoleChannelAccess {
                channel_id,
                viewable: preview
                    .access
                    .allows(channel_id, Capabilities::VIEW_PRESENCE),
                joinable: preview
                    .access
                    .allows(channel_id, Capabilities::LISTEN_OTHERS),
            };
            (channel_id, access)
        })
        .collect())
}
