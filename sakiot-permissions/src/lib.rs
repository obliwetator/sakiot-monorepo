//! Who may do what in a guild.
//!
//! Plain facts about a guild go in ([`GuildFacts`]); what a viewer may do
//! comes out ([`Access`]), as Sakiot's own [`Capabilities`] rather than
//! Discord's bits. Nothing here does I/O: the web server loads the facts
//! from its cache tables in one query, and the bot can build them from its
//! gateway cache, so both answer the same question the same way.
//!
//! Every capability currently follows Discord's permissions, through the
//! defaults in [`Capabilities`].

mod access;
mod capabilities;
mod discord;
mod facts;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use access::{
    Access, DiscordPermissions, Membership, MembershipUnavailable, RoleNotFound, RolePreview,
};
pub use capabilities::Capabilities;
pub use discord::{Overwrite, Permissions};
pub use facts::{ChannelFacts, GuildFacts, MemberFacts, Roster, Viewer};

#[cfg(test)]
mod tests;
