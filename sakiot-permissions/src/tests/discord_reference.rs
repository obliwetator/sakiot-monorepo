//! The Discord layer against serenity's implementation of Discord's rules,
//! an independent and widely used one, on random guilds.

use proptest::prelude::*;
use serenity::model::channel::ChannelType;

use crate::Permissions;
use crate::testing::{self, gateway};

/// Compare only the bits this crate defines: serenity knows newer ones, and
/// its "everything" for owners and administrators includes them.
const DEFINED: i64 = Permissions::all().bits();

fn defined(bits: u64) -> i64 {
    i64::try_from(bits).expect("Discord uses fewer than 63 bits") & DEFINED
}

proptest! {
    #[test]
    fn member_permissions_match_serenity(world in testing::world()) {
        for g in 0..world.guilds.len() {
            let facts = world.facts(Some(g));
            let guild = gateway::guild(&world, g);
            for member in guild.members.values() {
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

                let recorded = guild
                    .channels
                    .values()
                    .filter(|channel| matches!(channel.kind, ChannelType::Voice | ChannelType::Stage));
                for channel in recorded {
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
