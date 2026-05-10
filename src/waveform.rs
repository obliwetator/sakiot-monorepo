use actix_web::web;
use std::error::Error;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use uuid::Uuid;

use crate::audio::WaveformProgressContainer;

/// Generates an audiowaveform track.dat file for a given audio file with a specific target number of points (pixels).
/// Updates progress in a shared HashMap container.
pub async fn generate_peaks_background(
    input_file: String,
    output_file: String,
    file_name: String,
    target_points: Option<f64>,
    progress_map: web::Data<WaveformProgressContainer>,
    completed_progress: Option<i16>,
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
                    if trimmed.starts_with("Done: ") {
                        if let Some(pct_str) = trimmed.strip_prefix("Done: ") {
                            if let Some(pct) = pct_str.strip_suffix("%") {
                                if let Ok(pct_val) = pct.parse::<i16>() {
                                    progress_map
                                        .0
                                        .write()
                                        .await
                                        .insert(file_name.clone(), pct_val.min(99));
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
