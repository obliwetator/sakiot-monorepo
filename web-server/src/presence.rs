//! Who is in which voice or stage channel, from the `voice_presence`
//! projection the bot maintains.

use std::collections::HashMap;

use actix_web::{HttpResponse, get, web};
use serde::Serialize;
use serde_with::{As, DisplayFromStr};
use sqlx::{Pool, Postgres};

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::permissions::{
    AsRoleQuery, Viewer, begin_snapshot, presence_channels, require_role_preview,
};

type DisplayFromstr = As<DisplayFromStr>;

#[derive(Serialize, Debug, Clone, PartialEq, Eq, utoipa::ToSchema)]
pub struct PresenceMember {
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub user_id: i64,
    /// Nickname, display name or username, whichever is set first.
    pub name: Option<String>,
    pub is_bot: bool,
    pub self_mute: bool,
    pub self_deaf: bool,
    pub server_mute: bool,
    pub server_deaf: bool,
    pub streaming: bool,
    pub video: bool,
}

#[derive(Serialize, Debug, PartialEq, Eq, utoipa::ToSchema)]
pub struct PresenceChannel {
    #[serde(with = "DisplayFromstr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub channel_id: i64,
    pub name: String,
    pub members: Vec<PresenceMember>,
}

#[derive(Serialize, Debug, PartialEq, Eq, utoipa::ToSchema)]
pub struct VoicePresence {
    /// False while no running bot keeps presence current (during a first
    /// rollout, or after the bot stopped): presence is unknown, not empty.
    pub available: bool,
    /// Occupied channels the viewer can view, by name. Empty when not
    /// `available`.
    pub channels: Vec<PresenceChannel>,
}

/// Where one member is in voice, as the voice-presence list shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seat {
    pub channel_id: i64,
    pub channel_name: String,
    pub member: PresenceMember,
}

/// Where each of `members` (guild, user) is in voice now. A member who is not
/// in voice, or whose channel the list would not show, has no entry. Realtime
/// routes presence changes with it, so it reads what `voice_presence` lists.
pub async fn seats(
    pool: &Pool<Postgres>,
    members: &[(i64, i64)],
) -> Result<HashMap<(i64, i64), Seat>, sqlx::Error> {
    if members.is_empty() {
        return Ok(HashMap::new());
    }
    let (guild_ids, user_ids): (Vec<i64>, Vec<i64>) = members.iter().copied().unzip();
    let rows = sqlx::query!(
        r#"SELECT vp.guild_id,
                  vp.channel_id,
                  COALESCE(c.name, '') AS "channel_name!",
                  vp.user_id,
                  COALESCE(gm.nickname, gm.global_name, gm.username,
                           un.global_name, un.username) AS name,
                  COALESCE(gm.is_bot, false) AS "is_bot!",
                  vp.self_mute, vp.self_deaf, vp.server_mute, vp.server_deaf,
                  vp.streaming, vp.video
             FROM unnest($1::bigint[], $2::bigint[]) AS wanted(guild_id, user_id)
             JOIN voice_presence vp
               ON vp.guild_id = wanted.guild_id AND vp.user_id = wanted.user_id
             JOIN channels c ON c.channel_id = vp.channel_id AND c.guild_id = vp.guild_id
             LEFT JOIN guild_members gm
                    ON gm.guild_id = vp.guild_id AND gm.user_id = vp.user_id
             LEFT JOIN user_names un ON un.user_id = vp.user_id"#,
        &guild_ids,
        &user_ids
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                (row.guild_id, row.user_id),
                Seat {
                    channel_id: row.channel_id,
                    channel_name: row.channel_name,
                    member: PresenceMember {
                        user_id: row.user_id,
                        name: row.name,
                        is_bot: row.is_bot,
                        self_mute: row.self_mute,
                        self_deaf: row.self_deaf,
                        server_mute: row.server_mute,
                        server_deaf: row.server_deaf,
                        streaming: row.streaming,
                        video: row.video,
                    },
                },
            )
        })
        .collect())
}

/// Presence for the channels the viewer (or the previewed role) can view,
/// read in one snapshot with that visibility.
pub async fn voice_presence(
    pool: &Pool<Postgres>,
    guild_id: i64,
    viewer: Viewer,
    as_role: Option<i64>,
) -> Result<VoicePresence, AppError> {
    let mut snapshot = begin_snapshot(pool).await?;
    let channels: Vec<i64> = presence_channels(&mut snapshot, guild_id, viewer, as_role)
        .await?
        .into_iter()
        .collect();

    // Known only while the owning instance is alive: a crashed or stopped
    // owner leaves rows that no longer change.
    let available = sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1
                 FROM guild_projection_state s
                 JOIN bot_instances b ON b.instance_id = s.owner_instance_id
                WHERE s.guild_id = $1
                  AND s.presence_synced_at IS NOT NULL
                  AND b.state <> 'stopped'
                  AND b.heartbeat_at > now() - interval '120 seconds'
           ) AS "available!""#,
        guild_id
    )
    .fetch_one(&mut *snapshot)
    .await?;
    if !available {
        snapshot.commit().await?;
        return Ok(VoicePresence {
            available,
            channels: Vec::new(),
        });
    }

    let rows = sqlx::query!(
        r#"SELECT vp.channel_id,
                  COALESCE(c.name, '') AS "channel_name!",
                  vp.user_id,
                  COALESCE(gm.nickname, gm.global_name, gm.username,
                           un.global_name, un.username) AS name,
                  COALESCE(gm.is_bot, false) AS "is_bot!",
                  vp.self_mute, vp.self_deaf, vp.server_mute, vp.server_deaf,
                  vp.streaming, vp.video
             FROM voice_presence vp
             JOIN channels c ON c.channel_id = vp.channel_id AND c.guild_id = vp.guild_id
             LEFT JOIN guild_members gm
                    ON gm.guild_id = vp.guild_id AND gm.user_id = vp.user_id
             LEFT JOIN user_names un ON un.user_id = vp.user_id
            WHERE vp.guild_id = $1
              AND vp.channel_id = ANY($2)
            ORDER BY lower(c.name), vp.channel_id,
                     lower(COALESCE(gm.nickname, gm.global_name, gm.username,
                                    un.global_name, un.username)) NULLS LAST,
                     vp.user_id"#,
        guild_id,
        &channels
    )
    .fetch_all(&mut *snapshot)
    .await?;
    snapshot.commit().await?;

    let mut channels: Vec<PresenceChannel> = Vec::new();
    for row in rows {
        let member = PresenceMember {
            user_id: row.user_id,
            name: row.name,
            is_bot: row.is_bot,
            self_mute: row.self_mute,
            self_deaf: row.self_deaf,
            server_mute: row.server_mute,
            server_deaf: row.server_deaf,
            streaming: row.streaming,
            video: row.video,
        };
        match channels.last_mut() {
            Some(channel) if channel.channel_id == row.channel_id => channel.members.push(member),
            _ => channels.push(PresenceChannel {
                channel_id: row.channel_id,
                name: row.channel_name,
                members: vec![member],
            }),
        }
    }
    Ok(VoicePresence {
        available,
        channels,
    })
}

#[utoipa::path(
    get,
    path = "/api/current/{guild_id}/voice-presence",
    tag = "audio",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("as_role" = i64, Query, description = "Impersonate a guild role (managers only)"),
    ),
    responses(
        (status = 200, description = "Who is in the voice and stage channels the viewer can view", body = VoicePresence),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "Not a member, or a role preview by a non-manager", body = crate::errors::ApiError),
        (status = 404, description = "Role does not exist in this guild", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/current/{guild_id}/voice-presence")]
pub async fn get_voice_presence(
    req: actix_web::HttpRequest,
    query: web::Query<AsRoleQuery>,
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let guild_id = path.into_inner();
    require_role_preview(&req, &pool, guild_id, query.as_role).await?;
    let presence = voice_presence(&pool, guild_id, Viewer::of(&token), query.as_role).await?;
    Ok(HttpResponse::Ok().json(presence))
}
