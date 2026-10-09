use std::collections::{HashMap, HashSet};

use actix_web::web;
use sqlx::{PgConnection, Pool, Postgres};

use crate::auth::{Access, AuthKind, Token};
use crate::errors::AppError;

mod bits;
mod channels;
mod roles;

pub use bits::*;
pub use channels::*;
pub use roles::*;

/// Begin a read-only transaction that sees one consistent snapshot of the
/// permission cache. An authorization result reads up to seven queries; in
/// separate statements a cache write committing between them could mix old
/// and new state (a role grant from before a change with overwrites from
/// after it). Every public check below runs its queries in one snapshot, and
/// callers that combine a check with their own reads, such as session access,
/// can run them in the same one.
pub async fn begin_snapshot(
    pool: &Pool<Postgres>,
) -> Result<sqlx::Transaction<'static, Postgres>, AppError> {
    Ok(pool
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .await?)
}

/// Who is asking. A dev login exists only in local, staging and preview
/// builds (`dev-login` feature), where guilds are seeded rather than seen by
/// the bot; membership follows different rules there (`membership`).
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Membership {
    Owner,
    Member,
    Outsider,
}

/// Whether the viewer belongs to the guild: the one membership rule behind
/// every authorization check, the realtime fan-out and the guild picker.
///
/// For a Discord login the bot's roster (`guild_members`) decides, not the
/// guild list Discord returned at login:
/// - Ownership comes from `guilds.owner_id`, which the bot keeps current.
/// - A roster that was complete once stays authoritative, even while the bot
///   is down or between deploys, as the role cache does.
/// - A guild the bot is in but whose roster was never complete is
///   `MembershipUnavailable`: retryable, never guessed from the login list,
///   and never read as everyone having left.
/// - A guild the bot is not in and never had a roster for has no members.
///
/// A dev login keeps the seeded `user_guilds` rows (membership, and the
/// owner flag the seeding tools set): its guilds are imported from
/// production and the bot never sees them.
async fn membership(
    conn: &mut PgConnection,
    guild_id: i64,
    viewer: Viewer,
) -> Result<Membership, AppError> {
    if viewer.dev {
        let row = sqlx::query!(
            r#"SELECT EXISTS (
                      SELECT 1 FROM guilds WHERE id = $1 AND owner_id = $2
                  ) AS "owner_id!",
                  (SELECT owner FROM user_guilds WHERE id = $1 AND user_id = $2) AS seeded_owner"#,
            guild_id,
            viewer.user_id
        )
        .fetch_one(&mut *conn)
        .await?;
        return Ok(match (row.owner_id, row.seeded_owner) {
            (true, _) | (_, Some(true)) => Membership::Owner,
            (false, Some(false)) => Membership::Member,
            (false, None) => Membership::Outsider,
        });
    }

    let row = sqlx::query!(
        r#"SELECT EXISTS (
                  SELECT 1 FROM guilds WHERE id = $1 AND owner_id = $2
              ) AS "owner!",
              (SELECT roster_complete_at IS NOT NULL
                 FROM guild_projection_state
                WHERE guild_id = $1) AS roster_complete,
              EXISTS (
                  SELECT 1 FROM guilds_present WHERE guild_id = $1
              ) AS "bot_present!",
              EXISTS (
                  SELECT 1 FROM guild_members WHERE guild_id = $1 AND user_id = $2
              ) AS "listed!""#,
        guild_id,
        viewer.user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(
        match (
            row.owner,
            row.roster_complete.unwrap_or(false),
            row.bot_present,
        ) {
            (true, _, _) => Membership::Owner,
            (false, true, _) if row.listed => Membership::Member,
            (false, true, _) | (false, false, false) => Membership::Outsider,
            (false, false, true) => return Err(AppError::MembershipUnavailable),
        },
    )
}

pub async fn get_combined_perm_for_user(
    pool: &Pool<Postgres>,
    guild_id: i64,
    viewer: Viewer,
) -> Result<Permissions, AppError> {
    let mut snapshot = begin_snapshot(pool).await?;
    let permissions = combined_perm_for_user(&mut snapshot, guild_id, viewer).await?;
    snapshot.commit().await?;
    Ok(permissions)
}

/// The viewer's guild-level permissions, read on `conn` (use
/// `begin_snapshot`): everything for the owner, nothing for outsiders.
pub async fn combined_perm_for_user(
    conn: &mut PgConnection,
    guild_id: i64,
    viewer: Viewer,
) -> Result<Permissions, AppError> {
    // Membership is a precondition for every other grant. The aggregation below
    // always includes `@everyone` (`role_id = guild_id`), so without this check
    // a stranger who was never in the guild would inherit `@everyone`'s bits —
    // including ADMINISTRATOR when a guild grants it there.
    match membership(conn, guild_id, viewer).await? {
        Membership::Owner => return Ok(Permissions::all()),
        Membership::Outsider => return Ok(Permissions::empty()),
        Membership::Member => {}
    }
    let user_id = viewer.user_id;

    // The OAuth guild list stores a combined permission snapshot from login.
    // Build the value from the agent-maintained role cache instead so role and
    // membership revocations take effect without requiring a new login.
    let permissions = sqlx::query_scalar!(
        "SELECT bit_or(r.permission)
           FROM roles r
          WHERE r.guild_id = $1
            AND (
                r.role_id = $1
                OR EXISTS (
                    SELECT 1
                      FROM user_roles ur
                     WHERE ur.user_id = $2
                       AND ur.role_id = r.role_id
                )
            )",
        guild_id,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?
    .unwrap_or(0);

    Ok(permissions_from_bits(permissions))
}

/// Resolve the channel set a listing should show: the caller's own channels,
/// or — when impersonating a role — the channels that role alone could join,
/// clipped to the caller's own set by `role_access_for_preview`.
/// A foreign role is a 404 rather than a silently empty preview.
pub async fn listing_channels_for(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    viewer: Viewer,
    as_role: Option<i64>,
) -> Result<HashSet<i64>, AppError> {
    match as_role {
        None => visible_channels_for_user(pool, guild_id, viewer).await,
        Some(role_id) => Ok(role_access_for_preview(pool, guild_id, viewer, role_id)
            .await?
            .into_values()
            .filter(|access| access.joinable)
            .map(|access| access.channel_id)
            .collect()),
    }
}

const MANAGER_PERMISSIONS: Permissions =
    Permissions::ADMINISTRATOR.union(Permissions::MANAGE_GUILD);

/// What one realtime subscription may receive, read in one snapshot: the
/// channels whose sessions its recording tree lists, the channels whose clips
/// and stamps it lists, the channels whose voice presence it shows, and
/// whether the viewer manages the guild.
///
/// For a normal view the tree and media sets are the viewer's own visible
/// channels. For a manager's role preview they follow the HTTP listings: the
/// tree lists every channel of the preview map (the manager's own channels,
/// annotated for the role), while clips and stamps list the channels the role
/// could join. Presence follows `presence_channels`, as the voice-presence
/// endpoint does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionAccess {
    pub tree_channels: HashSet<i64>,
    pub media_channels: HashSet<i64>,
    pub presence_channels: HashSet<i64>,
    pub manager: bool,
}

pub async fn subscription_access(
    pool: &Pool<Postgres>,
    guild_id: i64,
    viewer: Viewer,
    as_role: Option<i64>,
) -> Result<SubscriptionAccess, AppError> {
    let mut snapshot = begin_snapshot(pool).await?;
    let manager = combined_perm_for_user(&mut snapshot, guild_id, viewer)
        .await?
        .intersects(MANAGER_PERMISSIONS);
    let access = match as_role {
        None => {
            let channels = visible_channels(&mut snapshot, guild_id, viewer).await?;
            SubscriptionAccess {
                tree_channels: channels.clone(),
                media_channels: channels,
                presence_channels: presence_channels(&mut snapshot, guild_id, viewer, None).await?,
                manager,
            }
        }
        Some(_) if !manager => return Err(AppError::Forbidden),
        Some(role_id) => {
            let preview = preview_access(&mut snapshot, guild_id, viewer, role_id).await?;
            SubscriptionAccess {
                tree_channels: preview.keys().copied().collect(),
                media_channels: preview
                    .values()
                    .filter(|access| access.joinable)
                    .map(|access| access.channel_id)
                    .collect(),
                presence_channels: presence_channels(
                    &mut snapshot,
                    guild_id,
                    viewer,
                    Some(role_id),
                )
                .await?,
                manager,
            }
        }
    };
    snapshot.commit().await?;
    Ok(access)
}

#[derive(Debug, serde::Deserialize)]
pub struct AsRoleQuery {
    pub as_role: Option<i64>,
}

/// Role impersonation (view-as-role) is manager-only: someone who cannot
/// already manage the guild must not preview what one of its roles can see.
pub async fn require_role_preview(
    req: &actix_web::HttpRequest,
    pool: &actix_web::web::Data<sqlx::Pool<sqlx::Postgres>>,
    guild_id: i64,
    as_role: Option<i64>,
) -> Result<(), crate::errors::AppError> {
    if as_role.is_some() {
        require_guild_manager(req, pool, guild_id).await?;
    }
    Ok(())
}

/// Fails unless the viewer belongs to the guild (`membership`).
pub async fn require_guild_member(
    pool: &Pool<Postgres>,
    guild_id: i64,
    viewer: Viewer,
) -> Result<(), AppError> {
    let mut snapshot = begin_snapshot(pool).await?;
    require_member(&mut snapshot, guild_id, viewer).await?;
    snapshot.commit().await?;
    Ok(())
}

async fn require_member(
    conn: &mut PgConnection,
    guild_id: i64,
    viewer: Viewer,
) -> Result<(), AppError> {
    match membership(conn, guild_id, viewer).await? {
        Membership::Owner | Membership::Member => Ok(()),
        Membership::Outsider => Err(AppError::Forbidden),
    }
}

async fn require_role_in_guild(
    conn: &mut PgConnection,
    guild_id: i64,
    role_id: i64,
) -> Result<(), AppError> {
    let belongs_to_guild = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM roles WHERE role_id = $1 AND guild_id = $2) AS "exists!""#,
        role_id,
        guild_id
    )
    .fetch_one(&mut *conn)
    .await?;
    if !belongs_to_guild {
        return Err(AppError::RoleNotFound);
    }
    Ok(())
}

pub async fn require_channel_access(
    pool: &actix_web::web::Data<sqlx::Pool<sqlx::Postgres>>,
    guild_id: i64,
    channel_id: i64,
    viewer: Viewer,
) -> Result<(), crate::errors::AppError> {
    let permitted = visible_channels_for_user(pool, guild_id, viewer).await?;
    if permitted.contains(&channel_id) {
        Ok(())
    } else {
        Err(crate::errors::AppError::Forbidden)
    }
}

pub async fn require_guild_manager(
    req: &actix_web::HttpRequest,
    pool: &actix_web::web::Data<sqlx::Pool<sqlx::Postgres>>,
    guild_id: i64,
) -> Result<i64, crate::errors::AppError> {
    require_guild_permission(req, pool, guild_id, MANAGER_PERMISSIONS).await
}

async fn require_guild_permission(
    req: &actix_web::HttpRequest,
    pool: &actix_web::web::Data<sqlx::Pool<sqlx::Postgres>>,
    guild_id: i64,
    required_mask: Permissions,
) -> Result<i64, crate::errors::AppError> {
    // Dev logins (local, staging and preview builds) manage the guilds their
    // seeding granted them through `membership`'s owner rule.
    let viewer = Viewer::of_request(req)?;
    if get_combined_perm_for_user(pool, guild_id, viewer)
        .await?
        .intersects(required_mask)
    {
        Ok(viewer.user_id)
    } else {
        Err(crate::errors::AppError::Forbidden)
    }
}

#[cfg(test)]
mod tests;
