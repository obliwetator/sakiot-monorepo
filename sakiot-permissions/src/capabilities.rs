//! What Sakiot lets a viewer do, in its own vocabulary rather than Discord's
//! bits. Each capability has a Discord default, so until something
//! overrides it a capability means exactly what the Discord permissions
//! behind it mean.

use crate::discord::Permissions;

bitflags::bitflags! {
    /// A set of capabilities. Management capabilities apply guild-wide,
    /// content capabilities per voice or stage channel.
    ///
    /// The bit positions are meant to be stored, so never renumber one; a new
    /// capability takes the next free bit.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Capabilities: u32 {
        /// Cooldowns, recording policy and voice settings.
        const MANAGE_SETTINGS = 1 << 0;
        /// Editing Sakiot's own permission rules.
        const MANAGE_PERMISSIONS = 1 << 1;
        /// Role preview, and the roles and members admin pages.
        const PREVIEW_ROLES = 1 << 2;
        /// Deleting recordings.
        const DELETE_RECORDINGS = 1 << 3;
        /// Renaming, deleting or overwriting another user's clip.
        const MODERATE_CLIPS = 1 << 4;
        /// Seeing a voice channel and who is in it. Every other content
        /// capability takes it (`seen`).
        const VIEW_PRESENCE = 1 << 5;
        /// Hearing other members' voices: recordings, playback, waveforms,
        /// live audio, downloads and mixes.
        const LISTEN_OTHERS = 1 << 6;
        /// Clipping and composing.
        const CREATE_CLIPS = 1 << 7;
        /// Playing a clip through the bot (`/jam`).
        const JAM = 1 << 8;
        /// Stamping a moment in someone's voice.
        const STAMP = 1 << 9;

        const MANAGEMENT = Self::MANAGE_SETTINGS.bits()
            | Self::MANAGE_PERMISSIONS.bits()
            | Self::PREVIEW_ROLES.bits()
            | Self::DELETE_RECORDINGS.bits()
            | Self::MODERATE_CLIPS.bits();
        const CONTENT = Self::VIEW_PRESENCE.bits()
            | Self::LISTEN_OTHERS.bits()
            | Self::CREATE_CLIPS.bits()
            | Self::JAM.bits()
            | Self::STAMP.bits();
    }
}

impl Capabilities {
    /// As rules store them.
    pub fn stored(self) -> i64 {
        i64::from(self.bits())
    }

    /// From their stored form. Bits this build does not name (a newer
    /// build's capability) are dropped: nothing here could check them.
    pub fn from_stored(bits: i64) -> Self {
        Self::from_bits_truncate(u32::try_from(bits & i64::from(u32::MAX)).unwrap_or_default())
    }

    /// The management capabilities Discord's guild-level permissions grant:
    /// all of them with ADMINISTRATOR or MANAGE_GUILD, none otherwise.
    pub fn from_guild_permissions(permissions: Permissions) -> Self {
        if permissions.intersects(Permissions::ADMINISTRATOR | Permissions::MANAGE_GUILD) {
            Self::MANAGEMENT
        } else {
            Self::empty()
        }
    }

    /// The content capabilities a member's Discord permissions in one
    /// channel grant. VIEW_CHANNEL shows who is there and lets them stamp;
    /// with CONNECT too, they reach what was said. Without VIEW_CHANNEL,
    /// nothing (`seen`).
    pub fn from_channel_permissions(permissions: Permissions) -> Self {
        let mut capabilities = Self::VIEW_PRESENCE | Self::STAMP;
        if permissions.contains(Permissions::CONNECT) {
            capabilities |= Self::LISTEN_OTHERS | Self::CREATE_CLIPS | Self::JAM;
        }
        if permissions.contains(Permissions::VIEW_CHANNEL) {
            capabilities
        } else {
            Self::empty()
        }
    }

    /// What is held in a channel of `self`, once seeing it is settled: all
    /// of it with `VIEW_PRESENCE`, nothing without, as Discord's
    /// VIEW_CHANNEL gates everything in a channel. A rule that takes
    /// seeing a channel takes everything else there with it.
    #[must_use]
    pub fn seen(self) -> Self {
        if self.contains(Self::VIEW_PRESENCE) {
            self
        } else {
            Self::empty()
        }
    }
}
