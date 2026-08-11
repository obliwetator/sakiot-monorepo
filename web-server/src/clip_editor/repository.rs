use super::*;

pub(super) async fn resolve_sources(
    pool: &web::Data<Pool<Postgres>>,
    media: &MediaArchive,
    guild_id: i64,
    user_id: i64,
    segments: &[ComposeSegment],
) -> Result<Vec<ResolvedSource>, AppError> {
    let mut resolved = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        let row = sqlx::query(
            "SELECT saved_file_name, channel_id, recording_session_id, length,
                    original_file_name
               FROM clips
              WHERE guild_id = $1 AND clip_id = $2 AND deleted_at IS NULL",
        )
        .bind(guild_id)
        .bind(&segment.source_id)
        .fetch_optional(pool.get_ref())
        .await?
        .ok_or(AppError::ClipNotFound)?;
        if row
            .try_get::<Option<String>, _>("original_file_name")?
            .as_deref()
            == Some("compose")
        {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: composed clips cannot be used as sources"
            )));
        }
        let recording_session_id: Option<i64> = row.try_get("recording_session_id")?;
        if let Some(recording_session_id) = recording_session_id {
            crate::audio::sessions::require_session_access(pool, recording_session_id, user_id)
                .await?;
        } else {
            let channel_id: Option<i64> = row.try_get("channel_id")?;
            crate::permissions::require_channel_access(
                pool,
                guild_id,
                channel_id.ok_or(AppError::ClipNotFound)?,
                user_id,
            )
            .await?;
        }
        let saved_file_name: Option<String> = row.try_get("saved_file_name")?;
        let saved_file_name = saved_file_name.ok_or(AppError::ClipNotFound)?;
        let path = crate::media_archive::clip_local_path(&saved_file_name)?;
        media
            .ensure_clip_local(pool.get_ref(), &segment.source_id, &path)
            .await?;
        let length: Option<f32> = row.try_get("length")?;
        let length = match length {
            Some(length) if length.is_finite() && length > 0.0 => length,
            _ => probe_duration(&path).await? as f32,
        };
        let channel_id: i64 = row
            .try_get::<Option<i64>, _>("channel_id")?
            .ok_or(AppError::ClipNotFound)?;
        resolved.push(ResolvedSource {
            path,
            channel_id,
            length,
        });
    }
    Ok(resolved)
}

pub(super) async fn resolve_composition(
    pool: &web::Data<Pool<Postgres>>,
    media: &MediaArchive,
    guild_id: i64,
    user_id: i64,
    validated: ValidatedComposition,
) -> Result<ResolvedComposition, AppError> {
    let sources = resolve_sources(pool, media, guild_id, user_id, &validated.0.segments).await?;
    for (segment, source) in validated.0.segments.iter().zip(&sources) {
        if segment.source_out > source.length + 0.1 {
            return Err(AppError::BadRequest(format!(
                "Segment source_out {:.3}s exceeds clip length {:.3}s",
                segment.source_out, source.length
            )));
        }
    }
    Ok(ResolvedComposition { validated, sources })
}
