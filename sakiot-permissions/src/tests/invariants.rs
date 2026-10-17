//! The evaluator's invariants on random guilds: who is a member, which
//! guild-wide capabilities they hold, which channels they get, and what a
//! role preview shows. `discord_reference` checks the Discord rules channel
//! by channel; this checks what Sakiot builds on them.
//!
//! The test fails if the generator stops reaching an interesting outcome, so
//! it cannot pass because every world is trivial. `PROPTEST_CASES`
//! overrides the number of worlds.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};

use crate::testing::{self, RosterState, World};
use crate::{
    Access, Capabilities, GuildFacts, Membership, MembershipUnavailable, Permissions, RoleNotFound,
    RolePreview, Viewer,
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
    if World::owner_id(guild) == viewer.user_id {
        return Ok(Membership::Owner);
    }
    let member = &guild.members[user];
    if viewer.dev {
        return Ok(match member.seed {
            Some(true) => Membership::Owner,
            Some(false) => Membership::Member,
            None => Membership::Outsider,
        });
    }
    match guild.roster {
        RosterState::Complete if member.listed => Ok(Membership::Member),
        RosterState::Complete => Ok(Membership::Outsider),
        RosterState::Incomplete | RosterState::Missing if guild.bot_present => {
            Err(MembershipUnavailable)
        }
        RosterState::Incomplete | RosterState::Missing => Ok(Membership::Outsider),
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
            return Ok(());
        }
    };
    let manager = bits & (ADMINISTRATOR | MANAGE_GUILD) != 0;
    let management = if manager {
        Capabilities::MANAGEMENT
    } else {
        Capabilities::empty()
    };
    prop_assert_eq!(access.guild, management);

    let recorded = recorded_channels(facts);
    let channels: BTreeSet<i64> = access.channels.keys().copied().collect();
    prop_assert_eq!(&channels, &recorded, "every voice and stage channel");
    for (&channel, &held) in &access.channels {
        prop_assert!(
            !held.intersects(Capabilities::MANAGEMENT),
            "channel {} holds a guild-wide capability",
            channel
        );
        prop_assert!(held.contains(Capabilities::STAMP), "channel {}", channel);
        let listens = held.contains(Capabilities::LISTEN_OTHERS);
        prop_assert!(
            !listens || held.contains(Capabilities::VIEW_PRESENCE),
            "channel {} listens without viewing",
            channel
        );
        prop_assert_eq!(listens, held.contains(Capabilities::CREATE_CLIPS));
        prop_assert_eq!(listens, held.contains(Capabilities::JAM));
    }
    let listening = channels_with(&access, Capabilities::LISTEN_OTHERS);
    let presence = channels_with(&access, Capabilities::VIEW_PRESENCE);
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
    let joinable = channels_with(&preview.access, Capabilities::LISTEN_OTHERS);
    let presence = channels_with(&preview.access, Capabilities::VIEW_PRESENCE);
    prop_assert!(
        joinable.is_subset(&presence),
        "joins a channel it cannot view"
    );
    let everyone = guild.and_then(|g| world.guilds[g].everyone).unwrap_or(0);
    if (everyone | role_bits) & ADMINISTRATOR != 0 {
        prop_assert_eq!(&joinable, &listening);
        prop_assert_eq!(
            &presence,
            &channels_with(manager, Capabilities::VIEW_PRESENCE)
        );
    }
    if manages && joinable.len() < listening.len() {
        coverage.reach(Reached::PreviewJoinsLess);
    }
    Ok(())
}

/// Outcomes the generator must reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Reached {
    Owner,
    Member,
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
}

const EVERY_OUTCOME: [Reached; 12] = [
    Reached::Owner,
    Reached::Member,
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
