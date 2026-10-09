//! A role's view of a guild: its permission bits and channel access alone,
//! and the clipped preview a manager sees when viewing as that role.

use super::*;

/// Combined permission bits a member would hold if their only role were
/// `role_id`: the role itself plus `@everyone` (`role_id = guild_id`). This is
/// the "view server as role" lens — Discord's preview of what a hypothetical
/// member with a single role would see.
pub async fn get_combined_perm_for_role(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    role_id: i64,
) -> Result<Permissions, AppError> {
    combined_perm_for_role(&mut *pool.acquire().await?, guild_id, role_id).await
}

async fn combined_perm_for_role(
    conn: &mut PgConnection,
    guild_id: i64,
    role_id: i64,
) -> Result<Permissions, AppError> {
    let permissions = sqlx::query_scalar!(
        "SELECT bit_or(r.permission)
           FROM roles r
          WHERE r.guild_id = $1
            AND (r.role_id = $1 OR r.role_id = $2)",
        guild_id,
        role_id
    )
    .fetch_one(&mut *conn)
    .await?
    .unwrap_or(0);

    Ok(permissions_from_bits(permissions))
}

async fn apply_single_role_overwrites(
    conn: &mut PgConnection,
    role_id: i64,
    guild_id: i64,
    channels: &mut HashMap<i64, ChannelPermissionState>,
) -> Result<(), AppError> {
    let role_overwrites = sqlx::query!(
        "SELECT allow as \"allow!\", deny as \"deny!\", channel_id as \"channel_id!\"
           FROM channel_permissions
          WHERE kind = 'role' AND target_id = $1",
        role_id
    )
    .fetch_all(&mut *conn)
    .await?;

    for overwrite in role_overwrites {
        if overwrite.channel_id == guild_id {
            // Generic @everyone is already included in the base guild permissions.
            continue;
        }
        if let Some(channel) = channels.get_mut(&overwrite.channel_id) {
            channel.roles.allow |= permissions_from_bits(overwrite.allow);
            channel.roles.deny |= permissions_from_bits(overwrite.deny);
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleChannelAccess {
    pub channel_id: i64,
    pub viewable: bool,
    pub joinable: bool,
}

/// Per-channel access a member whose only role is `role_id` would have,
/// mirroring the user path without member-specific overwrites. `viewable` is
/// Discord's "see the channel" (VIEW_CHANNEL); `joinable` also requires
/// CONNECT — a channel can be visible without being joinable.
pub(super) async fn channel_access_for_role(
    conn: &mut PgConnection,
    guild_id: i64,
    role_id: i64,
) -> Result<Vec<RoleChannelAccess>, AppError> {
    let base_permissions = combined_perm_for_role(conn, guild_id, role_id).await?;

    let channels = voice_channel_permission_states(conn, guild_id).await?;
    if base_permissions.contains(Permissions::ADMINISTRATOR) {
        return Ok(channels
            .into_keys()
            .map(|channel_id| RoleChannelAccess {
                channel_id,
                viewable: true,
                joinable: true,
            })
            .collect());
    }

    let mut channels = channels;
    if role_id != guild_id {
        // @everyone (role_id = guild_id) needs no role-overwrite pass: its
        // per-channel overwrites are already the "everyone" state, and its
        // guild-level permission is in the base.
        apply_single_role_overwrites(conn, role_id, guild_id, &mut channels).await?;
    }

    Ok(channels
        .into_iter()
        .map(|(channel_id, state)| RoleChannelAccess {
            channel_id,
            viewable: state.can_view(base_permissions),
            joinable: state.can_view_and_connect(base_permissions),
        })
        .collect())
}

/// Per-channel access map for the role-preview lens, keyed by channel id and
/// validating that the role belongs to the guild. Every role-preview path
/// (recording tree, live stems, clips, stamps, the role view) goes through
/// here.
///
/// SECURITY: a preview never shows more than the manager's own normal view.
/// The map holds only channels in `visible_channels_for_user(manager_id)`
/// (VIEW_CHANNEL and CONNECT); everything else is dropped, not annotated.
/// Previewing needs only ADMINISTRATOR or MANAGE_GUILD
/// (`require_role_preview`), and in Discord only ADMINISTRATOR bypasses
/// channel overwrites, so a MANAGE_GUILD-only manager must not learn about
/// channels they cannot view and join themselves. Owners and Administrators
/// can view every channel, so their previews are unclipped.
pub async fn role_access_for_preview(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    manager: Viewer,
    role_id: i64,
) -> Result<HashMap<i64, RoleChannelAccess>, AppError> {
    let mut snapshot = begin_snapshot(pool).await?;
    let access = preview_access(&mut snapshot, guild_id, manager, role_id).await?;
    snapshot.commit().await?;
    Ok(access)
}

pub(super) async fn preview_access(
    conn: &mut PgConnection,
    guild_id: i64,
    manager: Viewer,
    role_id: i64,
) -> Result<HashMap<i64, RoleChannelAccess>, AppError> {
    require_role_in_guild(conn, guild_id, role_id).await?;

    let manager_channels = visible_channels(conn, guild_id, manager).await?;
    Ok(channel_access_for_role(conn, guild_id, role_id)
        .await?
        .into_iter()
        .filter(|access| manager_channels.contains(&access.channel_id))
        .map(|access| (access.channel_id, access))
        .collect())
}

pub(super) async fn apply_role_overwrites(
    conn: &mut PgConnection,
    user_id: i64,
    guild_id: i64,
    channels: &mut HashMap<i64, ChannelPermissionState>,
) -> Result<(), AppError> {
    let role_overwrites = sqlx::query!(
        "SELECT  allow as \"allow!\", deny as \"deny!\", channel_id as \"channel_id!\", role_id as
		\"role_id!\" FROM get_roles_overwrites_for_channels_from_user($1, $2)",
        user_id,
        guild_id
    )
    .fetch_all(&mut *conn)
    .await?;

    for overwrite in role_overwrites {
        if overwrite.channel_id == guild_id {
            // Generic @everyone is already included in the base guild permissions.
            continue;
        }

        if let Some(channel) = channels.get_mut(&overwrite.channel_id) {
            channel.roles.allow |= permissions_from_bits(overwrite.allow);
            channel.roles.deny |= permissions_from_bits(overwrite.deny);
        }
    }

    Ok(())
}
