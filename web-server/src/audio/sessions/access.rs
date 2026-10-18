use super::*;
use crate::permissions::{GuildAccess, Viewer};

/// Fails unless the viewer may hear the whole session: it is their own, or
/// they hold `LISTEN_OTHERS` in every channel its audio was recorded in.
pub(crate) async fn require_session_access(
    pool: &web::Data<Pool<Postgres>>,
    recording_session_id: i64,
    viewer: Viewer,
) -> Result<SessionAccess, AppError> {
    Ok(authorize(pool, recording_session_id, viewer)
        .await?
        .0
        .access)
}

/// `require_session_access` for mixing the session with others' or making
/// something of it, also returning the viewer's access to the session's
/// guild for the request's further checks. A session its speaker is
/// erasing is theirs to play, and nothing more.
pub(crate) async fn authorize_session(
    pool: &web::Data<Pool<Postgres>>,
    recording_session_id: i64,
    viewer: Viewer,
) -> Result<(SessionAccess, GuildAccess), AppError> {
    let (session, guild) = authorize(pool, recording_session_id, viewer).await?;
    Ok((session.listed()?.access, guild))
}

/// `authorize_session`, also returning the session's journey: every
/// channel it was recorded in.
pub(super) async fn authorize_session_journey(
    pool: &web::Data<Pool<Postgres>>,
    recording_session_id: i64,
    viewer: Viewer,
) -> Result<(SessionAccess, GuildAccess, HashSet<i64>), AppError> {
    let (session, guild) = authorize(pool, recording_session_id, viewer).await?;
    let session = session.listed()?;
    Ok((session.access, guild, session.journey))
}

/// Two statements, the session and then the viewer's access, rather than one
/// snapshot. A journey only grows, and whatever a handler reads after any
/// check can be newer than the check, so a snapshot would not narrow what
/// the request can see.
async fn authorize(
    pool: &web::Data<Pool<Postgres>>,
    recording_session_id: i64,
    viewer: Viewer,
) -> Result<(LoadedSession, GuildAccess), AppError> {
    crate::server_timing::measure("session", async {
        let session = load_session(pool.get_ref(), recording_session_id).await?;
        // Hidden from everyone but its speaker, as if deleted.
        if session.erasing && session.access.user_id != viewer.user_id {
            return Err(AppError::FileNotFound);
        }
        let guild =
            GuildAccess::load(pool.get_ref(), session.access.guild_id, viewer, None).await?;
        guild.require_item(&[session.access.user_id], session.journey.iter().copied())?;
        Ok((session, guild))
    })
    .await
}

struct LoadedSession {
    access: SessionAccess,
    /// The channels its audio was recorded in, or the channel it started in
    /// before it has any audio.
    journey: HashSet<i64>,
    /// Its speaker asked to erase it, and it is theirs alone to play until
    /// the purge.
    erasing: bool,
}

impl LoadedSession {
    /// Refuses a session its speaker is erasing: no mixes, no clips.
    fn listed(self) -> Result<Self, AppError> {
        if self.erasing {
            return Err(AppError::FileNotFound);
        }
        Ok(self)
    }
}

/// A session that is not deleted, with the channels it was recorded in. Or
/// one its speaker is erasing that was visible when they asked, while they
/// can still undo it and the server has not removed it since.
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
                       WHERE recording_session_id = rs.id) AS "channels!",
                deletion_requested_at IS NOT NULL AS "erasing!"
           FROM recording_sessions rs
          WHERE id = $1
            AND (deletion_requested_at IS NULL
                 OR (EXISTS (SELECT 1
                               FROM recording_erasures e
                              WHERE e.recording_session_id = rs.id
                                AND e.state = 'grace'
                                AND e.hid_session)
                     AND NOT EXISTS (SELECT 1
                                       FROM recording_deletion_jobs j
                                      WHERE j.recording_session_id = rs.id)))"#,
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
        erasing: row.erasing,
    })
}

/// Keeps stem-based routes compatible while applying logical-session
/// authorization whenever the physical file has a logical parent. A viewer
/// must never retrieve one permitted fragment from an otherwise forbidden
/// multi-channel session. Returns the viewer's access to the guild.
pub(crate) async fn require_recording_access(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    channel_id: i64,
    year: i32,
    month: i32,
    file_name: &str,
    viewer: Viewer,
) -> Result<GuildAccess, AppError> {
    let file = FileRef {
        guild_id,
        channel_id,
        year,
        month,
        file_name,
    };
    recording_access(pool, file, viewer, false).await
}

/// `require_recording_access` for making a clip of the recording, which a
/// session its speaker is erasing does not allow.
pub(crate) async fn require_recording_source(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    channel_id: i64,
    year: i32,
    month: i32,
    file_name: &str,
    viewer: Viewer,
) -> Result<GuildAccess, AppError> {
    let file = FileRef {
        guild_id,
        channel_id,
        year,
        month,
        file_name,
    };
    recording_access(pool, file, viewer, true).await
}

/// A physical recording as stem-based routes name it.
struct FileRef<'a> {
    guild_id: i64,
    channel_id: i64,
    year: i32,
    month: i32,
    file_name: &'a str,
}

async fn recording_access(
    pool: &web::Data<Pool<Postgres>>,
    FileRef {
        guild_id,
        channel_id,
        year,
        month,
        file_name,
    }: FileRef<'_>,
    viewer: Viewer,
    source: bool,
) -> Result<GuildAccess, AppError> {
    let stem = file_name.strip_suffix(".ogg").unwrap_or(file_name);
    let file = sqlx::query!(
        r#"SELECT recording_session_id, user_id
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
    .await?;
    if let Some(session_id) = file.as_ref().and_then(|file| file.recording_session_id) {
        // A fragment is the session's.
        let (session, guild) = authorize(pool, session_id, viewer).await?;
        if source {
            session.listed()?;
        }
        return Ok(guild);
    }
    let guild = GuildAccess::load(pool.get_ref(), guild_id, viewer, None).await?;
    // A recording without a session holds its speaker's voice alone.
    let speakers: Vec<i64> = file.map(|file| file.user_id).into_iter().collect();
    guild.require_item(&speakers, [channel_id])?;
    Ok(guild)
}
