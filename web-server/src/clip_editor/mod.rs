use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::{fs::File, io::BufWriter, io::Write};

use actix_web::{HttpRequest, HttpResponse, get, post, web};
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres, Row};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::error;

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::media_archive::MediaArchive;
use crate::permissions::require_guild_manager;

use crate::audio::clips_path;
use crate::audio::types::WaveformProgressContainer;

const MAX_SEGMENTS: usize = 200;
const MAX_TRACKS: i32 = 32;
const MAX_TOTAL_SECONDS: f64 = 3600.0;
const MIN_SEGMENT_SECONDS: f32 = 0.05;
const VOLUME_MIN: f32 = -40.0;
const VOLUME_MAX: f32 = 12.0;
const NORMALIZED_MIN: f32 = 0.0;
const NORMALIZED_MAX: f32 = 1.0;
const DELAY_MAX_SECONDS: f32 = 5.0;
const MID_FREQUENCY_HZ: u16 = 1_000;
const SAMPLE_RATE: f64 = 48_000.0;
const MAX_FFMPEG_ERROR_BYTES: usize = 4096;
// The offline phase-vocoder path keeps one segment in memory. Longer source
// windows retain the existing FFmpeg/Rubber Band renderer until the shared DSP
// gains a streaming length-changing API.
const MAX_SHARED_DSP_SEGMENT_SECONDS: f32 = 60.0;
// The phase-vocoder transient is proportional to pitch_ratio/rate; beyond 16x
// a 60s segment holds ~370 MB in memory, so those renders use the FFmpeg path.
const MAX_SHARED_DSP_STRETCH: f64 = 16.0;
// Absolute safety caps the adjustable slider limits are clamped to. Above
// these the renderers either overflow f32 to INF/NaN (gain past ~±770 dB) or
// allocate hundreds of megabytes per segment (pitch beyond 16x resampling).
const LIMIT_GAIN_MAX_ABS_DB: f32 = 240.0;
const LIMIT_PITCH_MAX_ABS_CENTS: f32 = 4_800.0;
const LIMIT_RATE_MIN: f32 = 0.1;
const LIMIT_RATE_MAX: f32 = 10.0;
const OUTPUT_CHANNELS: usize = 2;

mod contract;
mod handlers;
mod jobs;
mod render;
mod repository;
mod validation;

pub use contract::{
    AdvancedSegmentEffectsDto, ComposeClipAccepted, ComposeClipBody, ComposeClipStatus,
    ComposeLimitsDto, ComposeSegment, SegmentEffectsDto,
};
pub use handlers::*;

use jobs::*;
use render::*;
use repository::*;
use validation::*;

struct ResolvedSource {
    path: PathBuf,
    channel_id: i64,
    length: f32,
}

struct ComposeOverwrite {
    clip_id: String,
    old_saved_file_name: String,
    fallback_name: Option<String>,
}

#[derive(Debug, Clone)]
struct SegmentRender {
    path: PathBuf,
    source_in: f32,
    source_out: f32,
    effects: sakiot_dsp::SegmentEffects,
    timeline_start: f32,
}

struct TemporaryRawFiles {
    paths: Vec<PathBuf>,
}

impl Drop for TemporaryRawFiles {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

struct ValidatedComposition(ComposeClipBody);

struct ResolvedComposition {
    validated: ValidatedComposition,
    sources: Vec<ResolvedSource>,
}

#[cfg(test)]
mod tests;
