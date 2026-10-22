//! What the evaluator knows about a guild: plain data, however it was
//! loaded.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::discord::{Overwrite, Permissions};
use crate::rules::Rules;

/// Who is asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Viewer {
    pub user_id: i64,
    /// A dev login. Those exist only in local, staging and preview builds,
    /// whose guilds are seeded rather than seen by the bot, so their
    /// membership comes from the seeding (`GuildFacts::seeds`).
    pub dev: bool,
}

impl Viewer {
    /// A Discord login.
    pub fn discord(user_id: i64) -> Self {
        Self {
            user_id,
            dev: false,
        }
    }
}

/// Whether the bot's member list for the guild was ever complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Roster {
    /// Authoritative, even while the bot is away or between deploys.
    Complete,
    /// Never complete, or never started.
    Incomplete,
}

/// One guild, loaded for a set of users and roles. Only what those users
/// and roles need is here, so the facts stay proportional to the question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuildFacts {
    pub guild_id: i64,
    /// `None` when nothing about the guild is known.
    pub owner_id: Option<i64>,
    pub roster: Roster,
    pub bot_present: bool,
    /// The permissions of `@everyone`, the role whose id is the guild's;
    /// `None` until that role is known.
    pub everyone: Option<Permissions>,
    /// The guild's other loaded roles: those the loaded users hold, and any
    /// role asked about by id. A role missing here is not the guild's.
    pub roles: HashMap<i64, Permissions>,
    /// Every voice and stage channel, the only channels the bot records.
    pub channels: BTreeMap<i64, ChannelFacts>,
    /// The loaded users. One missing here is unlisted, holds no roles and
    /// has no recordings.
    pub members: HashMap<i64, MemberFacts>,
    /// Dev seeding: the owner flag of each loaded user's seeded entry.
    pub seeds: HashMap<i64, bool>,
    /// Everyone who keeps their recordings to themselves here (listening
    /// consent `private`), loaded or not: nobody else may hear their voice,
    /// the owner included.
    pub private: BTreeSet<i64>,
    /// Sakiot's own rules for `@everyone`, the loaded roles and the loaded
    /// users, guild-wide and in voice and stage channels.
    pub rules: Rules,
}

impl GuildFacts {
    /// A guild nothing is known about.
    pub fn new(guild_id: i64) -> Self {
        Self {
            guild_id,
            owner_id: None,
            roster: Roster::Incomplete,
            bot_present: false,
            everyone: None,
            roles: HashMap::new(),
            channels: BTreeMap::new(),
            members: HashMap::new(),
            seeds: HashMap::new(),
            private: BTreeSet::new(),
            rules: Rules::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemberFacts {
    /// Listed in the bot's member list.
    pub listed: bool,
    /// The guild's roles the user holds. Never `@everyone`, which everyone
    /// holds implicitly.
    pub roles: Vec<i64>,
    /// Has recordings of their own in the guild that are not erased yet,
    /// which they keep reaching after leaving (`Membership::Former`).
    pub recorded: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelFacts {
    /// The `@everyone` overwrite; empty when the channel has none.
    pub everyone: Overwrite,
    /// The overwrites for loaded roles.
    pub roles: HashMap<i64, Overwrite>,
    /// The overwrites for loaded users.
    pub members: HashMap<i64, Overwrite>,
}
