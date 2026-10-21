//! What a member may do in a guild, answered from the gateway cache by the
//! evaluator the web server uses (`sakiot-permissions`), so `/jam` and the
//! dashboard agree.

use sakiot_permissions::{
    Capabilities, ChannelFacts, GuildFacts, MemberFacts, Membership, Overwrite, Permissions,
    Roster, Viewer,
};
use serenity::client::Cache;
use serenity::model::channel::{ChannelType, PermissionOverwriteType};
use serenity::model::guild::Guild;
use serenity::model::id::{GuildId, RoleId, UserId};

use crate::cast::ToI64;

/// Which clips a user may hear (`Access::allows_item`), for a query: those
/// of only `own`'s voice, and those whose every channel is in `channels`.
/// The query asks the speakers' consent itself (`recording_private`): the
/// gateway cache does not know it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Listening {
    /// The user, while a member: their own voice is always theirs.
    pub(crate) own: Option<i64>,
    /// The voice and stage channels where they may hear other members.
    pub(crate) channels: Vec<i64>,
}

/// What the user may hear in the guild; nothing while it is not cached.
pub(crate) fn listening(cache: &Cache, guild_id: GuildId, user_id: UserId) -> Listening {
    cache
        .guild(guild_id)
        .map_or_else(Listening::default, |guild| listening_in(&guild, user_id))
}

pub(crate) fn listening_in(guild: &Guild, user_id: UserId) -> Listening {
    let Ok(access) = guild_facts(guild, user_id).access(Viewer::discord(user_id.to_i64())) else {
        return Listening::default();
    };
    Listening {
        own: access
            .user_id
            .filter(|_| access.membership != Membership::Outsider),
        channels: access.channels_with(Capabilities::LISTEN_OTHERS).collect(),
    }
}

/// What the evaluator needs about `user_id` in `guild`, as the gateway cache
/// knows it. The cache's member list is the roster: someone missing from it
/// is not a member, unless they own the guild. The cache knows nothing of
/// recordings, so `/jam` treats a former member as an outsider: they are no
/// longer in the guild to use it.
fn guild_facts(guild: &Guild, user_id: UserId) -> GuildFacts {
    let member = guild.members.get(&user_id);
    let roles: &[RoleId] = member.map_or(&[], |member| &member.roles);
    let mut facts = GuildFacts::new(guild.id.to_i64());
    facts.owner_id = Some(guild.owner_id.to_i64());
    facts.roster = Roster::Complete;
    facts.bot_present = true;
    facts.everyone = guild
        .roles
        .get(&RoleId::new(guild.id.get()))
        .map(|role| bits(role.permissions));
    facts.roles = roles
        .iter()
        .filter_map(|role_id| guild.roles.get(role_id))
        .map(|role| (role.id.to_i64(), bits(role.permissions)))
        .collect();
    facts.channels = guild
        .channels
        .values()
        .filter(|channel| matches!(channel.kind, ChannelType::Voice | ChannelType::Stage))
        .map(|channel| {
            let mut overwrites = ChannelFacts::default();
            for overwrite in &channel.permission_overwrites {
                let value = Overwrite {
                    allow: bits(overwrite.allow),
                    deny: bits(overwrite.deny),
                };
                match overwrite.kind {
                    PermissionOverwriteType::Role(role) if role.get() == guild.id.get() => {
                        overwrites.everyone = value;
                    }
                    PermissionOverwriteType::Role(role) if roles.contains(&role) => {
                        overwrites.roles.insert(role.to_i64(), value);
                    }
                    PermissionOverwriteType::Member(member) if member == user_id => {
                        overwrites.members.insert(member.to_i64(), value);
                    }
                    _ => {}
                }
            }
            (channel.id.to_i64(), overwrites)
        })
        .collect();
    if member.is_some() {
        facts.members.insert(
            user_id.to_i64(),
            MemberFacts {
                listed: true,
                roles: roles.iter().map(|role| role.to_i64()).collect(),
                // A listed member belongs whatever they have recorded.
                recorded: false,
            },
        );
    }
    facts
}

fn bits(permissions: serenity::model::Permissions) -> Permissions {
    Permissions::from_bits_retain(permissions.bits().to_i64())
}
