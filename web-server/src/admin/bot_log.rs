//! What others did to the bot in voice, for holders of `manage_settings`. The
//! bot writes it: see `fbi-agent/src/events/voice/bot_log.rs`.

use actix_web::{HttpRequest, HttpResponse, get, web};
use sakiot_permissions::Capabilities;
use serde::Serialize;
use sqlx::{Pool, Postgres};

use crate::errors::AppError;
use crate::permissions::require_capability;

/// How many entries are listed, newest first.
const LIMIT: i64 = 100;

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BotLogKind {
    Deafened,
    /// By someone else, or by the bot clearing a leftover deafen as it joined.
    Undeafened,
    Moved,
    Disconnected,
    /// The bot joined still deafened and could not clear it.
    DeafenClearFailed,
}

impl BotLogKind {
    fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "deafened" => Ok(Self::Deafened),
            "undeafened" => Ok(Self::Undeafened),
            "moved" => Ok(Self::Moved),
            "disconnected" => Ok(Self::Disconnected),
            "deafen_clear_failed" => Ok(Self::DeafenClearFailed),
            _ => Err(AppError::InternalError),
        }
    }
}

/// How far to trust `actor_id`.
#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BotLogAttribution {
    /// Discord's audit log names the bot as the target.
    Exact,
    /// An audit entry of the right kind and age that names no target, which is
    /// how Discord records moves and disconnects.
    Likely,
    /// The audit log had no matching entry.
    Unknown,
    /// The bot may not read the audit log.
    NoAccess,
}

impl BotLogAttribution {
    fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "exact" => Ok(Self::Exact),
            "likely" => Ok(Self::Likely),
            "unknown" => Ok(Self::Unknown),
            "no_access" => Ok(Self::NoAccess),
            _ => Err(AppError::InternalError),
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BotLogEntry {
    pub id: i64,
    pub kind: BotLogKind,
    pub occurred_at_ms: i64,
    /// Null for a channel that no longer exists.
    pub channel_name: Option<String>,
    /// Where a move put the bot; null otherwise, or if it no longer exists.
    pub to_channel_name: Option<String>,
    pub actor_id: Option<String>,
    /// Null for nobody, or a user never seen.
    pub actor_name: Option<String>,
    pub attribution: BotLogAttribution,
}

#[utoipa::path(
    get,
    path = "/api/admin/guilds/{guild_id}/bot-log",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "What others did to the bot in voice, newest first", body = Vec<BotLogEntry>),
        (status = 403, description = "Manage settings required", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/admin/guilds/{guild_id}/bot-log")]
pub async fn get_bot_log(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    require_capability(&req, &pool, guild_id, Capabilities::MANAGE_SETTINGS).await?;
    let entries = sqlx::query!(
        r#"SELECT l.id, l.kind, l.attribution, l.actor_id,
                  (EXTRACT(EPOCH FROM l.occurred_at) * 1000)::bigint AS "occurred_at_ms!",
                  (SELECT c.name FROM channels c
                    WHERE c.guild_id = l.guild_id AND c.channel_id = l.channel_id)
                      AS channel_name,
                  (SELECT c.name FROM channels c
                    WHERE c.guild_id = l.guild_id AND c.channel_id = l.to_channel_id)
                      AS to_channel_name,
                  (SELECT COALESCE(nn.nickname, un.global_name, un.username)
                     FROM user_names un
                     LEFT JOIN user_nicknames nn
                       ON nn.user_id = un.user_id AND nn.guild_id = l.guild_id
                    WHERE un.user_id = l.actor_id) AS actor_name
             FROM bot_voice_log l
            WHERE l.guild_id = $1
            ORDER BY l.occurred_at DESC, l.id DESC
            LIMIT $2"#,
        guild_id,
        LIMIT
    )
    .fetch_all(pool.get_ref())
    .await?
    .into_iter()
    .map(|row| {
        Ok(BotLogEntry {
            id: row.id,
            kind: BotLogKind::parse(&row.kind)?,
            occurred_at_ms: row.occurred_at_ms,
            channel_name: row.channel_name,
            to_channel_name: row.to_channel_name,
            actor_id: row.actor_id.map(|id| id.to_string()),
            actor_name: row.actor_name,
            attribution: BotLogAttribution::parse(&row.attribution)?,
        })
    })
    .collect::<Result<Vec<_>, AppError>>()?;
    Ok(HttpResponse::Ok().json(entries))
}
