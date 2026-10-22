//! Sakiot's own permission rules, layered over Discord's.
//!
//! A rule allows or denies capabilities to `@everyone`, a role or a user,
//! either guild-wide or in one voice or stage channel. Rules apply after
//! Discord's permissions and in Discord's own order, the most specific last:
//! guild-wide `@everyone`, the holder's roles together, the user; then the
//! same three in the channel. Within one layer an allow beats a deny, as
//! with Discord's role overwrites.

use std::collections::{BTreeMap, HashMap};

use crate::capabilities::Capabilities;

/// Whom a rule is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Subject {
    Everyone,
    Role(i64),
    User(i64),
}

impl Subject {
    /// As rules are stored: a kind (`everyone`, `role` or `user`) and, for a
    /// role or a user, its id.
    pub fn stored(self) -> (&'static str, Option<i64>) {
        match self {
            Self::Everyone => ("everyone", None),
            Self::Role(role_id) => ("role", Some(role_id)),
            Self::User(user_id) => ("user", Some(user_id)),
        }
    }

    /// From its stored form; `None` for anything `stored` never writes.
    pub fn from_stored(kind: &str, id: Option<i64>) -> Option<Self> {
        match (kind, id) {
            ("everyone", None) => Some(Self::Everyone),
            ("role", Some(role_id)) => Some(Self::Role(role_id)),
            ("user", Some(user_id)) => Some(Self::User(user_id)),
            _ => None,
        }
    }
}

/// What one rule changes: the capabilities it removes, then those it
/// grants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rule {
    pub allow: Capabilities,
    pub deny: Capabilities,
}

impl Rule {
    pub fn apply(self, held: Capabilities) -> Capabilities {
        held.difference(self.deny).union(self.allow)
    }

    /// Several rules at one layer, such as all of a member's roles, act as
    /// one: every deny, then every allow.
    fn merge(self, other: Self) -> Self {
        Self {
            allow: self.allow.union(other.allow),
            deny: self.deny.union(other.deny),
        }
    }

    /// The rule limited to `scope`'s capabilities.
    pub(crate) fn within(self, scope: Capabilities) -> Self {
        Self {
            allow: self.allow & scope,
            deny: self.deny & scope,
        }
    }

    /// The capabilities it mentions, allowed or denied.
    pub fn mentions(self) -> Capabilities {
        self.allow | self.deny
    }

    pub fn is_empty(self) -> bool {
        self.mentions().is_empty()
    }
}

/// The rules at one scope: guild-wide, or in one channel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuleSet {
    pub everyone: Rule,
    pub roles: HashMap<i64, Rule>,
    pub users: HashMap<i64, Rule>,
}

impl RuleSet {
    pub fn get(&self, subject: Subject) -> Rule {
        match subject {
            Subject::Everyone => self.everyone,
            Subject::Role(role_id) => self.roles.get(&role_id).copied().unwrap_or_default(),
            Subject::User(user_id) => self.users.get(&user_id).copied().unwrap_or_default(),
        }
    }

    /// Sets the rule for `subject`; an empty one removes it.
    pub fn set(&mut self, subject: Subject, rule: Rule) {
        match subject {
            Subject::Everyone => self.everyone = rule,
            Subject::Role(role_id) if rule.is_empty() => {
                self.roles.remove(&role_id);
            }
            Subject::Role(role_id) => {
                self.roles.insert(role_id, rule);
            }
            Subject::User(user_id) if rule.is_empty() => {
                self.users.remove(&user_id);
            }
            Subject::User(user_id) => {
                self.users.insert(user_id, rule);
            }
        }
    }

    /// The layers that apply to a holder of `roles`, and to `user` unless a
    /// role is evaluated, in the order they apply, each with the rule that
    /// decides it. A layer's subject names a role whose rule decides some
    /// of what the layer does (`Layer::subject`).
    pub(crate) fn layers(&self, roles: &[i64], user: Option<i64>) -> [Layer; 3] {
        let held: Vec<(i64, Rule)> = roles
            .iter()
            .filter_map(|role_id| Some((*role_id, *self.roles.get(role_id)?)))
            .collect();
        [
            Layer::One(Subject::Everyone, self.everyone),
            Layer::Roles(held),
            user.map_or(Layer::None, |user_id| {
                Layer::One(Subject::User(user_id), self.get(Subject::User(user_id)))
            }),
        ]
    }

    fn is_empty(&self) -> bool {
        self.everyone.is_empty() && self.roles.is_empty() && self.users.is_empty()
    }
}

/// One layer of rules, as `RuleSet::layers` hands them out.
#[derive(Clone, Debug)]
pub(crate) enum Layer {
    None,
    One(Subject, Rule),
    /// The holder's roles' rules, which act together.
    Roles(Vec<(i64, Rule)>),
}

impl Layer {
    /// The layer's rules acting as one.
    pub(crate) fn rule(&self) -> Rule {
        match self {
            Self::None => Rule::default(),
            Self::One(_, rule) => *rule,
            Self::Roles(roles) => roles
                .iter()
                .fold(Rule::default(), |merged, (_, rule)| merged.merge(*rule)),
        }
    }

    /// Who decides `capability` at this layer, if anyone: the subject, or
    /// for roles, the lowest role id allowing it, else the lowest denying it
    /// (an allow beats a deny).
    pub(crate) fn subject(&self, capability: Capabilities) -> Option<Subject> {
        match self {
            Self::None => None,
            Self::One(subject, rule) => rule.mentions().contains(capability).then_some(*subject),
            Self::Roles(roles) => {
                let lowest = |side: fn(&Rule) -> Capabilities| {
                    roles
                        .iter()
                        .filter(|(_, rule)| side(rule).contains(capability))
                        .map(|(role_id, _)| *role_id)
                        .min()
                };
                lowest(|rule| rule.allow)
                    .or_else(|| lowest(|rule| rule.deny))
                    .map(Subject::Role)
            }
        }
    }
}

/// A guild's rules.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    pub guild: RuleSet,
    pub channels: BTreeMap<i64, RuleSet>,
}

impl Rules {
    /// The rule for `subject` guild-wide (`channel` `None`) or in `channel`.
    pub fn get(&self, channel: Option<i64>, subject: Subject) -> Rule {
        match channel {
            None => self.guild.get(subject),
            Some(channel_id) => self
                .channels
                .get(&channel_id)
                .map(|rules| rules.get(subject))
                .unwrap_or_default(),
        }
    }

    /// Sets the rule for `subject` guild-wide (`channel` `None`) or in
    /// `channel`; an empty one removes it.
    pub fn set(&mut self, channel: Option<i64>, subject: Subject, rule: Rule) {
        match channel {
            None => self.guild.set(subject, rule),
            Some(channel_id) => {
                let rules = self.channels.entry(channel_id).or_default();
                rules.set(subject, rule);
                if rules.is_empty() {
                    self.channels.remove(&channel_id);
                }
            }
        }
    }
}
