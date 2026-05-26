use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::events::ogg_opus_writer::OggOpusWriter;

#[repr(i32)]
#[derive(Clone, Copy)]
pub enum VoiceEventType {
    WriterOpen = 1,
    WriterClose = 2,
    WriterError = 3,
    ZombieReaped = 4,
}

/// One per-user recording: the streaming writer plus the metadata needed to
/// finalize the audio_files row when the writer closes.
pub(in crate::events) struct UserRecording {
    pub(in crate::events) writer: OggOpusWriter<BufWriter<File>>,
    pub(in crate::events) file_name: String,
    pub(in crate::events) start_time: chrono::DateTime<chrono::Utc>,
    pub(in crate::events) user_id: u64,
    pub(in crate::events) ssrc: u32,
}

#[derive(Clone)]
pub(in crate::events) struct PausedRecording {
    pub(in crate::events) recording: Arc<Mutex<UserRecording>>,
    pub(in crate::events) ssrc: u32,
    pub(in crate::events) paused_at: chrono::DateTime<chrono::Utc>,
    pub(in crate::events) token: u64,
}
