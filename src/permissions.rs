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

pub async fn get_everyone_permission_for_guild(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
) -> Result<i64, AppError> {
    let res = sqlx::query!(
        "SELECT permission FROM roles
			WHERE guild_id =$1 AND role_id =$1",
        guild_id
    )
    .fetch_one(pool.get_ref())
    .await?;

    Ok(res.permission)
}

pub async fn get_combined_perm_for_user(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    user_id: i64,
) -> Result<i64, AppError> {
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
        return Ok(Permissions::all().bits());
    }
    Ok(res.permissions)
}

pub async fn perms_for_roles_for_channel(
    pool: &web::Data<Pool<Postgres>>,
    user_id: i64,
    guild_id: i64,
    perm_hash: &mut HashMap<i64, [i64; 2]>,
) -> Result<(), AppError> {
    let perms_for_roles_for_channel = sqlx::query!(
        "SELECT  allow as \"allow!\", deny as \"deny!\", channel_id as \"channel_id!\", role_id as
		\"role_id!\" FROM get_roles_overwrites_for_channels_from_user($1, $2)",
        user_id,
        guild_id
    )
    .fetch_all(pool.get_ref())
    .await?;

    for perm in perms_for_roles_for_channel {
        if perm.channel_id == guild_id {
            // Dont include the generic @everyone
            continue;
        } else {
            match perm_hash.get_mut(&perm.channel_id) {
                Some(ok) => {
                    ok[0] |= perm.allow;
                    ok[1] |= perm.deny;
                }
                None => {
                    perm_hash.insert(perm.channel_id, [perm.allow, perm.deny]);
                }
            }
        }
    }
    Ok(())
}

pub async fn get_available_channels_for_user(
    pool: &actix_web::web::Data<Pool<Postgres>>,
    guild_id: i64,
    user_id: i64,
) -> Result<HashSet<i64>, AppError> {
    // [0] = allow, [1] = deny
    let mut perm_hash: HashMap<i64, [i64; 2]> = HashMap::new();
    let mut allowed_channels: HashSet<i64> = HashSet::new();
    let mut denied_channels: HashSet<i64> = HashSet::new();

    let everyone_permission = get_everyone_permission_for_guild(pool, guild_id).await?;
    let combined_permission = get_combined_perm_for_user(pool, guild_id, user_id).await?;

    // This is the highest non-specific permission for user
    let total_permission = everyone_permission | combined_permission;

    get_user_channel_overrides_for_user_id(user_id, guild_id, pool, &mut perm_hash).await?;

    // Is admin
    if (total_permission & Permissions::ADMINISTRATOR.bits()) == Permissions::ADMINISTRATOR.bits() {
        perm_hash.retain(|ch_id, _| {
            allowed_channels.insert(*ch_id);

            false
        });
    }

    perm_hash.retain(|ch_id, perm_vec| {
        let allow = (perm_vec[0] & Permissions::CONNECT.bits()) == Permissions::CONNECT.bits();
        let deny = (perm_vec[1] & Permissions::CONNECT.bits()) == Permissions::CONNECT.bits();

        if allow {
            allowed_channels.insert(*ch_id);
        }
        if deny {
            denied_channels.insert(*ch_id);
        }

        !allow && !deny
    });

    perms_for_roles_for_channel(pool, user_id, guild_id, &mut perm_hash).await?;

    perm_hash.retain(|ch_id, perm_vec| {
        let allow = (perm_vec[0] & Permissions::CONNECT.bits()) == Permissions::CONNECT.bits();
        let deny = (perm_vec[1] & Permissions::CONNECT.bits()) == Permissions::CONNECT.bits();

        // If both allow and deny are true the allow value overrides the deny
        if (allow) && (deny) {
            allowed_channels.insert(*ch_id);
            return !allow && !deny;
        }

        if allow {
            allowed_channels.insert(*ch_id);
        }
        if deny {
            denied_channels.insert(*ch_id);
        }

        !allow && !deny
    });

    get_everyone_permission_for_each_channel(pool, guild_id, &mut perm_hash).await?;

    perm_hash.retain(|ch_id, perm_vec| {
        let allow = (perm_vec[0] & Permissions::CONNECT.bits()) == Permissions::CONNECT.bits();
        let deny = (perm_vec[1] & Permissions::CONNECT.bits()) == Permissions::CONNECT.bits();

        if allow {
            allowed_channels.insert(*ch_id);
        }
        if deny {
            denied_channels.insert(*ch_id);
        }

        !allow && !deny
    });

    // Final most general check. Check @everyone
    perm_hash.retain(|ch_id, perm_vec| {
        let allow = ((perm_vec[0] | total_permission) & Permissions::CONNECT.bits())
            == Permissions::CONNECT.bits();

        if allow {
            allowed_channels.insert(*ch_id);
        }

        !allow
    });

    Ok(allowed_channels)
}

async fn get_user_channel_overrides_for_user_id(
    user_id: i64,
    guild_id: i64,
    pool: &actix_web::web::Data<Pool<Postgres>>,
    perm_hash: &mut HashMap<i64, [i64; 2]>,
) -> Result<(), AppError> {
    let specific_perm_for_channel = sqlx::query!(
        "SELECT allow, deny, channel_id as \"channel_id!\", name as
			\"name!\" FROM get_user_channel_overriders_for_user_id($1, $2)",
        user_id,
        guild_id
    )
    .fetch_all(pool.get_ref())
    .await?;

    // member specific override
    for specific_perm in specific_perm_for_channel {
        perm_hash.insert(
            specific_perm.channel_id,
            [
                specific_perm.allow.unwrap_or(0),
                specific_perm.deny.unwrap_or(0),
            ],
        );
    }
    Ok(())
}

pub async fn get_everyone_permission_for_each_channel(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    perm_hash: &mut HashMap<i64, [i64; 2]>,
) -> Result<(), AppError> {
    let everyone_permissions = sqlx::query!(
        "SELECT channel_permissions.allow as \"allow?\", channel_permissions.deny as \"deny?\",  channels.channel_id, channels.name
		FROM channels
		LEFT JOIN channel_permissions
		ON channels.channel_id=channel_permissions.channel_id
		WHERE channels.type = 2
		AND channels.guild_id=$1
		AND (channel_permissions.target_id=$1 OR channel_permissions.target_id IS NULL)", guild_id
    )
    .fetch_all(pool.get_ref())
    .await?;

    for channel_perm in everyone_permissions {
        match perm_hash.get_mut(&channel_perm.channel_id) {
            Some(ok) => {
                ok[0] |= channel_perm.allow.unwrap_or(0);
                ok[1] |= channel_perm.deny.unwrap_or(0);
            }
            None => {
                perm_hash.insert(
                    channel_perm.channel_id,
                    [
                        channel_perm.allow.unwrap_or(0),
                        channel_perm.deny.unwrap_or(0),
                    ],
                );
            }
        }
    }
    Ok(())
}

pub async fn require_guild_admin(
    req: &actix_web::HttpRequest,
    pool: &actix_web::web::Data<sqlx::Pool<sqlx::Postgres>>,
    guild_id: i64,
) -> Result<i64, crate::errors::AppError> {
    use crate::auth::{Access, Token};
    use actix_web::HttpMessage;
    let user_id = req
        .extensions()
        .get::<Token<Access>>()
        .map(|t| t.user_id)
        .ok_or(crate::errors::AppError::Unauthorized)?;
    let bits = get_combined_perm_for_user(pool, guild_id, user_id).await?;
    let admin_mask = Permissions::ADMINISTRATOR.bits() | Permissions::MANAGE_GUILD.bits();
    if bits & admin_mask != 0 {
        Ok(user_id)
    } else {
        Err(crate::errors::AppError::Forbidden)
    }
}
