use sakiot_permissions::Capabilities;

use super::*;
use crate::permissions::{GuildAccess, Viewer};

/// Fails unless the viewer may hear the whole session: they must hold
/// `LISTEN_OTHERS` in every channel its audio was recorded in.
pub(crate) async fn require_session_access(
    pool: &web::Data<Pool<Postgres>>,
    recording_session_id: i64,
    viewer: Viewer,
) -> Result<SessionAccess, AppError> {
    Ok(authorize_session(pool, recording_session_id, viewer)
        .await?
        .0)
}

/// `require_session_access`, also returning the viewer's access to the
/// session's guild for the request's further checks.
///
/// Two statements, the session and then the viewer's access, rather than one
/// snapshot. A journey only grows, and whatever a handler reads after any
/// check can be newer than the check, so a snapshot would not narrow what
/// the request can see.
pub(crate) async fn authorize_session(
    pool: &web::Data<Pool<Postgres>>,
    recording_session_id: i64,
    viewer: Viewer,
) -> Result<(SessionAccess, GuildAccess), AppError> {
    crate::server_timing::measure("session", async {
        let session = load_session(pool.get_ref(), recording_session_id).await?;
        let guild =
            GuildAccess::load(pool.get_ref(), session.access.guild_id, viewer, None).await?;
        require_journey(&guild, &session.journey)?;
        Ok((session.access, guild))
    })
    .await
}

/// A session of `guild`'s guild, which its viewer may hear in full.
pub(crate) async fn require_session_in(
    pool: &web::Data<Pool<Postgres>>,
    guild: &GuildAccess,
    recording_session_id: i64,
) -> Result<SessionAccess, AppError> {
    let session = load_session(pool.get_ref(), recording_session_id).await?;
    if session.access.guild_id != guild.guild_id() {
        return Err(AppError::Forbidden);
    }
    require_journey(guild, &session.journey)?;
    Ok(session.access)
}

/// Fails unless the viewer may listen in every channel of a session's
/// journey.
pub(super) fn require_journey(guild: &GuildAccess, journey: &HashSet<i64>) -> Result<(), AppError> {
    guild.require_channels(journey.iter().copied(), Capabilities::LISTEN_OTHERS)
}

struct LoadedSession {
    access: SessionAccess,
    /// The channels its audio was recorded in, or the channel it started in
    /// before it has any audio.
    journey: HashSet<i64>,
}

/// A session that is not deleted, with the channels it was recorded in.
async fn load_session<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    recording_session_id: i64,
) -> Result<LoadedSession, AppError> {
    let row = sqlx::query!(
        r#"SELECT id,
                guild_id,
                user_id,
                starting_channel_id,
                state,
                (EXTRACT(EPOCH FROM started_at) * 1000)::bigint AS "started_at_ms!",
                (EXTRACT(EPOCH FROM ended_at) * 1000)::bigint AS ended_at_ms,
                (EXTRACT(EPOCH FROM pause_started_at) * 1000)::bigint AS pause_started_at_ms,
                ARRAY(SELECT DISTINCT channel_id
                        FROM audio_files
                       WHERE recording_session_id = rs.id) AS "channels!"
           FROM recording_sessions rs
          WHERE id = $1 AND deletion_requested_at IS NULL"#,
        recording_session_id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(AppError::FileNotFound)?;

    let mut journey: HashSet<i64> = row.channels.into_iter().collect();
    if journey.is_empty() {
        journey.insert(row.starting_channel_id);
    }
    Ok(LoadedSession {
        access: SessionAccess {
            session_id: row.id,
            guild_id: row.guild_id,
            user_id: row.user_id,
            starting_channel_id: row.starting_channel_id,
            state: row.state,
            started_at_ms: row.started_at_ms,
            ended_at_ms: row.ended_at_ms,
            pause_started_at_ms: row.pause_started_at_ms,
        },
        journey,
    })
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
    viewer: Viewer,
) -> Result<(), AppError> {
    let stem = file_name.strip_suffix(".ogg").unwrap_or(file_name);
    let session_id = sqlx::query_scalar!(
        r#"SELECT recording_session_id
           FROM audio_files
          WHERE guild_id = $1
            AND channel_id = $2
            AND (file_name = $5 OR file_name = $6)
          ORDER BY (year = $3 AND month = $4) DESC, id DESC
          LIMIT 1"#,
        guild_id,
        channel_id,
        year,
        month,
        file_name,
        stem
    )
    .fetch_optional(pool.get_ref())
    .await?
    .flatten();
    if let Some(session_id) = session_id {
        require_session_access(pool, session_id, viewer).await?;
    } else {
        GuildAccess::load(pool.get_ref(), guild_id, viewer, None)
            .await?
            .require_channel(channel_id, Capabilities::LISTEN_OTHERS)?;
    }
    Ok(())
}
