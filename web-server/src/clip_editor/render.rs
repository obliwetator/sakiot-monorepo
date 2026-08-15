use super::*;

/// The shared DSP keeps one segment in memory, so it only handles windows and
/// pitch/rate stretches within its transient bounds; everything else renders
/// through the FFmpeg/Rubber Band path.
pub(super) fn shared_dsp_capable(segments: &[SegmentRender]) -> bool {
    segments.iter().all(|segment| {
        segment.source_out - segment.source_in <= MAX_SHARED_DSP_SEGMENT_SECONDS
            && pitch_factor(segment.effects.pitch_cents) / f64::from(segment.effects.rate)
                <= MAX_SHARED_DSP_STRETCH
    })
}

pub(super) async fn render_compose(
    segments: &[SegmentRender],
    master_volume_db: f32,
    output: &Path,
    expected_total_ms: i64,
    progress: &web::Data<WaveformProgressContainer>,
    cache_key: &str,
) -> Result<(), AppError> {
    if !shared_dsp_capable(segments) {
        return render_compose_legacy(
            segments,
            master_volume_db,
            output,
            expected_total_ms,
            progress,
            cache_key,
        )
        .await;
    }

    render_compose_shared(
        segments,
        master_volume_db,
        output,
        expected_total_ms,
        progress,
        cache_key,
    )
    .await
}

pub(super) async fn render_compose_shared(
    segments: &[SegmentRender],
    master_volume_db: f32,
    output: &Path,
    expected_total_ms: i64,
    progress: &web::Data<WaveformProgressContainer>,
    cache_key: &str,
) -> Result<(), AppError> {
    let raw_files = prepare_shared_dsp_segments(segments, output).await?;
    let filter_graph = build_shared_mix_graph(segments, master_volume_db);
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .arg("-y")
        .args(["-hide_banner", "-loglevel", "error"]);
    for path in &raw_files.paths {
        command
            .args(["-f", "f32le", "-ar", "48000", "-ac", "2", "-i"])
            .arg(path);
    }
    command
        .args(["-filter_complex", &filter_graph, "-map", "[out]"])
        .args(["-c:a", "libopus", "-b:a", "96k"])
        .args(["-progress", "pipe:2", "-nostats"])
        .arg(output);
    run_ffmpeg_with_progress(command, expected_total_ms, progress, cache_key).await
}

pub(super) async fn render_compose_legacy(
    segments: &[SegmentRender],
    master_volume_db: f32,
    output: &Path,
    expected_total_ms: i64,
    progress: &web::Data<WaveformProgressContainer>,
    cache_key: &str,
) -> Result<(), AppError> {
    let filter_graph = build_filter_graph(segments, master_volume_db);
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .arg("-y")
        .args(["-hide_banner", "-loglevel", "error"]);
    for segment in segments {
        command.arg("-i").arg(&segment.path);
    }
    command
        .args(["-filter_complex", &filter_graph, "-map", "[out]"])
        .args(["-c:a", "libopus", "-b:a", "96k"])
        .args(["-progress", "pipe:2", "-nostats"])
        .arg(output);
    run_ffmpeg_with_progress(command, expected_total_ms, progress, cache_key).await
}

pub(super) async fn run_ffmpeg_with_progress(
    mut command: tokio::process::Command,
    expected_total_ms: i64,
    progress: &web::Data<WaveformProgressContainer>,
    cache_key: &str,
) -> Result<(), AppError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::ServiceUnavailable(
                "ffmpeg executable is unavailable; install FFmpeg on the web server".into(),
            )
        } else {
            AppError::IoError(error)
        }
    })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::FfmpegError("FFmpeg stderr pipe unavailable".into()))?;
    let mut lines = BufReader::new(stderr).lines();
    let mut error_output = Vec::new();
    while let Some(line) = lines.next_line().await.map_err(AppError::IoError)? {
        if let Some(value) = compose_progress_percent(&line, expected_total_ms) {
            let mut values = progress.0.write().await;
            let current = values.entry(cache_key.to_owned()).or_insert(0);
            if *current >= 0 && value > *current {
                *current = value;
            }
        } else if !is_ffmpeg_progress_line(&line) && error_output.len() < MAX_FFMPEG_ERROR_BYTES {
            let remaining = MAX_FFMPEG_ERROR_BYTES - error_output.len();
            error_output.extend_from_slice(&line.as_bytes()[..line.len().min(remaining)]);
            error_output.push(b'\n');
        }
    }

    let status = child.wait().await.map_err(AppError::IoError)?;
    if !status.success() {
        return Err(AppError::FfmpegError(
            String::from_utf8_lossy(&error_output).into_owned(),
        ));
    }
    Ok(())
}

pub(super) async fn prepare_shared_dsp_segments(
    segments: &[SegmentRender],
    output: &Path,
) -> Result<TemporaryRawFiles, AppError> {
    let mut files = TemporaryRawFiles { paths: Vec::new() };
    for (index, segment) in segments.iter().enumerate() {
        let path = temporary_segment_path(output, index);
        files.paths.push(path.clone());
        let input = decode_segment_f32(segment).await?;
        let effects = shared_segment_effects(segment);
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let rendered =
                sakiot_dsp::render_clip_interleaved(&input, SAMPLE_RATE, OUTPUT_CHANNELS, effects)
                    .map_err(|error| error.to_string())?;
            let file = File::create(&path).map_err(|error| error.to_string())?;
            let mut writer = BufWriter::new(file);
            for sample in rendered {
                writer
                    .write_all(&sample.to_le_bytes())
                    .map_err(|error| error.to_string())?;
            }
            writer.flush().map_err(|error| error.to_string())
        })
        .await
        .map_err(|_| AppError::InternalError)?
        .map_err(|error| AppError::FfmpegError(format!("shared DSP failed: {error}")))?;
    }
    Ok(files)
}

pub(super) async fn decode_segment_f32(segment: &SegmentRender) -> Result<Vec<f32>, AppError> {
    let start_frame = (f64::from(segment.source_in) * SAMPLE_RATE).round() as u64;
    let end_frame = (f64::from(segment.source_out) * SAMPLE_RATE).round() as u64;
    let filter = format!(
        "aresample={SAMPLE_RATE:.0},atrim=start_sample={start_frame}:end_sample={end_frame},asetpts=PTS-STARTPTS,aformat=sample_fmts=flt:channel_layouts=stereo"
    );
    let output = tokio::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(&segment.path)
        .args([
            "-map",
            "0:a:0",
            "-af",
            &filter,
            "-ar",
            "48000",
            "-ac",
            "2",
            "-c:a",
            "pcm_f32le",
            "-f",
            "f32le",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AppError::ServiceUnavailable(
                    "ffmpeg executable is unavailable; install FFmpeg on the web server".into(),
                )
            } else {
                AppError::IoError(error)
            }
        })?;
    if !output.status.success() {
        return Err(AppError::FfmpegError(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    if !output
        .stdout
        .len()
        .is_multiple_of(size_of::<f32>() * OUTPUT_CHANNELS)
    {
        return Err(AppError::FfmpegError(
            "decoded segment returned incomplete stereo f32 frames".into(),
        ));
    }
    Ok(output
        .stdout
        .chunks_exact(size_of::<f32>())
        .map(|sample| f32::from_le_bytes(sample.try_into().unwrap_or_default()))
        .collect())
}

pub(super) fn temporary_segment_path(output: &Path, index: usize) -> PathBuf {
    let stem = output
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("compose");
    output.with_file_name(format!(".{stem}.dsp-segment-{index}.f32"))
}

pub(super) fn shared_segment_effects(segment: &SegmentRender) -> sakiot_dsp::SegmentEffects {
    segment.effects
}

pub(super) fn shared_effects_from_dto(effects: &SegmentEffectsDto) -> sakiot_dsp::SegmentEffects {
    let advanced = &effects.advanced;
    sakiot_dsp::SegmentEffects {
        volume_db: effects.volume_db,
        pitch_cents: effects.pitch_cents,
        rate: effects.rate,
        tail_seconds: advanced.tail_seconds,
        bass_db: effects.bass_db,
        mid_db: effects.mid_db,
        treble_db: effects.treble_db,
        distortion_amount: advanced.distortion_amount,
        distortion_wet: advanced.distortion_wet,
        delay_seconds: advanced.delay_seconds,
        delay_feedback: advanced.delay_feedback,
        delay_wet: advanced.delay_wet,
        compressor_enabled: advanced.compressor_enabled,
        compressor_threshold_db: advanced.compressor_threshold_db,
        compressor_knee_db: advanced.compressor_knee_db,
        compressor_ratio: advanced.compressor_ratio,
        compressor_attack_seconds: advanced.compressor_attack_seconds,
        compressor_release_seconds: advanced.compressor_release_seconds,
        chorus_enabled: advanced.chorus_enabled,
        chorus_frequency_hz: advanced.chorus_frequency_hz,
        chorus_delay_ms: advanced.chorus_delay_ms,
        chorus_depth: advanced.chorus_depth,
        chorus_spread_degrees: advanced.chorus_spread_degrees,
        chorus_feedback: advanced.chorus_feedback,
        chorus_wet: advanced.chorus_wet,
        reverb_enabled: advanced.reverb_enabled,
        reverb_decay_seconds: advanced.reverb_decay_seconds,
        reverb_pre_delay_seconds: advanced.reverb_pre_delay_seconds,
        reverb_wet: advanced.reverb_wet,
        reverb_seed: advanced.reverb_seed,
        reverse: effects.reverse,
    }
}

pub(super) fn build_shared_mix_graph(segments: &[SegmentRender], master_volume_db: f32) -> String {
    let mut graph = String::new();
    for (index, segment) in segments.iter().enumerate() {
        let delay_ms = (f64::from(segment.timeline_start) * 1_000.0).round() as i64;
        let mute = if segment.muted { "volume=0," } else { "" };
        graph.push_str(&format!(
            "[{index}:a]{mute}adelay={delay_ms}:all=1[s{index}];"
        ));
    }
    for index in 0..segments.len() {
        graph.push_str(&format!("[s{index}]"));
    }
    graph.push_str(&format!(
        "amix=inputs={}:duration=longest:normalize=0,aformat=sample_fmts=fltp:channel_layouts=stereo,volume={:.6}[out]",
        segments.len(),
        db_to_linear(master_volume_db),
    ));
    graph
}

pub(super) fn db_to_linear(db: f32) -> f64 {
    10f64.powf(f64::from(db) / 20.0)
}

pub(super) fn build_filter_graph(segments: &[SegmentRender], master_volume_db: f32) -> String {
    let mut graph = String::new();
    for (index, segment) in segments.iter().enumerate() {
        let tempo = effective_rate(segment);
        let pitch = pitch_factor(segment.effects.pitch_cents);
        let time_pitch = if (tempo - 1.0).abs() < f64::EPSILON && (pitch - 1.0).abs() < f64::EPSILON
        {
            String::new()
        } else {
            format!(",rubberband=tempo={tempo:.6}:pitch={pitch:.6}")
        };
        let volume = db_to_linear(segment.effects.volume_db);
        let delay_ms = (f64::from(segment.timeline_start) * 1_000.0).round() as i64;
        let mute = if segment.muted { ",volume=0" } else { "" };
        // Reversed segments trim the source window first, then flip it, so the
        // audible content is exactly the [source_in, source_out] window played
        // backwards - the same as the client's negative playbackRate preview.
        let reverse = if segment.effects.reverse {
            ",areverse"
        } else {
            ""
        };
        graph.push_str(&format!(
            "[{index}:a]atrim=start={:.6}:end={:.6}{reverse},aresample={SAMPLE_RATE:.4}{time_pitch},volume={volume:.6},bass=g={:.3}:f=250:t=s:w=1,equalizer=g={:.3}:f={MID_FREQUENCY_HZ}:t=q:w=1,treble=g={:.3}:f=3000:t=s:w=1,aresample={SAMPLE_RATE:.4},aformat=sample_fmts=fltp:channel_layouts=mono{mute},adelay={delay_ms}:all=1[s{index}];",
            segment.source_in,
            segment.source_out,
            segment.effects.bass_db,
            segment.effects.mid_db,
            segment.effects.treble_db,
        ));
    }
    for index in 0..segments.len() {
        graph.push_str(&format!("[s{index}]"));
    }
    graph.push_str(&format!(
        "amix=inputs={}:duration=longest:normalize=0,volume={:.6}[out]",
        segments.len(),
        db_to_linear(master_volume_db),
    ));
    graph
}

pub(super) fn compose_progress_percent(line: &str, total_ms: i64) -> Option<i16> {
    let elapsed_us = line.strip_prefix("out_time_us=")?.parse::<u64>().ok()?;
    let total_ms = u64::try_from(total_ms).ok()?.max(1);
    let progress = (elapsed_us / 1_000)
        .saturating_mul(99)
        .checked_div(total_ms)?
        .clamp(1, 99);
    i16::try_from(progress).ok()
}

pub(super) fn is_ffmpeg_progress_line(line: &str) -> bool {
    matches!(
        line.split_once('=').map(|(key, _)| key),
        Some(
            "bitrate"
                | "drop_frames"
                | "dup_frames"
                | "fps"
                | "frame"
                | "out_time"
                | "out_time_ms"
                | "out_time_us"
                | "progress"
                | "speed"
                | "stream_0_0_q"
                | "total_size"
        )
    )
}

pub(super) async fn probe_duration(path: &Path) -> Result<f64, AppError> {
    let probe = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AppError::ServiceUnavailable(
                    "ffprobe executable is unavailable; install FFmpeg on the web server".into(),
                )
            } else {
                AppError::IoError(error)
            }
        })?;
    if !probe.status.success() {
        return Err(AppError::FfmpegError(
            String::from_utf8_lossy(&probe.stderr).into_owned(),
        ));
    }
    String::from_utf8_lossy(&probe.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .ok_or_else(|| AppError::FfmpegError("ffprobe returned no audio duration".into()))
}
