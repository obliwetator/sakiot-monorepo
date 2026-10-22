//! The bot's commands ask the evaluator the web server uses, over facts
//! built from the gateway cache and the rules from the database. `/jam` used
//! to ask serenity which voice channels a cached member may view and join; on
//! random guilds, the answer is the same.

use std::collections::BTreeSet;

use proptest::prelude::*;
use sakiot_permissions::testing::{self, STRANGER_ID, World, gateway};
use serenity::model::Permissions;
use serenity::model::channel::ChannelType;
use serenity::model::guild::Guild;
use serenity::model::id::UserId;

use crate::cast::ToI64;
use proptest::test_runner::TestCaseError;
use sakiot_permissions::{Capabilities, Rule, Rules, Subject, Viewer};

use crate::permissions::{Listening, access_in};

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
                // Discord's rules alone: serenity knows nothing of Sakiot's.
                let listening = Listening::of(access_in(&guild, user_id, Rules::default()).as_ref());
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

proptest! {
    /// With Sakiot's rules, a member the cache lists holds what the web
    /// server's evaluation of the same guild gives them: the bot's facts
    /// carry the roles the rules are for.
    #[test]
    fn cached_members_hold_what_the_web_server_gives_them(world in testing::world()) {
        for (g, spec) in world.guilds.iter().enumerate() {
            if !spec.bot_present || spec.roster != testing::RosterState::Complete {
                continue;
            }
            let guild = gateway::guild(&world, g);
            let facts = world.facts(Some(g));
            for u in 0..world.users {
                let user_id = World::user_id(u);
                let owner = World::owner_id(spec) == user_id;
                if !spec.members[u].listed && !owner {
                    continue;
                }
                let expected = facts.access(Viewer::discord(user_id));
                let ours = access_in(&guild, UserId::new(gateway::id(user_id)), facts.rules.clone());
                let (Ok(expected), Some(ours)) = (expected, ours) else {
                    return Err(TestCaseError::fail(format!("user {user_id} in guild {g} unknown")));
                };
                prop_assert_eq!(ours.membership, expected.membership);
                prop_assert_eq!(ours.guild, expected.guild, "user {} in guild {}", user_id, g);
                prop_assert_eq!(ours.channels, expected.channels, "user {} in guild {}", user_id, g);
            }
        }
    }
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn permission_rules_load_as_stored(
    pool: sqlx::PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let guild_id = 1;
    sqlx::query!(
        "INSERT INTO permission_rules
            (guild_id, channel_id, subject_kind, subject_id, allow, deny, updated_by)
         VALUES ($1, NULL, 'everyone', NULL, $2, 0, 9),
                ($1, 100, 'role', 1001, 0, $3, 9),
                ($1, 100, 'user', 7, $3, 0, 9),
                (2, NULL, 'everyone', NULL, $3, 0, 9)",
        guild_id,
        Capabilities::DELETE_RECORDINGS.stored(),
        // A capability a newer build named, besides one this one knows.
        Capabilities::JAM.stored() | (1 << 30),
    )
    .execute(&pool)
    .await?;

    let rules = crate::database::permission_rules::load(&pool, guild_id).await?;

    let mut expected = Rules::default();
    expected.set(
        None,
        Subject::Everyone,
        Rule {
            allow: Capabilities::DELETE_RECORDINGS,
            deny: Capabilities::empty(),
        },
    );
    expected.set(
        Some(100),
        Subject::Role(1001),
        Rule {
            allow: Capabilities::empty(),
            deny: Capabilities::JAM,
        },
    );
    expected.set(
        Some(100),
        Subject::User(7),
        Rule {
            allow: Capabilities::JAM,
            deny: Capabilities::empty(),
        },
    );
    assert_eq!(rules, expected);
    Ok(())
}
