use std::collections::HashMap;

use crate::{
    Capabilities, ChannelFacts, GuildFacts, MemberFacts, Membership, MembershipUnavailable,
    Overwrite, Permissions, RoleNotFound, RolePreview, Roster, Viewer,
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

    // VIEW_CHANNEL alone shows presence; stamping has never been gated.
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
