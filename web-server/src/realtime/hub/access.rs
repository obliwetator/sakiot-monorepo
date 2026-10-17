//! What one subscription may receive, recomputed on permission events.

use std::collections::HashSet;

use sakiot_permissions::{Capabilities, Viewer};
use sqlx::{Pool, Postgres};

use crate::errors::AppError;
use crate::permissions::GuildAccess;

/// What one realtime subscription may receive: the channels whose sessions
/// its recording tree lists, the channels whose clips and stamps it lists,
/// the channels whose voice presence it shows, and whether the viewer
/// manages the guild.
///
/// For a normal view the tree and media sets are the viewer's own listening
/// channels. For a manager's role preview they follow the HTTP listings: the
/// tree lists the manager's own channels (annotated for the role), while
/// clips and stamps list the channels the role could listen in. Presence
/// follows the voice-presence endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionAccess {
    pub tree_channels: HashSet<i64>,
    pub media_channels: HashSet<i64>,
    pub presence_channels: HashSet<i64>,
    pub manager: bool,
}

impl SubscriptionAccess {
    /// The viewer's access to `guild_id`, or as `as_role` when previewing.
    pub async fn load(
        pool: &Pool<Postgres>,
        guild_id: i64,
        viewer: Viewer,
        as_role: Option<i64>,
    ) -> Result<Self, AppError> {
        let access = GuildAccess::load(pool, guild_id, viewer, as_role).await?;
        let manager = access.access()?.guild.intersects(Capabilities::MANAGEMENT);
        Ok(match access.role_preview()? {
            None => {
                let channels = access.channels(Capabilities::LISTEN_OTHERS)?;
                Self {
                    tree_channels: channels.clone(),
                    media_channels: channels,
                    presence_channels: access.channels(Capabilities::VIEW_PRESENCE)?,
                    manager,
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
                manager,
            },
        })
    }
}
