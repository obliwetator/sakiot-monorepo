use actix_web::web;
use std::error::Error;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::WaveformProgressContainer;

use tracing::info;

/// Generates an audiowaveform track.dat file for a given audio file with a specific target number of points (pixels).
pub async fn generate_peaks(
    input_file: &str,
    output_file: &str,
    target_points: Option<f64>,
) -> Result<Vec<u8>, Box<dyn Error>> {
    // 1. Get duration and sample rate using a single ffprobe call
    // We request both format duration and stream sample_rate at the same time.
    let ffprobe_output = Command::new("ffprobe")
        .args([
            "-v",
            "error", // "error" or "quiet" is better before -i
            "-show_entries",
            "format=duration:stream=sample_rate",
            "-of",
            "csv=p=0",
            "-i",
            input_file,
        ])
        .output()
        .await?;

    info!("ffprobe output: {:?}", ffprobe_output);

    let output_str = String::from_utf8(ffprobe_output.stdout)?;

    // The output will typically be two lines:
    // 44100
    // 123.456000
    // We parse them line by line.
    let mut lines = output_str.trim().lines();

    // Parse the first available line (usually sample rate)
    let val1: f64 = lines
        .next()
        .ok_or("No sample rate or duration found")?
        .parse()?;

    // Parse the second available line (usually duration)
    let val2: f64 = lines
        .next()
        .ok_or("Expected both sample rate and duration")?
        .parse()?;

    // Usually, ffprobe outputs sample_rate (stream property) first, then duration (format property).
    // However, depending on stream orders, we can safely assume the one that is much larger
    // and an integer-like value (e.g., 44100, 48000) is the sample rate.
    // To be perfectly safe, we simply need the product of both for our math, so their exact order doesn't actually matter!
    // sample_rate * duration == duration * sample_rate
    let duration = val1;
    let sample_rate = val2;

    if duration <= 0.0 || sample_rate <= 0.0 {
        return Err("Duration and Sample Rate must be strictly positive".into());
    }

    // 2. Calculate the zoom level
    // Zoom = (duration * sample_rate) / target_points
    let zoom = ((duration * sample_rate) / target_points.unwrap_or(2500.0)).floor() as u64;

    // Ensure zoom is at least 1
    let zoom_val = std::cmp::max(1, zoom).to_string();

    info!("Calculated Zoom Level: {}", zoom_val);

    // 3. Generate peaks using audiowaveform
    // audiowaveform -i input_file -o output_file -z zoom -b 8
    let status = Command::new("audiowaveform")
        .args([
            "-i",
            input_file,
            "-o",
            output_file,
            "-z",
            &zoom_val,
            "-b",
            "8", // 8-bit output for smaller file size
        ])
        .status()
        .await?;

    if !status.success() {
        return Err("audiowaveform exited with non-zero status".into());
    }

    let file_content = tokio::fs::read(output_file).await?;

    Ok(file_content)
}

/// Generates an audiowaveform track.dat file for a given audio file with a specific target number of points (pixels).
/// Updates progress in a shared HashMap container.
pub async fn generate_peaks_background(
    input_file: String,
    output_file: String,
    file_name: String,
    target_points: Option<f64>,
    progress_map: web::Data<WaveformProgressContainer>,
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
    let zoom = ((duration * sample_rate) / target_points.unwrap_or(2500.0)).floor() as u64;
    let zoom_val = std::cmp::max(1, zoom).to_string();

    // 3. Generate peaks using audiowaveform with streaming output
    let mut command = Command::new("audiowaveform")
        .args([
            "-i",
            &input_file,
            "-o",
            &output_file,
            "-z",
            &zoom_val,
            "-b",
            "8",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    let stderr = command.stderr.take().unwrap();
    let mut reader = BufReader::new(stderr);
    let mut buf = Vec::new();

    loop {
        buf.clear();
        // audiowaveform uses carriage returns (\r) or newlines (\n) to update progress
        match reader.read_until(b'\r', &mut buf).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                info!("audiowaveform output: {}", String::from_utf8_lossy(&buf));
                if let Ok(line) = std::str::from_utf8(&buf) {
                    let trimmed = line.trim();
                    if trimmed.starts_with("Done: ") {
                        if let Some(pct_str) = trimmed.strip_prefix("Done: ") {
                            if let Some(pct) = pct_str.strip_suffix("%") {
                                if let Ok(pct_val) = pct.parse::<i16>() {
                                    progress_map
                                        .0
                                        .write()
                                        .await
                                        .insert(file_name.clone(), pct_val);
                                }
                            }
                        }
                    }
                }
            }
            Err(_) => break, // Error reading
        }
    }

    let status = command.wait().await?;
    if !status.success() {
        progress_map.0.write().await.insert(file_name.clone(), -1);
        return Err("audiowaveform exited with non-zero status".into());
    }

    info!("audiowaveform completed successfully.");

    // Remove from the processing map now that the file is safely written to disk
    progress_map.0.write().await.remove(&file_name);

    Ok(())
}
