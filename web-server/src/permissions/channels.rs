//! The channels a member can see and join, from their roles and the
//! channel overwrites that apply to them.

use super::*;

pub async fn get_available_channels_for_user(
    pool: &actix_web::web::Data<Pool<Postgres>>,
    guild_id: i64,
    viewer: Viewer,
) -> Result<HashSet<i64>, AppError> {
    let mut snapshot = begin_snapshot(pool).await?;
    let channels = available_channels_for_user(&mut snapshot, guild_id, viewer).await?;
    snapshot.commit().await?;
    Ok(channels)
}

async fn available_channels_for_user(
    conn: &mut PgConnection,
    guild_id: i64,
    viewer: Viewer,
) -> Result<HashSet<i64>, AppError> {
    Ok(channel_access_for_user(conn, guild_id, viewer)
        .await?
        .into_iter()
        .filter(|access| access.joinable)
        .map(|access| access.channel_id)
        .collect())
}

/// Per-channel access for one member, over every voice and stage channel:
/// `viewable` is VIEW_CHANNEL, `joinable` adds CONNECT.
async fn channel_access_for_user(
    conn: &mut PgConnection,
    guild_id: i64,
    viewer: Viewer,
) -> Result<Vec<RoleChannelAccess>, AppError> {
    let base_permissions = combined_perm_for_user(conn, guild_id, viewer).await?;

    let mut channels = voice_channel_permission_states(conn, guild_id).await?;
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

    apply_role_overwrites(conn, viewer.user_id, guild_id, &mut channels).await?;
    apply_member_overwrites(conn, viewer.user_id, guild_id, &mut channels).await?;

    Ok(channels
        .into_values()
        .map(|channel| RoleChannelAccess {
            channel_id: channel.channel_id,
            viewable: channel.can_view(base_permissions),
            joinable: channel.can_view_and_connect(base_permissions),
        })
        .collect())
}

/// The voice and stage channels whose occupants a viewer may see, read on
/// `conn` (use `begin_snapshot`). As in Discord's channel list, VIEW_CHANNEL
/// is enough: seeing who is in a channel does not need CONNECT. A
/// non-member is forbidden.
///
/// With `as_role` (the caller must already have passed
/// `require_role_preview`), the channels that role alone could view,
/// clipped to the manager's own presence view: a preview never shows more
/// than the manager's normal view (phase 0.1).
pub async fn presence_channels(
    conn: &mut PgConnection,
    guild_id: i64,
    viewer: Viewer,
    as_role: Option<i64>,
) -> Result<HashSet<i64>, AppError> {
    require_member(conn, guild_id, viewer).await?;
    let own: HashSet<i64> = channel_access_for_user(conn, guild_id, viewer)
        .await?
        .into_iter()
        .filter(|access| access.viewable)
        .map(|access| access.channel_id)
        .collect();
    let Some(role_id) = as_role else {
        return Ok(own);
    };
    require_role_in_guild(conn, guild_id, role_id).await?;
    Ok(channel_access_for_role(conn, guild_id, role_id)
        .await?
        .into_iter()
        .filter(|access| access.viewable && own.contains(&access.channel_id))
        .map(|access| access.channel_id)
        .collect())
}

pub async fn visible_channels_for_user(
    pool: &actix_web::web::Data<sqlx::Pool<sqlx::Postgres>>,
    guild_id: i64,
    viewer: Viewer,
) -> Result<HashSet<i64>, crate::errors::AppError> {
    crate::server_timing::measure("perm", async {
        let mut snapshot = begin_snapshot(pool).await?;
        let channels = visible_channels(&mut snapshot, guild_id, viewer).await?;
        snapshot.commit().await?;
        Ok(channels)
    })
    .await
}

/// The channels a guild member can view and join, read on `conn` so a
/// caller can combine it with its own reads in one snapshot
/// (`begin_snapshot`). A non-member is forbidden.
pub async fn visible_channels(
    conn: &mut PgConnection,
    guild_id: i64,
    viewer: Viewer,
) -> Result<HashSet<i64>, AppError> {
    require_member(conn, guild_id, viewer).await?;
    available_channels_for_user(conn, guild_id, viewer).await
}

async fn apply_member_overwrites(
    conn: &mut PgConnection,
    user_id: i64,
    guild_id: i64,
    channels: &mut HashMap<i64, ChannelPermissionState>,
) -> Result<(), AppError> {
    let member_overwrites = sqlx::query!(
        "SELECT allow as \"allow!\", deny as \"deny!\", channel_id as \"channel_id!\"
        FROM get_user_channel_overriders_for_user_id($1, $2)",
        user_id,
        guild_id
    )
    .fetch_all(&mut *conn)
    .await?;

    for overwrite in member_overwrites {
        if let Some(channel) = channels.get_mut(&overwrite.channel_id) {
            channel.member.allow |= permissions_from_bits(overwrite.allow);
            channel.member.deny |= permissions_from_bits(overwrite.deny);
        }
    }

    Ok(())
}

/// `@everyone` overwrite state for every channel the agent can record: voice
/// (type 2) and stage (type 13). The SQL overwrite helpers used by
/// `apply_role_overwrites` and `apply_member_overwrites` filter on the same
/// types; keep them in step.
pub(super) async fn voice_channel_permission_states(
    conn: &mut PgConnection,
    guild_id: i64,
) -> Result<HashMap<i64, ChannelPermissionState>, AppError> {
    let channel_overwrites = sqlx::query!(
        "SELECT
            channels.channel_id as \"channel_id!\",
            COALESCE(channel_permissions.allow, 0) as \"allow!\",
            COALESCE(channel_permissions.deny, 0) as \"deny!\"
        FROM channels
        LEFT JOIN channel_permissions
            ON channels.channel_id = channel_permissions.channel_id
            AND channel_permissions.target_id = $1
        WHERE channels.type IN (2, 13)
        AND channels.guild_id = $1",
        guild_id
    )
    .fetch_all(&mut *conn)
    .await?;

    Ok(channel_overwrites
        .into_iter()
        .map(|overwrite| {
            (
                overwrite.channel_id,
                ChannelPermissionState::new(
                    overwrite.channel_id,
                    PermissionOverwriteBits {
                        allow: permissions_from_bits(overwrite.allow),
                        deny: permissions_from_bits(overwrite.deny),
                    },
                ),
            )
        })
        .collect())
}
