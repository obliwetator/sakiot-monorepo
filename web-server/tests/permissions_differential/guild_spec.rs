//! Random guilds for the permissions harness.
//!
//! Plain data and proptest strategies only, with no database or web-server
//! types, so the redesign can move it into the shared permissions crate and
//! reuse it for the evaluator's unit tests and the serenity comparison.
//!
//! The generator stays inside what the bot can write: member role lists never
//! hold `@everyone`, and a role overwrite only targets `@everyone`, one of the
//! guild's roles, or a role deleted from `roles` before its overwrites were.

use proptest::prelude::*;

pub const ADMINISTRATOR: i64 = 1 << 3;
pub const MANAGE_GUILD: i64 = 1 << 5;
pub const VIEW_CHANNEL: i64 = 1 << 10;
pub const CONNECT: i64 = 1 << 20;
/// A bit Discord has not defined: stored and combined like any other.
pub const UNDEFINED: i64 = 1 << 50;

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

/// The bot's member roster for the guild (`guild_projection_state`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Roster {
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
pub struct Overwrite {
    pub target: Target,
    pub allow: i64,
    pub deny: i64,
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub kind: ChannelKind,
    /// At most one per target, as `channel_permissions` allows.
    pub overwrites: Vec<Overwrite>,
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
}

#[derive(Clone, Debug)]
pub struct Guild {
    /// An index into the world's users, or `None` for `STRANGER_ID`.
    pub owner: Option<usize>,
    pub roster: Roster,
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
                7 => Just(Roster::Complete),
                2 => Just(Roster::Incomplete),
                1 => Just(Roster::Missing),
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
    )
        .prop_map(|(listed, roles, seed)| Member {
            listed,
            roles,
            seed,
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
                    .map(|(target, (allow, deny))| Overwrite {
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
