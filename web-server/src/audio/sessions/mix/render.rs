//! Channel-mix job lifecycle, media materialization, and FFmpeg rendering.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use actix_web::web;
use sqlx::{Pool, Postgres};
use tokio::sync::Mutex;

use crate::errors::AppError;
use crate::media_archive::MediaArchive;
use crate::media_jobs::JobAttempt;

use super::super::milliseconds_as_seconds;
use super::cache::MixCacheMetadata;
use super::{MIX_FINGERPRINT, MIX_OUTPUT, MIX_SETTINGS, MixJob, MixPlan};

/// Rollback ledger for the three-file mix cache publication.
///
/// Every temp file starts in `pending`. Each successful rename moves it to
/// `published`, so a failure at any step removes exactly what is on disk at
/// that moment: the not-yet-renamed temporaries plus the files already
/// published under their final names.
#[derive(Default)]
struct PublicationGuard {
    pending: Vec<PathBuf>,
    published: Vec<PathBuf>,
}

impl PublicationGuard {
    fn track(&mut self, temporary: &Path) {
        self.pending.push(temporary.to_path_buf());
    }

    /// Rename a tracked temp file to its final name. On success the final
    /// path becomes a rollback target.
    async fn publish(&mut self, temporary: &Path, destination: PathBuf) -> Result<(), AppError> {
        tokio::fs::rename(temporary, &destination).await?;
        self.pending.retain(|path| path.as_path() != temporary);
        self.published.push(destination);
        Ok(())
    }

    /// Best-effort removal of everything this publication still owns.
    async fn rollback(&self) {
        for path in self.pending.iter().chain(self.published.iter()) {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

pub(super) async fn render_mix(
    pool: &web::Data<Pool<Postgres>>,
    media: &MediaArchive,
    plan: &MixPlan,
    job: &Arc<Mutex<MixJob>>,
    fence: Option<&JobAttempt<'_>>,
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

    let mut publication = PublicationGuard::default();
    publication.track(&temporary);
    publication.track(&fingerprint_temporary);
    publication.track(&settings_temporary);

    // Everything up to and including the renames rolls the filesystem back on
    // failure; `begin_publication` is inside the scope because a failed fence
    // claim must not leave the temp files behind.
    let result = async {
        run_mix_ffmpeg(plan, job, &temporary).await?;
        tokio::fs::write(&fingerprint_temporary, &plan.fingerprint).await?;
        let metadata = MixCacheMetadata {
            source_fingerprint: plan.source_fingerprint.clone(),
            fingerprint: plan.fingerprint.clone(),
            settings: plan.settings.clone(),
        };
        let settings = serde_json::to_vec_pretty(&metadata).map_err(std::io::Error::other)?;
        tokio::fs::write(&settings_temporary, &settings).await?;
        let publication_tx = if let Some(attempt) = fence {
            Some(
                crate::media_jobs::begin_publication(attempt.pool, attempt.id, attempt.token)
                    .await?,
            )
        } else {
            None
        };
        publication
            .publish(&temporary, plan.cache_dir.join(MIX_OUTPUT))
            .await?;
        publication
            .publish(&fingerprint_temporary, plan.cache_dir.join(MIX_FINGERPRINT))
            .await?;
        publication
            .publish(&settings_temporary, plan.cache_dir.join(MIX_SETTINGS))
            .await?;
        Ok::<_, AppError>(publication_tx)
    }
    .await;

    let publication_tx = match result {
        Ok(publication_tx) => publication_tx,
        Err(error) => {
            publication.rollback().await;
            return Err(error);
        }
    };

    // The cache files are in place; a failure completing the job record must
    // not remove them - the next attempt would have to re-render the mix.
    if let (Some(tx), Some(attempt)) = (publication_tx, fence) {
        let url = format!(
            "/api/audio/sessions/{}/channel-mix/media?scope={}",
            plan.session_id,
            plan.scope.as_str()
        );
        crate::media_jobs::complete_publication(tx, attempt.id, attempt.token, &url, None).await?;
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
        .arg(output);
    let duration_ms = plan.duration_ms.max(1) as u64;
    let job = Arc::clone(job);
    crate::ffmpeg::run_ffmpeg_with_progress(command, move |elapsed_us| {
        let job = Arc::clone(&job);
        async move {
            let progress = elapsed_us
                .saturating_mul(99)
                .checked_div(duration_ms.saturating_mul(1_000))
                .unwrap_or(0)
                .clamp(1, 99) as i16;
            let mut state = job.lock().await;
            state.progress = state.progress.max(progress);
        }
    })
    .await
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
