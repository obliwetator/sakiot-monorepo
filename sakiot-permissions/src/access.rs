//! The evaluator: facts in, access out.

use std::collections::{BTreeMap, BTreeSet};

use crate::capabilities::Capabilities;
use crate::discord::{Overwrite, Permissions};
use crate::facts::{GuildFacts, Roster, Viewer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Membership {
    Owner,
    Member,
    /// Never a member, or no longer one. Gets nothing.
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
    pub membership: Membership,
    /// Guild-wide (management) capabilities.
    pub guild: Capabilities,
    /// Content capabilities per voice and stage channel. Empty for an
    /// outsider.
    pub channels: BTreeMap<i64, Capabilities>,
}

impl Access {
    fn new(membership: Membership, discord: &DiscordPermissions) -> Self {
        if membership == Membership::Outsider {
            return Self {
                membership,
                guild: Capabilities::empty(),
                channels: BTreeMap::new(),
            };
        }
        Self {
            membership,
            guild: Capabilities::from_guild_permissions(discord.base),
            channels: discord
                .channels
                .iter()
                .map(|(&channel, &permissions)| {
                    (channel, Capabilities::from_channel_permissions(permissions))
                })
                .collect(),
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

    /// Only what both `self` and `limit` hold.
    fn clipped_to(&self, limit: &Self) -> Self {
        Self {
            membership: self.membership,
            guild: self.guild & limit.guild,
            channels: self
                .channels
                .iter()
                .map(|(&channel, &held)| {
                    let allowed = limit.channels.get(&channel).copied().unwrap_or_default();
                    (channel, held & allowed)
                })
                .collect(),
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
    /// - The owner always does.
    /// - A dev login follows its seeding: a seeded owner flag makes it an
    ///   owner, a seeded entry a member.
    /// - Otherwise the bot's member list decides. A list that was complete
    ///   once stays authoritative, even while the bot is away. One that never
    ///   was is unknown while the bot is in the guild, and holds nobody once
    ///   the bot is gone.
    pub fn membership(&self, viewer: Viewer) -> Result<Membership, MembershipUnavailable> {
        if self.owner_id == Some(viewer.user_id) {
            return Ok(Membership::Owner);
        }
        if viewer.dev {
            return Ok(match self.seeds.get(&viewer.user_id) {
                Some(true) => Membership::Owner,
                Some(false) => Membership::Member,
                None => Membership::Outsider,
            });
        }
        let listed = self
            .members
            .get(&viewer.user_id)
            .is_some_and(|member| member.listed);
        match self.roster {
            Roster::Complete if listed => Ok(Membership::Member),
            Roster::Complete => Ok(Membership::Outsider),
            Roster::Incomplete if self.bot_present => Err(MembershipUnavailable),
            Roster::Incomplete => Ok(Membership::Outsider),
        }
    }

    /// The viewer's Discord permissions: everything for an owner, nothing
    /// for an outsider.
    pub fn discord_permissions(
        &self,
        viewer: Viewer,
    ) -> Result<DiscordPermissions, MembershipUnavailable> {
        Ok(self.permissions_as(self.membership(viewer)?, viewer.user_id))
    }

    pub fn access(&self, viewer: Viewer) -> Result<Access, MembershipUnavailable> {
        let membership = self.membership(viewer)?;
        Ok(Access::new(
            membership,
            &self.permissions_as(membership, viewer.user_id),
        ))
    }

    fn permissions_as(&self, membership: Membership, user_id: i64) -> DiscordPermissions {
        match membership {
            Membership::Owner => self.permissions_in_channels(Permissions::all(), &[], None),
            Membership::Member => self.member_permissions(user_id),
            Membership::Outsider => DiscordPermissions {
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

    /// The access of a member whose only role is `role_id`.
    pub fn role_access(&self, role_id: i64) -> Result<Access, RoleNotFound> {
        if !self.has_role(role_id) {
            return Err(RoleNotFound);
        }
        Ok(Access::new(
            Membership::Member,
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
