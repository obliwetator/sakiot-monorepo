//! Keeping recordings past retention. A guild keeps as many recordings as
//! the bot's operators allow it ([`crate::operators`]); within that, each
//! member keeps as many as their allowance, which holders of
//! `manage_settings` set ([`crate::admin::keep_allowances`]). Retention
//! never removes a kept recording.
//!
//! For now a member keeps a recording retention has already removed, which
//! brings it back for everyone who may hear it. Any other removal (a
//! manager's, its speaker's erasure) frees the recording's keeps.

use std::collections::HashMap;

use actix_web::{HttpResponse, delete, get, put, web};
use sakiot_permissions::Capabilities;
use serde::{Deserialize, Serialize};
use serde_with::{As, DisplayFromStr};
use sqlx::{Pool, Postgres};

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::permissions::GuildAccess;
use crate::recording_deletion::{load_lifecycle, lock_guild, restore_in};
use crate::snowflake_serde::SnowflakeAsStr;

/// How many recordings each member of a guild may keep.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Allowances {
    pub everyone: Option<i32>,
    pub roles: HashMap<i64, i32>,
    pub users: HashMap<i64, i32>,
}

impl Allowances {
    pub async fn load<'e>(
        executor: impl sqlx::PgExecutor<'e>,
        guild_id: i64,
    ) -> Result<Self, AppError> {
        let rows = sqlx::query!(
            "SELECT subject_kind, subject_id, allowance FROM keep_allowances WHERE guild_id = $1",
            guild_id
        )
        .fetch_all(executor)
        .await?;
        let mut allowances = Self::default();
        for row in rows {
            match (row.subject_kind.as_str(), row.subject_id) {
                ("role", Some(role_id)) => {
                    allowances.roles.insert(role_id, row.allowance);
                }
                ("user", Some(user_id)) => {
                    allowances.users.insert(user_id, row.allowance);
                }
                _ => allowances.everyone = Some(row.allowance),
            }
        }
        Ok(allowances)
    }

    /// How many recordings `user_id`, holding `roles`, may keep: their own
    /// allowance, else the largest of their roles', else @everyone's.
    /// `None` when only the guild's cap limits them.
    pub fn of(&self, user_id: i64, roles: &[i64]) -> Option<i32> {
        self.users
            .get(&user_id)
            .copied()
            .or_else(|| {
                roles
                    .iter()
                    .filter_map(|role_id| self.roles.get(role_id).copied())
                    .max()
            })
            .or(self.everyone)
    }
}

/// A guild's keeping, and one member's part in it.
struct Usage {
    /// How many recordings the guild may keep.
    cap: i32,
    /// How many it keeps, each once however many members keep it.
    kept: i64,
    /// How many the member keeps.
    used: i64,
    retention_days: Option<i32>,
}

async fn usage<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    guild_id: i64,
    user_id: i64,
) -> Result<Usage, AppError> {
    Ok(sqlx::query_as!(
        Usage,
        r#"SELECT COALESCE((SELECT kept_recordings FROM guild_limits WHERE guild_id = $1), 0)
                      AS "cap!",
                  (SELECT count(DISTINCT recording_session_id)
                     FROM kept_recordings
                    WHERE guild_id = $1) AS "kept!",
                  (SELECT count(*)
                     FROM kept_recordings
                    WHERE guild_id = $1 AND user_id = $2) AS "used!",
                  (SELECT retention_days
                     FROM guild_recording_policy
                    WHERE guild_id = $1) AS retention_days"#,
        guild_id,
        user_id
    )
    .fetch_one(executor)
    .await?)
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Keeping {
    /// How many recordings the server may keep, as the bot's operators set
    /// it.
    pub cap: i32,
    /// How many it keeps, each once however many members keep it.
    pub kept: i64,
    /// How many the viewer may keep; absent when only the server's cap
    /// limits them.
    pub allowance: Option<i32>,
    /// How many the viewer keeps.
    pub used: i64,
    /// After how many days retention removes a recording; absent when it
    /// never does.
    pub retention_days: Option<i32>,
    /// The kept recordings the viewer may hear, newest first, and the
    /// viewer's own keeps they may no longer hear (`audible: false`).
    pub recordings: Vec<KeptRecording>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct KeptRecording {
    #[serde(with = "SnowflakeAsStr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub recording_session_id: i64,
    /// Whether the viewer may still hear it. When not, it is one of their
    /// own keeps, and nothing else about it is told.
    pub audible: bool,
    #[serde(
        with = "As::<Option<DisplayFromStr>>",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, example = "146638124288704513")]
    pub speaker_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_name: Option<String>,
    /// The channel it started in.
    #[serde(
        with = "As::<Option<DisplayFromStr>>",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, example = "146638124288704513")]
    pub channel_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<i64>,
    /// The viewer keeps it.
    pub mine: bool,
    /// How many members keep it.
    pub keepers: i64,
    /// When the viewer kept it, or else when it was first kept.
    pub kept_at_ms: i64,
}

fn token_viewer(
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<crate::permissions::Viewer, AppError> {
    Ok(token.ok_or(AppError::Unauthorized)?.viewer())
}

#[utoipa::path(
    get,
    path = "/api/guilds/{guild_id}/kept-recordings",
    tag = "keeping",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "How many recordings the server and the viewer may keep and keep, and the kept recordings the viewer may hear", body = Keeping),
        (status = 403, description = "Not a member or former member", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/guilds/{guild_id}/kept-recordings")]
pub async fn get_kept_recordings(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let viewer = token_viewer(token)?;
    let guild_id = path.into_inner();
    let guild = GuildAccess::load(pool.get_ref(), guild_id, viewer, None).await?;
    let access = guild.require_member_or_former()?;
    let allowance = Allowances::load(pool.get_ref(), guild_id)
        .await?
        .of(viewer.user_id, guild.roles());
    let usage = usage(pool.get_ref(), guild_id, viewer.user_id).await?;
    let rows = sqlx::query!(
        r#"SELECT rs.id,
                  rs.user_id,
                  rs.starting_channel_id,
                  ARRAY(SELECT DISTINCT channel_id
                          FROM audio_files
                         WHERE recording_session_id = rs.id) AS "channels!",
                  (EXTRACT(EPOCH FROM rs.started_at) * 1000)::bigint AS "started_at_ms!",
                  (EXTRACT(EPOCH FROM rs.ended_at) * 1000)::bigint AS ended_at_ms,
                  (SELECT COALESCE(nn.nickname, un.global_name, un.username)
                     FROM user_names un
                     LEFT JOIN user_nicknames nn
                       ON nn.user_id = un.user_id AND nn.guild_id = rs.guild_id
                    WHERE un.user_id = rs.user_id) AS speaker_name,
                  (SELECT c.name FROM channels c
                    WHERE c.channel_id = rs.starting_channel_id) AS channel_name,
                  count(*) AS "keepers!",
                  bool_or(k.user_id = $2) AS "mine!",
                  (EXTRACT(EPOCH FROM COALESCE(max(k.kept_at) FILTER (WHERE k.user_id = $2),
                                               min(k.kept_at))) * 1000)::bigint AS "kept_at_ms!"
             FROM kept_recordings k
             JOIN recording_sessions rs ON rs.id = k.recording_session_id
            WHERE k.guild_id = $1
            GROUP BY rs.id
            ORDER BY rs.started_at DESC, rs.id DESC"#,
        guild_id,
        viewer.user_id
    )
    .fetch_all(pool.get_ref())
    .await?;
    let recordings = rows
        .into_iter()
        .filter_map(|row| {
            let journey = journey(row.channels, row.starting_channel_id);
            let audible = access.allows_item(&[row.user_id], journey);
            if !audible && !row.mine {
                return None;
            }
            Some(KeptRecording {
                recording_session_id: row.id,
                audible,
                speaker_id: audible.then_some(row.user_id),
                speaker_name: row.speaker_name.filter(|_| audible),
                channel_id: audible.then_some(row.starting_channel_id),
                channel_name: row.channel_name.filter(|_| audible),
                started_at_ms: audible.then_some(row.started_at_ms),
                ended_at_ms: row.ended_at_ms.filter(|_| audible),
                mine: row.mine,
                keepers: row.keepers,
                kept_at_ms: row.kept_at_ms,
            })
        })
        .collect();
    Ok(HttpResponse::Ok().json(Keeping {
        cap: usage.cap,
        kept: usage.kept,
        allowance,
        used: usage.used,
        retention_days: usage.retention_days,
        recordings,
    }))
}

/// The channels a session's audio was recorded in, or the one it started
/// in before it has any.
fn journey(channels: Vec<i64>, starting_channel_id: i64) -> Vec<i64> {
    if channels.is_empty() {
        vec![starting_channel_id]
    } else {
        channels
    }
}

#[utoipa::path(
    put,
    path = "/api/users/current/guilds/{guild_id}/kept-recordings/{recording_session_id}",
    tag = "keeping",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("recording_session_id" = i64, Path, description = "A recording the viewer may hear"),
    ),
    responses(
        (status = 204, description = "Kept: retention leaves it be, and if retention had removed it, it is back for everyone who may hear it; keeping it again changes nothing"),
        (status = 403, description = "Not a member", body = crate::errors::ApiError),
        (status = 404, description = "No such recording the viewer may keep: missing, beyond their hearing, or removed other than by retention", body = crate::errors::ApiError),
        (status = 409, description = "Still recording, or the viewer's allowance or the server's cap is used up", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[put("/users/current/guilds/{guild_id}/kept-recordings/{recording_session_id}")]
pub async fn keep_recording(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, i64)>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let viewer = token_viewer(token)?;
    let (guild_id, session_id) = path.into_inner();
    let guild = GuildAccess::load(pool.get_ref(), guild_id, viewer, None).await?;
    keep(pool.get_ref(), &guild, session_id).await?;
    Ok(HttpResponse::NoContent().finish())
}

/// Keeps `session_id` for the member whose access is `guild`, restoring it
/// if retention removed it.
async fn keep(pool: &Pool<Postgres>, guild: &GuildAccess, session_id: i64) -> Result<(), AppError> {
    let access = guild.require_member()?;
    let user_id = access.user_id.ok_or(AppError::Forbidden)?;
    let manager = guild.holds(Capabilities::DELETE_RECORDINGS)?;
    let guild_id = guild.guild_id();
    let mut tx = pool.begin().await?;
    // Serialized with removals and with the retention sweep.
    lock_guild(&mut tx, guild_id).await?;
    let session = load_lifecycle(&mut tx, guild_id, session_id).await?;
    let speakers = [session.speaker];
    let journey = session.journey.iter().copied();
    // A manager keeps what they could remove: what they see redacted too.
    if !access.allows_item(&speakers, journey.clone()) && !access.redacts_item(&speakers, journey) {
        return Err(AppError::FileNotFound);
    }
    let removal = if session.removed() {
        let removal = session.restorable()?;
        // A manager's removal is for managers to undo.
        if removal.reason != "retention" && !manager {
            return Err(AppError::FileNotFound);
        }
        Some(removal)
    } else {
        None
    };
    if !session.finalized {
        return Err(AppError::Conflict(
            "Only finished recordings can be kept.".into(),
        ));
    }
    let keepers = sqlx::query_scalar!(
        "SELECT user_id FROM kept_recordings WHERE recording_session_id = $1",
        session_id
    )
    .fetch_all(&mut *tx)
    .await?;
    if keepers.contains(&user_id) {
        return Ok(());
    }
    let usage = usage(&mut *tx, guild_id, user_id).await?;
    let allowance = Allowances::load(&mut *tx, guild_id)
        .await?
        .of(user_id, guild.roles());
    if let Some(allowance) = allowance
        && usage.used >= i64::from(allowance)
    {
        return Err(AppError::Conflict(match allowance {
            0 => "You may not keep recordings in this server.".into(),
            _ => format!(
                "You already keep {} recordings, all you may keep in this server.",
                usage.used
            ),
        }));
    }
    // Another member's keep already counts it.
    if keepers.is_empty() && usage.kept >= i64::from(usage.cap) {
        return Err(AppError::Conflict(match usage.cap {
            0 => "This server may not keep recordings yet: the bot's operators decide how many."
                .into(),
            cap => format!("This server already keeps all {cap} recordings it may keep."),
        }));
    }
    if let Some(removal) = removal {
        restore_in(&mut tx, guild_id, session_id, removal, user_id, true).await?;
    }
    sqlx::query!(
        "INSERT INTO kept_recordings (recording_session_id, user_id, guild_id) VALUES ($1, $2, $3)",
        session_id,
        user_id,
        guild_id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[utoipa::path(
    delete,
    path = "/api/users/current/guilds/{guild_id}/kept-recordings/{recording_session_id}",
    tag = "keeping",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("recording_session_id" = i64, Path, description = "A recording the viewer keeps"),
    ),
    responses(
        (status = 204, description = "No longer kept by the viewer; retention removes it once nobody keeps it, if it is old enough"),
        (status = 403, description = "Not a member or former member", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[delete("/users/current/guilds/{guild_id}/kept-recordings/{recording_session_id}")]
pub async fn release_recording(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, i64)>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let viewer = token_viewer(token)?;
    let (guild_id, session_id) = path.into_inner();
    GuildAccess::load(pool.get_ref(), guild_id, viewer, None)
        .await?
        .require_member_or_former()?;
    sqlx::query!(
        "DELETE FROM kept_recordings
          WHERE guild_id = $1 AND recording_session_id = $2 AND user_id = $3",
        guild_id,
        session_id,
        viewer.user_id
    )
    .execute(pool.get_ref())
    .await?;
    Ok(HttpResponse::NoContent().finish())
}

#[utoipa::path(
    delete,
    path = "/api/admin/guilds/{guild_id}/kept-recordings/{recording_session_id}",
    tag = "admin",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("recording_session_id" = i64, Path, description = "A kept recording"),
    ),
    responses(
        (status = 204, description = "Nobody keeps it any more; retention removes it if it is old enough"),
        (status = 403, description = "Delete recordings required", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[delete("/admin/guilds/{guild_id}/kept-recordings/{recording_session_id}")]
pub async fn release_recording_for_everyone(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<(i64, i64)>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let viewer = token_viewer(token)?;
    let (guild_id, session_id) = path.into_inner();
    let guild = GuildAccess::load(pool.get_ref(), guild_id, viewer, None).await?;
    guild.require_member()?;
    guild.require(Capabilities::DELETE_RECORDINGS)?;
    sqlx::query!(
        "DELETE FROM kept_recordings WHERE guild_id = $1 AND recording_session_id = $2",
        guild_id,
        session_id
    )
    .execute(pool.get_ref())
    .await?;
    Ok(HttpResponse::NoContent().finish())
}

/// How many removed recordings one page lists.
const REMOVED_PAGE: usize = 50;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct RemovedQuery {
    /// The previous page's `next`.
    pub before: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RemovedRecordings {
    /// Most recently removed first.
    pub recordings: Vec<RemovedRecording>,
    /// Pass as `before` for the next page; absent on the last.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RemovalReason {
    Manager,
    Retention,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RemovedRecording {
    #[serde(with = "SnowflakeAsStr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub recording_session_id: i64,
    #[serde(with = "SnowflakeAsStr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub speaker_id: i64,
    pub speaker_name: Option<String>,
    /// The channel it started in.
    #[serde(with = "SnowflakeAsStr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub channel_id: i64,
    pub channel_name: Option<String>,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub removed_at_ms: i64,
    pub reason: RemovalReason,
    /// The manager who removed it; absent for retention.
    #[serde(
        with = "As::<Option<DisplayFromStr>>",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, example = "146638124288704513")]
    pub removed_by: Option<i64>,
    /// Its speaker keeps their recordings private: the viewer, a manager,
    /// may bring it back but not hear it.
    pub redacted: bool,
    /// Older than the retention period: only keeping it brings it back.
    pub past_retention: bool,
    /// The viewer may bring it back without keeping it: a manager, while it
    /// is within the retention period.
    pub restorable: bool,
}

/// The removed recordings the viewer may bring back: those retention
/// removed that they may hear, and for a holder of `delete_recordings`
/// managers' removals too, including those they see redacted. Never one its
/// speaker is erasing.
#[utoipa::path(
    get,
    path = "/api/guilds/{guild_id}/removed-recordings",
    tag = "keeping",
    params(("guild_id" = i64, Path, description = "Discord guild id"), RemovedQuery),
    responses(
        (status = 200, description = "One page of removed recordings the viewer may keep or restore", body = RemovedRecordings),
        (status = 400, description = "Invalid `before`", body = crate::errors::ApiError),
        (status = 403, description = "Not a member", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/guilds/{guild_id}/removed-recordings")]
pub async fn get_removed_recordings(
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    query: web::Query<RemovedQuery>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let viewer = token_viewer(token)?;
    let guild_id = path.into_inner();
    let guild = GuildAccess::load(pool.get_ref(), guild_id, viewer, None).await?;
    let access = guild.require_member()?;
    let manager = guild.holds(Capabilities::DELETE_RECORDINGS)?;
    let before = query.before.as_deref().map(parse_cursor).transpose()?;
    let (before_ms, before_id) = before.unzip();
    let rows = sqlx::query!(
        r#"SELECT rs.id,
                  rs.user_id,
                  rs.starting_channel_id,
                  ARRAY(SELECT DISTINCT channel_id
                          FROM audio_files
                         WHERE recording_session_id = rs.id) AS "channels!",
                  (EXTRACT(EPOCH FROM rs.started_at) * 1000)::bigint AS "started_at_ms!",
                  (EXTRACT(EPOCH FROM rs.ended_at) * 1000)::bigint AS ended_at_ms,
                  (EXTRACT(EPOCH FROM j.created_at) * 1000)::bigint AS "removed_at_ms!",
                  j.reason,
                  j.requested_by,
                  COALESCE(rs.ended_at < now() - p.retention_days * interval '1 day', FALSE)
                      AS "past_retention!",
                  (SELECT COALESCE(nn.nickname, un.global_name, un.username)
                     FROM user_names un
                     LEFT JOIN user_nicknames nn
                       ON nn.user_id = un.user_id AND nn.guild_id = rs.guild_id
                    WHERE un.user_id = rs.user_id) AS speaker_name,
                  (SELECT c.name FROM channels c
                    WHERE c.channel_id = rs.starting_channel_id) AS channel_name
             FROM recording_deletion_jobs j
             JOIN recording_sessions rs ON rs.id = j.recording_session_id
             LEFT JOIN guild_recording_policy p ON p.guild_id = rs.guild_id
            WHERE j.guild_id = $1
              AND j.mode = 'soft' AND j.state = 'soft_deleted'
              AND (j.reason = 'retention' OR ($2 AND j.reason = 'manager'))
              AND NOT EXISTS (SELECT 1
                                FROM recording_erasures e
                               WHERE e.recording_session_id = rs.id
                                 AND e.state <> 'cancelled')
              AND ($3::bigint IS NULL
                   OR ((EXTRACT(EPOCH FROM j.created_at) * 1000)::bigint, rs.id) < ($3, $4))
            ORDER BY "removed_at_ms!" DESC, rs.id DESC"#,
        guild_id,
        manager,
        before_ms,
        before_id
    )
    .fetch_all(pool.get_ref())
    .await?;
    let mut recordings: Vec<RemovedRecording> = rows
        .into_iter()
        .filter_map(|row| {
            let journey = journey(row.channels, row.starting_channel_id);
            let speakers = [row.user_id];
            let redacted = !access.allows_item(&speakers, journey.iter().copied());
            if redacted && !access.redacts_item(&speakers, journey) {
                return None;
            }
            let reason = match row.reason.as_str() {
                "manager" => RemovalReason::Manager,
                _ => RemovalReason::Retention,
            };
            Some(RemovedRecording {
                recording_session_id: row.id,
                speaker_id: row.user_id,
                speaker_name: row.speaker_name,
                channel_id: row.starting_channel_id,
                channel_name: row.channel_name,
                started_at_ms: row.started_at_ms,
                ended_at_ms: row.ended_at_ms,
                removed_at_ms: row.removed_at_ms,
                reason,
                removed_by: row.requested_by,
                redacted,
                past_retention: row.past_retention,
                restorable: manager && !row.past_retention,
            })
        })
        .take(REMOVED_PAGE + 1)
        .collect();
    let next = if recordings.len() > REMOVED_PAGE {
        recordings.truncate(REMOVED_PAGE);
        recordings
            .last()
            .map(|last| format!("{}.{}", last.removed_at_ms, last.recording_session_id))
    } else {
        None
    };
    Ok(HttpResponse::Ok().json(RemovedRecordings { recordings, next }))
}

/// A `next`: when the last listed recording was removed, and its id.
fn parse_cursor(cursor: &str) -> Result<(i64, i64), AppError> {
    cursor
        .split_once('.')
        .and_then(|(ms, id)| Some((ms.parse().ok()?, id.parse().ok()?)))
        .ok_or_else(|| AppError::InvalidParam("before".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_s_own_allowance_beats_their_roles_whose_best_beats_everyone_s() {
        let allowances = Allowances {
            everyone: Some(2),
            roles: HashMap::from([(10, 5), (11, 1)]),
            users: HashMap::from([(7, 0)]),
        };
        // Their own, even below everything else.
        assert_eq!(allowances.of(7, &[10]), Some(0));
        // The best of their roles that have one, even below @everyone's.
        assert_eq!(allowances.of(8, &[10, 11]), Some(5));
        assert_eq!(allowances.of(8, &[11]), Some(1));
        // @everyone's, for roles without one.
        assert_eq!(allowances.of(8, &[12]), Some(2));
        // Nothing set: only the guild's cap.
        assert_eq!(Allowances::default().of(8, &[10]), None);
    }

    #[test]
    fn cursors_name_a_removal_time_and_a_session() {
        assert_eq!(
            parse_cursor("1700000000000.42").ok(),
            Some((1700000000000, 42))
        );
        assert!(parse_cursor("1700000000000").is_err());
        assert!(parse_cursor("a.b").is_err());
    }
}
