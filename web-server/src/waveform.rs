use actix_web::web;
use std::error::Error;
use std::io;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
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

/// Generates an audiowaveform track.dat file at the requested peak density.
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
    let audio = probe_audio(&input_file).await?;
    let samples_per_point = ((audio.duration_seconds * f64::from(audio.sample_rate)
        / density.target_points(audio.duration_seconds))
    .floor() as u64)
        .clamp(1, u64::from(u32::MAX)) as u32;
    let samples_per_point = samples_per_point.to_string();
    if let Some(parent) = Path::new(&output_file).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let temp_output_file = format!("{}.{}.tmp.dat", output_file, Uuid::new_v4());

    let mut command = Command::new("audiowaveform");
    command
        .args([
            "-i",
            &input_file,
            "-o",
            &temp_output_file,
            "-z",
            &samples_per_point,
            "-b",
            "8",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("could not start audiowaveform for waveform generation: {error}"),
        )
    })?;

    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("audiowaveform progress pipe unavailable"))?;
    let diagnostic_output =
        read_audiowaveform_progress(stderr, &file_name, &progress_map, progress_range).await?;

    let status = child.wait().await?;
    if !status.success() {
        let _ = tokio::fs::remove_file(&temp_output_file).await;
        progress_map.0.write().await.insert(file_name.clone(), -1);
        return Err(io::Error::other(format!(
            "audiowaveform generation failed: {}",
            diagnostic_output.trim()
        ))
        .into());
    }
    if !tokio::fs::metadata(&temp_output_file)
        .await
        .is_ok_and(|metadata| metadata.len() > 20)
    {
        let _ = tokio::fs::remove_file(&temp_output_file).await;
        progress_map.0.write().await.insert(file_name.clone(), -1);
        return Err(io::Error::other("audiowaveform produced no waveform peaks").into());
    }

    if let Err(error) = tokio::fs::rename(&temp_output_file, &output_file).await {
        let _ = tokio::fs::remove_file(&temp_output_file).await;
        progress_map.0.write().await.insert(file_name.clone(), -1);
        return Err(error.into());
    }

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

#[derive(Clone, Copy, Debug, PartialEq)]
struct AudioInfo {
    duration_seconds: f64,
    sample_rate: u32,
}

async fn probe_audio(input_file: &str) -> Result<AudioInfo, Box<dyn Error + Send + Sync>> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=sample_rate:format=duration",
            "-of",
            "default=noprint_wrappers=1",
            "-i",
            input_file,
        ])
        .output()
        .await
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("could not start ffprobe for waveform generation: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "ffprobe failed for waveform generation: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
        .into());
    }
    parse_audio_info(&String::from_utf8(output.stdout)?)
}

fn parse_audio_info(output: &str) -> Result<AudioInfo, Box<dyn Error + Send + Sync>> {
    let mut duration_seconds = None;
    let mut sample_rate = None;
    for line in output.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        match key {
            "duration" => duration_seconds = value.parse::<f64>().ok(),
            "sample_rate" => sample_rate = value.parse::<u32>().ok(),
            _ => {}
        }
    }
    let info = AudioInfo {
        duration_seconds: duration_seconds
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| io::Error::other("ffprobe returned no valid audio duration"))?,
        sample_rate: sample_rate
            .filter(|value| *value > 0)
            .ok_or_else(|| io::Error::other("ffprobe returned no valid audio sample rate"))?,
    };
    Ok(info)
}

async fn read_audiowaveform_progress<R>(
    reader: R,
    file_name: &str,
    progress_map: &web::Data<WaveformProgressContainer>,
    progress_range: Option<(i16, i16)>,
) -> io::Result<String>
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    let mut diagnostic_output = String::new();
    loop {
        buffer.clear();
        if reader.read_until(b'\r', &mut buffer).await? == 0 {
            break;
        }
        let chunk = String::from_utf8_lossy(&buffer);
        for line in chunk.split(['\r', '\n']).map(str::trim) {
            if line.is_empty() {
                continue;
            }
            if let Some(percent) = parse_audiowaveform_progress(line) {
                progress_map.0.write().await.insert(
                    file_name.to_owned(),
                    scale_progress(percent, progress_range),
                );
            } else if diagnostic_output.len() < 4_096 {
                let remaining = 4_096 - diagnostic_output.len();
                diagnostic_output.extend(line.chars().take(remaining));
                diagnostic_output.push('\n');
            }
        }
    }
    Ok(diagnostic_output)
}

fn parse_audiowaveform_progress(line: &str) -> Option<i16> {
    line.strip_prefix("Done: ")?.strip_suffix('%')?.parse().ok()
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
    use super::{PeakDensity, parse_audio_info, parse_audiowaveform_progress, scale_progress};

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

    #[test]
    fn parses_named_ffprobe_fields_without_assuming_output_order() {
        let info = parse_audio_info("sample_rate=48000\nduration=5.019833\n").unwrap();
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(info.duration_seconds, 5.019833);
        assert!(parse_audio_info("sample_rate=48000\n").is_err());
    }

    #[test]
    fn parses_audiowaveform_carriage_return_progress() {
        assert_eq!(parse_audiowaveform_progress("Done: 0%"), Some(0));
        assert_eq!(parse_audiowaveform_progress("Done: 63%"), Some(63));
        assert_eq!(parse_audiowaveform_progress("Done: 100%"), Some(100));
        assert_eq!(parse_audiowaveform_progress("Done"), None);
    }
}
