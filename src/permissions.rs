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

    fn can_connect(self, base_permissions: Permissions) -> bool {
        let permissions = apply_overwrite(base_permissions, self.everyone);
        let permissions = apply_overwrite(permissions, self.roles);
        let permissions = apply_overwrite(permissions, self.member);

        permissions.contains(Permissions::CONNECT)
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
    .fetch_one(pool.get_ref())
    .await?;

    Ok(permissions_from_bits(res.permission))
}

pub async fn get_combined_perm_for_user(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    user_id: i64,
) -> Result<Permissions, AppError> {
    let res = sqlx::query!(
        "SELECT permissions, owner FROM user_guilds
		WHERE user_id = $1
		AND id = $2",
        user_id,
        guild_id
    )
    .fetch_one(pool.get_ref())
    .await?;
    if res.owner {
        return Ok(Permissions::all());
    }
    Ok(permissions_from_bits(res.permissions))
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
    let everyone_permission = get_everyone_permission_for_guild(pool, guild_id).await?;
    let combined_permission = get_combined_perm_for_user(pool, guild_id, user_id).await?;
    let base_permissions = everyone_permission | combined_permission;

    let mut channels = get_voice_channel_permission_states(pool, guild_id).await?;
    if base_permissions.contains(Permissions::ADMINISTRATOR) {
        return Ok(channels.keys().copied().collect());
    }

    apply_role_overwrites(pool, user_id, guild_id, &mut channels).await?;
    apply_member_overwrites(pool, user_id, guild_id, &mut channels).await?;

    Ok(channels
        .into_values()
        .filter(|channel| channel.can_connect(base_permissions))
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
    use crate::auth::{Access, Token};
    use actix_web::HttpMessage;
    let user_id = req
        .extensions()
        .get::<Token<Access>>()
        .map(|t| t.user_id)
        .ok_or(crate::errors::AppError::Unauthorized)?;
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

        assert!(!channel.can_connect(Permissions::CONNECT));
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

        assert!(channel.can_connect(Permissions::empty()));
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

        assert!(!channel.can_connect(Permissions::empty()));
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

        assert!(channel.can_connect(Permissions::empty()));
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
