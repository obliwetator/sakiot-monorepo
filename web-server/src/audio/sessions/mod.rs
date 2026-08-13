use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use actix_files::NamedFile;
use actix_web::{HttpRequest, HttpResponse, Responder, get, http::header, post, route, web};
use base64::prelude::*;
use chrono::Datelike;
use sakiot_paths::RecordingKey;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres, Row};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::media_archive::{MediaArchive, RemoteDisposition};
use crate::permissions::visible_channels_for_user;

use super::live::LiveContainer;
use super::paths::{clips_path, no_silence_recording_path, recording_path, waveform_path};
use super::types::{StartEnd, WaveformProgressContainer};

/// Waveform points per second of session audio. Four keeps a 20 second clip
/// window at ~80 points even before the cap applies.
const SESSION_PEAKS_PER_SECOND: f64 = 4.0;

mod access;
mod actions;
mod clips;
mod composition;
mod manifest;
mod mix;
mod waveforms;

pub(crate) use access::{require_recording_access, require_session_access};
pub use actions::*;
pub use clips::*;
pub use manifest::*;
pub use mix::*;
pub use waveforms::*;

use composition::*;

#[derive(Clone, Debug)]
pub(crate) struct SessionAccess {
    pub session_id: i64,
    pub guild_id: i64,
    pub user_id: i64,
    pub starting_channel_id: i64,
    pub state: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub pause_started_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct SessionSegmentDto {
    pub kind: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub channel_id: Option<String>,
    pub from_channel_id: Option<String>,
    pub to_channel_id: Option<String>,
    pub audio_file_id: Option<String>,
    pub file_name: Option<String>,
    pub segment_index: Option<i32>,
    pub reason: Option<String>,
    pub media_url: Option<String>,
    pub hls_playlist_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct SessionTimelineEventDto {
    pub source: String,
    pub event_type: String,
    pub offset_ms: i64,
    pub channel_id: Option<String>,
    pub previous_channel_id: Option<String>,
    pub details: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct SessionManifestDto {
    pub recording_session_id: String,
    pub guild_id: String,
    pub user_id: String,
    pub state: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub duration_ms: i64,
    pub starting_channel_id: String,
    pub current_channel_id: Option<String>,
    pub channel_journey: Vec<String>,
    pub segments: Vec<SessionSegmentDto>,
    pub events: Vec<SessionTimelineEventDto>,
}

#[derive(Clone, Debug)]
struct AudioFragment {
    id: i64,
    guild_id: i64,
    channel_id: i64,
    user_id: i64,
    recording_session_id: Option<i64>,
    file_name: String,
    year: i32,
    month: i32,
    start_ms: i64,
    end_ms: Option<i64>,
    segment_index: Option<i32>,
    live: bool,
}

#[derive(Clone, Debug)]
struct Gap {
    start_ms: i64,
    end_ms: i64,
    reason: String,
    from_channel_id: Option<i64>,
    to_channel_id: Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SessionDownloadQuery {
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub remove_silence: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SessionWaveformResponse {
    pub progress: i16,
    pub building: bool,
    pub data: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SilenceFreeSessionResponse {
    pub status: String,
    pub progress: i16,
}

#[derive(Debug, Deserialize)]
pub struct SilenceFreeSessionQuery {
    pub download: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SilenceRemovalQuery {
    /// Replace an existing silence-free session instead of reusing it.
    pub force: Option<bool>,
}

#[derive(Clone, Debug)]
enum PartKind {
    Audio { path: PathBuf, source_start_ms: i64 },
    Silence,
}

#[derive(Clone, Debug)]
struct TimelinePart {
    start_ms: i64,
    end_ms: i64,
    kind: PartKind,
}

struct SessionCompositionPlan {
    selected_start_ms: i64,
    selected_end_ms: i64,
    parts: Vec<TimelinePart>,
}

#[cfg(test)]
mod tests;
