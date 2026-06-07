#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct VoiceUserKey {
    pub guild_id: u64,
    pub user_id: u64,
}

#[derive(Clone, Copy)]
pub struct VoiceUserPresence {
    pub channel_id: u64,
    pub is_bot: bool,
    pub server_mute: bool,
    pub server_deaf: bool,
    pub self_mute: bool,
    pub self_deaf: bool,
    pub suppress: bool,
    pub streaming: bool,
    pub video: bool,
}
