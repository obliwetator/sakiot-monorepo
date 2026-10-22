//! Who may do what in a guild.
//!
//! Plain facts about a guild go in ([`GuildFacts`]); what a viewer may do
//! comes out ([`Access`]), as Sakiot's own [`Capabilities`] rather than
//! Discord's bits. Nothing here does I/O: the web server loads the facts
//! from its cache tables in one query, and the bot can build them from its
//! gateway cache, so both answer the same question the same way.
//!
//! Every capability follows Discord's permissions, through the defaults in
//! [`Capabilities`], until one of Sakiot's own [`Rules`] says otherwise.

mod access;
mod capabilities;
mod discord;
mod editing;
mod explain;
mod facts;
mod rules;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use access::{
    Access, DiscordPermissions, Membership, MembershipUnavailable, RoleNotFound, RolePreview,
};
pub use capabilities::Capabilities;
pub use discord::{Overwrite, Permissions};
pub use editing::RuleRefused;
pub use explain::{Decision, Explanation, Source};
pub use facts::{ChannelFacts, GuildFacts, MemberFacts, Roster, Viewer};
pub use rules::{Rule, RuleSet, Rules, Subject};

#[cfg(test)]
mod tests;
