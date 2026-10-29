//! The evaluator's invariants on random guilds: who is a member, which
//! guild-wide capabilities they hold, which channels they get, what Sakiot's
//! rules change, and what a role preview shows. `discord_reference` checks
//! the Discord rules channel by channel; this checks what Sakiot builds on
//! them, against a model of the rules written per capability.
//!
//! The test fails if the generator stops reaching an interesting outcome, so
//! it cannot pass because every world is trivial. `PROPTEST_CASES`
//! overrides the number of worlds.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};

use crate::testing::{self, RosterState, STRANGER_ID, Target, TargetedRule, World};
use crate::{
    Access, Capabilities, Decision, Explanation, GuildFacts, Membership, MembershipUnavailable,
    Permissions, RoleNotFound, RolePreview, Source, Viewer,
};

const ADMINISTRATOR: i64 = Permissions::ADMINISTRATOR.bits();
const MANAGE_GUILD: i64 = Permissions::MANAGE_GUILD.bits();

/// In memory, so many more worlds than the web server's harness.
const DEFAULT_CASES: u32 = 512;

#[test]
fn the_evaluator_keeps_its_invariants() {
    let mut config = Config::default();
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = DEFAULT_CASES;
    }
    let coverage = RefCell::new(Coverage::default());
    let result = TestRunner::new(config).run(&testing::world(), |world| {
        for guild in observed_guilds(&world) {
            check_guild(&world, guild, &mut coverage.borrow_mut()).map_err(within(format!(
                "guild {}",
                World::guild_id_or_unknown(guild)
            )))?;
        }
        Ok(())
    });
    if let Err(failure) = result {
        panic!("{failure}");
    }
    if let Err(failure) = coverage.into_inner().require_every_outcome() {
        panic!("{failure}");
    }
}

/// Each generated guild (`Some(index)`), and one nothing is known about.
fn observed_guilds(world: &World) -> Vec<Option<usize>> {
    (0..world.guilds.len()).map(Some).chain([None]).collect()
}

/// Prefixes a failure with where it happened.
fn within(place: String) -> impl FnOnce(TestCaseError) -> TestCaseError {
    move |error| TestCaseError::fail(format!("{place}: {error}"))
}

/// `@everyone` (whose id is the guild's), every other role of the guild, a
/// deleted role, and a role of another guild.
fn previewed_roles(world: &World, guild: Option<usize>) -> Vec<i64> {
    let Some(g) = guild else {
        return vec![World::guild_id_or_unknown(None)];
    };
    let foreign = (g + 1 < world.guilds.len()).then(|| World::role_id(g + 1, 0));
    [World::guild_id(g), World::deleted_role_id(g)]
        .into_iter()
        .chain((0..world.guilds[g].roles.len()).map(|r| World::role_id(g, r)))
        .chain(foreign)
        .collect()
}

/// Membership as `GuildFacts::membership` documents it.
fn membership(
    world: &World,
    guild: Option<usize>,
    user: usize,
    viewer: Viewer,
) -> Result<Membership, MembershipUnavailable> {
    let Some(guild) = guild.map(|g| &world.guilds[g]) else {
        return Ok(Membership::Outsider);
    };
    let owner = World::owner_id(guild) == viewer.user_id;
    let member = &guild.members[user];
    if viewer.dev {
        return Ok(match member.seed {
            _ if owner => Membership::Owner,
            Some(true) => Membership::Owner,
            Some(false) => Membership::Member,
            None => Membership::Outsider,
        });
    }
    let not_member = if member.recorded {
        Membership::Former
    } else {
        Membership::Outsider
    };
    if !guild.bot_present {
        return Ok(not_member);
    }
    if owner {
        return Ok(Membership::Owner);
    }
    match guild.roster {
        RosterState::Complete if member.listed => Ok(Membership::Member),
        RosterState::Complete => Ok(not_member),
        RosterState::Incomplete | RosterState::Missing => Err(MembershipUnavailable),
    }
}

/// A member's guild-level bits: `@everyone` and the roles they hold here.
fn member_bits(world: &World, guild: usize, user: usize) -> i64 {
    let guild = &world.guilds[guild];
    guild
        .roles
        .iter()
        .zip(&guild.members[user].roles)
        .filter(|(_, held)| **held)
        .fold(guild.everyone.unwrap_or(0), |bits, (role, _)| bits | role)
}

/// The bits of `role` if it is a stored role of the guild (`@everyone`
/// included); `None` for a deleted or foreign role.
fn stored_role(world: &World, guild: Option<usize>, role: i64) -> Option<i64> {
    let g = guild?;
    let spec = &world.guilds[g];
    if role == World::guild_id(g) {
        return spec.everyone;
    }
    (0..spec.roles.len())
        .find(|&r| World::role_id(g, r) == role)
        .map(|r| spec.roles[r])
}

/// The index of `role` among the guild's roles; `None` for `@everyone`, a
/// deleted role or another guild's.
fn role_index(world: &World, guild: usize, role: i64) -> Option<usize> {
    (0..world.guilds[guild].roles.len()).find(|&r| World::role_id(guild, r) == role)
}

/// Who the rules are evaluated for: the roles held (indices) and the user,
/// `None` for a role preview.
struct Holder {
    roles: Vec<usize>,
    user: Option<usize>,
}

impl Holder {
    fn member(world: &World, guild: usize, user: usize) -> Self {
        let held = &world.guilds[guild].members[user].roles;
        Self {
            roles: (0..held.len()).filter(|&r| held[r]).collect(),
            user: Some(user),
        }
    }

    fn role(world: &World, guild: usize, role: i64) -> Self {
        Self {
            roles: role_index(world, guild, role).into_iter().collect(),
            user: None,
        }
    }
}

/// Whether `holder` holds `capability` once the rules have had their say
/// over what Discord gives (`discord`), modelled one capability at a time:
/// management capabilities through the guild-wide layers, content ones
/// through those and then `channel`'s (a channel index). Each layer that
/// mentions the capability decides it anew: `@everyone`, then the
/// holder's roles (any allow beats any deny), then the user.
fn ruled(
    world: &World,
    guild: usize,
    channel: Option<usize>,
    holder: &Holder,
    capability: Capabilities,
    discord: bool,
) -> bool {
    let spec = &world.guilds[guild];
    let management = Capabilities::MANAGEMENT.contains(capability);
    let mut scopes = vec![spec.rules.as_slice()];
    if let Some(c) = channel.filter(|_| !management) {
        scopes.push(spec.channels[c].rules.as_slice());
    }
    let says = |rules: &[TargetedRule], applies: &dyn Fn(Target) -> bool| {
        let applying = || rules.iter().filter(|rule| applies(rule.target));
        if applying().any(|rule| rule.allow.contains(capability)) {
            Some(true)
        } else if applying().any(|rule| rule.deny.contains(capability)) {
            Some(false)
        } else {
            None
        }
    };
    scopes.into_iter().fold(discord, |held, rules| {
        let layers: [&dyn Fn(Target) -> bool; 3] = [
            &|target| target == Target::Everyone,
            &|target| matches!(target, Target::Role(r) if holder.roles.contains(&r)),
            &|target| matches!(target, Target::User(u) if Some(u) == holder.user),
        ];
        layers
            .into_iter()
            .fold(held, |held, applies| says(rules, applies).unwrap_or(held))
    })
}

/// The channel index of a voice or stage channel's id.
fn channel_index(guild: usize, channel: i64) -> Option<usize> {
    usize::try_from(channel - World::channel_id(guild, 0)).ok()
}

/// `access` as the model has it, from Discord's `discord` answer
/// (`facts.discord_permissions`, checked against serenity elsewhere): each
/// capability `ruled`, then nothing in a channel the holder does not see.
fn check_rules(
    world: &World,
    guild: usize,
    holder: &Holder,
    owner: bool,
    discord: &crate::DiscordPermissions,
    access: &Access,
    coverage: &mut Coverage,
) -> Result<(), TestCaseError> {
    let granted = Capabilities::from_guild_permissions(discord.base);
    for capability in Capabilities::MANAGEMENT.iter() {
        let held = ruled(
            world,
            guild,
            None,
            holder,
            capability,
            granted.contains(capability),
        );
        prop_assert_eq!(
            access.guild.contains(capability),
            held || owner,
            "{:?} guild-wide",
            capability
        );
        if owner && !held {
            coverage.reach(Reached::OwnerKeepsManagement);
        }
    }
    let administrator = discord.base.contains(Permissions::ADMINISTRATOR);
    for (&channel, &permissions) in &discord.channels {
        let c = channel_index(guild, channel)
            .ok_or_else(|| TestCaseError::fail(format!("channel {channel} not generated")))?;
        let granted = Capabilities::from_channel_permissions(permissions);
        let expected = Capabilities::CONTENT
            .iter()
            .filter(|&capability| {
                ruled(
                    world,
                    guild,
                    Some(c),
                    holder,
                    capability,
                    granted.contains(capability),
                )
            })
            .fold(Capabilities::empty(), |held, capability| held | capability);
        let seen = if expected.contains(Capabilities::VIEW_PRESENCE) {
            expected
        } else {
            Capabilities::empty()
        };
        if seen != expected {
            coverage.reach(Reached::UnseenTakesTheRest);
        }
        let expected = seen;
        let held = access.channels.get(&channel).copied().unwrap_or_default();
        prop_assert_eq!(held, expected, "channel {}", channel);
        if !(expected - granted).is_empty() {
            coverage.reach(Reached::RuleGrants);
        }
        if !(granted - expected).is_empty() {
            coverage.reach(if administrator {
                Reached::RuleDeniesAdministrator
            } else {
                Reached::RuleDenies
            });
        }
    }
    Ok(())
}

/// The explanation agrees with the access it explains, capability by
/// capability, and with what Discord alone gives.
fn check_explanation(
    explanation: &Explanation,
    access: &Access,
    discord: &crate::DiscordPermissions,
    coverage: &mut Coverage,
) -> Result<(), TestCaseError> {
    prop_assert_eq!(explanation.membership, access.membership);
    let agrees = |decisions: &[Decision], held: Capabilities, granted: Capabilities, scope| {
        let capabilities: Vec<Capabilities> = decisions
            .iter()
            .map(|decision| decision.capability)
            .collect();
        let expected: Vec<Capabilities> = Capabilities::iter(&scope).collect();
        prop_assert_eq!(capabilities, expected, "one decision per capability");
        for decision in decisions {
            prop_assert_eq!(
                decision.held,
                held.contains(decision.capability),
                "{:?}",
                decision
            );
            prop_assert_eq!(
                decision.discord,
                granted.contains(decision.capability),
                "{:?}",
                decision
            );
            match decision.by {
                Source::Discord => {
                    prop_assert_eq!(decision.held, decision.discord, "{:?}", decision);
                }
                Source::Unseen | Source::Manager => prop_assert!(!decision.held, "{:?}", decision),
                Source::Rule { .. } | Source::Owner => {}
            }
        }
        Ok(())
    };
    agrees(
        &explanation.guild,
        access.guild,
        Capabilities::from_guild_permissions(discord.base),
        Capabilities::MANAGEMENT,
    )?;
    prop_assert_eq!(
        explanation.channels.keys().collect::<Vec<_>>(),
        access.channels.keys().collect::<Vec<_>>()
    );
    for (channel, decisions) in &explanation.channels {
        agrees(
            decisions,
            access.channels[channel],
            Capabilities::from_channel_permissions(discord.channels[channel]),
            Capabilities::CONTENT,
        )?;
        if decisions.iter().any(|decision| {
            matches!(
                decision.by,
                Source::Rule {
                    channel: Some(_),
                    ..
                }
            )
        }) {
            coverage.reach(Reached::ExplainedByChannelRule);
        }
    }
    Ok(())
}

fn recorded_channels(facts: &GuildFacts) -> BTreeSet<i64> {
    facts.channels.keys().copied().collect()
}

fn channels_with(access: &Access, capability: Capabilities) -> BTreeSet<i64> {
    access.channels_with(capability).collect()
}

fn check_guild(
    world: &World,
    guild: Option<usize>,
    coverage: &mut Coverage,
) -> Result<(), TestCaseError> {
    let facts = world.facts(guild);
    for user in 0..world.users {
        for dev in [false, true] {
            let viewer = Viewer {
                user_id: World::user_id(user),
                dev,
            };
            check_viewer(world, guild, &facts, user, viewer, coverage)
                .map_err(within(format!("{viewer:?}")))?;
        }
    }
    Ok(())
}

fn check_viewer(
    world: &World,
    guild: Option<usize>,
    facts: &GuildFacts,
    user: usize,
    viewer: Viewer,
    coverage: &mut Coverage,
) -> Result<(), TestCaseError> {
    let expected = membership(world, guild, user, viewer);
    prop_assert_eq!(facts.membership(viewer), expected);
    coverage.membership(world, guild, user, viewer, expected);
    let access = match facts.access(viewer) {
        Ok(access) => access,
        Err(MembershipUnavailable) => {
            prop_assert_eq!(expected, Err(MembershipUnavailable));
            return Ok(());
        }
    };
    prop_assert_eq!(Ok(access.membership), expected);

    let bits = match (access.membership, guild) {
        (Membership::Owner, _) => Permissions::all().bits(),
        (Membership::Member, Some(g)) => member_bits(world, g, user),
        _ => {
            prop_assert_eq!(access.guild, Capabilities::empty(), "an outsider manages");
            prop_assert!(access.channels.is_empty(), "an outsider gets channels");
            prop_assert_eq!(
                facts
                    .explain(viewer)
                    .map(|explained| explained.guild.is_empty() && explained.channels.is_empty()),
                Ok(true),
                "rules reach an outsider"
            );
            // A former member hears their own voice and nothing else; an
            // outsider nothing at all.
            let channels: Vec<i64> = recorded_channels(facts).into_iter().collect();
            let former = access.membership == Membership::Former;
            prop_assert_eq!(access.allows_item(&[viewer.user_id], []), former);
            prop_assert!(!access.allows_item(&[viewer.user_id, STRANGER_ID], channels));
            prop_assert!(access.private.is_empty(), "nobody else's to keep from them");
            return Ok(());
        }
    };
    // A member hears their own voice from anywhere.
    prop_assert!(access.allows_item(&[viewer.user_id], []));
    check_consent(world, guild, user, &access, coverage)?;
    let manager = bits & (ADMINISTRATOR | MANAGE_GUILD) != 0;
    let management = if manager {
        Capabilities::MANAGEMENT
    } else {
        Capabilities::empty()
    };
    let discord = facts
        .discord_permissions(viewer)
        .map_err(|_| TestCaseError::fail("membership known for access, not for Discord"))?;
    prop_assert_eq!(
        Capabilities::from_guild_permissions(discord.base),
        management
    );
    let g = guild.ok_or_else(|| TestCaseError::fail("a member of the unknown guild"))?;
    let owner = access.membership == Membership::Owner;
    check_rules(
        world,
        g,
        &Holder::member(world, g, user),
        owner,
        &discord,
        &access,
        coverage,
    )?;
    let explanation = facts
        .explain(viewer)
        .map_err(|_| TestCaseError::fail("membership known for access, not to explain"))?;
    check_explanation(&explanation, &access, &discord, coverage)?;

    // Discord's defaults, before any rule.
    let recorded = recorded_channels(facts);
    let channels: BTreeSet<i64> = access.channels.keys().copied().collect();
    prop_assert_eq!(&channels, &recorded, "every voice and stage channel");
    let defaults: BTreeMap<i64, Capabilities> = discord
        .channels
        .iter()
        .map(|(&channel, &permissions)| {
            (channel, Capabilities::from_channel_permissions(permissions))
        })
        .collect();
    for (&channel, &held) in &access.channels {
        prop_assert!(
            !held.intersects(Capabilities::MANAGEMENT),
            "channel {} holds a guild-wide capability",
            channel
        );
    }
    for (&channel, &held) in &defaults {
        prop_assert_eq!(held, held.seen(), "channel {} holds unseen", channel);
        prop_assert_eq!(
            held.contains(Capabilities::STAMP),
            held.contains(Capabilities::VIEW_PRESENCE),
            "channel {}",
            channel
        );
        let listens = held.contains(Capabilities::LISTEN_OTHERS);
        prop_assert!(
            !listens || held.contains(Capabilities::VIEW_PRESENCE),
            "channel {} listens without viewing",
            channel
        );
        prop_assert_eq!(listens, held.contains(Capabilities::CREATE_CLIPS));
        prop_assert_eq!(listens, held.contains(Capabilities::JAM));
    }
    let by_default = |capability: Capabilities| -> BTreeSet<i64> {
        defaults
            .iter()
            .filter(|(_, held)| held.contains(capability))
            .map(|(&channel, _)| channel)
            .collect()
    };
    let listening = by_default(Capabilities::LISTEN_OTHERS);
    let presence = by_default(Capabilities::VIEW_PRESENCE);
    if bits & ADMINISTRATOR != 0 {
        prop_assert_eq!(&listening, &recorded, "ADMINISTRATOR bypasses overwrites");
        prop_assert_eq!(&presence, &recorded, "ADMINISTRATOR bypasses overwrites");
    }
    if access.membership == Membership::Member && bits & ADMINISTRATOR == 0 {
        coverage.member_channels(&listening, &presence, &recorded);
    }

    for role in previewed_roles(world, guild) {
        check_preview(world, guild, facts, &access, role, manager, coverage)
            .map_err(within(format!("previewing role {role}")))?;
    }
    Ok(())
}

/// Listening consent, for a member: nobody hears the voice of someone who
/// keeps their recordings private but themselves, the owner included; and
/// whoever holds `DELETE_RECORDINGS` sees what consent alone keeps from them
/// as a redacted entry.
fn check_consent(
    world: &World,
    guild: Option<usize>,
    user: usize,
    access: &Access,
    coverage: &mut Coverage,
) -> Result<(), TestCaseError> {
    let own = World::user_id(user);
    let listening = channels_with(access, Capabilities::LISTEN_OTHERS);
    let deletes = access.guild.contains(Capabilities::DELETE_RECORDINGS);
    prop_assert!(
        !access.private.contains(&own),
        "their own voice kept from them"
    );
    for other in (0..world.users).filter(|&other| other != user) {
        let speaker = World::user_id(other);
        let private = guild.is_some_and(|g| world.guilds[g].members[other].private);
        let heard = !private && !listening.is_empty();
        let place = format!("speaker {speaker}, private: {private}");
        prop_assert_eq!(access.private.contains(&speaker), private, "{}", &place);
        let everywhere = || listening.iter().copied();
        prop_assert_eq!(
            access.allows_item(&[speaker], everywhere()),
            heard,
            "{}",
            &place
        );
        prop_assert_eq!(
            access.allows_item(&[own, speaker], everywhere()),
            heard,
            "{} with their own voice",
            &place
        );
        prop_assert_eq!(
            access.redacts_item(&[speaker], everywhere()),
            private && deletes && !listening.is_empty(),
            "{}",
            &place
        );
        if private && !listening.is_empty() {
            if access.membership == Membership::Owner {
                coverage.reach(Reached::KeptFromTheOwner);
            }
            if deletes {
                coverage.reach(Reached::Redacted);
            }
        }
    }
    // Unknown speakers cannot be asked: the channels alone decide.
    prop_assert_eq!(
        access.allows_item(&[], listening.iter().copied()),
        !listening.is_empty()
    );
    Ok(())
}

fn check_preview(
    world: &World,
    guild: Option<usize>,
    facts: &GuildFacts,
    manager: &Access,
    role: i64,
    manages: bool,
    coverage: &mut Coverage,
) -> Result<(), TestCaseError> {
    let Some(role_bits) = stored_role(world, guild, role) else {
        prop_assert_eq!(facts.role_access(role), Err(RoleNotFound));
        if manages {
            coverage.reach(Reached::PreviewOfMissingRole);
        }
        return Ok(());
    };
    let role_access = facts
        .role_access(role)
        .map_err(|RoleNotFound| TestCaseError::fail(format!("stored role {role} not found")))?;
    let preview = RolePreview::new(manager, &role_access);
    let listening = channels_with(manager, Capabilities::LISTEN_OTHERS);
    prop_assert_eq!(&preview.listed, &listening, "the manager's own channels");
    let clipped: BTreeMap<i64, bool> = preview
        .access
        .channels
        .iter()
        .map(|(&channel, &held)| {
            let own = manager.channels.get(&channel).copied().unwrap_or_default();
            (channel, own.contains(held))
        })
        .collect();
    prop_assert!(
        clipped.values().all(|&within| within),
        "a preview shows more than the manager's own view: {:?}",
        clipped
    );
    // The role's own access, before clipping: Discord's answer for a member
    // holding only that role, then the rules for `@everyone` and the role.
    let g = guild.ok_or_else(|| TestCaseError::fail("a stored role of the unknown guild"))?;
    let discord = facts.role_permissions(role);
    check_rules(
        world,
        g,
        &Holder::role(world, g, role),
        false,
        &discord,
        &role_access,
        coverage,
    )?;
    let explanation = facts
        .explain_role(role)
        .map_err(|RoleNotFound| TestCaseError::fail("a stored role not found to explain"))?;
    check_explanation(&explanation, &role_access, &discord, coverage)?;
    // Clipped like the preview, the explanation still agrees with it.
    check_explanation(
        &explanation.clipped_to(manager),
        &preview.access,
        &discord,
        coverage,
    )?;
    let joinable = channels_with(&preview.access, Capabilities::LISTEN_OTHERS);
    let defaults = |capability: Capabilities| -> BTreeSet<i64> {
        discord
            .channels
            .iter()
            .filter(|(_, permissions)| {
                Capabilities::from_channel_permissions(**permissions).contains(capability)
            })
            .map(|(&channel, _)| channel)
            .collect()
    };
    let everyone = guild.and_then(|g| world.guilds[g].everyone).unwrap_or(0);
    if (everyone | role_bits) & ADMINISTRATOR != 0 {
        prop_assert_eq!(
            defaults(Capabilities::LISTEN_OTHERS),
            recorded_channels(facts)
        );
        prop_assert_eq!(
            defaults(Capabilities::VIEW_PRESENCE),
            recorded_channels(facts)
        );
    }
    if manages && joinable.len() < listening.len() {
        coverage.reach(Reached::PreviewJoinsLess);
    }
    // A role hears nobody who keeps their recordings private, and redacts
    // nothing: a preview shows what the role could hear.
    let private: BTreeSet<i64> = guild
        .map(|g| {
            let members = &world.guilds[g].members;
            (0..world.users)
                .filter(|&u| members[u].private)
                .map(World::user_id)
                .collect()
        })
        .unwrap_or_default();
    prop_assert_eq!(&preview.access.private, &private);
    for &speaker in &private {
        prop_assert!(
            !preview
                .access
                .allows_item(&[speaker], joinable.iter().copied())
        );
        prop_assert!(
            !preview
                .access
                .redacts_item(&[speaker], joinable.iter().copied())
        );
    }
    Ok(())
}

/// Outcomes the generator must reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Reached {
    Owner,
    Member,
    Former,
    /// An owner who is a former member: the bot has left their guild.
    FormerOwner,
    Outsider,
    MembershipUnavailable,
    DevOwnerBySeed,
    DevMember,
    /// A member without ADMINISTRATOR who joins none of the guild's voice
    /// channels, some of them, or views one without joining it.
    MemberJoinsNone,
    MemberJoinsSome,
    MemberViewsWithoutJoining,
    ManagerWhoIsNotOwner,
    /// A manager's preview of a role that joins fewer channels than they do.
    PreviewJoinsLess,
    /// A manager's preview of a deleted or foreign role.
    PreviewOfMissingRole,
    /// The owner may listen where someone keeps their recordings private.
    KeptFromTheOwner,
    /// A holder of `DELETE_RECORDINGS` sees a private recording redacted.
    Redacted,
    /// A rule grants a content capability Discord does not.
    RuleGrants,
    /// A rule takes one Discord grants, from someone without
    /// ADMINISTRATOR and from someone with it.
    RuleDenies,
    RuleDeniesAdministrator,
    /// A rule denies the owner a management capability, which they keep.
    OwnerKeepsManagement,
    /// A channel's own rule decides a capability.
    ExplainedByChannelRule,
    /// A rule takes seeing a channel, and everything else there with it.
    UnseenTakesTheRest,
}

const EVERY_OUTCOME: [Reached; 22] = [
    Reached::Owner,
    Reached::Member,
    Reached::Former,
    Reached::FormerOwner,
    Reached::Outsider,
    Reached::MembershipUnavailable,
    Reached::DevOwnerBySeed,
    Reached::DevMember,
    Reached::MemberJoinsNone,
    Reached::MemberJoinsSome,
    Reached::MemberViewsWithoutJoining,
    Reached::ManagerWhoIsNotOwner,
    Reached::PreviewJoinsLess,
    Reached::PreviewOfMissingRole,
    Reached::KeptFromTheOwner,
    Reached::Redacted,
    Reached::RuleGrants,
    Reached::RuleDenies,
    Reached::RuleDeniesAdministrator,
    Reached::OwnerKeepsManagement,
    Reached::ExplainedByChannelRule,
    Reached::UnseenTakesTheRest,
];

#[derive(Debug, Default)]
struct Coverage(BTreeSet<Reached>);

impl Coverage {
    fn reach(&mut self, outcome: Reached) {
        self.0.insert(outcome);
    }

    fn membership(
        &mut self,
        world: &World,
        guild: Option<usize>,
        user: usize,
        viewer: Viewer,
        membership: Result<Membership, MembershipUnavailable>,
    ) {
        let seeded = guild.and_then(|g| world.guilds[g].members[user].seed);
        let reached = match (membership, viewer.dev, seeded) {
            (Ok(Membership::Owner), true, Some(true)) => Reached::DevOwnerBySeed,
            (Ok(Membership::Member), true, _) => Reached::DevMember,
            (_, true, _) => return,
            (Ok(Membership::Owner), false, _) => Reached::Owner,
            (Ok(Membership::Member), false, _) => Reached::Member,
            (Ok(Membership::Former), false, _) => {
                let owner =
                    guild.is_some_and(|g| World::owner_id(&world.guilds[g]) == viewer.user_id);
                if owner {
                    Reached::FormerOwner
                } else {
                    Reached::Former
                }
            }
            (Ok(Membership::Outsider), false, _) => Reached::Outsider,
            (Err(MembershipUnavailable), false, _) => Reached::MembershipUnavailable,
        };
        self.reach(reached);
        if membership == Ok(Membership::Member)
            && guild
                .is_some_and(|g| member_bits(world, g, user) & (ADMINISTRATOR | MANAGE_GUILD) != 0)
        {
            self.reach(Reached::ManagerWhoIsNotOwner);
        }
    }

    fn member_channels(
        &mut self,
        listening: &BTreeSet<i64>,
        presence: &BTreeSet<i64>,
        recorded: &BTreeSet<i64>,
    ) {
        if recorded.is_empty() {
            return;
        }
        if listening.is_empty() {
            self.reach(Reached::MemberJoinsNone);
        } else if listening.len() < recorded.len() {
            self.reach(Reached::MemberJoinsSome);
        }
        if presence.len() > listening.len() {
            self.reach(Reached::MemberViewsWithoutJoining);
        }
    }

    fn require_every_outcome(&self) -> Result<(), String> {
        let missing: Vec<Reached> = EVERY_OUTCOME
            .into_iter()
            .filter(|outcome| !self.0.contains(outcome))
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!("the generator never reached {missing:?}"))
        }
    }
}
