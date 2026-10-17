use super::*;

pub(super) async fn resolve_sources(
    pool: &web::Data<Pool<Postgres>>,
    media: Option<&MediaArchive>,
    guild_id: i64,
    viewer: crate::permissions::Viewer,
    segments: &[ComposeSegment],
) -> Result<Vec<ResolvedSource>, AppError> {
    let guild =
        crate::permissions::GuildAccess::load(pool.get_ref(), guild_id, viewer, None).await?;
    let mut resolved = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        let row = sqlx::query!(
            "SELECT saved_file_name, channel_id, channel_ids, length, original_file_name
               FROM clips
              WHERE guild_id = $1 AND clip_id = $2 AND deleted_at IS NULL",
            guild_id,
            segment.source_id
        )
        .fetch_optional(pool.get_ref())
        .await?
        .ok_or(AppError::ClipNotFound)?;
        if row.original_file_name.as_deref() == Some("compose") {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: composed clips cannot be used as sources"
            )));
        }
        crate::clips::require_clip(&guild, &row.channel_ids)?;
        let saved_file_name = row.saved_file_name.ok_or(AppError::ClipNotFound)?;
        let path = crate::media_archive::clip_local_path(&saved_file_name)?;
        if let Some(media) = media {
            media
                .ensure_clip_local(pool.get_ref(), &segment.source_id, &path)
                .await?;
        }
        let length = match row.length {
            Some(length) if length.is_finite() && length > 0.0 => length,
            // Metadata-only admission does not download or probe audio in the
            // HTTP process. Unknown lengths are verified by the bounded worker.
            _ if media.is_none() => segment.source_out,
            _ => probe_duration(&path).await? as f32,
        };
        let channel_id = row.channel_id.ok_or(AppError::ClipNotFound)?;
        resolved.push(ResolvedSource {
            path,
            channel_id,
            channel_ids: row.channel_ids,
            length,
            saved_file_name,
        });
    }
    Ok(resolved)
}

pub(super) async fn resolve_composition(
    pool: &web::Data<Pool<Postgres>>,
    media: Option<&MediaArchive>,
    guild_id: i64,
    viewer: crate::permissions::Viewer,
    validated: ValidatedComposition,
) -> Result<ResolvedComposition, AppError> {
    let sources = resolve_sources(pool, media, guild_id, viewer, &validated.0.segments).await?;
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
