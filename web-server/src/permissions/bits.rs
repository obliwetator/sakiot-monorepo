// Discord's permission bit layout, vendored from serenity's `Permissions`.
//
// `web-server` deliberately does not depend on serenity: the agent owns the
// gateway, and pulling a Discord client into the HTTP service to reuse one
// bitflags type would drag its whole dependency tree behind it. The bits are
// fixed by Discord's API, so a local copy cannot drift the way a wrapper
// around a moving dependency would.
//
// The flag documentation below describes Discord's own model. Types named in
// it (`Member`, `Guild`, `PermissionOverwrite`, ...) are Discord concepts, not
// items in this crate - this crate stores the bits as `i64` and reads them
// back out of `roles` and the channel overwrite tables.
bitflags::bitflags! {
    /// Permission bits as Discord defines them, stored on roles and channel
    /// overwrites and combined by the helpers below.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Permissions: i64 {
        /// Allows for the creation of `RichInvite`s.
        const CREATE_INSTANT_INVITE = 1 << 0;
        /// Allows for the kicking of guild members.
        const KICK_MEMBERS = 1 << 1;
        /// Allows the banning of guild members.
        const BAN_MEMBERS = 1 << 2;
        /// Allows all permissions, bypassing channel permission overwrites.
        const ADMINISTRATOR = 1 << 3;
        /// Allows management and editing of guild channels.
        const MANAGE_CHANNELS = 1 << 4;
        /// Allows management and editing of the guild.
        const MANAGE_GUILD = 1 << 5;
        /// `Member`s with this permission can add new `Reaction`s to a
        /// `Message`. Members can still react using reactions already added
        /// to messages without this permission.
        const ADD_REACTIONS = 1 << 6;
        /// Allows viewing a guild's audit logs.
        const VIEW_AUDIT_LOG = 1 << 7;
        /// Allows the use of priority speaking in voice channels.
        const PRIORITY_SPEAKER = 1 << 8;
        /// Allows the user to go live.
        const STREAM = 1 << 9;
        /// Allows guild members to view a channel, which includes reading
        /// messages in text channels and joining voice channels.
        const VIEW_CHANNEL = 1 << 10;
        /// Allows sending messages in a guild channel.
        const SEND_MESSAGES = 1 << 11;
        /// Allows the sending of text-to-speech messages in a channel.
        const SEND_TTS_MESSAGES = 1 << 12;
        /// Allows the deleting of other messages in a guild channel.
        ///
        /// **Note**: This does not allow the editing of other messages.
        const MANAGE_MESSAGES = 1 << 13;
        /// Allows links from this user - or users of this role - to be
        /// embedded, with potential data such as a thumbnail, description, and
        /// page name.
        const EMBED_LINKS = 1 << 14;
        /// Allows uploading of files.
        const ATTACH_FILES = 1 << 15;
        /// Allows the reading of a channel's message history.
        const READ_MESSAGE_HISTORY = 1 << 16;
        /// Allows the usage of the `@everyone` mention, which will notify all
        /// users in a channel. The `@here` mention will also be available, and
        /// can be used to mention all non-offline users.
        ///
        /// **Note**: You probably want this to be disabled for most roles and
        /// users.
        const MENTION_EVERYONE = 1 << 17;
        /// Allows the usage of custom emojis from other guilds.
        ///
        /// This does not dictate whether custom emojis in this guild can be
        /// used in other guilds.
        const USE_EXTERNAL_EMOJIS = 1 << 18;
        /// Allows for viewing guild insights.
        const VIEW_GUILD_INSIGHTS = 1 << 19;
        /// Allows the joining of a voice channel.
        const CONNECT = 1 << 20;
        /// Allows the user to speak in a voice channel.
        const SPEAK = 1 << 21;
        /// Allows the muting of members in a voice channel.
        const MUTE_MEMBERS = 1 << 22;
        /// Allows the deafening of members in a voice channel.
        const DEAFEN_MEMBERS = 1 << 23;
        /// Allows the moving of members from one voice channel to another.
        const MOVE_MEMBERS = 1 << 24;
        /// Allows the usage of voice-activity-detection in a voice channel.
        ///
        /// If this is disabled, then `Member`s must use push-to-talk.
        const USE_VAD = 1 << 25;
        /// Allows members to change their own nickname in the guild.
        const CHANGE_NICKNAME = 1 << 26;
        /// Allows members to change other members' nicknames.
        const MANAGE_NICKNAMES = 1 << 27;
        /// Allows management and editing of roles below their own.
        const MANAGE_ROLES = 1 << 28;
        /// Allows management of webhooks.
        const MANAGE_WEBHOOKS = 1 << 29;
        /// Allows management of emojis and stickers created without the use of an
        /// `Integration`.
        const MANAGE_EMOJIS_AND_STICKERS = 1 << 30;
        /// Allows using slash commands.
        const USE_SLASH_COMMANDS = 1 << 31;
        /// Allows for requesting to speak in stage channels.
        const REQUEST_TO_SPEAK = 1 << 32;
        /// Allows for creating, editing, and deleting scheduled events
        const MANAGE_EVENTS = 1 << 33;
        /// Allows for deleting and archiving threads, and viewing all private threads.
        const MANAGE_THREADS = 1 << 34;
        /// Allows for creating threads.
        const CREATE_PUBLIC_THREADS = 1 << 35;
        /// Allows for creating private threads.
        const CREATE_PRIVATE_THREADS = 1 << 36;
        /// Allows the usage of custom stickers from other servers.
        const USE_EXTERNAL_STICKERS = 1 << 37;
        /// Allows for sending messages in threads
        const SEND_MESSAGES_IN_THREADS = 1 << 38;
        /// Allows for launching activities in a voice channel
        const USE_EMBEDDED_ACTIVITIES = 1 << 39;
        /// Allows for timing out users to prevent them from sending or reacting to messages in
        /// chat and threads, and from speaking in voice and stage channels.
        const MODERATE_MEMBERS = 1 << 40;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PermissionOverwriteBits {
    pub(super) allow: Permissions,
    pub(super) deny: Permissions,
}

impl Default for PermissionOverwriteBits {
    fn default() -> Self {
        Self {
            allow: Permissions::empty(),
            deny: Permissions::empty(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ChannelPermissionState {
    pub(super) channel_id: i64,
    pub(super) everyone: PermissionOverwriteBits,
    pub(super) roles: PermissionOverwriteBits,
    pub(super) member: PermissionOverwriteBits,
}

impl ChannelPermissionState {
    pub(super) fn new(channel_id: i64, everyone: PermissionOverwriteBits) -> Self {
        Self {
            channel_id,
            everyone,
            roles: PermissionOverwriteBits::default(),
            member: PermissionOverwriteBits::default(),
        }
    }

    pub(super) fn can_view(self, base_permissions: Permissions) -> bool {
        self.applied(base_permissions)
            .contains(Permissions::VIEW_CHANNEL)
    }

    pub(super) fn can_view_and_connect(self, base_permissions: Permissions) -> bool {
        self.applied(base_permissions)
            .contains(Permissions::VIEW_CHANNEL | Permissions::CONNECT)
    }

    fn applied(self, base_permissions: Permissions) -> Permissions {
        let permissions = apply_overwrite(base_permissions, self.everyone);
        let permissions = apply_overwrite(permissions, self.roles);
        apply_overwrite(permissions, self.member)
    }
}

pub(super) fn permissions_from_bits(bits: i64) -> Permissions {
    Permissions::from_bits_retain(bits)
}

fn apply_overwrite(
    mut permissions: Permissions,
    overwrite: PermissionOverwriteBits,
) -> Permissions {
    permissions.remove(overwrite.deny);
    permissions.insert(overwrite.allow);
    permissions
}
