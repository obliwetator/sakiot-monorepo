//! Why a viewer holds what they hold: the evaluator's answer, capability by
//! capability, with the layer that decided each one. For the rule editor;
//! `GuildFacts::access` is the answer itself.

use std::collections::BTreeMap;

use crate::access::{Access, DiscordPermissions, Membership, MembershipUnavailable, RoleNotFound};
use crate::capabilities::Capabilities;
use crate::facts::{GuildFacts, Viewer};
use crate::rules::{Layer, Subject};

/// Where a capability's answer came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Discord's permissions, through the capability's default.
    Discord,
    /// A Sakiot rule, guild-wide (`channel` `None`) or in one channel.
    Rule {
        channel: Option<i64>,
        subject: Subject,
    },
    /// The owner keeps every management capability, whatever the rules say.
    Owner,
    /// Not seeing the channel (`VIEW_PRESENCE`) takes everything else in
    /// it (`Capabilities::seen`).
    Unseen,
    /// A role preview leaves out what the manager previewing it does not
    /// hold themselves (`Explanation::clipped_to`).
    Manager,
    /// A content rule (or not seeing the channel) would take what Discord
    /// gives, but takes nothing from whoever holds `MANAGE_PERMISSIONS`.
    Exempt,
}

/// One capability, at one scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub capability: Capabilities,
    pub held: bool,
    /// What Discord's permissions alone would give.
    pub discord: bool,
    /// The last layer that decided it.
    pub by: Source,
}

/// Every capability of one viewer, or of one role, and what decided it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Explanation {
    pub membership: Membership,
    /// The management capabilities. Empty for a former member or an
    /// outsider, whom rules never reach.
    pub guild: Vec<Decision>,
    /// The content capabilities in each voice and stage channel.
    pub channels: BTreeMap<i64, Vec<Decision>>,
}

impl GuildFacts {
    /// `access`, explained.
    pub fn explain(&self, viewer: Viewer) -> Result<Explanation, MembershipUnavailable> {
        let membership = self.membership(viewer)?;
        Ok(self.explain_as(
            membership,
            Some(viewer.user_id),
            self.held_roles(viewer.user_id),
            &self.permissions_as(membership, viewer.user_id),
        ))
    }

    /// `role_access`, explained.
    pub fn explain_role(&self, role_id: i64) -> Result<Explanation, RoleNotFound> {
        if !self.has_role(role_id) {
            return Err(RoleNotFound);
        }
        Ok(self.explain_as(
            Membership::Member,
            None,
            &self.preview_roles(role_id),
            &self.role_permissions(role_id),
        ))
    }

    fn explain_as(
        &self,
        membership: Membership,
        user_id: Option<i64>,
        roles: &[i64],
        discord: &DiscordPermissions,
    ) -> Explanation {
        if matches!(membership, Membership::Former | Membership::Outsider) {
            return Explanation {
                membership,
                guild: Vec::new(),
                channels: BTreeMap::new(),
            };
        }
        let guild_wide: Vec<(Option<i64>, Layer)> = self
            .rules
            .guild
            .layers(roles, user_id)
            .into_iter()
            .map(|layer| (None, layer))
            .collect();
        let granted = Capabilities::from_guild_permissions(discord.base);
        let guild: Vec<Decision> = Capabilities::MANAGEMENT
            .iter()
            .map(|capability| {
                let decision = decide(capability, granted, &guild_wide);
                if membership == Membership::Owner && !decision.held {
                    Decision {
                        held: true,
                        by: Source::Owner,
                        ..decision
                    }
                } else {
                    decision
                }
            })
            .collect();
        let bound = !guild.iter().any(|decision| {
            decision.capability == Capabilities::MANAGE_PERMISSIONS && decision.held
        });
        let channels = discord
            .channels
            .iter()
            .map(|(&channel, &permissions)| {
                let mut layers = guild_wide.clone();
                if let Some(rules) = self.rules.channels.get(&channel) {
                    layers.extend(
                        rules
                            .layers(roles, user_id)
                            .into_iter()
                            .map(|layer| (Some(channel), layer)),
                    );
                }
                let granted = Capabilities::from_channel_permissions(permissions);
                let decisions: Vec<Decision> = Capabilities::CONTENT
                    .iter()
                    .map(|capability| decide(capability, granted, &layers))
                    .collect();
                // An editor keeps what Discord gives, whatever the rules take.
                let decisions: Vec<Decision> = decisions
                    .into_iter()
                    .map(|decision| {
                        if bound || decision.held || !decision.discord {
                            decision
                        } else {
                            Decision {
                                held: true,
                                by: Source::Exempt,
                                ..decision
                            }
                        }
                    })
                    .collect();
                let seen = decisions.iter().any(|decision| {
                    decision.capability == Capabilities::VIEW_PRESENCE && decision.held
                });
                let decisions = decisions
                    .into_iter()
                    .map(|decision| {
                        if seen || !decision.held {
                            decision
                        } else {
                            Decision {
                                held: false,
                                by: Source::Unseen,
                                ..decision
                            }
                        }
                    })
                    .collect();
                (channel, decisions)
            })
            .collect();
        Explanation {
            membership,
            guild,
            channels,
        }
    }
}

impl Explanation {
    /// The explanation of a role preview (`RolePreview`): what the role
    /// holds and `manager` does not is left out, by `Source::Manager`.
    #[must_use]
    pub fn clipped_to(self, manager: &Access) -> Self {
        let clip = |held: Capabilities| {
            move |decision: Decision| {
                if decision.held && !held.contains(decision.capability) {
                    Decision {
                        held: false,
                        by: Source::Manager,
                        ..decision
                    }
                } else {
                    decision
                }
            }
        };
        Self {
            membership: self.membership,
            guild: self.guild.into_iter().map(clip(manager.guild)).collect(),
            channels: self
                .channels
                .into_iter()
                .map(|(channel, decisions)| {
                    let held = manager.channels.get(&channel).copied().unwrap_or_default();
                    (channel, decisions.into_iter().map(clip(held)).collect())
                })
                .collect(),
        }
    }
}

/// `capability` as Discord grants it (`granted`), then through `layers`:
/// each layer that mentions it decides it anew.
fn decide(
    capability: Capabilities,
    granted: Capabilities,
    layers: &[(Option<i64>, Layer)],
) -> Decision {
    let discord = granted.contains(capability);
    layers.iter().fold(
        Decision {
            capability,
            held: discord,
            discord,
            by: Source::Discord,
        },
        |decision, (channel, layer)| match layer.subject(capability) {
            None => decision,
            Some(subject) => Decision {
                held: layer.rule().allow.contains(capability),
                by: Source::Rule {
                    channel: *channel,
                    subject,
                },
                ..decision
            },
        },
    )
}
