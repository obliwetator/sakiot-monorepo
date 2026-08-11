use super::*;

pub(super) async fn compose_session(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    range_start_seconds: Option<f64>,
    range_end_seconds: Option<f64>,
    remove_silence: bool,
    output: &Path,
    media: &MediaArchive,
) -> Result<(), AppError> {
    compose_session_inner(
        pool,
        access,
        range_start_seconds,
        range_end_seconds,
        remove_silence,
        output,
        None,
        media,
    )
    .await
}

#[derive(Clone)]
pub(super) struct CompositionProgress {
    pub(super) cache_key: String,
    pub(super) progress: web::Data<WaveformProgressContainer>,
    pub(super) completed: i16,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn compose_session_with_progress(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    range_start_seconds: Option<f64>,
    range_end_seconds: Option<f64>,
    remove_silence: bool,
    output: &Path,
    progress: CompositionProgress,
    media: &MediaArchive,
) -> Result<(), AppError> {
    compose_session_inner(
        pool,
        access,
        range_start_seconds,
        range_end_seconds,
        remove_silence,
        output,
        Some(progress),
        media,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn compose_session_inner(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    range_start_seconds: Option<f64>,
    range_end_seconds: Option<f64>,
    remove_silence: bool,
    output: &Path,
    progress: Option<CompositionProgress>,
    media: &MediaArchive,
) -> Result<(), AppError> {
    let SessionCompositionPlan {
        selected_start_ms: selected_start,
        selected_end_ms: selected_end,
        parts: selected,
    } = composition_plan(pool, access, range_start_seconds, range_end_seconds, media).await?;
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .arg("-y")
        .args(["-hide_banner", "-loglevel", "error"]);
    for part in &selected {
        let duration = milliseconds_as_seconds(part.end_ms.saturating_sub(part.start_ms));
        match &part.kind {
            PartKind::Audio {
                path,
                source_start_ms,
            } => {
                command
                    .args([
                        "-ss",
                        &milliseconds_as_seconds(part.start_ms.saturating_sub(*source_start_ms)),
                        "-t",
                        &duration,
                        "-i",
                    ])
                    .arg(path);
            }
            PartKind::Silence => {
                command.args([
                    "-f",
                    "lavfi",
                    "-t",
                    &duration,
                    "-i",
                    "anullsrc=channel_layout=mono:sample_rate=48000",
                ]);
            }
        }
    }

    let mut filter = String::new();
    for index in 0..selected.len() {
        filter.push_str(&format!(
            "[{index}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=mono[a{index}];"
        ));
    }
    for index in 0..selected.len() {
        filter.push_str(&format!("[a{index}]"));
    }
    filter.push_str(&format!("concat=n={}:v=0:a=1[joined]", selected.len()));
    let track_original_timeline = remove_silence && progress.is_some();
    let output_label = if remove_silence {
        if track_original_timeline {
            filter.push_str(
                ";[joined]asplit=2[progress_audio][silence_input];\
                 [silence_input]silenceremove=stop_periods=-1:stop_duration=1:stop_threshold=-40dB[out]",
            );
        } else {
            filter.push_str(
                ";[joined]silenceremove=stop_periods=-1:stop_duration=1:stop_threshold=-40dB[out]",
            );
        }
        "[out]"
    } else {
        "[joined]"
    };
    command
        .args(["-filter_complex", &filter, "-map", output_label])
        .args(["-c:a", "libopus", "-b:a", "96k"])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    if track_original_timeline {
        // silenceremove compresses output timestamps, so its out_time_us cannot
        // be compared with the original session duration. A cheap null output
        // keeps FFmpeg's reported timestamp on the uncompressed input timeline.
        command.args(["-map", "[progress_audio]", "-f", "null", "-"]);
    }
    if progress.is_some() {
        command.args(["-progress", "pipe:2", "-nostats"]);
    }

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
        if let Some(progress) = &progress
            && let Some(value) = composition_progress_percent(
                &line,
                selected_end.saturating_sub(selected_start),
                progress.completed,
            )
        {
            let mut values = progress.progress.0.write().await;
            let current = values.entry(progress.cache_key.clone()).or_insert(0);
            if *current >= 0 && value > *current {
                *current = value;
            }
        } else if !is_ffmpeg_progress_line(&line) && error_output.len() < 4_096 {
            let remaining = 4_096 - error_output.len();
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
    if let Some(progress) = progress {
        progress
            .progress
            .0
            .write()
            .await
            .insert(progress.cache_key, progress.completed);
    }
    Ok(())
}

pub(super) fn composition_progress_percent(
    line: &str,
    duration_ms: i64,
    completed: i16,
) -> Option<i16> {
    let elapsed_us = line.strip_prefix("out_time_us=")?.parse::<u64>().ok()?;
    let duration_ms = u64::try_from(duration_ms).ok()?.max(1);
    let completed = u64::try_from(completed).ok()?.clamp(2, 99);
    let elapsed_ms = elapsed_us / 1_000;
    let progress = elapsed_ms
        .saturating_mul(completed)
        .checked_div(duration_ms)?
        .clamp(1, completed - 1);
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

pub(super) async fn composition_plan(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    range_start_seconds: Option<f64>,
    range_end_seconds: Option<f64>,
    media: &MediaArchive,
) -> Result<SessionCompositionPlan, AppError> {
    let parts = timeline_parts(pool, access, media).await?;
    let timeline_end = timeline_end_ms(access);
    let duration_ms = timeline_end.saturating_sub(access.started_at_ms);
    let start_ms = seconds_to_ms(range_start_seconds.unwrap_or(0.0))?;
    let end_ms = seconds_to_ms(range_end_seconds.unwrap_or(duration_ms as f64 / 1_000.0))?;
    if start_ms < 0 || end_ms <= start_ms || end_ms > duration_ms {
        return Err(AppError::BadRequest(
            "Invalid logical recording range".into(),
        ));
    }
    let selected_start_ms = access.started_at_ms.saturating_add(start_ms);
    let selected_end_ms = access.started_at_ms.saturating_add(end_ms.min(duration_ms));
    let parts = parts
        .into_iter()
        .filter_map(|part| {
            let start_ms = part.start_ms.max(selected_start_ms);
            let end_ms = part.end_ms.min(selected_end_ms);
            (end_ms > start_ms).then_some(TimelinePart {
                start_ms,
                end_ms,
                kind: part.kind,
            })
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return Err(AppError::BadRequest(
            "Selected range contains no timeline".into(),
        ));
    }
    Ok(SessionCompositionPlan {
        selected_start_ms,
        selected_end_ms,
        parts,
    })
}

pub(super) async fn timeline_parts(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    media: &MediaArchive,
) -> Result<Vec<TimelinePart>, AppError> {
    let timeline_end = timeline_end_ms(access);
    let mut raw = Vec::new();
    for fragment in load_fragments(pool, access.session_id).await? {
        let end_ms = fragment.end_ms.unwrap_or(timeline_end).min(timeline_end);
        if end_ms > fragment.start_ms {
            let path = fragment_path(&fragment);
            media
                .ensure_recording_local(pool.get_ref(), fragment.id, &path)
                .await?;
            raw.push(TimelinePart {
                start_ms: fragment.start_ms,
                end_ms,
                kind: PartKind::Audio {
                    path,
                    source_start_ms: fragment.start_ms,
                },
            });
        }
    }
    for gap in load_gaps(pool, access.session_id).await? {
        if gap.end_ms > gap.start_ms {
            raw.push(TimelinePart {
                start_ms: gap.start_ms,
                end_ms: gap.end_ms,
                kind: PartKind::Silence,
            });
        }
    }
    raw.sort_by_key(|part| (part.start_ms, part.end_ms));

    let mut complete = Vec::new();
    let mut cursor = access.started_at_ms;
    for part in raw {
        if part.end_ms <= cursor || part.start_ms >= timeline_end {
            continue;
        }
        if part.start_ms > cursor {
            complete.push(TimelinePart {
                start_ms: cursor,
                end_ms: part.start_ms.min(timeline_end),
                kind: PartKind::Silence,
            });
        }
        let start_ms = part.start_ms.max(cursor);
        let end_ms = part.end_ms.min(timeline_end);
        if end_ms > start_ms {
            complete.push(TimelinePart {
                start_ms,
                end_ms,
                kind: part.kind,
            });
            cursor = end_ms;
        }
    }
    if cursor < timeline_end {
        complete.push(TimelinePart {
            start_ms: cursor,
            end_ms: timeline_end,
            kind: PartKind::Silence,
        });
    }
    Ok(complete)
}

pub(super) fn seconds_to_ms(seconds: f64) -> Result<i64, AppError> {
    if !seconds.is_finite() || seconds < 0.0 || seconds > i64::MAX as f64 / 1_000.0 {
        return Err(AppError::BadRequest("Invalid range timestamp".into()));
    }
    Ok((seconds * 1_000.0).round() as i64)
}

pub(super) fn milliseconds_as_seconds(milliseconds: i64) -> String {
    format!("{:.3}", milliseconds.max(0) as f64 / 1_000.0)
}

pub(super) fn session_silence_free_path(access: &SessionAccess) -> Result<PathBuf, AppError> {
    if access.state != "finalized" {
        return Err(AppError::BadRequest(
            "Silence removal is available after the recording is finalized".into(),
        ));
    }
    let ended_at_ms = access
        .ended_at_ms
        .ok_or_else(|| AppError::BadRequest("Finalized recording has no end timestamp".into()))?;
    Ok(PathBuf::from(no_silence_recording_path())
        .join("logical_sessions")
        .join(format!(
            "{}-{}-{ended_at_ms}.ogg",
            access.session_id, access.started_at_ms
        )))
}

pub(super) fn temporary_ogg_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}.ogg", uuid::Uuid::new_v4()))
}

pub(super) fn schedule_temporary_cleanup(path: PathBuf) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10 * 60)).await;
        let _ = tokio::fs::remove_file(path).await;
    });
}
