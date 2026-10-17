//! The generated guilds as the bot's gateway cache (serenity) holds them:
//! for checking the Discord rules against serenity's, and the bot's facts
//! against the cache they are built from (the `serenity` feature).

use serenity::model::Permissions;
use serenity::model::channel::{
    ChannelType, GuildChannel, PermissionOverwrite, PermissionOverwriteType,
};
use serenity::model::guild::{Guild, Member, Role};
use serenity::model::id::{ChannelId, GuildId, RoleId, UserId};
use serenity::model::user::User;

use super::{ChannelKind, Target, World};

/// A generated id as Discord's. Every generated id is positive.
pub fn id(value: i64) -> u64 {
    value.unsigned_abs()
}

/// Generated bits as serenity's; it drops the ones Discord has not defined.
fn bits(value: i64) -> Permissions {
    Permissions::from_bits_truncate(id(value))
}

/// Guild `g` as the cache holds it: its roles, `@everyone` included once
/// known; every channel; and every generated user as a member holding their
/// roles. Which of them the gateway has delivered is the caller's to model.
pub fn guild(world: &World, g: usize) -> Guild {
    let spec = &world.guilds[g];
    let guild_id = GuildId::new(id(World::guild_id(g)));
    let mut guild = Guild::default();
    guild.id = guild_id;
    guild.owner_id = UserId::new(id(World::owner_id(spec)));

    let everyone = spec.everyone.map(|bits| (World::guild_id(g), bits));
    let roles = spec
        .roles
        .iter()
        .enumerate()
        .map(|(r, &bits)| (World::role_id(g, r), bits));
    for (role_id, permissions) in everyone.into_iter().chain(roles) {
        let mut role = Role::default();
        role.id = RoleId::new(id(role_id));
        role.guild_id = guild_id;
        role.permissions = bits(permissions);
        guild.roles.insert(role.id, role);
    }

    for (c, spec) in spec.channels.iter().enumerate() {
        let mut channel = GuildChannel::default();
        channel.id = ChannelId::new(id(World::channel_id(g, c)));
        channel.guild_id = guild_id;
        channel.kind = match spec.kind {
            ChannelKind::Text => ChannelType::Text,
            ChannelKind::Voice => ChannelType::Voice,
            ChannelKind::Category => ChannelType::Category,
            ChannelKind::Stage => ChannelType::Stage,
        };
        channel.permission_overwrites = spec
            .overwrites
            .iter()
            .map(|overwrite| PermissionOverwrite {
                allow: bits(overwrite.allow),
                deny: bits(overwrite.deny),
                kind: match overwrite.target {
                    Target::Everyone => {
                        PermissionOverwriteType::Role(RoleId::new(id(World::guild_id(g))))
                    }
                    Target::Role(r) => {
                        PermissionOverwriteType::Role(RoleId::new(id(World::role_id(g, r))))
                    }
                    Target::User(u) => {
                        PermissionOverwriteType::Member(UserId::new(id(World::user_id(u))))
                    }
                    Target::DeletedRole => {
                        PermissionOverwriteType::Role(RoleId::new(id(World::deleted_role_id(g))))
                    }
                },
            })
            .collect();
        guild.channels.insert(channel.id, channel);
    }

    for (u, spec) in spec.members.iter().enumerate() {
        let mut user = User::default();
        user.id = UserId::new(id(World::user_id(u)));
        let mut member = Member::default();
        member.user = user;
        member.guild_id = guild_id;
        member.roles = spec
            .roles
            .iter()
            .enumerate()
            .filter(|(_, held)| **held)
            .map(|(r, _)| RoleId::new(id(World::role_id(g, r))))
            .collect();
        guild.members.insert(member.user.id, member);
    }
    guild
}
