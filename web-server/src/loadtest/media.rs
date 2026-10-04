//! Synthetic audio. A few speech-like Opus files are generated once per data
//! directory and every synthetic recording and clip is a hard link to one of
//! them, so seeding hundreds of hours of history costs seconds and no disk.
//!
//! The sources imitate what the recorder writes: 48 kHz stereo Opus in 20 ms
//! frames and ~500 ms Ogg pages, with bursts of "speech" (band-limited pink
//! noise) separated by digital silence, so silence detection, waveforms and
//! mixes have realistic work to do.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::errors::AppError;
use crate::ffmpeg::{parse_probe_duration, run_ffmpeg, run_ffprobe};

/// Distinct talk patterns; recordings pick one at random.
pub(super) const VARIANTS: u8 = 3;
/// Every recording length a fixture can use, in minutes. Historical
/// fragments use these exact files, so their database duration matches the
/// audio. The longest also bounds a live recording.
pub(super) const DURATIONS_MINUTES: [u32; 6] = [5, 15, 30, 60, 90, 120];
const BASE_SECONDS: u32 = 600;
const CLIP_SECONDS: u32 = 15;
const MANIFEST: &str = "manifest.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Source {
    pub variant: u8,
    pub minutes: u32,
    pub path: PathBuf,
    pub duration_ms: i64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Sources {
    pub recordings: Vec<Source>,
    pub clips: Vec<Source>,
}

impl Sources {
    pub fn recording(&self, variant: u8, minutes: u32) -> Option<&Source> {
        self.recordings
            .iter()
            .find(|source| source.variant == variant && source.minutes == minutes)
    }

    pub fn longest(&self, variant: u8) -> Option<&Source> {
        self.recordings
            .iter()
            .filter(|source| source.variant == variant)
            .max_by_key(|source| source.duration_ms)
    }

    pub fn clip(&self, variant: u8) -> Option<&Source> {
        self.clips.iter().find(|source| source.variant == variant)
    }
}

pub(super) fn sources_dir() -> PathBuf {
    sakiot_paths::DataRoots::from_env()
        .base
        .join(".loadtest")
        .join("sources")
}

/// Load the generated sources, generating whatever is missing first.
pub(super) async fn ensure_sources() -> Result<Sources, AppError> {
    let dir = sources_dir();
    let manifest = dir.join(MANIFEST);
    if let Ok(bytes) = tokio::fs::read(&manifest).await
        && let Ok(sources) = serde_json::from_slice::<Sources>(&bytes)
        && sources
            .recordings
            .iter()
            .chain(&sources.clips)
            .all(|s| s.path.exists())
    {
        return Ok(sources);
    }
    tokio::fs::create_dir_all(&dir).await?;

    // The variants are independent; generate them side by side.
    let mut tasks = tokio::task::JoinSet::new();
    for variant in 0..VARIANTS {
        let dir = dir.clone();
        tasks.spawn(async move { generate_variant(&dir, variant).await });
    }
    let mut sources = Sources {
        recordings: Vec::new(),
        clips: Vec::new(),
    };
    while let Some(joined) = tasks.join_next().await {
        let (recordings, clip) = joined.map_err(|_| AppError::InternalError)??;
        sources.recordings.extend(recordings);
        sources.clips.push(clip);
    }
    sources
        .recordings
        .sort_by_key(|source| (source.variant, source.minutes));
    sources.clips.sort_by_key(|source| source.variant);
    let json = serde_json::to_vec_pretty(&sources).map_err(|_| AppError::InternalError)?;
    tokio::fs::write(&manifest, json).await?;
    tracing::info!(dir = %dir.display(), "load-test audio sources ready");
    Ok(sources)
}

async fn generate_variant(dir: &Path, variant: u8) -> Result<(Vec<Source>, Source), AppError> {
    let base = dir.join(format!("base-{variant}.ogg"));
    if !base.exists() {
        encode_base(&base, variant).await?;
    }

    let list = dir.join(format!("base-{variant}.concat"));
    let repeats = DURATIONS_MINUTES.iter().max().copied().unwrap_or(1) * 60 / BASE_SECONDS + 1;
    let entries: String = (0..repeats)
        .map(|_| format!("file '{}'\n", base.display()))
        .collect();
    tokio::fs::write(&list, entries).await?;

    let mut recordings = Vec::new();
    for minutes in DURATIONS_MINUTES {
        let path = dir.join(format!("rec-{variant}-{minutes}m.ogg"));
        if !path.exists() {
            produce(&path, |out| {
                let mut command = ffmpeg();
                command
                    .args(["-f", "concat", "-safe", "0", "-i"])
                    .arg(&list)
                    .args(["-t", &(minutes * 60).to_string(), "-c", "copy"])
                    .arg(out);
                command
            })
            .await?;
        }
        recordings.push(describe(path, variant, minutes).await?);
    }

    let clip_path = dir.join(format!("clip-{variant}.ogg"));
    if !clip_path.exists() {
        produce(&clip_path, |out| {
            let mut command = ffmpeg();
            command
                .args(["-ss", "60", "-i"])
                .arg(&base)
                .args(["-t", &CLIP_SECONDS.to_string(), "-c", "copy"])
                .arg(out);
            command
        })
        .await?;
    }
    let clip = describe(clip_path, variant, 0).await?;
    Ok((recordings, clip))
}

/// Ten minutes of speech-like audio. Two sine waves with incommensurate
/// periods gate the noise, so talk spurts and pauses vary in length.
async fn encode_base(path: &Path, variant: u8) -> Result<(), AppError> {
    let (slow, fast, phase) = match variant {
        0 => (23.0, 7.0, 1.3),
        1 => (31.0, 5.0, 0.4),
        _ => (17.0, 11.0, 2.2),
    };
    let source = format!(
        "anoisesrc=color=pink:amplitude=0.3:seed={seed}:sample_rate=48000,\
         lowpass=f=3400,highpass=f=120,\
         volume=volume='gt(sin(2*PI*t/{slow})+0.6*sin(2*PI*t/{fast}+{phase})\\,0.35)':eval=frame",
        seed = u32::from(variant) * 7919 + 7,
    );
    produce(path, |out| {
        let mut command = ffmpeg();
        command
            .args(["-f", "lavfi", "-i", &source])
            .args(["-t", &BASE_SECONDS.to_string()])
            .args(["-ac", "2", "-ar", "48000"])
            .args(["-c:a", "libopus", "-b:a", "64k", "-vbr", "on"])
            .args(["-frame_duration", "20", "-application", "voip"])
            .args(["-page_duration", "500000"])
            .arg(out);
        command
    })
    .await
}

fn ffmpeg() -> Command {
    let mut command = Command::new("ffmpeg");
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    command
}

/// Write `path` through a temporary name, so an interrupted run never leaves
/// a truncated source behind for the next one to trust.
async fn produce(path: &Path, build: impl FnOnce(&Path) -> Command) -> Result<(), AppError> {
    let temp = path.with_extension("partial.ogg");
    run_ffmpeg(build(&temp)).await?;
    tokio::fs::rename(&temp, path).await?;
    Ok(())
}

async fn describe(path: PathBuf, variant: u8, minutes: u32) -> Result<Source, AppError> {
    let output = run_ffprobe(&path).await?;
    let duration = parse_probe_duration(&output.stdout)
        .ok_or_else(|| AppError::MediaInspectionFailed(path.display().to_string()))?;
    let bytes = tokio::fs::metadata(&path).await?.len();
    Ok(Source {
        variant,
        minutes,
        path,
        duration_ms: (duration * 1000.0).round() as i64,
        bytes,
    })
}

/// Make `dest` a hard link to `source`, falling back to a copy across
/// filesystems. An existing `dest` is left alone.
pub(super) fn link(source: &Path, dest: &Path) -> std::io::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::hard_link(source, dest) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(_) => std::fs::copy(source, dest).map(|_| ()),
    }
}
