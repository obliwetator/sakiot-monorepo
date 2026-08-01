use actix_web::web;
use std::error::Error;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use uuid::Uuid;

use crate::audio::WaveformProgressContainer;

const DEFAULT_TARGET_POINTS: f64 = 2500.0;
const MIN_TARGET_POINTS: f64 = 2500.0;
/// Above this the payload costs more than the extra detail is worth: 60k points
/// is ~120 KB of 8-bit min/max pairs before base64.
const MAX_TARGET_POINTS: f64 = 60_000.0;

/// How finely a waveform is sampled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PeakDensity {
    /// A fixed number of points across the file, however long it is.
    Fixed(f64),
    /// Points per second, so a long recording still resolves detail when the
    /// dashboard zooms into it. Clamped at both ends.
    PerSecond(f64),
}

impl PeakDensity {
    pub const DEFAULT: Self = Self::Fixed(DEFAULT_TARGET_POINTS);

    fn target_points(self, duration_seconds: f64) -> f64 {
        match self {
            Self::Fixed(points) => points,
            Self::PerSecond(rate) => {
                (duration_seconds * rate).clamp(MIN_TARGET_POINTS, MAX_TARGET_POINTS)
            }
        }
    }
}

/// Generates an audiowaveform track.dat file for a given audio file at the requested peak density.
/// Updates progress in a shared HashMap container.
pub async fn generate_peaks_background(
    input_file: String,
    output_file: String,
    file_name: String,
    density: PeakDensity,
    progress_map: web::Data<WaveformProgressContainer>,
    completed_progress: Option<i16>,
    progress_range: Option<(i16, i16)>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // 1. Get duration and sample rate using a single ffprobe call
    let ffprobe_output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=sample_rate",
            "-of",
            "csv=p=0",
            "-i",
            &input_file,
        ])
        .output()
        .await?;

    let output_str = String::from_utf8(ffprobe_output.stdout)?;
    let mut lines = output_str.trim().lines();

    let val1: f64 = match lines.next().and_then(|l| l.parse().ok()) {
        Some(v) => v,
        None => {
            progress_map.0.write().await.insert(file_name.clone(), -1);
            return Err("No sample rate or duration found".into());
        }
    };

    let val2: f64 = match lines.next().and_then(|l| l.parse().ok()) {
        Some(v) => v,
        None => {
            progress_map.0.write().await.insert(file_name.clone(), -1);
            return Err("Expected both sample rate and duration".into());
        }
    };

    let duration = val1;
    let sample_rate = val2;

    if duration <= 0.0 || sample_rate <= 0.0 {
        progress_map.0.write().await.insert(file_name.clone(), -1);
        return Err("Duration and Sample Rate must be strictly positive".into());
    }

    // 2. Calculate the zoom level
    let zoom = ((duration * sample_rate) / density.target_points(duration)).floor() as u64;
    let zoom_val = std::cmp::max(1, zoom).to_string();

    // 3. Generate peaks using audiowaveform with streaming output
    let temp_output_file = format!("{}.{}.tmp.dat", output_file, Uuid::new_v4());
    let mut command = Command::new("audiowaveform")
        .args([
            "-i",
            &input_file,
            "-o",
            &temp_output_file,
            "-z",
            &zoom_val,
            "-b",
            "8",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    let stderr = command
        .stderr
        .take()
        .ok_or("audiowaveform child missing stderr pipe")?;
    let mut reader = BufReader::new(stderr);
    let mut buf = Vec::new();

    loop {
        buf.clear();
        // audiowaveform uses carriage returns (\r) or newlines (\n) to update progress
        match reader.read_until(b'\r', &mut buf).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                if let Ok(line) = std::str::from_utf8(&buf) {
                    let trimmed = line.trim();
                    if let Some(pct_str) = trimmed.strip_prefix("Done: ")
                        && let Some(pct) = pct_str.strip_suffix("%")
                        && let Ok(pct_val) = pct.parse::<i16>()
                    {
                        progress_map
                            .0
                            .write()
                            .await
                            .insert(file_name.clone(), scale_progress(pct_val, progress_range));
                    }
                }
            }
            Err(_) => break, // Error reading
        }
    }

    let status = command.wait().await?;
    if !status.success() {
        let _ = tokio::fs::remove_file(&temp_output_file).await;
        progress_map.0.write().await.insert(file_name.clone(), -1);
        return Err("audiowaveform exited with non-zero status".into());
    }

    tokio::fs::rename(&temp_output_file, &output_file).await?;

    if let Some(completed_progress) = completed_progress {
        progress_map
            .0
            .write()
            .await
            .insert(file_name.clone(), completed_progress);
    } else {
        // Remove from the processing map now that the file is safely written to disk
        progress_map.0.write().await.remove(&file_name);
    }

    Ok(())
}

fn scale_progress(progress: i16, range: Option<(i16, i16)>) -> i16 {
    let progress = progress.clamp(0, 100);
    let Some((start, end)) = range else {
        return progress.min(99);
    };
    let start = start.clamp(0, 99);
    let end = end.clamp(start, 99);
    start + (progress * (end - start) / 100)
}

#[cfg(test)]
mod tests {
    use super::{PeakDensity, scale_progress};

    #[test]
    fn scales_peak_generation_into_reserved_progress_range() {
        assert_eq!(scale_progress(0, Some((85, 99))), 85);
        assert_eq!(scale_progress(50, Some((85, 99))), 92);
        assert_eq!(scale_progress(100, Some((85, 99))), 99);
        assert_eq!(scale_progress(100, None), 99);
    }

    #[test]
    fn fixed_density_ignores_duration() {
        assert_eq!(PeakDensity::DEFAULT.target_points(30.0), 2500.0);
        assert_eq!(PeakDensity::DEFAULT.target_points(22_484.0), 2500.0);
    }

    #[test]
    fn per_second_density_scales_between_its_bounds() {
        let density = PeakDensity::PerSecond(4.0);
        // A short recording keeps the old resolution.
        assert_eq!(density.target_points(60.0), 2500.0);
        assert_eq!(density.target_points(1_000.0), 4_000.0);
        // A six hour session caps out at ~0.37s per point.
        assert_eq!(density.target_points(22_484.0), 60_000.0);
    }
}
