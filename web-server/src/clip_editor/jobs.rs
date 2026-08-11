use super::*;

pub(super) fn compose_progress_key(clip_id: &str) -> String {
    format!("clip-compose-{clip_id}")
}

pub(super) fn expected_duration_ms(segments: &[SegmentRender]) -> i64 {
    let max_end = segments
        .iter()
        .map(|segment| {
            f64::from(segment.timeline_start)
                + f64::from(segment.source_out - segment.source_in) / effective_rate(segment)
                + f64::from(segment.effects.tail_seconds)
        })
        .fold(0.0, f64::max);
    (max_end * 1_000.0).round() as i64
}

/// Timeline consumption rate of a segment. Pitch shifting preserves duration,
/// so only the speed control changes the visible and exported extent.
pub(super) fn effective_rate(segment: &SegmentRender) -> f64 {
    f64::from(segment.effects.rate)
}

pub(super) fn pitch_factor(pitch_cents: f32) -> f64 {
    2f64.powf(f64::from(pitch_cents) / 1200.0)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_compose_job(
    pool: &web::Data<Pool<Postgres>>,
    segments: &[SegmentRender],
    master_volume_db: f32,
    full_path: &Path,
    expected_total_ms: i64,
    progress: &web::Data<WaveformProgressContainer>,
    cache_key: &str,
    clip_id: &str,
    guild_id: i64,
    user_id: i64,
    channel_id: i64,
    name: &str,
    saved_file_name: &str,
    composition: &serde_json::Value,
    overwrite: Option<ComposeOverwrite>,
) -> Result<(), AppError> {
    render_compose(
        segments,
        master_volume_db,
        full_path,
        expected_total_ms,
        progress,
        cache_key,
    )
    .await?;
    let duration = probe_duration(full_path).await?;
    let size = tokio::fs::metadata(full_path).await?.len() as i64;

    match overwrite {
        Some(target) => {
            let result = sqlx::query(
                "UPDATE clips
                    SET saved_file_name = $3, length = $4, size = $5, name = $6,
                        start_time = $7, composition = $8
                  WHERE guild_id = $1 AND clip_id = $2 AND deleted_at IS NULL",
            )
            .bind(guild_id)
            .bind(&target.clip_id)
            .bind(saved_file_name)
            .bind(duration as f32)
            .bind(size)
            .bind(name)
            .bind(0.0f32)
            .bind(composition)
            .execute(pool.get_ref())
            .await;
            if let Err(err) = result {
                return Err(AppError::DbError(err));
            }
            // The archive ledger keeps at most one object per clip id; reset
            // it so the worker re-uploads the fresh render instead of serving
            // the stale version once the local file is pruned.
            if let Err(error) = sqlx::query(
                "UPDATE media_objects
                    SET state = 'pending',
                        retry_at = now(),
                        lease_owner = NULL,
                        lease_expires_at = NULL,
                        object_key = NULL,
                        bytes = NULL,
                        sha256 = NULL,
                        etag = NULL,
                        attempts = 0,
                        last_error = NULL,
                        uploaded_at = NULL,
                        verified_at = NULL,
                        local_delete_after = NULL,
                        updated_at = now()
                  WHERE clip_id = $1
                    AND state <> 'pending'",
            )
            .bind(&target.clip_id)
            .execute(pool.get_ref())
            .await
            {
                tracing::warn!(
                    clip_id = %target.clip_id,
                    ?error,
                    "media archive ledger reset for overwritten clip failed"
                );
            }
            // The replaced file and its waveform are stale; drop them so the
            // fresh render (and its regenerated peaks) is the only version.
            if let Ok(old_path) = crate::media_archive::clip_local_path(&target.old_saved_file_name)
            {
                let _ = tokio::fs::remove_file(&old_path).await;
            }
            let old_waveform = format!(
                "{}clip-{}.dat",
                crate::audio::waveform_path(),
                target.clip_id
            );
            let _ = tokio::fs::remove_file(old_waveform).await;
            crate::audio::spawn_clip_waveform(
                target.clip_id.clone(),
                full_path.to_path_buf(),
                progress.clone(),
            );
        }
        None => {
            let insert = sqlx::query(
                "INSERT INTO clips
                    (clip_id, length, size, channel_id, guild_id, user_id,
                     original_file_name, saved_file_name, name, start_time, composition)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            )
            .bind(clip_id)
            .bind(duration as f32)
            .bind(size)
            .bind(channel_id)
            .bind(guild_id)
            .bind(user_id)
            .bind("compose")
            .bind(saved_file_name)
            .bind(name)
            .bind(0.0f32)
            .bind(composition)
            .execute(pool.get_ref())
            .await;
            if let Err(err) = insert {
                return Err(AppError::DbError(err));
            }
            crate::audio::spawn_clip_waveform(
                clip_id.to_string(),
                full_path.to_path_buf(),
                progress.clone(),
            );
        }
    }
    Ok(())
}
