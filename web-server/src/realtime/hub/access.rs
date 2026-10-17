//! What one subscription may receive, recomputed on permission events.

use std::collections::HashSet;

use sakiot_permissions::{Capabilities, Viewer};
use sqlx::{Pool, Postgres};

use crate::errors::AppError;
use crate::permissions::GuildAccess;

/// What one realtime subscription may receive: the channels whose sessions
/// its recording tree lists, the channels whose clips and stamps it lists,
/// the channels whose voice presence it shows, and the viewer's own
/// guild-wide capabilities.
///
/// For a normal view the tree and media sets are the viewer's own listening
/// channels. For a manager's role preview they follow the HTTP listings: the
/// tree lists the manager's own channels (annotated for the role), while
/// clips and stamps list the channels the role could listen in. Presence
/// follows the voice-presence endpoint.
///
/// Everything the dashboard shows for the scope follows from these, so two
/// equal values mean the client has nothing to refetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionAccess {
    pub tree_channels: HashSet<i64>,
    pub media_channels: HashSet<i64>,
    pub presence_channels: HashSet<i64>,
    /// The viewer's own guild-wide capabilities, even while previewing a
    /// role: they decide the settings, members and deletion-job events.
    pub guild: Capabilities,
}

impl SubscriptionAccess {
    /// The viewer's access to `guild_id`, or as `as_role` when previewing.
    pub async fn load(
        pool: &Pool<Postgres>,
        guild_id: i64,
        viewer: Viewer,
        as_role: Option<i64>,
    ) -> Result<Self, AppError> {
        Self::of(&GuildAccess::load(pool, guild_id, viewer, as_role).await?)
    }

    /// What `access` lets the subscription receive; the scope is refused
    /// when it lets the viewer see nothing (not a member, membership still
    /// loading, a preview they may not make).
    pub fn of(access: &GuildAccess) -> Result<Self, AppError> {
        let guild = access.access()?.guild;
        Ok(match access.role_preview()? {
            None => {
                let channels = access.channels(Capabilities::LISTEN_OTHERS)?;
                Self {
                    tree_channels: channels.clone(),
                    media_channels: channels,
                    presence_channels: access.channels(Capabilities::VIEW_PRESENCE)?,
                    guild,
                }
            }
            Some(preview) => Self {
                media_channels: preview
                    .access
                    .channels_with(Capabilities::LISTEN_OTHERS)
                    .collect(),
                presence_channels: preview
                    .access
                    .channels_with(Capabilities::VIEW_PRESENCE)
                    .collect(),
                tree_channels: preview.listed.into_iter().collect(),
                guild,
            },
        })
    }

    pub fn holds(&self, capability: Capabilities) -> bool {
        self.guild.contains(capability)
    }

    /// Who sees the member roster and the roles: the members page
    /// (`PREVIEW_ROLES`) and the cooldowns page's member picker
    /// (`MANAGE_SETTINGS`).
    pub fn sees_members(&self) -> bool {
        self.guild
            .intersects(Capabilities::PREVIEW_ROLES | Capabilities::MANAGE_SETTINGS)
    }
}
