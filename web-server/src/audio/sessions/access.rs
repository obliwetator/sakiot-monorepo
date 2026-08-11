use super::*;

pub(crate) async fn require_session_access(
    pool: &web::Data<Pool<Postgres>>,
    recording_session_id: i64,
    viewer_user_id: i64,
) -> Result<SessionAccess, AppError> {
    let row = sqlx::query(
        "SELECT id,
                guild_id,
                user_id,
                starting_channel_id,
                state,
                (EXTRACT(EPOCH FROM started_at) * 1000)::bigint AS started_at_ms,
                (EXTRACT(EPOCH FROM ended_at) * 1000)::bigint AS ended_at_ms,
                (EXTRACT(EPOCH FROM pause_started_at) * 1000)::bigint AS pause_started_at_ms
           FROM recording_sessions
          WHERE id = $1",
    )
    .bind(recording_session_id)
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or(AppError::FileNotFound)?;

    let access = SessionAccess {
        session_id: row.try_get("id")?,
        guild_id: row.try_get("guild_id")?,
        user_id: row.try_get("user_id")?,
        starting_channel_id: row.try_get("starting_channel_id")?,
        state: row.try_get("state")?,
        started_at_ms: row.try_get("started_at_ms")?,
        ended_at_ms: row.try_get("ended_at_ms")?,
        pause_started_at_ms: row.try_get("pause_started_at_ms")?,
    };

    let permitted = visible_channels_for_user(pool, access.guild_id, viewer_user_id).await?;
    let rows = sqlx::query(
        "SELECT DISTINCT channel_id
           FROM audio_files
          WHERE recording_session_id = $1",
    )
    .bind(recording_session_id)
    .fetch_all(pool.get_ref())
    .await?;
    let mut audible_channels: HashSet<i64> = rows
        .into_iter()
        .map(|row| row.get::<i64, _>("channel_id"))
        .collect();
    if audible_channels.is_empty() {
        audible_channels.insert(access.starting_channel_id);
    }
    if audible_channels
        .iter()
        .all(|channel_id| permitted.contains(channel_id))
    {
        Ok(access)
    } else {
        Err(AppError::Forbidden)
    }
}

/// Keeps stem-based routes compatible while applying logical-session
/// authorization whenever the physical file has a logical parent. A viewer
/// must never retrieve one permitted fragment from an otherwise forbidden
/// multi-channel session.
pub(crate) async fn require_recording_access(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    channel_id: i64,
    year: i32,
    month: i32,
    file_name: &str,
    viewer_user_id: i64,
) -> Result<(), AppError> {
    let stem = file_name.strip_suffix(".ogg").unwrap_or(file_name);
    let session_id = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT recording_session_id
           FROM audio_files
          WHERE guild_id = $1
            AND channel_id = $2
            AND (file_name = $5 OR file_name = $6)
          ORDER BY (year = $3 AND month = $4) DESC, id DESC
          LIMIT 1",
    )
    .bind(guild_id)
    .bind(channel_id)
    .bind(year)
    .bind(month)
    .bind(file_name)
    .bind(stem)
    .fetch_optional(pool.get_ref())
    .await?
    .flatten();
    if let Some(session_id) = session_id {
        require_session_access(pool, session_id, viewer_user_id).await?;
    } else {
        crate::permissions::require_channel_access(pool, guild_id, channel_id, viewer_user_id)
            .await?;
    }
    Ok(())
}
