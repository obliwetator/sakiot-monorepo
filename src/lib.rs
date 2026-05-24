//! Canonical on-disk / object-store layout for sakiot voice recordings.
//!
//! One source of truth shared by FBI-agent (writer) and web_server (reader).
//! Change the scheme here and both ends stay in sync.

use std::path::PathBuf;

pub const RECORDING_ROOT: &str = "./voice_recordings";
pub const NO_SILENCE_ROOT: &str = "./no_silence_voice_recordings";
pub const WAVEFORM_ROOT: &str = "./waveform_data";
pub const CLIPS_ROOT: &str = "./clips";
pub const NO_SILENCE_PREFIX: &str = "_no_silence_";

/// Logical identity of one recording. `stem` is the DB PK (`audio_files.file_name`).
#[derive(Debug, Clone)]
pub struct RecordingKey {
    pub guild_id: i64,
    pub channel_id: i64,
    pub year: i32,
    pub month: u32,
    pub stem: String,
}

impl RecordingKey {
    pub fn new(
        guild_id: i64,
        channel_id: i64,
        year: i32,
        month: u32,
        stem: impl Into<String>,
    ) -> Self {
        Self { guild_id, channel_id, year, month, stem: stem.into() }
    }

    /// Canonical file_name PK — timestamp_ms and user_id. No username.
    pub fn stem_for(ts_ms: i64, user_id: i64) -> String {
        format!("{}-{}", ts_ms, user_id)
    }

    /// `{guild}/{channel}/{YYYY}/{MM}` — zero-padded.
    pub fn dir_suffix(&self) -> String {
        format!(
            "{}/{}/{:04}/{:02}",
            self.guild_id, self.channel_id, self.year, self.month
        )
    }

    pub fn recording_dir(&self, root: &str) -> PathBuf {
        PathBuf::from(trim(root)).join(self.dir_suffix())
    }

    pub fn recording_path(&self, root: &str) -> PathBuf {
        self.recording_dir(root).join(format!("{}.ogg", self.stem))
    }

    pub fn no_silence_path(&self, root: &str) -> PathBuf {
        self.recording_dir(root)
            .join(format!("{}{}.ogg", NO_SILENCE_PREFIX, self.stem))
    }

    pub fn waveform_path(&self, root: &str) -> PathBuf {
        PathBuf::from(trim(root)).join(format!("{}.dat", self.stem))
    }

    /// Per-recording HLS cache: `{root}/{dir_suffix}/hls-{stem}/`.
    pub fn live_dir(&self, root: &str) -> PathBuf {
        self.recording_dir(root).join(format!("hls-{}", self.stem))
    }

    pub fn live_playlist_path(&self, root: &str) -> PathBuf {
        self.live_dir(root).join("playlist.m3u8")
    }

    pub fn live_segment_path(&self, root: &str, name: &str) -> PathBuf {
        self.live_dir(root).join(name)
    }

    pub fn audio_url(&self) -> String {
        format!(
            "/api/audio/{}/{}/{:04}/{:02}/{}",
            self.guild_id, self.channel_id, self.year, self.month, self.stem
        )
    }

    pub fn waveform_url(&self) -> String {
        format!(
            "/api/audio/waveform/{}/{}/{:04}/{:02}/{}",
            self.guild_id, self.channel_id, self.year, self.month, self.stem
        )
    }
}

fn trim(s: &str) -> &str {
    s.trim_end_matches('/')
}

/// Logical identity of a recording *session* — one bot-join in a voice channel.
/// `session_ts_ms` is the wallclock millisecond the session started (earliest
/// per-user file's timestamp, also written by the bot when it joins).
#[derive(Debug, Clone)]
pub struct SessionKey {
    pub guild_id: i64,
    pub channel_id: i64,
    pub year: i32,
    pub month: u32,
    pub session_ts_ms: i64,
}

impl SessionKey {
    pub fn new(
        guild_id: i64,
        channel_id: i64,
        year: i32,
        month: u32,
        session_ts_ms: i64,
    ) -> Self {
        Self { guild_id, channel_id, year, month, session_ts_ms }
    }

    /// `{guild}/{channel}/{YYYY}/{MM}` — same as RecordingKey.
    pub fn dir_suffix(&self) -> String {
        format!(
            "{}/{}/{:04}/{:02}",
            self.guild_id, self.channel_id, self.year, self.month
        )
    }

    /// Directory holding mix cache: `{root}/{dir_suffix}/mix-{session_ts_ms}/`.
    pub fn mix_dir(&self, root: &str) -> PathBuf {
        PathBuf::from(trim(root))
            .join(self.dir_suffix())
            .join(format!("mix-{}", self.session_ts_ms))
    }

    pub fn playlist_path(&self, root: &str) -> PathBuf {
        self.mix_dir(root).join("playlist.m3u8")
    }

    pub fn init_path(&self, root: &str) -> PathBuf {
        self.mix_dir(root).join("init.mp4")
    }

    pub fn segment_path(&self, root: &str, name: &str) -> PathBuf {
        self.mix_dir(root).join(name)
    }

    /// URL prefix the frontend builds segment/playlist requests off.
    pub fn session_url_prefix(&self) -> String {
        format!(
            "/api/audio/{}/{}/{:04}/{:02}/{}",
            self.guild_id, self.channel_id, self.year, self.month, self.session_ts_ms
        )
    }

    pub fn playlist_url(&self) -> String {
        format!("{}/playlist.m3u8", self.session_url_prefix())
    }

    pub fn state_url(&self) -> String {
        format!("{}/state", self.session_url_prefix())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_suffix_zero_pads_month() {
        let k = RecordingKey::new(1, 2, 2026, 4, "s");
        assert_eq!(k.dir_suffix(), "1/2/2026/04");
    }

    #[test]
    fn stem_has_no_username() {
        assert_eq!(RecordingKey::stem_for(1700000000000, 42), "1700000000000-42");
    }

    #[test]
    fn recording_path_joins_ogg() {
        let k = RecordingKey::new(1, 2, 2026, 4, "1700000000000-42");
        assert_eq!(
            k.recording_path("./voice_recordings").to_string_lossy(),
            "./voice_recordings/1/2/2026/04/1700000000000-42.ogg"
        );
    }

    #[test]
    fn no_silence_path_has_prefix() {
        let k = RecordingKey::new(1, 2, 2026, 4, "abc");
        assert_eq!(
            k.no_silence_path("./no_silence_voice_recordings")
                .to_string_lossy(),
            "./no_silence_voice_recordings/1/2/2026/04/_no_silence_abc.ogg"
        );
    }

    #[test]
    fn audio_url_pads_month() {
        let k = RecordingKey::new(1, 2, 2026, 4, "s");
        assert_eq!(k.audio_url(), "/api/audio/1/2/2026/04/s");
    }

    #[test]
    fn live_dir_layout() {
        let k = RecordingKey::new(1, 2, 2026, 4, "1700000000000-42");
        assert_eq!(
            k.live_dir("./voice_recordings").to_string_lossy(),
            "./voice_recordings/1/2/2026/04/hls-1700000000000-42"
        );
        assert_eq!(
            k.live_playlist_path("./voice_recordings").to_string_lossy(),
            "./voice_recordings/1/2/2026/04/hls-1700000000000-42/playlist.m3u8"
        );
        assert_eq!(
            k.live_segment_path("./voice_recordings", "seg_00001.m4s")
                .to_string_lossy(),
            "./voice_recordings/1/2/2026/04/hls-1700000000000-42/seg_00001.m4s"
        );
    }

    #[test]
    fn session_mix_dir_layout() {
        let s = SessionKey::new(1, 2, 2026, 4, 1700000000000);
        assert_eq!(
            s.mix_dir("./voice_recordings").to_string_lossy(),
            "./voice_recordings/1/2/2026/04/mix-1700000000000"
        );
        assert_eq!(
            s.playlist_path("./voice_recordings").to_string_lossy(),
            "./voice_recordings/1/2/2026/04/mix-1700000000000/playlist.m3u8"
        );
        assert_eq!(
            s.init_path("./voice_recordings").to_string_lossy(),
            "./voice_recordings/1/2/2026/04/mix-1700000000000/init.mp4"
        );
        assert_eq!(
            s.segment_path("./voice_recordings", "seg_00001.m4s")
                .to_string_lossy(),
            "./voice_recordings/1/2/2026/04/mix-1700000000000/seg_00001.m4s"
        );
    }

    #[test]
    fn session_urls() {
        let s = SessionKey::new(1, 2, 2026, 4, 1700000000000);
        assert_eq!(
            s.session_url_prefix(),
            "/api/audio/1/2/2026/04/1700000000000"
        );
        assert_eq!(
            s.playlist_url(),
            "/api/audio/1/2/2026/04/1700000000000/playlist.m3u8"
        );
        assert_eq!(
            s.state_url(),
            "/api/audio/1/2/2026/04/1700000000000/state"
        );
    }
}
