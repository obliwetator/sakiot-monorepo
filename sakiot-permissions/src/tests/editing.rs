//! Rule edits on random guilds: what `GuildFacts::edit_rule` refuses, and
//! that an accepted edit never reaches past its editor. Whatever anyone
//! gains or loses through it, guild-wide or in a channel, the editor held
//! there themselves, unless they own the guild.

use std::cell::RefCell;
use std::collections::BTreeSet;

use proptest::prelude::*;
use proptest::sample::Index;
use proptest::test_runner::{Config, TestRunner};

use crate::testing::{self, Target, World};
use crate::{Access, Capabilities, GuildFacts, Membership, Rule, RuleRefused, Viewer};

/// In memory and cheap; enough that every refusal shows up.
const DEFAULT_CASES: u32 = 2048;

#[derive(Clone, Debug)]
struct Edit {
    guild: Index,
    editor: Index,
    /// Guild-wide, or a channel of the guild (any kind).
    channel: Option<Index>,
    /// `@everyone`, a role, a user or a deleted role.
    target: Index,
    rule: Rule,
}

fn edit() -> impl Strategy<Value = Edit> {
    (
        any::<Index>(),
        any::<Index>(),
        prop::option::weighted(0.4, any::<Index>()),
        any::<Index>(),
        prop_oneof![
            // Removing a rule.
            1 => Just((Capabilities::empty(), Capabilities::empty())),
            // The one an editor may not make for themselves.
            1 => Just((Capabilities::empty(), Capabilities::MANAGE_PERMISSIONS)),
            4 => testing::rule_bits(),
        ],
    )
        .prop_map(|(guild, editor, channel, target, (allow, deny))| Edit {
            guild,
            editor,
            channel,
            target,
            rule: Rule { allow, deny },
        })
}

#[test]
fn rule_edits_reach_no_further_than_the_editor() {
    let mut config = Config::default();
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = DEFAULT_CASES;
    }
    let coverage = RefCell::new(BTreeSet::new());
    let result = TestRunner::new(config).run(&(testing::world(), edit()), |(world, edit)| {
        check_edit(&world, &edit, &mut coverage.borrow_mut())
    });
    if let Err(failure) = result {
        panic!("{failure}");
    }
    let missing: Vec<Reached> = EVERY_OUTCOME
        .into_iter()
        .filter(|outcome| !coverage.borrow().contains(outcome))
        .collect();
    assert!(
        missing.is_empty(),
        "the generator never reached {missing:?}"
    );
}

fn check_edit(
    world: &World,
    edit: &Edit,
    coverage: &mut BTreeSet<Reached>,
) -> Result<(), TestCaseError> {
    let g = edit.guild.index(world.guilds.len());
    let spec = &world.guilds[g];
    let editor = Viewer::discord(World::user_id(edit.editor.index(world.users)));
    let channel = edit
        .channel
        .map(|c| World::channel_id(g, c.index(spec.channels.len())));
    let targets = 2 + spec.roles.len() + world.users;
    let target = match edit.target.index(targets) {
        0 => Target::Everyone,
        1 => Target::DeletedRole,
        t if t < 2 + spec.roles.len() => Target::Role(t - 2),
        t => Target::User(t - 2 - spec.roles.len()),
    };
    let subject = World::subject(g, target);
    let facts = world.facts(Some(g));
    let Ok(before) = facts.access(editor) else {
        let mut edited = facts.clone();
        prop_assert_eq!(
            edited.edit_rule(editor, channel, subject, edit.rule),
            Err(RuleRefused::MembershipUnavailable)
        );
        return Ok(());
    };
    let mut edited = facts.clone();
    let result = edited.edit_rule(editor, channel, subject, edit.rule);
    let owner = before.membership == Membership::Owner;
    match result {
        Err(refused) => {
            prop_assert_eq!(&edited, &facts, "a refused edit changed the facts");
            match refused {
                RuleRefused::MembershipUnavailable => {
                    return Err(TestCaseError::fail("membership known, then not"));
                }
                RuleRefused::NotAnEditor => {
                    prop_assert!(!before.guild.contains(Capabilities::MANAGE_PERMISSIONS));
                    coverage.insert(Reached::NotAnEditor);
                }
                RuleRefused::AllowsAndDenies => {
                    prop_assert!(edit.rule.allow.intersects(edit.rule.deny));
                }
                RuleRefused::ManagementInChannel => {
                    prop_assert!(channel.is_some());
                    prop_assert!(edit.rule.mentions().intersects(Capabilities::MANAGEMENT));
                    coverage.insert(Reached::ManagementInChannel);
                }
                RuleRefused::NotHeld(beyond) => {
                    prop_assert!(!owner, "the owner held back");
                    prop_assert!(!beyond.is_empty());
                    prop_assert!(!before.editable(channel).intersects(beyond));
                    coverage.insert(Reached::NotHeld);
                }
                RuleRefused::OwnManagePermissions => {
                    prop_assert!(!owner, "the owner lost MANAGE_PERMISSIONS");
                    let mut applied = facts.clone();
                    applied.rules.set(channel, subject, edit.rule);
                    let after = applied
                        .access(editor)
                        .map_err(|_| TestCaseError::fail("membership lost by a rule"))?;
                    prop_assert!(!after.guild.contains(Capabilities::MANAGE_PERMISSIONS));
                    coverage.insert(Reached::OwnManagePermissions);
                }
            }
            Ok(())
        }
        Ok(()) => {
            prop_assert!(before.guild.contains(Capabilities::MANAGE_PERMISSIONS));
            prop_assert_eq!(edited.rules.get(channel, subject), edit.rule);
            let after = edited
                .access(editor)
                .map_err(|_| TestCaseError::fail("membership lost by a rule"))?;
            prop_assert!(after.guild.contains(Capabilities::MANAGE_PERMISSIONS));
            if !owner {
                coverage.insert(Reached::Accepted);
                check_reach(world, g, &facts, &edited, &before, coverage)?;
            }
            Ok(())
        }
    }
}

/// Whatever anyone, or any role, gained or lost through the edit, the
/// editor held there before it.
fn check_reach(
    world: &World,
    guild: usize,
    facts: &GuildFacts,
    edited: &GuildFacts,
    editor: &Access,
    coverage: &mut BTreeSet<Reached>,
) -> Result<(), TestCaseError> {
    let mut pairs = Vec::new();
    for user in 0..world.users {
        let viewer = Viewer::discord(World::user_id(user));
        if let (Ok(before), Ok(after)) = (facts.access(viewer), edited.access(viewer)) {
            pairs.push((format!("user {}", viewer.user_id), before, after));
        }
    }
    let roles = (0..world.guilds[guild].roles.len()).map(|r| World::role_id(guild, r));
    for role in roles.chain([World::guild_id(guild)]) {
        if let (Ok(before), Ok(after)) = (facts.role_access(role), edited.role_access(role)) {
            pairs.push((format!("role {role}"), before, after));
        }
    }
    for (who, before, after) in pairs {
        let changed = before.guild ^ after.guild;
        prop_assert!(
            editor.guild.contains(changed),
            "{} changed {:?} guild-wide, which the editor lacks",
            who,
            changed
        );
        for (channel, held) in &before.channels {
            let changed = *held ^ after.channels[channel];
            let editable = editor.channels.get(channel).copied().unwrap_or_default();
            prop_assert!(
                editable.contains(changed),
                "{} changed {:?} in channel {}, which the editor lacks there",
                who,
                changed,
                channel
            );
            if !changed.is_empty() && after.user_id != editor.user_id {
                coverage.insert(Reached::ChangedSomeoneElse);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Reached {
    NotAnEditor,
    ManagementInChannel,
    NotHeld,
    OwnManagePermissions,
    /// An edit by an editor who is not the owner.
    Accepted,
    /// It changed someone else's access, or a role's.
    ChangedSomeoneElse,
}

const EVERY_OUTCOME: [Reached; 6] = [
    Reached::NotAnEditor,
    Reached::ManagementInChannel,
    Reached::NotHeld,
    Reached::OwnManagePermissions,
    Reached::Accepted,
    Reached::ChangedSomeoneElse,
];
