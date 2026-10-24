//! Bringing back a session the server removed: a manager restoring it
//! within the retention period, or a member keeping it ([`crate::keeping`]).
//! A restoration reverts exactly what the removal hid, deletes its job and
//! keeps the job's audit in `recording_restorations`.

use actix_web::post;

use super::*;
use crate::permissions::GuildAccess;

/// A session's place in its lifecycle, as keeping and restoring need it.
pub(crate) struct Lifecycle {
    pub speaker: i64,
    /// The channels its audio was recorded in, or the channel it started in
    /// before it has any audio.
    pub journey: Vec<i64>,
    pub finalized: bool,
    /// Older than the guild's retention period: retention removes it unless
    /// someone keeps it.
    pub past_retention: bool,
    /// The server's removal of it.
    pub removal: Option<Removal>,
    /// Its speaker asked to erase it: hidden from everyone else.
    pub erasing: bool,
}

pub(crate) struct Removal {
    pub job_id: String,
    /// `manager` or `retention`. A speaker's erasure upgrades the job in
    /// place and keeps its reason; `soft` tells the two apart.
    pub reason: String,
    /// Nothing has been purged, and nothing is queued to be.
    pub soft: bool,
}

impl Lifecycle {
    /// Whether the session is removed from everyone's view but its
    /// speaker's.
    pub fn removed(&self) -> bool {
        self.removal.is_some() || self.erasing
    }

    /// The removal a restoration would revert: a soft one by a manager or
    /// by retention, which its speaker is not erasing. Anything else is
    /// missing to the caller.
    pub fn restorable(&self) -> Result<&Removal, AppError> {
        match &self.removal {
            Some(removal)
                if removal.soft
                    && !self.erasing
                    && matches!(removal.reason.as_str(), "manager" | "retention") =>
            {
                Ok(removal)
            }
            _ => Err(AppError::FileNotFound),
        }
    }
}

/// The session `session_id` of `guild_id`, locked for the caller's
/// transaction, which holds the guild's lock.
pub(crate) async fn load_lifecycle(
    tx: &mut sqlx::PgConnection,
    guild_id: i64,
    session_id: i64,
) -> Result<Lifecycle, AppError> {
    let row = sqlx::query!(
        r#"SELECT rs.user_id,
                  rs.starting_channel_id,
                  rs.state,
                  ARRAY(SELECT DISTINCT channel_id
                          FROM audio_files
                         WHERE recording_session_id = rs.id) AS "channels!",
                  COALESCE(rs.ended_at < now() - p.retention_days * interval '1 day', FALSE)
                      AS "past_retention!",
                  j.id AS "job_id?",
                  j.reason AS "reason?",
                  j.mode = 'soft' AND j.state = 'soft_deleted' AS "soft?",
                  EXISTS (SELECT 1
                            FROM recording_erasures e
                           WHERE e.recording_session_id = rs.id
                             AND e.state <> 'cancelled') AS "erasing!"
             FROM recording_sessions rs
             LEFT JOIN guild_recording_policy p ON p.guild_id = rs.guild_id
             LEFT JOIN recording_deletion_jobs j ON j.recording_session_id = rs.id
            WHERE rs.guild_id = $1 AND rs.id = $2
              FOR UPDATE OF rs"#,
        guild_id,
        session_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::FileNotFound)?;
    let mut journey = row.channels;
    if journey.is_empty() {
        journey.push(row.starting_channel_id);
    }
    let removal = match (row.job_id, row.reason) {
        (Some(job_id), Some(reason)) => Some(Removal {
            job_id,
            reason,
            soft: row.soft.unwrap_or(false),
        }),
        _ => None,
    };
    Ok(Lifecycle {
        speaker: row.user_id,
        journey,
        finalized: row.state == "finalized",
        past_retention: row.past_retention,
        removal,
        erasing: row.erasing,
    })
}

/// Reverts `removal` of `session_id`: shows the session again, and the
/// clips the removal hid unless something else still hides them. The
/// caller holds the guild's lock and has checked the removal is
/// [`Lifecycle::restorable`].
pub(crate) async fn restore_in(
    tx: &mut sqlx::PgConnection,
    guild_id: i64,
    session_id: i64,
    removal: &Removal,
    restored_by: i64,
    kept: bool,
) -> Result<(), AppError> {
    let job = sqlx::query!(
        "DELETE FROM recording_deletion_jobs WHERE id = $1
         RETURNING reason, requested_by, created_at, hidden_clip_ids",
        removal.job_id
    )
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO recording_restorations
            (id, guild_id, recording_session_id, reason, removed_by, removed_at, restored_by, kept)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        removal.job_id,
        guild_id,
        session_id,
        job.reason,
        job.requested_by,
        job.created_at,
        restored_by,
        kept
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE recording_sessions SET deletion_requested_at = NULL WHERE id = $1",
        session_id
    )
    .execute(&mut *tx)
    .await?;
    // After the session: its clips are covered by it until then.
    if let Some(hidden) = job.hidden_clip_ids {
        restore_clips(tx, guild_id, &hidden).await?;
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/api/admin/guilds/{guild_id}/recordings/{recording_session_id}/restore",
    tag = "admin",
    params(("guild_id" = i64, Path), ("recording_session_id" = i64, Path)),
    responses(
        (status = 204, description = "The recording, and the clips its removal hid, are visible again"),
        (status = 403, description = "Delete recordings required", body = crate::errors::ApiError),
        (status = 404, description = "No removal of this recording to undo: never removed, already restored, being erased by its speaker or purged", body = crate::errors::ApiError),
        (status = 409, description = "Older than the retention period: only keeping it brings it back", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/admin/guilds/{guild_id}/recordings/{recording_session_id}/restore")]
pub async fn restore_recording(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, i64)>,
    token: Option<web::ReqData<crate::auth::Token<crate::auth::Access>>>,
) -> Result<HttpResponse, AppError> {
    let viewer = token.ok_or(AppError::Unauthorized)?.viewer();
    let (guild_id, session_id) = path.into_inner();
    let guild = GuildAccess::load(pool.get_ref(), guild_id, viewer, None).await?;
    guild.require(Capabilities::DELETE_RECORDINGS)?;
    let access = guild.require_member()?;
    let mut tx = pool.begin().await?;
    lock_guild(&mut tx, guild_id).await?;
    let session = load_lifecycle(&mut tx, guild_id, session_id).await?;
    // What a manager could remove: what they may hear, or see redacted.
    let speakers = [session.speaker];
    let journey = session.journey.iter().copied();
    if !access.allows_item(&speakers, journey.clone()) && !access.redacts_item(&speakers, journey) {
        return Err(AppError::FileNotFound);
    }
    let removal = session.restorable()?;
    if session.past_retention {
        return Err(AppError::Conflict(
            "This recording is older than the retention period: keep it to bring it back.".into(),
        ));
    }
    restore_in(
        &mut tx,
        guild_id,
        session_id,
        removal,
        viewer.user_id,
        false,
    )
    .await?;
    tx.commit().await?;
    Ok(HttpResponse::NoContent().finish())
}
