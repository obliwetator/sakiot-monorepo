//! Erasure by the speaker: anyone may erase a finalized session of their own
//! voice, or a clip holding it, member or former member. The request hides
//! it from everyone else at once and records exactly what it hid; the
//! speaker can undo it for [`GRACE_DAYS`], which reverts that and nothing
//! else. Then the sweep converts it into a permanent deletion job (reason
//! `speaker`), and the deletion worker purges it.
//!
//! A manager's or retention's removal keeps its own job: erasing a session
//! the server removed changes nothing visible, and undoing it returns the
//! session to "removed by the server", never to a restored recording.

use actix_web::post;

use super::*;
use crate::auth::{Access, Token};
use crate::clips::require_clip;
use crate::permissions::GuildAccess;

/// How long a speaker can undo an erasure.
pub(super) const GRACE_DAYS: i32 = 30;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ErasureStatus {
    pub id: String,
    /// The erased session; absent for a clip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_session_id: Option<String>,
    /// The erased clip; absent for a session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip_id: Option<String>,
    /// `grace`: hidden from everyone else and undoable until
    /// `purge_at_ms`; `cancelled`: undone; `converted`: handed to the purge
    /// (`purge_state`).
    pub state: String,
    pub requested_at_ms: i64,
    pub purge_at_ms: i64,
    /// The purge once converted: `queued` or `running`, `ready` once erased,
    /// `failed` or `paused` while it waits for an administrator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purge_state: Option<String>,
}

/// The viewer's own erasure of an item, as their listings show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct ErasureSummary {
    pub id: String,
    /// `grace`: undoable until `purge_at_ms`; `converted`: being purged.
    pub state: String,
    pub purge_at_ms: i64,
}

impl ErasureSummary {
    /// From a listing's columns: present when the item has an open erasure.
    pub(crate) fn from_columns(
        id: Option<String>,
        state: Option<String>,
        purge_at_ms: Option<i64>,
    ) -> Option<Self> {
        Some(Self {
            id: id?,
            state: state?,
            purge_at_ms: purge_at_ms?,
        })
    }
}

/// The speaker's erasure; nobody else's.
async fn load_erasure(
    executor: impl sqlx::PgExecutor<'_>,
    guild_id: i64,
    id: &str,
    speaker_id: i64,
) -> Result<ErasureStatus, AppError> {
    let row = sqlx::query!(
        r#"SELECT e.recording_session_id,
                  e.clip_id,
                  e.state,
                  (EXTRACT(EPOCH FROM e.requested_at) * 1000)::bigint AS "requested_at_ms!",
                  (EXTRACT(EPOCH FROM e.purge_at) * 1000)::bigint AS "purge_at_ms!",
                  j.state AS "purge_state?"
             FROM recording_erasures e
             LEFT JOIN recording_deletion_jobs j ON j.id = e.deletion_job_id
            WHERE e.guild_id = $1 AND e.id = $2 AND e.speaker_id = $3"#,
        guild_id,
        id,
        speaker_id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(ErasureStatus {
        id: id.to_owned(),
        recording_session_id: row.recording_session_id.map(|id| id.to_string()),
        clip_id: row.clip_id,
        state: row.state,
        requested_at_ms: row.requested_at_ms,
        purge_at_ms: row.purge_at_ms,
        purge_state: row.purge_state,
    })
}

/// Answers an erasure request with the erasure, written or already open.
async fn accepted(
    pool: &Pool<Postgres>,
    guild_id: i64,
    id: &str,
    speaker_id: i64,
) -> Result<HttpResponse, AppError> {
    Ok(HttpResponse::Accepted().json(load_erasure(pool, guild_id, id, speaker_id).await?))
}

#[utoipa::path(
    post,
    path = "/api/users/current/guilds/{guild_id}/recordings/{recording_session_id}/erasure",
    tag = "user",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("recording_session_id" = i64, Path, description = "A session of the current user's voice"),
    ),
    responses(
        (status = 202, description = "The session is hidden from everyone else and will be erased once the grace period ends; asking again answers with the same erasure", body = ErasureStatus),
        (status = 403, description = "Not the session's speaker, or not a member or former member", body = crate::errors::ApiError),
        (status = 404, description = "Recording not found", body = crate::errors::ApiError),
        (status = 409, description = "Still recording, or already being deleted permanently", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/users/current/guilds/{guild_id}/recordings/{recording_session_id}/erasure")]
pub async fn request_session_erasure(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, i64)>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let viewer = token.ok_or(AppError::Unauthorized)?.viewer();
    let (guild_id, session_id) = path.into_inner();
    GuildAccess::load(pool.get_ref(), guild_id, viewer, None)
        .await?
        .require_member_or_former()?;
    let id = erase_session(pool.get_ref(), guild_id, session_id, viewer.user_id).await?;
    accepted(pool.get_ref(), guild_id, &id, viewer.user_id).await
}

/// Hides `speaker_id`'s session and the clips made of it, returning the
/// erasure: the one already open, if any.
pub(super) async fn erase_session(
    pool: &Pool<Postgres>,
    guild_id: i64,
    session_id: i64,
    speaker_id: i64,
) -> Result<String, AppError> {
    let mut tx = pool.begin().await?;
    lock_guild(&mut tx, guild_id).await?;
    let session = sqlx::query!(
        r#"SELECT user_id, state, deletion_requested_at IS NOT NULL AS "removed!"
             FROM recording_sessions
            WHERE guild_id = $1 AND id = $2
              FOR UPDATE"#,
        guild_id,
        session_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::FileNotFound)?;
    if session.user_id != speaker_id {
        return Err(AppError::Forbidden);
    }
    let open = sqlx::query_scalar!(
        "SELECT id FROM recording_erasures WHERE recording_session_id = $1 AND state <> 'cancelled'",
        session_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(id) = open {
        return Ok(id);
    }
    if session.state != "finalized" {
        return Err(AppError::Conflict(
            "Only finalized recordings can be erased".into(),
        ));
    }
    let purging = sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1 FROM recording_deletion_jobs
              WHERE recording_session_id = $1 AND mode = 'permanent'
           ) AS "purging!""#,
        session_id
    )
    .fetch_one(&mut *tx)
    .await?;
    if purging {
        return Err(AppError::Conflict(
            "This recording is already being deleted permanently".into(),
        ));
    }
    // A session the server removed stays removed; the erasure only adds
    // the purge.
    let hid_session = !session.removed;
    if hid_session {
        sqlx::query!(
            "UPDATE recording_sessions SET deletion_requested_at = now() WHERE id = $1",
            session_id
        )
        .execute(&mut *tx)
        .await?;
    }
    let fragments = session_fragments(&mut *tx, session_id).await?;
    let clips = related_clips(&mut tx, guild_id, session_id, &fragments).await?;
    let hidden = hide_clips(&mut tx, guild_id, &clip_ids(&clips)).await?;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query!(
        "INSERT INTO recording_erasures
            (id, guild_id, recording_session_id, speaker_id, purge_at, hid_session, hidden_clip_ids)
         VALUES ($1, $2, $3, $4, now() + make_interval(days => $5), $6, $7)",
        id,
        guild_id,
        session_id,
        speaker_id,
        GRACE_DAYS,
        hid_session,
        &hidden
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

#[utoipa::path(
    post,
    path = "/api/users/current/guilds/{guild_id}/clips/{clip_id}/erasure",
    tag = "user",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("clip_id" = String, Path, description = "A clip holding the current user's voice"),
    ),
    responses(
        (status = 202, description = "The clip, and every clip composed from it, is hidden from everyone else and will be erased once the grace period ends; asking again answers with the same erasure", body = ErasureStatus),
        (status = 403, description = "The clip does not hold the current user's voice, or they may not hear it", body = crate::errors::ApiError),
        (status = 404, description = "Clip not found", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/users/current/guilds/{guild_id}/clips/{clip_id}/erasure")]
pub async fn request_clip_erasure(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, String)>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let viewer = token.ok_or(AppError::Unauthorized)?.viewer();
    let (guild_id, clip_id) = path.into_inner();
    let guild = GuildAccess::load(pool.get_ref(), guild_id, viewer, None).await?;
    guild.require_member_or_former()?;
    let id = erase_clip(pool.get_ref(), &guild, &clip_id, viewer.user_id).await?;
    accepted(pool.get_ref(), guild_id, &id, viewer.user_id).await
}

/// Hides a clip of `speaker_id`'s voice, whose access to the guild is
/// `guild`, and the clips composed from it, returning the erasure: the one
/// already open, if any.
async fn erase_clip(
    pool: &Pool<Postgres>,
    guild: &GuildAccess,
    clip_id: &str,
    speaker_id: i64,
) -> Result<String, AppError> {
    let guild_id = guild.guild_id();
    let mut tx = pool.begin().await?;
    lock_guild(&mut tx, guild_id).await?;
    let clip = sqlx::query!(
        r#"SELECT speaker_ids, channel_ids, deleted_at IS NOT NULL AS "removed!"
             FROM clips
            WHERE guild_id = $1 AND clip_id = $2
              FOR UPDATE"#,
        guild_id,
        clip_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::ClipNotFound)?;
    let open = sqlx::query_scalar!(
        "SELECT id FROM recording_erasures
          WHERE clip_id = $1 AND speaker_id = $2 AND state <> 'cancelled'",
        clip_id,
        speaker_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(id) = open {
        return Ok(id);
    }
    if clip.removed {
        return Err(AppError::ClipNotFound);
    }
    // Only what the speaker can find: a clip of their voice alone, or one
    // they may hear in full. Erasing their session takes the rest.
    require_clip(guild, &clip.speaker_ids, &clip.channel_ids)?;
    if !clip.speaker_ids.contains(&speaker_id) {
        return Err(AppError::Forbidden);
    }
    let clips = clip_with_dependents(&mut tx, guild_id, clip_id).await?;
    let hidden = hide_clips(&mut tx, guild_id, &clip_ids(&clips)).await?;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query!(
        "INSERT INTO recording_erasures
            (id, guild_id, clip_id, speaker_id, purge_at, hidden_clip_ids)
         VALUES ($1, $2, $3, $4, now() + make_interval(days => $5), $6)",
        id,
        guild_id,
        clip_id,
        speaker_id,
        GRACE_DAYS,
        &hidden
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Soft-deletes the visible ones of `clip_ids`, returning those.
async fn hide_clips(
    tx: &mut sqlx::PgConnection,
    guild_id: i64,
    clip_ids: &[String],
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "UPDATE clips SET deleted_at = now()
          WHERE guild_id = $1 AND clip_id = ANY($2) AND deleted_at IS NULL
          RETURNING clip_id",
        guild_id,
        clip_ids
    )
    .fetch_all(&mut *tx)
    .await
}

#[utoipa::path(
    get,
    path = "/api/users/current/guilds/{guild_id}/erasures/{erasure_id}",
    tag = "user",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("erasure_id" = String, Path, description = "One of the current user's erasures"),
    ),
    responses(
        (status = 200, description = "The erasure and, once converted, its purge", body = ErasureStatus),
        (status = 404, description = "No such erasure of the current user's", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/users/current/guilds/{guild_id}/erasures/{erasure_id}")]
pub async fn get_erasure(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, String)>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let viewer = token.ok_or(AppError::Unauthorized)?.viewer();
    let (guild_id, id) = path.into_inner();
    // The speaker's own record: readable after the purge too, when the
    // erased recordings no longer make them a former member.
    Ok(HttpResponse::Ok().json(load_erasure(pool.get_ref(), guild_id, &id, viewer.user_id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/users/current/guilds/{guild_id}/erasures/{erasure_id}",
    tag = "user",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("erasure_id" = String, Path, description = "One of the current user's erasures"),
    ),
    responses(
        (status = 200, description = "Undone: what the request hid is visible again, unless something else still hides it; undoing again answers the same", body = ErasureStatus),
        (status = 403, description = "Not a member or former member", body = crate::errors::ApiError),
        (status = 404, description = "No such erasure of the current user's", body = crate::errors::ApiError),
        (status = 409, description = "The grace period is over", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[delete("/users/current/guilds/{guild_id}/erasures/{erasure_id}")]
pub async fn undo_erasure(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, String)>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let viewer = token.ok_or(AppError::Unauthorized)?.viewer();
    let (guild_id, id) = path.into_inner();
    GuildAccess::load(pool.get_ref(), guild_id, viewer, None)
        .await?
        .require_member_or_former()?;
    undo(pool.get_ref(), guild_id, &id, viewer.user_id).await?;
    Ok(HttpResponse::Ok().json(load_erasure(pool.get_ref(), guild_id, &id, viewer.user_id).await?))
}

/// Reverts exactly what the erasure hid, while its grace period lasts.
/// Undoing it again changes nothing.
pub(super) async fn undo(
    pool: &Pool<Postgres>,
    guild_id: i64,
    id: &str,
    speaker_id: i64,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    lock_guild(&mut tx, guild_id).await?;
    let erasure = sqlx::query!(
        r#"SELECT recording_session_id, state, purge_at > now() AS "undoable!",
                  hid_session, hidden_clip_ids
             FROM recording_erasures
            WHERE guild_id = $1 AND id = $2 AND speaker_id = $3
              FOR UPDATE"#,
        guild_id,
        id,
        speaker_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    // The sweep converts only once `purge_at` has passed, under the same
    // lock, so exactly one of the two wins at the deadline.
    match erasure.state.as_str() {
        "cancelled" => {}
        "grace" if erasure.undoable => {
            sqlx::query!(
                "UPDATE recording_erasures SET state = 'cancelled', finished_at = now() WHERE id = $1",
                id
            )
            .execute(&mut *tx)
            .await?;
            // A removal by the server since the request has a job of its
            // own, and keeps the session hidden.
            if let Some(session_id) = erasure.recording_session_id
                && erasure.hid_session
            {
                sqlx::query!(
                    "UPDATE recording_sessions SET deletion_requested_at = NULL
                      WHERE id = $1
                        AND NOT EXISTS (
                            SELECT 1 FROM recording_deletion_jobs WHERE recording_session_id = $1
                        )",
                    session_id
                )
                .execute(&mut *tx)
                .await?;
            }
            restore_clips(&mut tx, guild_id, &erasure.hidden_clip_ids).await?;
        }
        _ => {
            return Err(AppError::Conflict(
                "The grace period is over: this erasure can no longer be undone".into(),
            ));
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Shows again the clips an erasure hid, except those something else still
/// hides: a removed session their audio, or a part's, came from, or an open
/// erasure of one of their parts.
async fn restore_clips(
    tx: &mut sqlx::PgConnection,
    guild_id: i64,
    hidden: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"WITH hidden AS (
               SELECT clip_id, composition
                 FROM clips
                WHERE guild_id = $1 AND clip_id = ANY($2) AND deleted_at IS NOT NULL
           ), parts AS (
               SELECT clip_id AS hidden_id, clip_id AS part_id FROM hidden
               UNION ALL
               SELECT h.clip_id, segment ->> 'source_id'
                 FROM hidden h,
                      jsonb_array_elements(
                          CASE WHEN jsonb_typeof(h.composition -> 'segments') = 'array'
                               THEN h.composition -> 'segments' ELSE '[]'::jsonb END
                      ) segment
           ), covered AS (
               SELECT p.hidden_id
                 FROM parts p
                 JOIN clips part ON part.clip_id = p.part_id AND part.guild_id = $1
                WHERE EXISTS (
                          SELECT 1 FROM recording_sessions rs
                           WHERE rs.id = part.recording_session_id
                             AND rs.deletion_requested_at IS NOT NULL)
                   OR EXISTS (
                          SELECT 1
                            FROM audio_files af
                            JOIN recording_sessions rs ON rs.id = af.recording_session_id
                           WHERE af.guild_id = part.guild_id
                             AND af.channel_id = part.channel_id
                             AND af.file_name = part.original_file_name
                             AND rs.deletion_requested_at IS NOT NULL)
                   OR EXISTS (
                          SELECT 1 FROM recording_erasures e
                           WHERE e.clip_id = part.clip_id AND e.state <> 'cancelled')
           )
           UPDATE clips c
              SET deleted_at = NULL
             FROM hidden h
            WHERE c.clip_id = h.clip_id
              AND h.clip_id NOT IN (SELECT hidden_id FROM covered)"#,
        guild_id,
        hidden
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Converts erasures whose grace period is over into permanent deletion
/// jobs, a small page per sweep.
pub(super) async fn convert_due_erasures(pool: &Pool<Postgres>) -> Result<(), AppError> {
    let due = sqlx::query!(
        "SELECT id, guild_id FROM recording_erasures
          WHERE state = 'grace' AND purge_at <= now()
          ORDER BY purge_at, id
          LIMIT 25"
    )
    .fetch_all(pool)
    .await?;
    for erasure in due {
        if let Err(error) = convert(pool, erasure.guild_id, &erasure.id).await {
            tracing::warn!(erasure_id = %erasure.id, ?error, "erasure conversion skipped");
        }
    }
    Ok(())
}

async fn convert(pool: &Pool<Postgres>, guild_id: i64, id: &str) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    lock_guild(&mut tx, guild_id).await?;
    // Undone meanwhile: nothing to do.
    let Some(erasure) = sqlx::query!(
        "SELECT recording_session_id, clip_id, speaker_id
           FROM recording_erasures
          WHERE id = $1 AND state = 'grace' AND purge_at <= now()
            FOR UPDATE",
        id
    )
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(());
    };
    let job_id = match (erasure.recording_session_id, erasure.clip_id) {
        // A removal by the server already has a job: it is upgraded in
        // place, keeping its reason, with the speaker as the one who asked
        // for the purge.
        (Some(session_id), _) => {
            enqueue_in(
                &mut tx,
                guild_id,
                session_id,
                Some(erasure.speaker_id),
                "speaker",
                DeletionMode::Permanent,
            )
            .await?
        }
        (None, Some(clip_id)) => {
            enqueue_clip_purge(&mut tx, guild_id, &clip_id, erasure.speaker_id).await?
        }
        (None, None) => return Err(AppError::InternalError),
    };
    sqlx::query!(
        "UPDATE recording_erasures
            SET state = 'converted', deletion_job_id = $2, finished_at = now()
          WHERE id = $1",
        id,
        job_id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// A permanent deletion job for a clip the speaker erased.
async fn enqueue_clip_purge(
    tx: &mut sqlx::PgConnection,
    guild_id: i64,
    clip_id: &str,
    speaker_id: i64,
) -> Result<String, AppError> {
    sqlx::query!(
        "INSERT INTO recording_deletion_jobs
            (id, clip_id, guild_id, requested_by, reason, mode,
             permanent_requested_by, permanent_requested_at)
         VALUES ($1, $2, $3, $4, 'speaker', 'permanent', $4, now())
         ON CONFLICT (clip_id) DO NOTHING",
        uuid::Uuid::new_v4().to_string(),
        clip_id,
        guild_id,
        speaker_id
    )
    .execute(&mut *tx)
    .await?;
    Ok(sqlx::query_scalar!(
        "SELECT id FROM recording_deletion_jobs WHERE clip_id = $1",
        clip_id
    )
    .fetch_one(&mut *tx)
    .await?)
}

#[cfg(test)]
mod tests;
