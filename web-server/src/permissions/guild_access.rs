//! A viewer's access to one guild, loaded once per request and asked
//! everything.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sakiot_permissions::{
    Access, Capabilities, GuildFacts, Membership, MembershipUnavailable, RoleNotFound, RolePreview,
    Viewer,
};

use super::facts;
use crate::errors::AppError;

/// The facts of each of `guilds` for every one of `users` and `roles`, in
/// one statement, keyed by guild: enough to evaluate any of those users, or
/// any of them previewing one of those roles, with `GuildAccess::new`.
pub async fn load_facts<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    guilds: &[i64],
    users: &[i64],
    roles: &[i64],
) -> Result<HashMap<i64, Arc<GuildFacts>>, AppError> {
    Ok(facts::load_many(executor, guilds, users, roles)
        .await?
        .into_iter()
        .map(|facts| (facts.guild_id, Arc::new(facts)))
        .collect())
}

pub struct GuildAccess {
    facts: Arc<GuildFacts>,
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
        let facts = facts::load(executor, guild_id, &[viewer.user_id], as_role.as_slice()).await?;
        Ok(Self::new(facts, viewer, as_role))
    }

    /// The viewer's access to each of `guilds`, in one statement, keyed by
    /// guild.
    pub async fn load_many<'e>(
        executor: impl sqlx::PgExecutor<'e>,
        guilds: &[i64],
        viewer: Viewer,
    ) -> Result<HashMap<i64, Self>, AppError> {
        Ok(load_facts(executor, guilds, &[viewer.user_id], &[])
            .await?
            .into_iter()
            .map(|(guild_id, facts)| (guild_id, Self::new(facts, viewer, None)))
            .collect())
    }

    /// The viewer's access as `facts` decide it. `facts` must hold the
    /// viewer and `as_role` (`load_facts`).
    pub fn new(facts: impl Into<Arc<GuildFacts>>, viewer: Viewer, as_role: Option<i64>) -> Self {
        let facts = facts.into();
        let access = facts.access(viewer);
        Self {
            facts,
            as_role,
            access,
        }
    }

    pub fn guild_id(&self) -> i64 {
        self.facts.guild_id
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

    /// Whether the viewer holds every guild-wide capability in `capability`.
    /// An outsider holds none.
    pub fn holds(&self, capability: Capabilities) -> Result<bool, AppError> {
        Ok(self.access()?.guild.contains(capability))
    }

    /// Fails unless the viewer holds the guild-wide `capability`.
    pub fn require(&self, capability: Capabilities) -> Result<(), AppError> {
        if self.holds(capability)? {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }

    /// The channels where the member holds `capability`.
    pub fn channels(&self, capability: Capabilities) -> Result<HashSet<i64>, AppError> {
        Ok(self.require_member()?.channels_with(capability).collect())
    }

    /// Fails unless the member holds `capability` in `channel_id`.
    pub fn require_channel(
        &self,
        channel_id: i64,
        capability: Capabilities,
    ) -> Result<(), AppError> {
        if self.require_member()?.allows(channel_id, capability) {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }

    /// Fails unless the member holds `capability` in every one of
    /// `channels`.
    pub fn require_channels(
        &self,
        channels: impl IntoIterator<Item = i64>,
        capability: Capabilities,
    ) -> Result<(), AppError> {
        let access = self.require_member()?;
        if channels
            .into_iter()
            .all(|channel_id| access.allows(channel_id, capability))
        {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }

    /// Whether the viewer may hear a recording of `speakers`' voices from
    /// `channels` (`Access::allows_item`): always their own voice alone,
    /// anyone else's with `LISTEN_OTHERS` in every one of the channels.
    pub fn allows_item(
        &self,
        speakers: &[i64],
        channels: impl IntoIterator<Item = i64>,
    ) -> Result<bool, AppError> {
        Ok(self.access()?.allows_item(speakers, channels))
    }

    /// Fails unless the viewer may hear a recording of `speakers`' voices
    /// from `channels` (`allows_item`).
    pub fn require_item(
        &self,
        speakers: &[i64],
        channels: impl IntoIterator<Item = i64>,
    ) -> Result<(), AppError> {
        if self.allows_item(speakers, channels)? {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }

    /// The previewed role's view, clipped to the viewer's own; `None` when
    /// not previewing. Previewing takes `PREVIEW_ROLES`, checked before
    /// whether the role is the guild's.
    pub fn role_preview(&self) -> Result<Option<RolePreview>, AppError> {
        let Some(role_id) = self.as_role else {
            return Ok(None);
        };
        self.require(Capabilities::PREVIEW_ROLES)?;
        let role = self
            .facts
            .role_access(role_id)
            .map_err(|RoleNotFound| AppError::RoleNotFound)?;
        Ok(Some(RolePreview::new(self.require_member()?, &role)))
    }

    /// The channels a listing shows for `capability`: the viewer's own, or
    /// the previewed role's.
    pub fn listing_channels(&self, capability: Capabilities) -> Result<HashSet<i64>, AppError> {
        match self.role_preview()? {
            None => self.channels(capability),
            Some(preview) => Ok(preview.access.channels_with(capability).collect()),
        }
    }

    /// What a listing of recordings, clips or stamps shows: the viewer's
    /// own access, or the previewed role's. A query applies
    /// `Access::allows_item` with it (`Listing`).
    pub fn listing(&self) -> Result<Listing, AppError> {
        let access = match self.role_preview()? {
            None => self.require_member()?.clone(),
            Some(preview) => preview.access,
        };
        Ok(Listing {
            channels: access.channels_with(Capabilities::LISTEN_OTHERS).collect(),
            own: access.user_id,
        })
    }
}

/// `Access::allows_item` for a listing's query: an item is listed when it
/// holds only `own`'s voice, or when every one of its channels is in
/// `channels`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// Where the viewer may listen to anyone.
    pub channels: HashSet<i64>,
    /// The viewer; `None` in a role preview, since a role has no
    /// recordings of its own.
    pub own: Option<i64>,
}
