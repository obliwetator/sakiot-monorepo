//! The Discord layer against serenity's implementation of Discord's rules,
//! an independent and widely used one, on random guilds.

use proptest::prelude::*;
use serenity::model::channel::{
    ChannelType, GuildChannel, PermissionOverwrite, PermissionOverwriteType,
};
use serenity::model::guild::{Guild, Member, Role};
use serenity::model::id::{ChannelId, GuildId, RoleId, UserId};
use serenity::model::user::User;

use crate::Permissions;
use crate::testing::{self, ChannelKind, Target, World};

/// Compare only the bits this crate defines: serenity knows newer ones, and
/// its "everything" for owners and administrators includes them.
const DEFINED: i64 = Permissions::all().bits();

fn id(value: i64) -> u64 {
    u64::try_from(value).expect("generated ids are positive")
}

fn serenity_bits(bits: i64) -> serenity::model::Permissions {
    serenity::model::Permissions::from_bits_truncate(id(bits))
}

fn defined(bits: u64) -> i64 {
    i64::try_from(bits).expect("Discord uses fewer than 63 bits") & DEFINED
}

/// The generated guild as serenity's cache would hold it.
fn serenity_guild(world: &World, g: usize) -> (Guild, Vec<GuildChannel>, Vec<Member>) {
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
    for (role_id, bits) in everyone.into_iter().chain(roles) {
        let mut role = Role::default();
        role.id = RoleId::new(id(role_id));
        role.guild_id = guild_id;
        role.permissions = serenity_bits(bits);
        guild.roles.insert(role.id, role);
    }

    let channels = spec
        .channels
        .iter()
        .enumerate()
        .filter(|(_, channel)| channel.kind.recorded())
        .map(|(c, channel)| {
            let mut serenity_channel = GuildChannel::default();
            serenity_channel.id = ChannelId::new(id(World::channel_id(g, c)));
            serenity_channel.guild_id = guild_id;
            serenity_channel.kind = if channel.kind == ChannelKind::Stage {
                ChannelType::Stage
            } else {
                ChannelType::Voice
            };
            serenity_channel.permission_overwrites = channel
                .overwrites
                .iter()
                .map(|overwrite| PermissionOverwrite {
                    allow: serenity_bits(overwrite.allow),
                    deny: serenity_bits(overwrite.deny),
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
                        Target::DeletedRole => PermissionOverwriteType::Role(RoleId::new(id(
                            World::deleted_role_id(g),
                        ))),
                    },
                })
                .collect();
            serenity_channel
        })
        .collect();

    let members = spec
        .members
        .iter()
        .enumerate()
        .map(|(u, member)| {
            let mut user = User::default();
            user.id = UserId::new(id(World::user_id(u)));
            let mut serenity_member = Member::default();
            serenity_member.user = user;
            serenity_member.guild_id = guild_id;
            serenity_member.roles = member
                .roles
                .iter()
                .enumerate()
                .filter(|(_, held)| **held)
                .map(|(r, _)| RoleId::new(id(World::role_id(g, r))))
                .collect();
            serenity_member
        })
        .collect();
    (guild, channels, members)
}

proptest! {
    #[test]
    fn member_permissions_match_serenity(world in testing::world()) {
        for g in 0..world.guilds.len() {
            let facts = world.facts(Some(g));
            let (guild, channels, members) = serenity_guild(&world, g);
            for member in &members {
                let user_id = i64::try_from(member.user.id.get()).expect("generated id");
                let ours = facts.member_permissions(user_id);

                // serenity reports everything for an administrator; the
                // cache-backed checks keep the roles' own bits.
                let theirs = defined(guild.member_permissions(member).bits());
                let expected = if ours.base.contains(Permissions::ADMINISTRATOR) {
                    DEFINED
                } else {
                    ours.base.bits() & DEFINED
                };
                prop_assert_eq!(theirs, expected, "base of user {}", user_id);

                for channel in &channels {
                    let channel_id = i64::try_from(channel.id.get()).expect("generated id");
                    let theirs = defined(guild.user_permissions_in(channel, member).bits());
                    let ours = ours.channels.get(&channel_id).map(|bits| bits.bits() & DEFINED);
                    prop_assert_eq!(
                        Some(theirs),
                        ours,
                        "user {} in channel {}",
                        user_id,
                        channel_id
                    );
                }
            }
        }
    }
}
