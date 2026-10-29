use std::collections::HashMap;

use crate::{
    Capabilities, ChannelFacts, GuildFacts, MemberFacts, Membership, MembershipUnavailable,
    Overwrite, Permissions, RoleNotFound, RolePreview, Roster, Rule, RuleRefused, Source, Subject,
    Viewer,
};

const GUILD: i64 = 1;
const OWNER: i64 = 2;
const MEMBER: i64 = 10;
const ROLE: i64 = 100;
const CHANNEL: i64 = 500;
const VOICE: Permissions = Permissions::VIEW_CHANNEL.union(Permissions::CONNECT);

/// A complete guild with one voice channel and one listed member holding
/// `ROLE`, whose `@everyone` grants `everyone`.
fn guild(everyone: Permissions) -> GuildFacts {
    let mut facts = GuildFacts::new(GUILD);
    facts.owner_id = Some(OWNER);
    facts.roster = Roster::Complete;
    facts.bot_present = true;
    facts.everyone = Some(everyone);
    facts.roles.insert(ROLE, Permissions::empty());
    facts.channels.insert(CHANNEL, ChannelFacts::default());
    facts.members.insert(
        MEMBER,
        MemberFacts {
            listed: true,
            roles: vec![ROLE],
            recorded: false,
        },
    );
    facts
}

fn overwrite(allow: Permissions, deny: Permissions) -> Overwrite {
    Overwrite { allow, deny }
}

fn channel(facts: &mut GuildFacts) -> &mut ChannelFacts {
    facts.channels.entry(CHANNEL).or_default()
}

fn member_in_channel(facts: &GuildFacts) -> Permissions {
    facts.member_permissions(MEMBER).channels[&CHANNEL]
}

#[test]
fn everyone_deny_blocks_base_connect() {
    let mut facts = guild(VOICE);
    channel(&mut facts).everyone = overwrite(Permissions::empty(), Permissions::CONNECT);

    assert!(!member_in_channel(&facts).contains(VOICE));
}

#[test]
fn role_allow_restores_everyone_deny() {
    let mut facts = guild(Permissions::VIEW_CHANNEL);
    channel(&mut facts).everyone = overwrite(Permissions::empty(), Permissions::CONNECT);
    channel(&mut facts)
        .roles
        .insert(ROLE, overwrite(Permissions::CONNECT, Permissions::empty()));

    assert!(member_in_channel(&facts).contains(VOICE));
}

#[test]
fn an_allow_on_one_role_beats_a_deny_on_another() {
    let other = ROLE + 1;
    let mut facts = guild(Permissions::VIEW_CHANNEL);
    facts.roles.insert(other, Permissions::empty());
    if let Some(member) = facts.members.get_mut(&MEMBER) {
        member.roles.push(other);
    }
    channel(&mut facts)
        .roles
        .insert(ROLE, overwrite(Permissions::CONNECT, Permissions::empty()));
    channel(&mut facts)
        .roles
        .insert(other, overwrite(Permissions::empty(), Permissions::CONNECT));

    assert!(member_in_channel(&facts).contains(VOICE));
}

#[test]
fn member_deny_overrides_role_allow() {
    let mut facts = guild(Permissions::VIEW_CHANNEL);
    channel(&mut facts)
        .roles
        .insert(ROLE, overwrite(Permissions::CONNECT, Permissions::empty()));
    channel(&mut facts).members.insert(
        MEMBER,
        overwrite(Permissions::empty(), Permissions::CONNECT),
    );

    assert!(!member_in_channel(&facts).contains(VOICE));
}

#[test]
fn member_allow_overrides_role_deny() {
    let mut facts = guild(Permissions::VIEW_CHANNEL);
    channel(&mut facts)
        .roles
        .insert(ROLE, overwrite(Permissions::empty(), Permissions::CONNECT));
    channel(&mut facts).members.insert(
        MEMBER,
        overwrite(Permissions::CONNECT, Permissions::empty()),
    );

    assert!(member_in_channel(&facts).contains(VOICE));
}

#[test]
fn view_channel_deny_blocks_inherited_connect() {
    let mut facts = guild(VOICE);
    channel(&mut facts).everyone = overwrite(Permissions::empty(), Permissions::VIEW_CHANNEL);

    let access = facts
        .access(Viewer::discord(MEMBER))
        .expect("membership known");
    assert!(!access.allows(CHANNEL, Capabilities::VIEW_PRESENCE));
    assert!(!access.allows(CHANNEL, Capabilities::LISTEN_OTHERS));
}

#[test]
fn administrator_bypasses_overwrites() {
    let mut facts = guild(Permissions::ADMINISTRATOR);
    channel(&mut facts).everyone = overwrite(Permissions::empty(), VOICE);

    assert_eq!(member_in_channel(&facts), Permissions::all());
}

#[test]
fn unknown_permission_bits_are_retained() {
    let future = Permissions::from_bits_retain(1 << 50);
    let facts = guild(future);

    assert_eq!(facts.member_permissions(MEMBER).base, future);
}

#[test]
fn membership_follows_the_owner_the_roster_and_dev_seeding() {
    let mut facts = guild(VOICE);
    facts.seeds = HashMap::from([(MEMBER, true), (MEMBER + 1, false)]);
    let dev = |user_id| Viewer { user_id, dev: true };

    assert_eq!(
        facts.membership(Viewer::discord(OWNER)),
        Ok(Membership::Owner)
    );
    assert_eq!(
        facts.membership(Viewer::discord(MEMBER)),
        Ok(Membership::Member)
    );
    assert_eq!(
        facts.membership(Viewer::discord(MEMBER + 1)),
        Ok(Membership::Outsider)
    );
    // A dev login ignores the roster and follows its seeding.
    assert_eq!(facts.membership(dev(MEMBER)), Ok(Membership::Owner));
    assert_eq!(facts.membership(dev(MEMBER + 1)), Ok(Membership::Member));
    assert_eq!(facts.membership(dev(MEMBER + 2)), Ok(Membership::Outsider));
    assert_eq!(facts.membership(dev(OWNER)), Ok(Membership::Owner));

    facts.roster = Roster::Incomplete;
    assert_eq!(
        facts.membership(Viewer::discord(MEMBER)),
        Err(MembershipUnavailable)
    );
    assert_eq!(
        facts.membership(Viewer::discord(OWNER)),
        Ok(Membership::Owner)
    );
    facts.bot_present = false;
    assert_eq!(
        facts.membership(Viewer::discord(MEMBER)),
        Ok(Membership::Outsider)
    );
}

#[test]
fn outsiders_get_nothing_even_from_an_administrator_everyone() {
    let facts = guild(Permissions::ADMINISTRATOR);
    let access = facts
        .access(Viewer::discord(MEMBER + 1))
        .expect("membership known");

    assert_eq!(access.membership, Membership::Outsider);
    assert_eq!(access.guild, Capabilities::empty());
    assert!(access.channels.is_empty());
}

#[test]
fn capabilities_follow_their_discord_defaults() {
    let mut facts = guild(Permissions::VIEW_CHANNEL);
    let owner = facts.access(Viewer::discord(OWNER)).expect("known");
    assert_eq!(owner.guild, Capabilities::MANAGEMENT);
    assert_eq!(owner.channels[&CHANNEL], Capabilities::CONTENT);

    // VIEW_CHANNEL alone shows presence and lets them stamp.
    let member = facts.access(Viewer::discord(MEMBER)).expect("known");
    assert_eq!(member.guild, Capabilities::empty());
    assert_eq!(
        member.channels[&CHANNEL],
        Capabilities::VIEW_PRESENCE | Capabilities::STAMP
    );

    facts
        .roles
        .insert(ROLE, Permissions::MANAGE_GUILD | Permissions::CONNECT);
    let manager = facts.access(Viewer::discord(MEMBER)).expect("known");
    assert_eq!(manager.guild, Capabilities::MANAGEMENT);
    assert_eq!(manager.channels[&CHANNEL], Capabilities::CONTENT);
}

#[test]
fn a_role_outside_the_guild_cannot_be_previewed() {
    let mut facts = guild(VOICE);
    assert!(facts.role_access(GUILD).is_ok());
    assert!(facts.role_access(ROLE).is_ok());
    assert_eq!(facts.role_access(ROLE + 1), Err(RoleNotFound));

    facts.everyone = None;
    assert_eq!(facts.role_access(GUILD), Err(RoleNotFound));
}

#[test]
fn a_preview_never_shows_more_than_the_manager_sees() {
    let hidden = CHANNEL + 1;
    let mut facts = guild(VOICE);
    facts.roles.insert(ROLE, Permissions::MANAGE_GUILD);
    facts.roles.insert(ROLE + 1, Permissions::ADMINISTRATOR);
    facts.channels.insert(
        hidden,
        ChannelFacts {
            members: HashMap::from([(MEMBER, overwrite(Permissions::empty(), VOICE))]),
            ..ChannelFacts::default()
        },
    );
    let manager = facts.access(Viewer::discord(MEMBER)).expect("known");
    let role = facts.role_access(ROLE + 1).expect("the guild's role");
    assert!(role.allows(hidden, Capabilities::LISTEN_OTHERS));

    let preview = RolePreview::new(&manager, &role);
    assert_eq!(preview.listed, [CHANNEL].into());
    assert!(preview.access.allows(CHANNEL, Capabilities::LISTEN_OTHERS));
    assert!(!preview.access.allows(hidden, Capabilities::VIEW_PRESENCE));
}

#[test]
fn a_role_preview_applies_only_that_roles_overwrites() {
    let mut facts = guild(VOICE);
    channel(&mut facts)
        .roles
        .insert(ROLE, overwrite(Permissions::empty(), Permissions::CONNECT));
    channel(&mut facts).members.insert(
        MEMBER,
        overwrite(Permissions::CONNECT, Permissions::empty()),
    );

    let everyone = facts.role_access(GUILD).expect("the guild's role");
    let role = facts.role_access(ROLE).expect("the guild's role");
    assert!(everyone.allows(CHANNEL, Capabilities::LISTEN_OTHERS));
    assert!(!role.allows(CHANNEL, Capabilities::LISTEN_OTHERS));
    assert!(role.allows(CHANNEL, Capabilities::VIEW_PRESENCE));
}

#[test]
fn a_recording_of_only_your_voice_is_yours_wherever_it_was_made() {
    let closed = CHANNEL + 1;
    let mut facts = guild(VOICE);
    facts.channels.insert(
        closed,
        ChannelFacts {
            everyone: overwrite(Permissions::empty(), VOICE),
            ..ChannelFacts::default()
        },
    );
    let member = facts.access(Viewer::discord(MEMBER)).expect("known");

    // Your own voice, even from a channel closed to you.
    assert!(member.allows_item(&[MEMBER], [closed]));
    assert!(member.allows_item(&[MEMBER, MEMBER], [CHANNEL, closed]));
    assert!(member.allows_item(&[MEMBER], []));
    // Anyone else's voice takes every one of its channels.
    assert!(member.allows_item(&[OWNER], [CHANNEL]));
    assert!(member.allows_item(&[MEMBER, OWNER], [CHANNEL]));
    assert!(!member.allows_item(&[MEMBER, OWNER], [CHANNEL, closed]));
    assert!(!member.allows_item(&[OWNER], []));
    // Unknown speakers are never yours.
    assert!(member.allows_item(&[], [CHANNEL]));
    assert!(!member.allows_item(&[], [closed]));

    // A role has no recordings of its own.
    let role = facts.role_access(ROLE).expect("the guild's role");
    assert!(!role.allows_item(&[MEMBER], [closed]));
    assert!(role.allows_item(&[MEMBER], [CHANNEL]));

    // An outsider hears nothing, not even their own voice.
    let outsider = facts.access(Viewer::discord(MEMBER + 1)).expect("known");
    assert!(!outsider.allows_item(&[MEMBER + 1], [CHANNEL]));
}

#[test]
fn whoever_has_recordings_here_but_does_not_belong_is_a_former_member() {
    let mut facts = guild(VOICE);
    let left = MEMBER + 1;
    facts.members.insert(
        left,
        MemberFacts {
            listed: false,
            roles: vec![ROLE],
            recorded: true,
        },
    );
    for user_id in [MEMBER, OWNER] {
        facts.members.entry(user_id).or_default().recorded = true;
    }
    let membership = |facts: &GuildFacts, user_id| facts.membership(Viewer::discord(user_id));

    assert_eq!(membership(&facts, MEMBER), Ok(Membership::Member));
    assert_eq!(membership(&facts, left), Ok(Membership::Former));
    assert_eq!(membership(&facts, left + 1), Ok(Membership::Outsider));
    let former = facts.access(Viewer::discord(left)).expect("known");
    assert_eq!(former.guild, Capabilities::empty());
    assert!(
        former.channels.is_empty(),
        "the roles they held count no more"
    );

    // An unknown roster is never guessed to be a departure.
    facts.roster = Roster::Incomplete;
    assert_eq!(membership(&facts, left), Err(MembershipUnavailable));
    facts.roster = Roster::Complete;

    // Once the bot has left, nobody belongs: whoever has recordings is a
    // former member, the owner included.
    facts.bot_present = false;
    assert_eq!(membership(&facts, OWNER), Ok(Membership::Former));
    assert_eq!(membership(&facts, MEMBER), Ok(Membership::Former));
    assert_eq!(membership(&facts, left + 1), Ok(Membership::Outsider));
    facts.roster = Roster::Incomplete;
    assert_eq!(membership(&facts, MEMBER), Ok(Membership::Former));
    // Dev logins follow their seeding wherever the bot is.
    let dev = Viewer {
        user_id: OWNER,
        dev: true,
    };
    assert_eq!(facts.membership(dev), Ok(Membership::Owner));
}

#[test]
fn a_private_voice_is_heard_by_its_speaker_alone_whatever_anyone_holds() {
    let mut facts = guild(VOICE);
    facts.private.insert(MEMBER);
    let member = facts.access(Viewer::discord(MEMBER)).expect("known");
    let owner = facts.access(Viewer::discord(OWNER)).expect("known");

    // Their own voice stays theirs, alone or with others'.
    assert!(member.allows_item(&[MEMBER], [CHANNEL]));
    assert!(member.allows_item(&[MEMBER, OWNER], [CHANNEL]));
    // Nobody else hears it, not even the owner, who holds everything.
    assert!(!owner.allows_item(&[MEMBER], [CHANNEL]));
    assert!(!owner.allows_item(&[OWNER, MEMBER], [CHANNEL]));
    assert!(owner.allows_item(&[OWNER], [CHANNEL]));
    // Unknown speakers cannot be asked; the channels decide.
    assert!(owner.allows_item(&[], [CHANNEL]));
    // A role hears nobody who keeps their voice private.
    let role = facts.role_access(ROLE).expect("the guild's role");
    assert!(!role.allows_item(&[MEMBER], [CHANNEL]));

    // The owner, who may delete recordings, sees it redacted; only where
    // they could otherwise listen, and never in a preview.
    assert!(owner.redacts_item(&[MEMBER], [CHANNEL]));
    assert!(!owner.redacts_item(&[MEMBER], []));
    assert!(!owner.redacts_item(&[OWNER], [CHANNEL]));
    assert!(!member.redacts_item(&[MEMBER], [CHANNEL]));
    let preview = RolePreview::new(&owner, &facts.role_access(GUILD).expect("@everyone"));
    assert!(!preview.access.redacts_item(&[MEMBER], [CHANNEL]));
    // A member who may not delete recordings sees nothing of it.
    facts.private = [OWNER].into();
    let member = facts.access(Viewer::discord(MEMBER)).expect("known");
    assert!(!member.allows_item(&[OWNER], [CHANNEL]));
    assert!(!member.redacts_item(&[OWNER], [CHANNEL]));
}

fn allow(capabilities: Capabilities) -> Rule {
    Rule {
        allow: capabilities,
        deny: Capabilities::empty(),
    }
}

fn deny(capabilities: Capabilities) -> Rule {
    Rule {
        allow: Capabilities::empty(),
        deny: capabilities,
    }
}

fn member_holds(facts: &GuildFacts, capability: Capabilities) -> bool {
    facts
        .access(Viewer::discord(MEMBER))
        .expect("known")
        .allows(CHANNEL, capability)
}

#[test]
fn a_sakiot_allow_grants_what_discord_denies() {
    let mut facts = guild(Permissions::VIEW_CHANNEL);
    assert!(!member_holds(&facts, Capabilities::LISTEN_OTHERS));

    facts.rules.set(
        None,
        Subject::Role(ROLE),
        allow(Capabilities::LISTEN_OTHERS),
    );

    assert!(member_holds(&facts, Capabilities::LISTEN_OTHERS));
    // Only what the rule names.
    assert!(!member_holds(&facts, Capabilities::CREATE_CLIPS));
}

#[test]
fn nothing_is_held_in_a_channel_without_seeing_it() {
    // Discord: CONNECT without VIEW_CHANNEL reaches nothing, stamping
    // included.
    let mut facts = guild(Permissions::CONNECT);
    let member = facts.access(Viewer::discord(MEMBER)).expect("known");
    assert_eq!(member.channels[&CHANNEL], Capabilities::empty());

    // A rule granting listening alone does not show the channel.
    facts.rules.set(
        None,
        Subject::Role(ROLE),
        allow(Capabilities::LISTEN_OTHERS),
    );
    assert!(!member_holds(&facts, Capabilities::LISTEN_OTHERS));

    // A rule taking seeing a channel takes everything else there, whatever
    // Discord or another rule grants, and the explanation says so.
    let mut facts = guild(VOICE);
    facts
        .rules
        .set(None, Subject::Role(ROLE), allow(Capabilities::JAM));
    facts.rules.set(
        Some(CHANNEL),
        Subject::Everyone,
        deny(Capabilities::VIEW_PRESENCE),
    );
    let member = facts.access(Viewer::discord(MEMBER)).expect("known");
    assert_eq!(member.channels[&CHANNEL], Capabilities::empty());
    let everyone = facts.role_access(GUILD).expect("@everyone");
    assert!(!everyone.allows(CHANNEL, Capabilities::LISTEN_OTHERS));
    let explained = facts.explain_role(GUILD).expect("@everyone");
    for decision in &explained.channels[&CHANNEL] {
        assert!(!decision.held, "{decision:?}");
        assert!(decision.discord, "{decision:?}");
        let expected = if decision.capability == Capabilities::VIEW_PRESENCE {
            Source::Rule {
                channel: Some(CHANNEL),
                subject: Subject::Everyone,
            }
        } else {
            Source::Unseen
        };
        assert_eq!(decision.by, expected, "{decision:?}");
    }
}

#[test]
fn changing_whether_a_channel_is_seen_takes_holding_all_it_carries() {
    // The editor sees the channel but cannot listen there, so taking seeing
    // from someone would take listening, which the editor lacks.
    let mut facts = guild(Permissions::VIEW_CHANNEL);
    facts.rules.set(
        None,
        Subject::User(MEMBER),
        allow(Capabilities::MANAGE_PERMISSIONS),
    );
    let editor = Viewer::discord(MEMBER);
    let editor_access = facts.access(editor).expect("known");
    assert!(editor_access.allows(CHANNEL, Capabilities::VIEW_PRESENCE));
    assert!(
        !editor_access
            .editable(Some(CHANNEL))
            .contains(Capabilities::VIEW_PRESENCE)
    );
    assert_eq!(
        facts.edit_rule(
            editor,
            Some(CHANNEL),
            Subject::Everyone,
            deny(Capabilities::VIEW_PRESENCE)
        ),
        Err(RuleRefused::NotHeld(Capabilities::VIEW_PRESENCE))
    );

    // Holding everything there, they may.
    channel(&mut facts).members.insert(
        MEMBER,
        overwrite(Permissions::CONNECT, Permissions::empty()),
    );
    assert_eq!(
        facts.edit_rule(
            editor,
            Some(CHANNEL),
            Subject::Role(ROLE + 1),
            deny(Capabilities::VIEW_PRESENCE)
        ),
        Ok(())
    );
}

#[test]
fn a_clipped_explanation_names_the_manager_for_what_it_leaves_out() {
    let mut facts = guild(VOICE);
    facts.roles.insert(ROLE, Permissions::MANAGE_GUILD);
    facts.roles.insert(ROLE + 1, Permissions::ADMINISTRATOR);
    facts.rules.set(
        Some(CHANNEL),
        Subject::User(MEMBER),
        deny(Capabilities::JAM),
    );
    // A manager who cannot edit rules, whom the rules therefore bind.
    facts.rules.set(
        None,
        Subject::User(MEMBER),
        deny(Capabilities::MODERATE_CLIPS | Capabilities::MANAGE_PERMISSIONS),
    );
    let manager = facts.access(Viewer::discord(MEMBER)).expect("known");
    let explained = facts
        .explain_role(ROLE + 1)
        .expect("the guild's role")
        .clipped_to(&manager);

    let find = |decisions: &[crate::Decision], capability| {
        decisions
            .iter()
            .find(|decision| decision.capability == capability)
            .copied()
            .expect("every capability")
    };
    let jam = find(&explained.channels[&CHANNEL], Capabilities::JAM);
    assert!(!jam.held && jam.discord);
    assert_eq!(jam.by, Source::Manager);
    let moderate = find(&explained.guild, Capabilities::MODERATE_CLIPS);
    assert!(!moderate.held);
    assert_eq!(moderate.by, Source::Manager);
    let listen = find(&explained.channels[&CHANNEL], Capabilities::LISTEN_OTHERS);
    assert!(listen.held);
    assert_eq!(listen.by, Source::Discord);
}

#[test]
fn content_rules_bind_an_administrator_only_once_they_cannot_edit_rules() {
    let mut facts = guild(Permissions::empty());
    facts.roles.insert(ROLE, Permissions::ADMINISTRATOR);
    facts
        .rules
        .set(Some(CHANNEL), Subject::Everyone, deny(Capabilities::JAM));
    // ADMINISTRATOR edits rules, so no rule takes from them: they could
    // undo it.
    assert!(member_holds(&facts, Capabilities::JAM));

    facts.rules.set(
        None,
        Subject::Role(ROLE),
        deny(Capabilities::MANAGE_PERMISSIONS),
    );
    assert!(!member_holds(&facts, Capabilities::JAM));
    assert!(member_holds(&facts, Capabilities::LISTEN_OTHERS));
}

#[test]
fn the_most_specific_rule_wins() {
    let mut facts = guild(VOICE);
    let stamp = Capabilities::STAMP;
    // Guild-wide: `@everyone` < the member's roles < the member.
    facts.rules.set(None, Subject::Everyone, deny(stamp));
    assert!(!member_holds(&facts, stamp));
    facts.rules.set(None, Subject::Role(ROLE), allow(stamp));
    assert!(member_holds(&facts, stamp));
    facts.rules.set(None, Subject::User(MEMBER), deny(stamp));
    assert!(!member_holds(&facts, stamp));
    // Any channel rule beats every guild-wide one, `@everyone`'s included.
    facts
        .rules
        .set(Some(CHANNEL), Subject::Everyone, allow(stamp));
    assert!(member_holds(&facts, stamp));
    facts
        .rules
        .set(Some(CHANNEL), Subject::Role(ROLE), deny(stamp));
    assert!(!member_holds(&facts, stamp));
    facts
        .rules
        .set(Some(CHANNEL), Subject::User(MEMBER), allow(stamp));
    assert!(member_holds(&facts, stamp));
    // A rule for someone else, or a role they do not hold, changes nothing.
    facts
        .rules
        .set(Some(CHANNEL), Subject::User(OWNER), deny(stamp));
    facts
        .rules
        .set(Some(CHANNEL), Subject::Role(ROLE + 1), deny(stamp));
    assert!(member_holds(&facts, stamp));
}

#[test]
fn among_roles_an_allow_beats_a_deny() {
    let other = ROLE + 1;
    let mut facts = guild(VOICE);
    facts.roles.insert(other, Permissions::empty());
    if let Some(member) = facts.members.get_mut(&MEMBER) {
        member.roles.push(other);
    }
    facts
        .rules
        .set(None, Subject::Role(ROLE), deny(Capabilities::CREATE_CLIPS));
    facts.rules.set(
        None,
        Subject::Role(other),
        allow(Capabilities::CREATE_CLIPS),
    );

    assert!(member_holds(&facts, Capabilities::CREATE_CLIPS));
}

#[test]
fn no_rule_takes_from_the_owner() {
    let mut facts = guild(VOICE);
    facts.rules.set(
        None,
        Subject::User(OWNER),
        deny(Capabilities::MANAGEMENT | Capabilities::LISTEN_OTHERS),
    );
    facts.rules.set(
        Some(CHANNEL),
        Subject::Everyone,
        deny(Capabilities::VIEW_PRESENCE),
    );

    let owner = facts.access(Viewer::discord(OWNER)).expect("known");
    assert_eq!(owner.guild, Capabilities::MANAGEMENT);
    assert_eq!(owner.channels[&CHANNEL], Capabilities::CONTENT);
    let explained = facts.explain(Viewer::discord(OWNER)).expect("known");
    assert!(
        explained
            .guild
            .iter()
            .all(|decision| decision.held && decision.by == Source::Owner)
    );
    // What the rules would take, they do not; the rest is Discord's.
    for decision in &explained.channels[&CHANNEL] {
        assert!(decision.held, "{decision:?}");
        let mentioned = matches!(
            decision.capability,
            Capabilities::VIEW_PRESENCE | Capabilities::LISTEN_OTHERS
        );
        let by = if mentioned {
            Source::Exempt
        } else {
            Source::Discord
        };
        assert_eq!(decision.by, by, "{decision:?}");
    }
}

#[test]
fn rules_take_nothing_from_an_editor_but_may_give_more() {
    let mut facts = guild(Permissions::VIEW_CHANNEL);
    facts.rules.set(
        None,
        Subject::User(MEMBER),
        allow(Capabilities::MANAGE_PERMISSIONS),
    );
    facts.rules.set(
        Some(CHANNEL),
        Subject::Everyone,
        Rule {
            allow: Capabilities::LISTEN_OTHERS,
            deny: Capabilities::VIEW_PRESENCE | Capabilities::STAMP,
        },
    );

    let editor = facts.access(Viewer::discord(MEMBER)).expect("known");
    assert_eq!(
        editor.channels[&CHANNEL],
        Capabilities::VIEW_PRESENCE | Capabilities::STAMP | Capabilities::LISTEN_OTHERS
    );
    let explained = facts.explain(Viewer::discord(MEMBER)).expect("known");
    let by = |capability| {
        explained.channels[&CHANNEL]
            .iter()
            .find(|decision| decision.capability == capability)
            .map(|decision| (decision.held, decision.by))
    };
    assert_eq!(
        by(Capabilities::VIEW_PRESENCE),
        Some((true, Source::Exempt))
    );
    assert_eq!(by(Capabilities::STAMP), Some((true, Source::Exempt)));
    assert_eq!(
        by(Capabilities::LISTEN_OTHERS),
        Some((
            true,
            Source::Rule {
                channel: Some(CHANNEL),
                subject: Subject::Everyone
            }
        ))
    );
    assert_eq!(by(Capabilities::JAM), Some((false, Source::Discord)));
}

#[test]
fn rules_apply_management_guild_wide_and_content_per_channel() {
    let mut facts = guild(VOICE);
    // A channel rule cannot reach a guild-wide capability.
    facts.rules.set(
        Some(CHANNEL),
        Subject::User(MEMBER),
        allow(Capabilities::DELETE_RECORDINGS),
    );
    let member = facts.access(Viewer::discord(MEMBER)).expect("known");
    assert_eq!(member.guild, Capabilities::empty());
    assert!(!member.allows(CHANNEL, Capabilities::DELETE_RECORDINGS));
    // A guild-wide one grants it, and its content capabilities reach every
    // channel.
    facts.rules.set(
        None,
        Subject::User(MEMBER),
        Rule {
            allow: Capabilities::DELETE_RECORDINGS,
            deny: Capabilities::STAMP,
        },
    );
    let member = facts.access(Viewer::discord(MEMBER)).expect("known");
    assert_eq!(member.guild, Capabilities::DELETE_RECORDINGS);
    assert!(!member.allows(CHANNEL, Capabilities::STAMP));
}

#[test]
fn rules_never_reach_a_former_member() {
    let mut facts = guild(VOICE);
    if let Some(member) = facts.members.get_mut(&MEMBER) {
        member.listed = false;
        member.recorded = true;
    }
    facts
        .rules
        .set(None, Subject::User(MEMBER), allow(Capabilities::all()));

    let former = facts.access(Viewer::discord(MEMBER)).expect("known");
    assert_eq!(former.membership, Membership::Former);
    assert_eq!(former.guild, Capabilities::empty());
    assert!(former.channels.is_empty());
}

#[test]
fn a_role_preview_applies_everyones_and_that_roles_rules() {
    let mut facts = guild(VOICE);
    facts
        .rules
        .set(None, Subject::Everyone, deny(Capabilities::JAM));
    facts.rules.set(
        Some(CHANNEL),
        Subject::Role(ROLE),
        allow(Capabilities::PREVIEW_ROLES | Capabilities::JAM),
    );
    facts
        .rules
        .set(None, Subject::User(MEMBER), allow(Capabilities::JAM));

    let everyone = facts.role_access(GUILD).expect("@everyone");
    assert!(!everyone.allows(CHANNEL, Capabilities::JAM));
    let role = facts.role_access(ROLE).expect("the guild's role");
    assert!(role.allows(CHANNEL, Capabilities::JAM));
    assert_eq!(role.guild, Capabilities::empty());
    let explained = facts.explain_role(ROLE).expect("the guild's role");
    let jam = explained.channels[&CHANNEL]
        .iter()
        .find(|decision| decision.capability == Capabilities::JAM)
        .expect("every content capability");
    assert!(jam.held && jam.discord);
    assert_eq!(
        jam.by,
        Source::Rule {
            channel: Some(CHANNEL),
            subject: Subject::Role(ROLE)
        }
    );
}

#[test]
fn an_editor_changes_only_what_they_hold_and_keeps_editing() {
    let mut facts = guild(Permissions::VIEW_CHANNEL);
    let editor = Viewer::discord(MEMBER);
    let change = |facts: &mut GuildFacts, channel, subject, rule| {
        facts.edit_rule(editor, channel, subject, rule)
    };
    assert_eq!(
        change(
            &mut facts,
            None,
            Subject::Everyone,
            deny(Capabilities::STAMP)
        ),
        Err(RuleRefused::NotAnEditor)
    );

    facts.rules.set(
        None,
        Subject::User(MEMBER),
        allow(Capabilities::MANAGE_PERMISSIONS),
    );
    // Not what they lack: they cannot listen in the channel.
    assert_eq!(
        change(
            &mut facts,
            Some(CHANNEL),
            Subject::Role(ROLE),
            allow(Capabilities::LISTEN_OTHERS | Capabilities::STAMP)
        ),
        Err(RuleRefused::NotHeld(Capabilities::LISTEN_OTHERS))
    );
    assert_eq!(
        change(
            &mut facts,
            None,
            Subject::Everyone,
            allow(Capabilities::DELETE_RECORDINGS)
        ),
        Err(RuleRefused::NotHeld(Capabilities::DELETE_RECORDINGS))
    );
    // What they hold, they may give or take.
    assert_eq!(
        change(
            &mut facts,
            Some(CHANNEL),
            Subject::Everyone,
            deny(Capabilities::STAMP)
        ),
        Ok(())
    );
    assert_eq!(
        facts.rules.get(Some(CHANNEL), Subject::Everyone),
        deny(Capabilities::STAMP)
    );
    // `MANAGE_PERMISSIONS` lifts every content rule for whoever holds it,
    // so changing it takes holding everything in every channel.
    assert_eq!(
        change(
            &mut facts,
            None,
            Subject::Role(ROLE),
            allow(Capabilities::MANAGE_PERMISSIONS)
        ),
        Err(RuleRefused::NotHeld(Capabilities::MANAGE_PERMISSIONS))
    );
    channel(&mut facts).members.insert(
        MEMBER,
        overwrite(Permissions::CONNECT, Permissions::empty()),
    );
    // Never `MANAGE_PERMISSIONS` from themselves.
    assert_eq!(
        change(&mut facts, None, Subject::User(MEMBER), Rule::default()),
        Err(RuleRefused::OwnManagePermissions)
    );
    assert_eq!(
        change(
            &mut facts,
            Some(CHANNEL),
            Subject::Everyone,
            deny(Capabilities::MANAGE_PERMISSIONS)
        ),
        Err(RuleRefused::ManagementInChannel)
    );
    // The owner may change anything, their own rules included.
    facts
        .rules
        .set(None, Subject::User(OWNER), deny(Capabilities::all()));
    assert_eq!(
        facts.edit_rule(
            Viewer::discord(OWNER),
            Some(CHANNEL),
            Subject::Role(ROLE),
            allow(Capabilities::LISTEN_OTHERS)
        ),
        Ok(())
    );
}
