use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::{fs::File, io::BufWriter, io::Write};

use actix_web::{HttpRequest, HttpResponse, get, post, web};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres, Row};
use tokio::io::{AsyncBufReadExt, BufReader};

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
const SAMPLE_RATE: f64 = 48_000.0;
const MAX_FFMPEG_ERROR_BYTES: usize = 4096;
// Absolute safety caps the adjustable slider limits are clamped to. Above
// these the renderers either overflow f32 to INF/NaN (gain past ~±770 dB) or
// exceed the validated pitch/rate DSP parameter ranges.
const LIMIT_GAIN_MAX_ABS_DB: f32 = 240.0;
const LIMIT_PITCH_MAX_ABS_CENTS: f32 = 4_800.0;
const LIMIT_RATE_MIN: f32 = 0.1;
const LIMIT_RATE_MAX: f32 = 10.0;
const OUTPUT_CHANNELS: usize = 2;

mod contract;
mod handlers;
mod jobs;
mod queue;
mod render;
mod repository;
mod validation;
mod worker;

pub use contract::{
    AdvancedSegmentEffectsDto, ComposeClipAccepted, ComposeClipBody, ComposeClipStatus,
    ComposeLimitsDto, ComposeSegment, SegmentEffectsDto,
};
pub use handlers::*;
pub use queue::RENDERER_VERSION;
pub use worker::{run_compose_worker_command, spawn_compose_worker};

use jobs::*;
use render::*;
use repository::*;
use validation::*;

struct ResolvedSource {
    path: PathBuf,
    channel_id: i64,
    length: f32,
    saved_file_name: String,
}

#[derive(Clone, Serialize, Deserialize)]
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
    muted: bool,
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

#[cfg(test)]
mod queue_tests;
