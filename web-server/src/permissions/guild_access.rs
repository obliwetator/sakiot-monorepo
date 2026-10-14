//! A viewer's access to one guild, loaded once and asked everything.

use std::collections::HashSet;

use sakiot_permissions::{
    Access, Capabilities, GuildFacts, Membership, MembershipUnavailable, Permissions, RoleNotFound,
    RolePreview,
};

use super::{Viewer, facts};
use crate::errors::AppError;

pub struct GuildAccess {
    facts: GuildFacts,
    viewer: sakiot_permissions::Viewer,
    /// The previewed role, for a manager's view-as-role.
    as_role: Option<i64>,
    access: Result<Access, MembershipUnavailable>,
}

impl GuildAccess {
    /// Loads the viewer's access to `guild_id` in one statement, with
    /// `as_role` for a role preview. Unknown membership is not an error
    /// yet: each question below reports it.
    pub async fn load<'e>(
        executor: impl sqlx::PgExecutor<'e>,
        guild_id: i64,
        viewer: Viewer,
        as_role: Option<i64>,
    ) -> Result<Self, AppError> {
        let viewer = sakiot_permissions::Viewer::from(viewer);
        let facts = facts::load(executor, guild_id, &[viewer.user_id], as_role.as_slice()).await?;
        let access = facts.access(viewer);
        Ok(Self {
            facts,
            viewer,
            as_role,
            access,
        })
    }

    pub fn access(&self) -> Result<&Access, AppError> {
        self.access
            .as_ref()
            .map_err(|MembershipUnavailable| AppError::MembershipUnavailable)
    }

    /// The viewer's access, which must be a member's.
    pub fn require_member(&self) -> Result<&Access, AppError> {
        let access = self.access()?;
        if access.membership == Membership::Outsider {
            return Err(AppError::Forbidden);
        }
        Ok(access)
    }

    /// Whether the viewer holds any management capability. An outsider
    /// holds none.
    pub fn manages(&self) -> Result<bool, AppError> {
        Ok(self.access()?.guild.intersects(Capabilities::MANAGEMENT))
    }

    /// The channels where the member holds `capability`.
    pub fn channels(&self, capability: Capabilities) -> Result<HashSet<i64>, AppError> {
        Ok(self.require_member()?.channels_with(capability).collect())
    }

    /// Discord's guild-level permissions: everything for the owner, nothing
    /// for an outsider.
    pub fn discord_permissions(&self) -> Result<Permissions, AppError> {
        self.facts
            .discord_permissions(self.viewer)
            .map(|permissions| permissions.base)
            .map_err(|MembershipUnavailable| AppError::MembershipUnavailable)
    }

    /// The member's view as the previewed role, clipped to their own; `None`
    /// when not previewing. A role that is not the guild's is reported
    /// before membership is checked.
    pub fn role_preview(&self) -> Result<Option<RolePreview>, AppError> {
        let Some(role_id) = self.as_role else {
            return Ok(None);
        };
        let role = self
            .facts
            .role_access(role_id)
            .map_err(|RoleNotFound| AppError::RoleNotFound)?;
        Ok(Some(RolePreview::new(self.require_member()?, &role)))
    }
}
