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

pub(super) fn effective_rate(segment: &SegmentRender) -> f64 {
    f64::from(segment.effects.rate)
}
pub(super) fn pitch_factor(pitch_cents: f32) -> f64 {
    2f64.powf(f64::from(pitch_cents) / 1200.0)
}

/// A complete immutable file exists before this transaction. A failed/ambiguous
/// commit leaves it for reconciliation, never deletes a possibly committed file.
pub(super) async fn publish(
    pool: &Pool<Postgres>,
    job: &queue::Job,
    saved_file_name: &str,
    duration: f32,
    size: i64,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    let owned: Option<String> = sqlx::query_scalar("SELECT id FROM composition_jobs WHERE id = $1 AND attempt_token = $2 AND state = 'running' AND lease_expires_at > now() FOR UPDATE")
        .bind(&job.id).bind(&job.token).fetch_optional(&mut *tx).await?;
    if owned.is_none() {
        return Err(AppError::Conflict("Export lease lost".into()));
    }
    let mut composition =
        serde_json::to_value(&job.snapshot.body).map_err(|_| AppError::InternalError)?;
    if let Some(object) = composition.as_object_mut() {
        object.remove("overwrite_clip_id");
        object.remove("limits");
    }
    if let Some(target) = &job.snapshot.overwrite {
        let result = sqlx::query("UPDATE clips SET saved_file_name = $3, length = $4, size = $5, name = $6, start_time = 0, composition = $7 WHERE guild_id = $1 AND clip_id = $2 AND deleted_at IS NULL AND saved_file_name = $8 AND original_file_name = 'compose'")
            .bind(job.guild_id).bind(&target.clip_id).bind(saved_file_name).bind(duration).bind(size)
            .bind(&job.snapshot.name).bind(&composition).bind(&target.old_saved_file_name).execute(&mut *tx).await?;
        if result.rows_affected() != 1 {
            return Err(AppError::Conflict("The destination clip changed or was deleted during export. Reopen it before overwriting.".into()));
        }
    } else {
        sqlx::query("INSERT INTO clips (clip_id, length, size, channel_id, guild_id, user_id, original_file_name, saved_file_name, name, start_time, composition) VALUES ($1,$2,$3,$4,$5,$6,'compose',$7,$8,0,$9)")
            .bind(&job.result_clip_id).bind(duration).bind(size).bind(job.snapshot.channel_id)
            .bind(job.guild_id).bind(job.user_id).bind(saved_file_name).bind(&job.snapshot.name).bind(&composition)
            .execute(&mut *tx).await?;
    }
    // Invalidate an in-flight upload's lease, as well as previously verified
    // bytes. Its old owner cannot mark this new media revision available.
    sqlx::query(
        "INSERT INTO media_objects (clip_id, clip_saved_file_name) VALUES ($1,$2)
        ON CONFLICT (clip_id) WHERE clip_id IS NOT NULL DO UPDATE SET
        clip_saved_file_name = EXCLUDED.clip_saved_file_name, state = 'pending', retry_at = now(),
        lease_owner = NULL, lease_expires_at = NULL, object_key = NULL, bytes = NULL, sha256 = NULL,
        etag = NULL, attempts = 0, last_error = NULL, uploaded_at = NULL, verified_at = NULL,
        local_delete_after = NULL, updated_at = now()",
    )
    .bind(&job.result_clip_id)
    .bind(saved_file_name)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE composition_jobs SET state = 'ready', stage = 'ready', progress = 100, error = NULL, attempt_token = NULL, lease_expires_at = NULL, finished_at = now(), updated_at = now() WHERE id = $1")
        .bind(&job.id).execute(&mut *tx).await?;
    tx.commit().await?;
    tracing::info!(job_id = %job.id, clip_id = %job.result_clip_id, "composition published");
    Ok(())
}
