//! Random guilds, for this crate's tests and the web server's permission
//! harness (the `testing` feature).
//!
//! A [`World`] describes guilds as the bot's cache would store them. The web
//! server's harness writes them into its cache tables; [`World::facts`] is
//! the same guild loaded in memory, for the evaluator.
//!
//! The generator stays inside what the bot can write: member role lists never
//! hold `@everyone`, and a role overwrite only targets `@everyone`, one of the
//! guild's roles, or a role deleted from `roles` before its overwrites were.

use std::collections::{BTreeMap, HashMap};

use proptest::prelude::*;

use crate::discord::Permissions;
use crate::facts::{ChannelFacts, GuildFacts, MemberFacts};

#[cfg(any(test, feature = "serenity"))]
pub mod gateway;

const ADMINISTRATOR: i64 = Permissions::ADMINISTRATOR.bits();
const MANAGE_GUILD: i64 = Permissions::MANAGE_GUILD.bits();
const VIEW_CHANNEL: i64 = Permissions::VIEW_CHANNEL.bits();
const CONNECT: i64 = Permissions::CONNECT.bits();
/// A bit Discord has not defined: stored and combined like any other.
const UNDEFINED: i64 = 1 << 50;

/// A guild id nothing is stored for.
pub const UNKNOWN_GUILD_ID: i64 = 9_000_000;
/// The owner of a guild whose owner is not one of the generated users.
pub const STRANGER_ID: i64 = 9_999;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    Text,
    Voice,
    Category,
    Stage,
}

impl ChannelKind {
    pub fn code(self) -> i32 {
        match self {
            Self::Text => 0,
            Self::Voice => 2,
            Self::Category => 4,
            Self::Stage => 13,
        }
    }

    /// Voice and stage channels: the only ones the bot records, and the only
    /// ones permissions are computed for.
    pub fn recorded(self) -> bool {
        matches!(self, Self::Voice | Self::Stage)
    }
}

/// The bot's member list for the guild (`guild_projection_state`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RosterState {
    /// No projection row yet.
    Missing,
    /// A projection row whose roster was never complete.
    Incomplete,
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Everyone,
    /// An index into `Guild::roles`.
    Role(usize),
    /// An index into the world's users.
    User(usize),
    /// A role no longer in `roles`.
    DeletedRole,
}

#[derive(Clone, Debug)]
pub struct TargetedOverwrite {
    pub target: Target,
    pub allow: i64,
    pub deny: i64,
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub kind: ChannelKind,
    /// At most one per target, as `channel_permissions` allows.
    pub overwrites: Vec<TargetedOverwrite>,
}

/// One user's standing in one guild.
#[derive(Clone, Debug)]
pub struct Member {
    /// Listed in the roster (`guild_members`); stored only when the guild
    /// has a projection row.
    pub listed: bool,
    /// Which of `Guild::roles` the user holds (`user_roles`). Unlisted users
    /// may keep roles: a partial sync leaves absent members' roles alone.
    pub roles: Vec<bool>,
    /// A dev login's seeded `user_guilds` row and its owner flag.
    pub seed: Option<bool>,
    /// Has a recording session of their own in the guild.
    pub recorded: bool,
}

#[derive(Clone, Debug)]
pub struct Guild {
    /// An index into the world's users, or `None` for `STRANGER_ID`.
    pub owner: Option<usize>,
    pub roster: RosterState,
    pub bot_present: bool,
    /// The `@everyone` role's permissions; `None` while its row is not
    /// cached yet.
    pub everyone: Option<i64>,
    /// The permissions of every other role.
    pub roles: Vec<i64>,
    pub channels: Vec<Channel>,
    /// One per user of the world.
    pub members: Vec<Member>,
}

/// Guilds that share one pool of users, so a role or overwrite leaking from
/// one guild into another shows up.
#[derive(Clone, Debug)]
pub struct World {
    pub users: usize,
    pub guilds: Vec<Guild>,
}

impl World {
    pub fn guild_id(guild: usize) -> i64 {
        (index(guild) + 1) * 1_000_000
    }

    pub fn role_id(guild: usize, role: usize) -> i64 {
        Self::guild_id(guild) + 100 + index(role)
    }

    pub fn deleted_role_id(guild: usize) -> i64 {
        Self::guild_id(guild) + 199
    }

    pub fn channel_id(guild: usize, channel: usize) -> i64 {
        Self::guild_id(guild) + 500 + index(channel)
    }

    pub fn user_id(user: usize) -> i64 {
        10 + index(user)
    }

    pub fn owner_id(guild: &Guild) -> i64 {
        guild.owner.map_or(STRANGER_ID, Self::user_id)
    }

    /// A generated guild's id, or `UNKNOWN_GUILD_ID` for `None`.
    pub fn guild_id_or_unknown(guild: Option<usize>) -> i64 {
        guild.map_or(UNKNOWN_GUILD_ID, Self::guild_id)
    }

    /// Everything about a guild (`None`: the unknown one) as the evaluator
    /// reads it, loaded for every user and role.
    pub fn facts(&self, guild: Option<usize>) -> GuildFacts {
        let mut facts = GuildFacts::new(Self::guild_id_or_unknown(guild));
        let Some((g, spec)) = guild.and_then(|g| Some((g, self.guilds.get(g)?))) else {
            return facts;
        };
        facts.owner_id = Some(Self::owner_id(spec));
        facts.roster = if spec.roster == RosterState::Complete {
            crate::Roster::Complete
        } else {
            crate::Roster::Incomplete
        };
        facts.bot_present = spec.bot_present;
        facts.everyone = spec.everyone.map(Permissions::from_bits_retain);
        facts.roles = spec
            .roles
            .iter()
            .enumerate()
            .map(|(r, &bits)| (Self::role_id(g, r), Permissions::from_bits_retain(bits)))
            .collect();
        facts.channels = spec
            .channels
            .iter()
            .enumerate()
            .filter(|(_, channel)| channel.kind.recorded())
            .map(|(c, channel)| (Self::channel_id(g, c), channel_facts(g, channel)))
            .collect::<BTreeMap<_, _>>();
        for (u, member) in spec.members.iter().enumerate() {
            let user_id = Self::user_id(u);
            let roles = member
                .roles
                .iter()
                .enumerate()
                .filter(|(_, held)| **held)
                .map(|(r, _)| Self::role_id(g, r))
                .collect();
            let listed = member.listed && spec.roster != RosterState::Missing;
            facts.members.insert(
                user_id,
                MemberFacts {
                    listed,
                    roles,
                    recorded: member.recorded,
                },
            );
            if let Some(owner) = member.seed {
                facts.seeds.insert(user_id, owner);
            }
        }
        facts
    }
}

fn channel_facts(guild: usize, channel: &Channel) -> ChannelFacts {
    let mut facts = ChannelFacts {
        everyone: crate::Overwrite::default(),
        roles: HashMap::new(),
        members: HashMap::new(),
    };
    for overwrite in &channel.overwrites {
        let bits = crate::Overwrite {
            allow: Permissions::from_bits_retain(overwrite.allow),
            deny: Permissions::from_bits_retain(overwrite.deny),
        };
        match overwrite.target {
            Target::Everyone => facts.everyone = bits,
            Target::Role(r) => {
                facts.roles.insert(World::role_id(guild, r), bits);
            }
            Target::User(u) => {
                facts.members.insert(World::user_id(u), bits);
            }
            // Only the roles in `roles` are loaded.
            Target::DeletedRole => {}
        }
    }
    facts
}

fn index(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub fn world() -> impl Strategy<Value = World> {
    (1..=3usize, 1..=2usize).prop_flat_map(|(users, guilds)| {
        prop::collection::vec(guild(users), guilds).prop_map(move |guilds| World { users, guilds })
    })
}

fn guild(users: usize) -> impl Strategy<Value = Guild> {
    (0..=3usize, 1..=4usize).prop_flat_map(move |(roles, channels)| {
        (
            prop::option::weighted(0.4, 0..users),
            prop_oneof![
                7 => Just(RosterState::Complete),
                2 => Just(RosterState::Incomplete),
                1 => Just(RosterState::Missing),
            ],
            prop::bool::weighted(0.85),
            prop::option::weighted(0.9, role_bits()),
            prop::collection::vec(role_bits(), roles),
            prop::collection::vec(channel(roles, users), channels),
            prop::collection::vec(member(roles), users),
        )
            .prop_map(
                |(owner, roster, bot_present, everyone, roles, channels, members)| Guild {
                    owner,
                    roster,
                    bot_present,
                    everyone,
                    roles,
                    channels,
                    members,
                },
            )
    })
}

fn member(roles: usize) -> impl Strategy<Value = Member> {
    (
        prop::bool::weighted(0.75),
        prop::collection::vec(prop::bool::weighted(0.4), roles),
        prop_oneof![
            5 => Just(None),
            3 => Just(Some(false)),
            2 => Just(Some(true)),
        ],
        prop::bool::weighted(0.5),
    )
        .prop_map(|(listed, roles, seed, recorded)| Member {
            listed,
            roles,
            seed,
            recorded,
        })
}

fn channel(roles: usize, users: usize) -> impl Strategy<Value = Channel> {
    (
        prop_oneof![
            6 => Just(ChannelKind::Voice),
            2 => Just(ChannelKind::Stage),
            1 => Just(ChannelKind::Text),
            1 => Just(ChannelKind::Category),
        ],
        prop::option::weighted(0.6, overwrite_bits()),
        prop::collection::vec(prop::option::weighted(0.4, overwrite_bits()), roles),
        prop::collection::vec(prop::option::weighted(0.25, overwrite_bits()), users),
        prop::option::weighted(0.1, overwrite_bits()),
    )
        .prop_map(|(kind, everyone, roles, users, deleted)| {
            let targeted = everyone
                .map(|bits| (Target::Everyone, bits))
                .into_iter()
                .chain(
                    roles
                        .into_iter()
                        .enumerate()
                        .filter_map(|(role, bits)| bits.map(|bits| (Target::Role(role), bits))),
                )
                .chain(
                    users
                        .into_iter()
                        .enumerate()
                        .filter_map(|(user, bits)| bits.map(|bits| (Target::User(user), bits))),
                )
                .chain(deleted.map(|bits| (Target::DeletedRole, bits)));
            Channel {
                kind,
                overwrites: targeted
                    .map(|(target, (allow, deny))| TargetedOverwrite {
                        target,
                        allow,
                        deny,
                    })
                    .collect(),
            }
        })
}

/// Guild-level role permissions, weighted toward the bits that decide
/// access, plus an occasional unrelated or undefined bit.
fn role_bits() -> impl Strategy<Value = i64> {
    (
        prop::bool::weighted(0.6),
        prop::bool::weighted(0.6),
        prop::bool::weighted(0.08),
        prop::bool::weighted(0.2),
        noise(),
    )
        .prop_map(|(view, connect, administrator, manage_guild, noise)| {
            [
                (view, VIEW_CHANNEL),
                (connect, CONNECT),
                (administrator, ADMINISTRATOR),
                (manage_guild, MANAGE_GUILD),
            ]
            .into_iter()
            .filter(|(set, _)| *set)
            .fold(noise, |bits, (_, bit)| bits | bit)
        })
}

/// An overwrite's `(allow, deny)`. VIEW_CHANNEL and CONNECT are each left
/// alone, allowed, denied, or (rarely) both, which Discord resolves as allow.
fn overwrite_bits() -> impl Strategy<Value = (i64, i64)> {
    (
        allow_or_deny(VIEW_CHANNEL),
        allow_or_deny(CONNECT),
        noise(),
        noise(),
    )
        .prop_map(
            |((allow_view, deny_view), (allow_connect, deny_connect), allow, deny)| {
                (
                    allow_view | allow_connect | allow,
                    deny_view | deny_connect | deny,
                )
            },
        )
}

fn allow_or_deny(bit: i64) -> impl Strategy<Value = (i64, i64)> {
    prop_oneof![
        4 => Just((0, 0)),
        2 => Just((bit, 0)),
        3 => Just((0, bit)),
        1 => Just((bit, bit)),
    ]
}

fn noise() -> impl Strategy<Value = i64> {
    prop_oneof![
        3 => Just(0),
        1 => (0..41u32).prop_map(|bit| 1_i64 << bit),
        1 => Just(UNDEFINED),
    ]
}
