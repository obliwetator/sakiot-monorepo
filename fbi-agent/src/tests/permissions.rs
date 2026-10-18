//! `/jam` asks the evaluator the web server uses, over facts built from the
//! gateway cache. It used to ask serenity which voice channels a cached
//! member may view and join; on random guilds, the answer is the same.

use std::collections::BTreeSet;

use proptest::prelude::*;
use sakiot_permissions::testing::{self, STRANGER_ID, World, gateway};
use serenity::model::Permissions;
use serenity::model::channel::ChannelType;
use serenity::model::guild::Guild;
use serenity::model::id::UserId;

use crate::cast::ToI64;
use crate::permissions::listening_in;

/// Serenity's answer: the voice and stage channels where a cached member
/// has VIEW_CHANNEL and CONNECT. Someone the cache does not list gets none,
/// unless they own the guild, who gets every one.
fn serenity_listening_channels(guild: &Guild, user_id: UserId) -> BTreeSet<i64> {
    let recorded = guild
        .channels
        .values()
        .filter(|channel| matches!(channel.kind, ChannelType::Voice | ChannelType::Stage));
    match guild.members.get(&user_id) {
        Some(member) => recorded
            .filter(|channel| {
                guild
                    .user_permissions_in(channel, member)
                    .contains(Permissions::VIEW_CHANNEL | Permissions::CONNECT)
            })
            .map(|channel| channel.id.to_i64())
            .collect(),
        None if user_id == guild.owner_id => recorded.map(|channel| channel.id.to_i64()).collect(),
        None => BTreeSet::new(),
    }
}

proptest! {
    #[test]
    fn listening_channels_match_serenity(world in testing::world()) {
        for (g, spec) in world.guilds.iter().enumerate() {
            let mut guild = gateway::guild(&world, g);
            // The gateway has not delivered every member: those the
            // generator leaves unlisted are missing from the cache.
            guild.members.retain(|user_id, _| {
                (0..world.users).any(|u| {
                    gateway::id(World::user_id(u)) == user_id.get() && spec.members[u].listed
                })
            });
            let users = (0..world.users).map(World::user_id).chain([STRANGER_ID]);
            for user_id in users.map(|id| UserId::new(gateway::id(id))) {
                let listening = listening_in(&guild, user_id);
                // Only a member's own voice is theirs to hear.
                let member = guild.members.contains_key(&user_id) || user_id == guild.owner_id;
                prop_assert_eq!(
                    listening.own,
                    member.then(|| user_id.to_i64()),
                    "user {} in guild {}",
                    user_id,
                    g
                );
                let ours: BTreeSet<i64> = listening.channels.into_iter().collect();
                prop_assert_eq!(
                    ours,
                    serenity_listening_channels(&guild, user_id),
                    "user {} in guild {}",
                    user_id,
                    g
                );
            }
        }
    }
}
