//! Channel-mix job lifecycle, media materialization, and FFmpeg rendering.

use std::collections::HashSet;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use actix_web::web;
use sqlx::{Pool, Postgres};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;

use crate::errors::AppError;
use crate::media_archive::MediaArchive;

use super::super::{is_ffmpeg_progress_line, milliseconds_as_seconds};
use super::cache::{MixCacheMetadata, cache_is_valid};
use super::{MIX_FINGERPRINT, MIX_OUTPUT, MIX_SETTINGS, MixJob, MixPlan, SessionMixContainer};

pub(super) async fn start_mix_job(
    pool: &web::Data<Pool<Postgres>>,
    container: &web::Data<SessionMixContainer>,
    media: &MediaArchive,
    plan: MixPlan,
) -> Result<(), AppError> {
    let lock = container.key_lock(plan.session_id).await;
    let _guard = lock.lock().await;
    if let Some(existing) = container.job(plan.session_id, plan.scope).await {
        let job = existing.lock().await;
        if job.failed.is_none() {
            return Ok(());
        }
    }
    if cache_is_valid(&plan).await {
        return Ok(());
    }

    let job = Arc::new(Mutex::new(MixJob {
        source_fingerprint: plan.source_fingerprint.clone(),
        settings: plan.settings.clone(),
        progress: 0,
        failed: None,
    }));
    container
        .jobs
        .write()
        .await
        .insert((plan.session_id, plan.scope), job.clone());
    let pool = pool.clone();
    let container = container.clone();
    let media = media.clone();
    tokio::spawn(async move {
        let render_lock = container.key_lock(plan.session_id).await;
        let _render_guard = render_lock.lock().await;
        let result = render_mix(&pool, &media, &plan, &job).await;
        match result {
            Ok(()) => {
                container
                    .remove_if_same(plan.session_id, plan.scope, &job)
                    .await
            }
            Err(error) => {
                tracing::error!(
                    session_id = plan.session_id,
                    "channel mix render failed: {}",
                    error
                );
                let mut state = job.lock().await;
                state.progress = 0;
                state.failed = Some("Channel mix generation failed. Try again.".into());
            }
        }
    });
    Ok(())
}

async fn render_mix(
    pool: &web::Data<Pool<Postgres>>,
    media: &MediaArchive,
    plan: &MixPlan,
    job: &Arc<Mutex<MixJob>>,
) -> Result<(), AppError> {
    if plan.duration_ms <= 0 || plan.sources.is_empty() {
        return Err(AppError::BadRequest(
            "Channel mix has no renderable audio".into(),
        ));
    }
    tokio::fs::create_dir_all(&plan.cache_dir).await?;
    let mut materialized = HashSet::new();
    for source in plan.renderable_sources() {
        if materialized.insert(source.audio_file_id) {
            media
                .ensure_recording_local(pool.get_ref(), source.audio_file_id, &source.path)
                .await?;
        }
    }

    let temporary = plan
        .cache_dir
        .join(format!(".combined.{}.tmp.ogg", uuid::Uuid::new_v4()));
    let fingerprint_temporary = plan
        .cache_dir
        .join(format!(".fingerprint.{}.tmp", uuid::Uuid::new_v4()));
    let settings_temporary = plan
        .cache_dir
        .join(format!(".settings.{}.tmp", uuid::Uuid::new_v4()));
    let result = run_mix_ffmpeg(plan, job, &temporary).await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&temporary).await;
        let _ = tokio::fs::remove_file(&fingerprint_temporary).await;
        let _ = tokio::fs::remove_file(&settings_temporary).await;
        return Err(error);
    }
    if let Err(error) = tokio::fs::write(&fingerprint_temporary, &plan.fingerprint).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        let _ = tokio::fs::remove_file(&fingerprint_temporary).await;
        let _ = tokio::fs::remove_file(&settings_temporary).await;
        return Err(error.into());
    }
    let metadata = MixCacheMetadata {
        source_fingerprint: plan.source_fingerprint.clone(),
        fingerprint: plan.fingerprint.clone(),
        settings: plan.settings.clone(),
    };
    if let Err(error) = tokio::fs::write(
        &settings_temporary,
        serde_json::to_vec_pretty(&metadata).map_err(std::io::Error::other)?,
    )
    .await
    {
        let _ = tokio::fs::remove_file(&temporary).await;
        let _ = tokio::fs::remove_file(&fingerprint_temporary).await;
        let _ = tokio::fs::remove_file(&settings_temporary).await;
        return Err(error.into());
    }
    if let Err(error) = tokio::fs::rename(&temporary, plan.cache_dir.join(MIX_OUTPUT)).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        let _ = tokio::fs::remove_file(&fingerprint_temporary).await;
        let _ = tokio::fs::remove_file(&settings_temporary).await;
        return Err(error.into());
    }
    if let Err(error) =
        tokio::fs::rename(&fingerprint_temporary, plan.cache_dir.join(MIX_FINGERPRINT)).await
    {
        let _ = tokio::fs::remove_file(plan.cache_dir.join(MIX_OUTPUT)).await;
        let _ = tokio::fs::remove_file(&fingerprint_temporary).await;
        let _ = tokio::fs::remove_file(&settings_temporary).await;
        return Err(error.into());
    }
    if let Err(error) =
        tokio::fs::rename(&settings_temporary, plan.cache_dir.join(MIX_SETTINGS)).await
    {
        let _ = tokio::fs::remove_file(plan.cache_dir.join(MIX_OUTPUT)).await;
        let _ = tokio::fs::remove_file(plan.cache_dir.join(MIX_FINGERPRINT)).await;
        let _ = tokio::fs::remove_file(&settings_temporary).await;
        return Err(error.into());
    }
    job.lock().await.progress = 100;
    Ok(())
}

pub(super) async fn run_mix_ffmpeg(
    plan: &MixPlan,
    job: &Arc<Mutex<MixJob>>,
    output: &Path,
) -> Result<(), AppError> {
    let duration_seconds = milliseconds_as_seconds(plan.duration_ms);
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .arg("-y")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    for source in plan.renderable_sources() {
        command.args(["-i"]).arg(&source.path);
    }

    let filter = build_mix_filter(plan);
    command
        .args(["-filter_complex", &filter, "-map", "[out]"])
        .args([
            "-ar",
            "48000",
            "-ac",
            "1",
            "-c:a",
            "libopus",
            "-b:a",
            "96k",
            "-t",
            &duration_seconds,
            "-progress",
            "pipe:2",
            "-nostats",
        ])
        .arg(output)
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
    while let Some(line) = lines.next_line().await? {
        if let Some(value) = line.strip_prefix("out_time_us=")
            && let Ok(elapsed_us) = value.parse::<u64>()
        {
            let progress = elapsed_us
                .saturating_mul(99)
                .checked_div((plan.duration_ms.max(1) as u64).saturating_mul(1_000))
                .unwrap_or(0)
                .clamp(1, 99) as i16;
            let mut state = job.lock().await;
            state.progress = state.progress.max(progress);
        } else if !is_ffmpeg_progress_line(&line) && error_output.len() < 4_096 {
            let remaining = 4_096 - error_output.len();
            error_output.extend_from_slice(&line.as_bytes()[..line.len().min(remaining)]);
            error_output.push(b'\n');
        }
    }
    let status = child.wait().await?;
    if !status.success() {
        return Err(AppError::FfmpegError(
            String::from_utf8_lossy(&error_output).into_owned(),
        ));
    }
    Ok(())
}

pub(super) fn build_mix_filter(plan: &MixPlan) -> String {
    let duration_seconds = milliseconds_as_seconds(plan.duration_ms);
    let duration_samples = plan.duration_ms.saturating_mul(48);
    let mut filter = String::new();
    let sources = plan.renderable_sources();
    for (index, source) in sources.iter().enumerate() {
        let trim_start = milliseconds_as_seconds(
            source
                .overlap_start_ms
                .saturating_sub(source.source_start_ms),
        );
        let trim_end =
            milliseconds_as_seconds(source.overlap_end_ms.saturating_sub(source.source_start_ms));
        let delay_samples = source.delay_ms.saturating_mul(48);
        let gain_db = plan
            .settings
            .participants
            .iter()
            .find(|participant| participant.user_id == source.participant_user_id.to_string())
            .map(|participant| participant.gain_db)
            .unwrap_or(0.0);
        filter.push_str(&format!(
            "[{index}:a]aresample=48000, aformat=sample_fmts=fltp:channel_layouts=mono, atrim=start={trim_start}:end={trim_end}, asetpts=PTS-STARTPTS, volume={gain_db}dB, adelay={delay_samples}S|{delay_samples}S, apad=whole_len={duration_samples}, atrim=duration={duration_seconds}[mix{index}];"
        ));
    }
    for index in 0..sources.len() {
        filter.push_str(&format!("[mix{index}]"));
    }
    filter.push_str(&format!(
        "amix=inputs={}:duration=longest:normalize=0:dropout_transition=0, alimiter=limit=0.95:attack=5:release=50:level=false, atrim=duration={duration_seconds}, asetpts=N/SR/TB[out]",
        sources.len()
    ));
    filter
}
