//! The evaluator: facts in, access out.

use std::collections::{BTreeMap, BTreeSet};

use crate::capabilities::Capabilities;
use crate::discord::{Overwrite, Permissions};
use crate::facts::{GuildFacts, Roster, Viewer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Membership {
    Owner,
    Member,
    /// No longer a member, with recordings of their own still here: they
    /// reach those (`Access::allows_item`) and nothing else. Everyone with
    /// recordings is one once the bot has left the guild, the owner
    /// included: the cached roles and overwrites have stopped updating.
    Former,
    /// Never a member, or no longer one with nothing of their own here.
    /// Gets nothing.
    Outsider,
}

/// The bot is in the guild but its member list was never complete, so
/// whether the viewer belongs is unknown. Retryable; never guessed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MembershipUnavailable;

/// A role that is not one of the guild's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoleNotFound;

/// Discord's answer for one member: their guild-level permissions, and their
/// permissions in each voice and stage channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscordPermissions {
    pub base: Permissions,
    pub channels: BTreeMap<i64, Permissions>,
}

/// What a viewer may do in a guild.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    /// Never `Former` or `Outsider` with any capability.
    pub membership: Membership,
    /// Whose access this is; `None` for a role's, which has no recordings
    /// of its own.
    pub user_id: Option<i64>,
    /// Guild-wide (management) capabilities.
    pub guild: Capabilities,
    /// Content capabilities per voice and stage channel. Empty for a former
    /// member or an outsider.
    pub channels: BTreeMap<i64, Capabilities>,
    /// The others whose voices the viewer may not hear anywhere: everyone
    /// else who keeps their recordings to themselves. Empty for a former
    /// member or an outsider, who hear nobody else's anyway.
    pub private: BTreeSet<i64>,
}

impl Access {
    /// No access at all: a former member's or an outsider's.
    fn none(membership: Membership, user_id: Option<i64>) -> Self {
        Self {
            membership,
            user_id,
            guild: Capabilities::empty(),
            channels: BTreeMap::new(),
            private: BTreeSet::new(),
        }
    }

    /// The channels where the viewer holds `capability`.
    pub fn channels_with(&self, capability: Capabilities) -> impl Iterator<Item = i64> + '_ {
        self.channels
            .iter()
            .filter(move |(_, held)| held.contains(capability))
            .map(|(&channel, _)| channel)
    }

    pub fn allows(&self, channel: i64, capability: Capabilities) -> bool {
        self.channels
            .get(&channel)
            .is_some_and(|held| held.contains(capability))
    }

    /// What the viewer may allow or deny in a Sakiot rule guild-wide
    /// (`channel` `None`) or in `channel`: only what they hold there
    /// themselves, as Discord lets a member grant only permissions they
    /// have. A guild-wide content capability reaches every channel, so it
    /// takes holding it in every one. Seeing a channel carries everything
    /// else in it (`Capabilities::seen`), so changing `VIEW_PRESENCE` takes
    /// holding every content capability. The owner may change anything.
    pub fn editable(&self, channel: Option<i64>) -> Capabilities {
        let carried = |held: Capabilities| {
            if held.contains(Capabilities::CONTENT) {
                held
            } else {
                held - Capabilities::VIEW_PRESENCE
            }
        };
        match (self.membership, channel) {
            (Membership::Owner, None) => Capabilities::all(),
            (Membership::Owner, Some(_)) => Capabilities::CONTENT,
            (Membership::Member, None) => {
                let everywhere = self
                    .channels
                    .values()
                    .fold(Capabilities::CONTENT, |held, channel| held & *channel);
                self.guild | carried(everywhere)
            }
            (Membership::Member, Some(channel)) => {
                carried(self.channels.get(&channel).copied().unwrap_or_default())
            }
            (Membership::Former | Membership::Outsider, _) => Capabilities::empty(),
        }
    }

    /// Whether the viewer sees `channel` in the rule editor: the owner sees
    /// every one, anyone else those they can see in Discord's sense
    /// (`VIEW_PRESENCE`).
    pub fn sees_channel(&self, channel: i64) -> bool {
        self.membership == Membership::Owner || self.allows(channel, Capabilities::VIEW_PRESENCE)
    }

    /// Whether the viewer may hear a recording of `speakers`' voices from
    /// `channels`: a recording, a clip, a stamp's moment, one track of a
    /// mix.
    ///
    /// 1. One of only the viewer's own voice is theirs, wherever it was
    ///    recorded: no channel needs to be open to them.
    /// 2. Anyone else's voice takes `LISTEN_OTHERS` in every one of
    ///    `channels` (with no channels, no one may hear it), and none of the
    ///    other speakers may keep their recordings private. No capability
    ///    outweighs that consent, the owner's included.
    ///
    /// `speakers` empty means not known, which is never the viewer's own,
    /// and whose consent cannot be asked: the channels alone decide.
    /// A former member hears only their own voice, an outsider nothing.
    pub fn allows_item(&self, speakers: &[i64], channels: impl IntoIterator<Item = i64>) -> bool {
        if self.membership == Membership::Outsider {
            return false;
        }
        self.is_own(speakers) || (self.hears_in(channels) && !self.keeps_private(speakers))
    }

    /// Whether the viewer sees a recording they may not hear
    /// (`allows_item`) as a redacted entry, which tells who spoke, where
    /// and when, and offers to remove it: when they hold
    /// `DELETE_RECORDINGS`, and only a speaker's consent keeps it from
    /// them. A role's access never does: a preview shows what the role
    /// could hear.
    pub fn redacts_item(&self, speakers: &[i64], channels: impl IntoIterator<Item = i64>) -> bool {
        self.user_id.is_some()
            && self.guild.contains(Capabilities::DELETE_RECORDINGS)
            && !self.is_own(speakers)
            && self.keeps_private(speakers)
            && self.hears_in(channels)
    }

    /// Whether `speakers` is the viewer alone.
    fn is_own(&self, speakers: &[i64]) -> bool {
        self.user_id.is_some_and(|user_id| {
            !speakers.is_empty() && speakers.iter().all(|&speaker| speaker == user_id)
        })
    }

    /// Whether the viewer may listen to others in every one of `channels`,
    /// of which there is at least one.
    fn hears_in(&self, channels: impl IntoIterator<Item = i64>) -> bool {
        let mut channels = channels.into_iter().peekable();
        channels.peek().is_some()
            && channels.all(|channel| self.allows(channel, Capabilities::LISTEN_OTHERS))
    }

    /// Whether one of `speakers` keeps their voice from the viewer.
    fn keeps_private(&self, speakers: &[i64]) -> bool {
        speakers
            .iter()
            .any(|speaker| self.private.contains(speaker))
    }

    /// Only what both `self` and `limit` hold.
    fn clipped_to(&self, limit: &Self) -> Self {
        Self {
            membership: self.membership,
            user_id: self.user_id,
            guild: self.guild & limit.guild,
            channels: self
                .channels
                .iter()
                .map(|(&channel, &held)| {
                    let allowed = limit.channels.get(&channel).copied().unwrap_or_default();
                    (channel, held & allowed)
                })
                .collect(),
            private: self.private.union(&limit.private).copied().collect(),
        }
    }
}

/// What a manager sees when viewing the guild as one of its roles.
///
/// A preview never shows more than the manager's own view: previewing needs
/// only management capabilities, and in Discord only ADMINISTRATOR bypasses
/// channel overwrites, so a manager must not learn about channels they could
/// not use themselves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RolePreview {
    /// The channels the manager can listen in. The preview lists all of
    /// them, each annotated with the role's access there.
    pub listed: BTreeSet<i64>,
    /// The role's access, clipped to the manager's.
    pub access: Access,
}

impl RolePreview {
    pub fn new(manager: &Access, role: &Access) -> Self {
        Self {
            listed: manager.channels_with(Capabilities::LISTEN_OTHERS).collect(),
            access: role.clipped_to(manager),
        }
    }
}

impl GuildFacts {
    /// Whether the viewer belongs to the guild.
    ///
    /// - A dev login follows its seeding: the owner, or a seeded owner flag,
    ///   makes it an owner, a seeded entry a member.
    /// - Once the bot has left the guild, nobody does: its cached roles and
    ///   overwrites stop updating.
    /// - Otherwise the owner does, and the bot's member list decides for
    ///   everyone else. A list that was complete once stays authoritative,
    ///   even while the bot is away; one that never was leaves membership
    ///   unknown.
    ///
    /// Someone who does not belong is a former member while recordings of
    /// their own remain here, and an outsider otherwise.
    pub fn membership(&self, viewer: Viewer) -> Result<Membership, MembershipUnavailable> {
        let member = self.members.get(&viewer.user_id);
        if viewer.dev {
            return Ok(if self.owner_id == Some(viewer.user_id) {
                Membership::Owner
            } else {
                match self.seeds.get(&viewer.user_id) {
                    Some(true) => Membership::Owner,
                    Some(false) => Membership::Member,
                    None => Membership::Outsider,
                }
            });
        }
        let belongs = if !self.bot_present {
            None
        } else if self.owner_id == Some(viewer.user_id) {
            Some(Membership::Owner)
        } else {
            match self.roster {
                Roster::Complete if member.is_some_and(|member| member.listed) => {
                    Some(Membership::Member)
                }
                Roster::Complete => None,
                Roster::Incomplete => return Err(MembershipUnavailable),
            }
        };
        Ok(
            belongs.unwrap_or(if member.is_some_and(|member| member.recorded) {
                Membership::Former
            } else {
                Membership::Outsider
            }),
        )
    }

    /// The viewer's Discord permissions: everything for an owner, nothing
    /// for a former member or an outsider.
    pub fn discord_permissions(
        &self,
        viewer: Viewer,
    ) -> Result<DiscordPermissions, MembershipUnavailable> {
        Ok(self.permissions_as(self.membership(viewer)?, viewer.user_id))
    }

    /// What the viewer may do: Discord's permissions, then the Sakiot
    /// rules for their roles and themselves, then the guards no rule can
    /// break (the owner keeps every management capability; nothing is held
    /// in a channel without seeing it; a former member or an outsider holds
    /// nothing, whatever the rules say).
    pub fn access(&self, viewer: Viewer) -> Result<Access, MembershipUnavailable> {
        let membership = self.membership(viewer)?;
        Ok(self.evaluate(
            membership,
            Some(viewer.user_id),
            self.held_roles(viewer.user_id),
            &self.permissions_as(membership, viewer.user_id),
        ))
    }

    /// The roles whose rules apply to `user_id`.
    pub(crate) fn held_roles(&self, user_id: i64) -> &[i64] {
        self.members
            .get(&user_id)
            .map_or(&[][..], |member| member.roles.as_slice())
    }

    /// The roles whose rules apply to a member whose only role is
    /// `role_id`: none beyond `@everyone`'s for `@everyone` itself.
    pub(crate) fn preview_roles(&self, role_id: i64) -> Vec<i64> {
        if role_id == self.guild_id {
            Vec::new()
        } else {
            vec![role_id]
        }
    }

    fn evaluate(
        &self,
        membership: Membership,
        user_id: Option<i64>,
        roles: &[i64],
        discord: &DiscordPermissions,
    ) -> Access {
        if matches!(membership, Membership::Former | Membership::Outsider) {
            return Access::none(membership, user_id);
        }
        let guild_wide = self
            .rules
            .guild
            .layers(roles, user_id)
            .map(|layer| layer.rule());
        let mut guild = guild_wide.iter().fold(
            Capabilities::from_guild_permissions(discord.base),
            |held, rule| rule.within(Capabilities::MANAGEMENT).apply(held),
        );
        if membership == Membership::Owner {
            guild |= Capabilities::MANAGEMENT;
        }
        let channels = discord
            .channels
            .iter()
            .map(|(&channel, &permissions)| {
                let held = guild_wide.iter().fold(
                    Capabilities::from_channel_permissions(permissions),
                    |held, rule| rule.within(Capabilities::CONTENT).apply(held),
                );
                let held = self.rules.channels.get(&channel).map_or(held, |rules| {
                    rules
                        .layers(roles, user_id)
                        .iter()
                        .fold(held, |held, layer| {
                            layer.rule().within(Capabilities::CONTENT).apply(held)
                        })
                });
                (channel, held.seen())
            })
            .collect();
        Access {
            membership,
            user_id,
            guild,
            channels,
            private: self
                .private
                .iter()
                .copied()
                .filter(|&speaker| Some(speaker) != user_id)
                .collect(),
        }
    }

    pub(crate) fn permissions_as(
        &self,
        membership: Membership,
        user_id: i64,
    ) -> DiscordPermissions {
        match membership {
            Membership::Owner => self.permissions_in_channels(Permissions::all(), &[], None),
            Membership::Member => self.member_permissions(user_id),
            Membership::Former | Membership::Outsider => DiscordPermissions {
                base: Permissions::empty(),
                channels: BTreeMap::new(),
            },
        }
    }

    /// Discord's permissions for a member of the guild, as Discord computes
    /// them: `@everyone` and the member's roles at guild level, then in each
    /// channel the `@everyone` overwrite, the member's role overwrites
    /// together, and the member's own overwrite. ADMINISTRATOR bypasses the
    /// overwrites. Whether the user is a member is the caller's question.
    pub fn member_permissions(&self, user_id: i64) -> DiscordPermissions {
        if self.owner_id == Some(user_id) {
            return self.permissions_in_channels(Permissions::all(), &[], None);
        }
        let roles = self
            .members
            .get(&user_id)
            .map_or(&[][..], |member| member.roles.as_slice());
        let base = roles
            .iter()
            .filter_map(|role| self.roles.get(role))
            .fold(self.everyone.unwrap_or_default(), |base, role| base | *role);
        self.permissions_in_channels(base, roles, Some(user_id))
    }

    /// The permissions of a member whose only role is `role_id`
    /// (`@everyone` when it is the guild's id): Discord's "view server as
    /// role". A role that is not the guild's adds nothing.
    pub fn role_permissions(&self, role_id: i64) -> DiscordPermissions {
        let everyone = self.everyone.unwrap_or_default();
        if role_id == self.guild_id {
            return self.permissions_in_channels(everyone, &[], None);
        }
        let base = everyone
            | self
                .roles
                .get(&role_id)
                .copied()
                .unwrap_or(Permissions::empty());
        self.permissions_in_channels(base, &[role_id], None)
    }

    pub fn has_role(&self, role_id: i64) -> bool {
        if role_id == self.guild_id {
            self.everyone.is_some()
        } else {
            self.roles.contains_key(&role_id)
        }
    }

    /// The access of a member whose only role is `role_id`, Sakiot's rules
    /// for `@everyone` and that role included.
    pub fn role_access(&self, role_id: i64) -> Result<Access, RoleNotFound> {
        if !self.has_role(role_id) {
            return Err(RoleNotFound);
        }
        Ok(self.evaluate(
            Membership::Member,
            None,
            &self.preview_roles(role_id),
            &self.role_permissions(role_id),
        ))
    }

    fn permissions_in_channels(
        &self,
        base: Permissions,
        roles: &[i64],
        member: Option<i64>,
    ) -> DiscordPermissions {
        let channels = self
            .channels
            .iter()
            .map(|(&channel_id, channel)| {
                if base.contains(Permissions::ADMINISTRATOR) {
                    return (channel_id, Permissions::all());
                }
                let roles = roles
                    .iter()
                    .filter_map(|role| channel.roles.get(role))
                    .fold(Overwrite::default(), |merged, overwrite| {
                        merged.merge(*overwrite)
                    });
                let member = member
                    .and_then(|user_id| channel.members.get(&user_id))
                    .copied()
                    .unwrap_or_default();
                let permissions = member.apply(roles.apply(channel.everyone.apply(base)));
                (channel_id, permissions)
            })
            .collect();
        DiscordPermissions { base, channels }
    }
}
