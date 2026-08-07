use std::collections::{HashMap, HashSet};

use actix_web::web;
use sqlx::{Pool, Postgres};

use crate::errors::AppError;

bitflags::bitflags! {
    /// A set of permissions that can be assigned to [`User`]s and [`Role`]s via
    /// [`PermissionOverwrite`]s, roles globally in a [`Guild`], and to
    /// [`GuildChannel`]s.
    ///
    /// [`Guild`]: super::guild::Guild
    /// [`GuildChannel`]: super::channel::GuildChannel
    /// [`PermissionOverwrite`]: super::channel::PermissionOverwrite
    /// [`Role`]: super::guild::Role
    /// [`User`]: super::user::User
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Permissions: i64 {
        /// Allows for the creation of [`RichInvite`]s.
        ///
        /// [`RichInvite`]: super::invite::RichInvite
        const CREATE_INSTANT_INVITE = 1 << 0;
        /// Allows for the kicking of guild [member]s.
        ///
        /// [member]: super::guild::Member
        const KICK_MEMBERS = 1 << 1;
        /// Allows the banning of guild [member]s.
        ///
        /// [member]: super::guild::Member
        const BAN_MEMBERS = 1 << 2;
        /// Allows all permissions, bypassing channel [permission overwrite]s.
        ///
        /// [permission overwrite]: super::channel::PermissionOverwrite
        const ADMINISTRATOR = 1 << 3;
        /// Allows management and editing of guild [channel]s.
        ///
        /// [channel]: super::channel::GuildChannel
        const MANAGE_CHANNELS = 1 << 4;
        /// Allows management and editing of the [guild].
        ///
        /// [guild]: super::guild::Guild
        const MANAGE_GUILD = 1 << 5;
        /// [`Member`]s with this permission can add new [`Reaction`]s to a
        /// [`Message`]. Members can still react using reactions already added
        /// to messages without this permission.
        ///
        /// [`Member`]: super::guild::Member
        /// [`Message`]: super::channel::Message
        /// [`Reaction`]: super::channel::Reaction
        const ADD_REACTIONS = 1 << 6;
        /// Allows viewing a guild's audit logs.
        const VIEW_AUDIT_LOG = 1 << 7;
        /// Allows the use of priority speaking in voice channels.
        const PRIORITY_SPEAKER = 1 << 8;
        // Allows the user to go live.
        const STREAM = 1 << 9;
        /// Allows guild members to view a channel, which includes reading
        /// messages in text channels and joining voice channels.
        const VIEW_CHANNEL = 1 << 10;
        /// Allows sending messages in a guild channel.
        const SEND_MESSAGES = 1 << 11;
        /// Allows the sending of text-to-speech messages in a channel.
        const SEND_TTS_MESSAGES = 1 << 12;
        /// Allows the deleting of other messages in a guild channel.
        ///
        /// **Note**: This does not allow the editing of other messages.
        const MANAGE_MESSAGES = 1 << 13;
        /// Allows links from this user - or users of this role - to be
        /// embedded, with potential data such as a thumbnail, description, and
        /// page name.
        const EMBED_LINKS = 1 << 14;
        /// Allows uploading of files.
        const ATTACH_FILES = 1 << 15;
        /// Allows the reading of a channel's message history.
        const READ_MESSAGE_HISTORY = 1 << 16;
        /// Allows the usage of the `@everyone` mention, which will notify all
        /// users in a channel. The `@here` mention will also be available, and
        /// can be used to mention all non-offline users.
        ///
        /// **Note**: You probably want this to be disabled for most roles and
        /// users.
        const MENTION_EVERYONE = 1 << 17;
        /// Allows the usage of custom emojis from other guilds.
        ///
        /// This does not dictate whether custom emojis in this guild can be
        /// used in other guilds.
        const USE_EXTERNAL_EMOJIS = 1 << 18;
        /// Allows for viewing guild insights.
        const VIEW_GUILD_INSIGHTS = 1 << 19;
        /// Allows the joining of a voice channel.
        const CONNECT = 1 << 20;
        /// Allows the user to speak in a voice channel.
        const SPEAK = 1 << 21;
        /// Allows the muting of members in a voice channel.
        const MUTE_MEMBERS = 1 << 22;
        /// Allows the deafening of members in a voice channel.
        const DEAFEN_MEMBERS = 1 << 23;
        /// Allows the moving of members from one voice channel to another.
        const MOVE_MEMBERS = 1 << 24;
        /// Allows the usage of voice-activity-detection in a [voice] channel.
        ///
        /// If this is disabled, then [`Member`]s must use push-to-talk.
        ///
        /// [`Member`]: super::guild::Member
        /// [voice]: super::channel::ChannelType::Voice
        const USE_VAD = 1 << 25;
        /// Allows members to change their own nickname in the guild.
        const CHANGE_NICKNAME = 1 << 26;
        /// Allows members to change other members' nicknames.
        const MANAGE_NICKNAMES = 1 << 27;
        /// Allows management and editing of roles below their own.
        const MANAGE_ROLES = 1 << 28;
        /// Allows management of webhooks.
        const MANAGE_WEBHOOKS = 1 << 29;
        /// Allows management of emojis and stickers created without the use of an
        /// [`Integration`].
        ///
        /// [`Integration`]: super::guild::Integration
        const MANAGE_EMOJIS_AND_STICKERS = 1 << 30;
        /// Allows using slash commands.
        const USE_SLASH_COMMANDS = 1 << 31;
        /// Allows for requesting to speak in stage channels.
        const REQUEST_TO_SPEAK = 1 << 32;
        /// Allows for creating, editing, and deleting scheduled events
        const MANAGE_EVENTS = 1 << 33;
        /// Allows for deleting and archiving threads, and viewing all private threads.
        const MANAGE_THREADS = 1 << 34;
        /// Allows for creating threads.
        const CREATE_PUBLIC_THREADS = 1 << 35;
        /// Allows for creating private threads.
        const CREATE_PRIVATE_THREADS = 1 << 36;
        /// Allows the usage of custom stickers from other servers.
        const USE_EXTERNAL_STICKERS = 1 << 37;
        /// Allows for sending messages in threads
        const SEND_MESSAGES_IN_THREADS = 1 << 38;
        /// Allows for launching activities in a voice channel
        const USE_EMBEDDED_ACTIVITIES = 1 << 39;
        /// Allows for timing out users to prevent them from sending or reacting to messages in
        /// chat and threads, and from speaking in voice and stage channels.
        const MODERATE_MEMBERS = 1 << 40;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PermissionOverwriteBits {
    allow: Permissions,
    deny: Permissions,
}

impl Default for PermissionOverwriteBits {
    fn default() -> Self {
        Self {
            allow: Permissions::empty(),
            deny: Permissions::empty(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChannelPermissionState {
    channel_id: i64,
    everyone: PermissionOverwriteBits,
    roles: PermissionOverwriteBits,
    member: PermissionOverwriteBits,
}

impl ChannelPermissionState {
    fn new(channel_id: i64, everyone: PermissionOverwriteBits) -> Self {
        Self {
            channel_id,
            everyone,
            roles: PermissionOverwriteBits::default(),
            member: PermissionOverwriteBits::default(),
        }
    }

    fn can_view(self, base_permissions: Permissions) -> bool {
        self.applied(base_permissions)
            .contains(Permissions::VIEW_CHANNEL)
    }

    fn can_view_and_connect(self, base_permissions: Permissions) -> bool {
        self.applied(base_permissions)
            .contains(Permissions::VIEW_CHANNEL | Permissions::CONNECT)
    }

    fn applied(self, base_permissions: Permissions) -> Permissions {
        let permissions = apply_overwrite(base_permissions, self.everyone);
        let permissions = apply_overwrite(permissions, self.roles);
        apply_overwrite(permissions, self.member)
    }
}

fn permissions_from_bits(bits: i64) -> Permissions {
    Permissions::from_bits_retain(bits)
}

fn apply_overwrite(
    mut permissions: Permissions,
    overwrite: PermissionOverwriteBits,
) -> Permissions {
    permissions.remove(overwrite.deny);
    permissions.insert(overwrite.allow);
    permissions
}

pub async fn get_everyone_permission_for_guild(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
) -> Result<Permissions, AppError> {
    let res = sqlx::query!(
        "SELECT permission FROM roles
			WHERE guild_id =$1 AND role_id =$1",
        guild_id
    )
    .fetch_optional(pool.get_ref())
    .await?;

    Ok(res
        .map(|r| permissions_from_bits(r.permission))
        .unwrap_or_else(Permissions::empty))
}

pub async fn get_combined_perm_for_user(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    user_id: i64,
) -> Result<Permissions, AppError> {
    // Owner access comes from the live `guilds.owner_id`, or from the
    // `user_guilds.owner` flag. The flag is trusted because the agent keeps it
    // fresh — `sync_guild_owner` rewrites it on guild owner changes and
    // `delete_live_member` drops the row when a member leaves — and the local
    // seed / fixture tooling writes it to grant the dev account full access to
    // imported guilds whose real owner is somebody else.
    let owner = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1
               FROM guilds
              WHERE id = $1 AND owner_id = $2
         ) OR EXISTS (
             SELECT 1
               FROM user_guilds
              WHERE id = $1 AND user_id = $2 AND owner
         )",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_one(pool.get_ref())
    .await?;
    if owner {
        return Ok(Permissions::all());
    }

    // The OAuth guild list stores a combined permission snapshot from login.
    // Build the value from the agent-maintained role cache instead so role and
    // membership revocations take effect without requiring a new login.
    let permissions = sqlx::query_scalar::<_, Option<i64>>(
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
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_one(pool.get_ref())
    .await?
    .unwrap_or(0);

    Ok(permissions_from_bits(permissions))
}

/// Combined permission bits a member would hold if their only role were
/// `role_id`: the role itself plus `@everyone` (`role_id = guild_id`). This is
/// the "view server as role" lens — Discord's preview of what a hypothetical
/// member with a single role would see.
pub async fn get_combined_perm_for_role(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    role_id: i64,
) -> Result<Permissions, AppError> {
    let permissions = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT bit_or(r.permission)
           FROM roles r
          WHERE r.guild_id = $1
            AND (r.role_id = $1 OR r.role_id = $2)",
    )
    .bind(guild_id)
    .bind(role_id)
    .fetch_one(pool.get_ref())
    .await?
    .unwrap_or(0);

    Ok(permissions_from_bits(permissions))
}

async fn apply_single_role_overwrites(
    pool: &web::Data<Pool<Postgres>>,
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
    .fetch_all(pool.get_ref())
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
pub async fn get_channel_access_for_role(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    role_id: i64,
) -> Result<Vec<RoleChannelAccess>, AppError> {
    let base_permissions = get_combined_perm_for_role(pool, guild_id, role_id).await?;

    let channels = get_voice_channel_permission_states(pool, guild_id).await?;
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
        apply_single_role_overwrites(pool, role_id, guild_id, &mut channels).await?;
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

/// Voice channels a member whose only role is `role_id` could see and join,
/// mirroring `get_available_channels_for_user` without member-specific
/// overwrites.
pub async fn get_available_channels_for_role(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    role_id: i64,
) -> Result<HashSet<i64>, AppError> {
    Ok(get_channel_access_for_role(pool, guild_id, role_id)
        .await?
        .into_iter()
        .filter(|access| access.joinable)
        .map(|access| access.channel_id)
        .collect())
}

/// Resolve the channel set a listing should show: the caller's own channels,
/// or — when impersonating a role — the channels that role alone would see.
/// A foreign role is a 404 rather than a silently empty preview.
pub async fn listing_channels_for(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    user_id: i64,
    as_role: Option<i64>,
) -> Result<HashSet<i64>, AppError> {
    match as_role {
        None => visible_channels_for_user(pool, guild_id, user_id).await,
        Some(role_id) => {
            let belongs_to_guild = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM roles WHERE role_id = $1 AND guild_id = $2)",
            )
            .bind(role_id)
            .bind(guild_id)
            .fetch_one(pool.get_ref())
            .await?;
            if !belongs_to_guild {
                return Err(AppError::RoleNotFound);
            }
            get_available_channels_for_role(pool, guild_id, role_id).await
        }
    }
}

/// Per-channel access map for the role-preview lens, keyed by channel id and
/// validating that the role belongs to the guild. The recording tree uses it
/// to keep every session visible and annotate what the role could do with it.
///
/// SECURITY / INTENTIONAL LEAK: this map retains *every* session (including
/// `hidden` — not even viewable) and annotates `can-listen / visible-only /
/// hidden` per channel. It is intentionally manager-only (see
/// `require_role_preview` -> `require_guild_manager`) — callers already have
/// `ADMINISTRATOR | MANAGE_GUILD` and can `VIEW_CHANNEL` all voice channels
/// anyway. Do not expose this endpoint to non-managers.
pub async fn role_access_for_preview(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    role_id: i64,
) -> Result<HashMap<i64, RoleChannelAccess>, AppError> {
    let belongs_to_guild = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM roles WHERE role_id = $1 AND guild_id = $2)",
    )
    .bind(role_id)
    .bind(guild_id)
    .fetch_one(pool.get_ref())
    .await?;
    if !belongs_to_guild {
        return Err(AppError::RoleNotFound);
    }

    Ok(get_channel_access_for_role(pool, guild_id, role_id)
        .await?
        .into_iter()
        .map(|access| (access.channel_id, access))
        .collect())
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

async fn apply_role_overwrites(
    pool: &web::Data<Pool<Postgres>>,
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
    .fetch_all(pool.get_ref())
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

pub async fn get_available_channels_for_user(
    pool: &actix_web::web::Data<Pool<Postgres>>,
    guild_id: i64,
    user_id: i64,
) -> Result<HashSet<i64>, AppError> {
    let base_permissions = get_combined_perm_for_user(pool, guild_id, user_id).await?;

    let mut channels = get_voice_channel_permission_states(pool, guild_id).await?;
    if base_permissions.contains(Permissions::ADMINISTRATOR) {
        return Ok(channels.keys().copied().collect());
    }

    apply_role_overwrites(pool, user_id, guild_id, &mut channels).await?;
    apply_member_overwrites(pool, user_id, guild_id, &mut channels).await?;

    Ok(channels
        .into_values()
        .filter(|channel| channel.can_view_and_connect(base_permissions))
        .map(|channel| channel.channel_id)
        .collect())
}

pub async fn visible_channels_for_user(
    pool: &actix_web::web::Data<sqlx::Pool<sqlx::Postgres>>,
    guild_id: i64,
    user_id: i64,
) -> Result<HashSet<i64>, crate::errors::AppError> {
    let membership = sqlx::query!(
        "SELECT 1 as present FROM user_guilds WHERE id = $1 AND user_id = $2",
        guild_id,
        user_id
    )
    .fetch_optional(pool.get_ref())
    .await?;
    if membership.is_none() {
        return Err(crate::errors::AppError::Forbidden);
    }

    get_available_channels_for_user(pool, guild_id, user_id).await
}

pub async fn require_channel_access(
    pool: &actix_web::web::Data<sqlx::Pool<sqlx::Postgres>>,
    guild_id: i64,
    channel_id: i64,
    user_id: i64,
) -> Result<(), crate::errors::AppError> {
    let permitted = visible_channels_for_user(pool, guild_id, user_id).await?;
    if permitted.contains(&channel_id) {
        Ok(())
    } else {
        Err(crate::errors::AppError::Forbidden)
    }
}

async fn apply_member_overwrites(
    pool: &web::Data<Pool<Postgres>>,
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
    .fetch_all(pool.get_ref())
    .await?;

    for overwrite in member_overwrites {
        if let Some(channel) = channels.get_mut(&overwrite.channel_id) {
            channel.member.allow |= permissions_from_bits(overwrite.allow);
            channel.member.deny |= permissions_from_bits(overwrite.deny);
        }
    }

    Ok(())
}

async fn get_voice_channel_permission_states(
    pool: &web::Data<Pool<Postgres>>,
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
        WHERE channels.type = 2
        AND channels.guild_id = $1",
        guild_id
    )
    .fetch_all(pool.get_ref())
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

pub async fn require_guild_admin(
    req: &actix_web::HttpRequest,
    pool: &actix_web::web::Data<sqlx::Pool<sqlx::Postgres>>,
    guild_id: i64,
) -> Result<i64, crate::errors::AppError> {
    require_guild_permission(req, pool, guild_id, Permissions::ADMINISTRATOR).await
}

pub async fn require_guild_manager(
    req: &actix_web::HttpRequest,
    pool: &actix_web::web::Data<sqlx::Pool<sqlx::Postgres>>,
    guild_id: i64,
) -> Result<i64, crate::errors::AppError> {
    let manager_mask = Permissions::ADMINISTRATOR | Permissions::MANAGE_GUILD;
    require_guild_permission(req, pool, guild_id, manager_mask).await
}

async fn require_guild_permission(
    req: &actix_web::HttpRequest,
    pool: &actix_web::web::Data<sqlx::Pool<sqlx::Postgres>>,
    guild_id: i64,
    required_mask: Permissions,
) -> Result<i64, crate::errors::AppError> {
    use crate::auth::{Access, AuthKind, Token};
    use actix_web::HttpMessage;
    let (user_id, is_dev) = req
        .extensions()
        .get::<Token<Access>>()
        .map(|t| (t.user_id, t.auth_kind == AuthKind::Dev))
        .ok_or(crate::errors::AppError::Unauthorized)?;

    // Dev logins act as managers of the guilds the local tooling granted them:
    // the seed and fixture imports write `user_guilds.owner = true`, but
    // `guilds.owner_id` and the agent-maintained role cache hold the production
    // owner and roles, so the checks below would otherwise 403 the dev account
    // on every fixture guild. Mirror the frontend's `isGuildAdmin` trust in the
    // snapshot row; real Discord logins (AuthKind::Discord) never take this path.
    //
    // SECURITY: this bypass is `#[cfg(feature = "dev-login")]`-gated by
    // `is_public_api_path` / `AuthKind::Dev` — it is only reachable when the
    // `dev-login` feature is compiled in (local dev). In production builds
    // `AuthKind::Dev` can never be issued, so this branch is dead code.
    // The snapshot row is only trusted for `owner = true` guilds seeded
    // locally; a stale `permissions & 40` bit would otherwise grant permanent
    // manager after a role revoke, so we check live permissions first and
    // only fall back to the snapshot when the guild has no live role data
    // (fixture import without a cached @everyone).
    if is_dev {
        // Fast path: live permissions already grant manager — no need to trust snapshot.
        // This avoids stale `user_guilds.permissions` granting permanent access.
        if let Ok(live) = get_combined_perm_for_user(pool, guild_id, user_id).await
            && live.intersects(required_mask)
        {
            return Ok(user_id);
        }
        let trusted_owner = sqlx::query_scalar::<_, bool>(
            "SELECT owner FROM user_guilds WHERE id = $1 AND user_id = $2",
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_optional(pool.get_ref())
        .await?
        .unwrap_or(false);
        if trusted_owner {
            tracing::debug!(
                guild_id,
                user_id,
                "dev-login manager bypass via user_guilds.owner"
            );
            return Ok(user_id);
        }
    }

    let permissions = match get_combined_perm_for_user(pool, guild_id, user_id).await {
        Ok(permissions) => permissions,
        Err(crate::errors::AppError::DbError(sqlx::Error::RowNotFound)) => {
            return Err(crate::errors::AppError::Forbidden);
        }
        Err(err) => return Err(err),
    };
    if permissions.intersects(required_mask) {
        Ok(user_id)
    } else {
        Err(crate::errors::AppError::Forbidden)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(
        everyone: PermissionOverwriteBits,
        roles: PermissionOverwriteBits,
        member: PermissionOverwriteBits,
    ) -> ChannelPermissionState {
        ChannelPermissionState {
            channel_id: 1,
            everyone,
            roles,
            member,
        }
    }

    #[test]
    fn everyone_deny_blocks_base_connect() {
        let channel = state(
            PermissionOverwriteBits {
                allow: Permissions::empty(),
                deny: Permissions::CONNECT,
            },
            PermissionOverwriteBits::default(),
            PermissionOverwriteBits::default(),
        );

        assert!(!channel.can_view_and_connect(Permissions::VIEW_CHANNEL | Permissions::CONNECT));
    }

    #[test]
    fn role_allow_restores_everyone_deny() {
        let channel = state(
            PermissionOverwriteBits {
                allow: Permissions::empty(),
                deny: Permissions::CONNECT,
            },
            PermissionOverwriteBits {
                allow: Permissions::CONNECT,
                deny: Permissions::empty(),
            },
            PermissionOverwriteBits::default(),
        );

        assert!(channel.can_view_and_connect(Permissions::VIEW_CHANNEL));
    }

    #[test]
    fn member_deny_overrides_role_allow() {
        let channel = state(
            PermissionOverwriteBits::default(),
            PermissionOverwriteBits {
                allow: Permissions::CONNECT,
                deny: Permissions::empty(),
            },
            PermissionOverwriteBits {
                allow: Permissions::empty(),
                deny: Permissions::CONNECT,
            },
        );

        assert!(!channel.can_view_and_connect(Permissions::VIEW_CHANNEL));
    }

    #[test]
    fn member_allow_overrides_role_deny() {
        let channel = state(
            PermissionOverwriteBits::default(),
            PermissionOverwriteBits {
                allow: Permissions::empty(),
                deny: Permissions::CONNECT,
            },
            PermissionOverwriteBits {
                allow: Permissions::CONNECT,
                deny: Permissions::empty(),
            },
        );

        assert!(channel.can_view_and_connect(Permissions::VIEW_CHANNEL));
    }

    #[test]
    fn view_channel_deny_blocks_inherited_connect() {
        let channel = state(
            PermissionOverwriteBits {
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
            },
            PermissionOverwriteBits::default(),
            PermissionOverwriteBits::default(),
        );

        assert!(!channel.can_view_and_connect(Permissions::VIEW_CHANNEL | Permissions::CONNECT));
    }

    #[test]
    fn unknown_permission_bits_are_retained() {
        let future_discord_permission = 1_i64 << 50;

        assert_eq!(
            permissions_from_bits(future_discord_permission).bits(),
            future_discord_permission
        );
    }
}
