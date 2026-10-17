//! What a member may do in a guild, answered from the gateway cache by the
//! evaluator the web server uses (`sakiot-permissions`), so `/jam` and the
//! dashboard agree.

use sakiot_permissions::{
    Capabilities, ChannelFacts, GuildFacts, MemberFacts, Overwrite, Permissions, Roster, Viewer,
};
use serenity::client::Cache;
use serenity::model::channel::{ChannelType, PermissionOverwriteType};
use serenity::model::guild::Guild;
use serenity::model::id::{GuildId, RoleId, UserId};

use crate::cast::ToI64;

/// The voice and stage channels where the user may hear other members; a
/// clip is theirs to play when every channel its audio came from is one of
/// them. Empty while the guild is not cached.
pub(crate) fn listening_channels(cache: &Cache, guild_id: GuildId, user_id: UserId) -> Vec<i64> {
    cache
        .guild(guild_id)
        .map_or_else(Vec::new, |guild| listening_channels_in(&guild, user_id))
}

pub(crate) fn listening_channels_in(guild: &Guild, user_id: UserId) -> Vec<i64> {
    guild_facts(guild, user_id)
        .access(Viewer::discord(user_id.to_i64()))
        .map(|access| access.channels_with(Capabilities::LISTEN_OTHERS).collect())
        .unwrap_or_default()
}

/// What the evaluator needs about `user_id` in `guild`, as the gateway cache
/// knows it. The cache's member list is the roster: someone missing from it
/// is not a member, unless they own the guild.
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
            },
        );
    }
    facts
}

fn bits(permissions: serenity::model::Permissions) -> Permissions {
    Permissions::from_bits_retain(permissions.bits().to_i64())
}
