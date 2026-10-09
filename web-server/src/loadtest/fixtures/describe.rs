//! Describing a seeded fixture for load-test clients.

use super::*;

#[derive(Serialize)]
pub(super) struct Catalog {
    state: &'static str,
    fixture: u32,
    guild_id: String,
    owner_id: String,
    channels: Vec<ChannelEntry>,
    /// Every member, insiders and moderators included.
    members: Vec<String>,
    insiders: Vec<String>,
    moderators: Vec<String>,
    sessions: Vec<SessionEntry>,
    clips: Vec<ClipEntry>,
    live: Vec<super::super::live::SimSummary>,
}

#[derive(Serialize)]
struct ChannelEntry {
    id: String,
    name: String,
    restricted: bool,
}

#[derive(Serialize)]
struct SessionEntry {
    id: String,
    user_id: String,
    channel_id: String,
    state: String,
    audio_file_id: String,
    file_name: String,
    year: i32,
    month: i32,
    start_ms: i64,
    end_ms: Option<i64>,
    restricted: bool,
}

#[derive(Serialize)]
struct ClipEntry {
    clip_id: String,
    channel_id: String,
    recording_session_id: Option<String>,
    restricted: bool,
}

pub(super) async fn describe(
    state: &LoadtestState,
    fixture: u32,
) -> Result<Option<Catalog>, AppError> {
    let guild_id = ids::guild_id(fixture);
    let owner: Option<i64> = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = $1")
        .bind(guild_id)
        .fetch_optional(&state.pool)
        .await?;
    let Some(owner) = owner else {
        return Ok(None);
    };

    let channel_rows: Vec<(i64, Option<String>, bool)> = sqlx::query_as(
        "SELECT c.channel_id, c.name,
                EXISTS (SELECT 1 FROM channel_permissions p
                         WHERE p.channel_id = c.channel_id AND p.target_id = c.guild_id
                           AND p.kind = 'role' AND (p.deny & $2) <> 0)
           FROM channels c WHERE c.guild_id = $1 ORDER BY c.channel_id",
    )
    .bind(guild_id)
    .bind(VIEW_CHANNEL)
    .fetch_all(&state.pool)
    .await?;
    let restricted: HashSet<i64> = channel_rows
        .iter()
        .filter(|(_, _, restricted)| *restricted)
        .map(|(id, _, _)| *id)
        .collect();

    let members: Vec<i64> = sqlx::query_scalar(
        "SELECT user_id FROM guild_members WHERE guild_id = $1 ORDER BY user_id",
    )
    .bind(guild_id)
    .fetch_all(&state.pool)
    .await?;
    let with_role = |role: i64| {
        sqlx::query_scalar::<_, i64>(
            "SELECT user_id FROM user_roles WHERE role_id = $1 ORDER BY user_id",
        )
        .bind(role)
        .fetch_all(&state.pool)
    };
    let insiders = with_role(ids::role_id(guild_id, INSIDERS)).await?;
    let moderators = with_role(ids::role_id(guild_id, MODERATORS)).await?;

    type SessionRow = (
        i64,
        i64,
        String,
        i64,
        String,
        i64,
        i32,
        i32,
        Option<i64>,
        Option<i64>,
    );
    let session_rows: Vec<SessionRow> = sqlx::query_as(
        "SELECT rs.id, rs.user_id, rs.state, af.id, af.file_name, af.channel_id, af.year,
                af.month, af.start_ts, af.end_ts
           FROM recording_sessions rs
           JOIN audio_files af ON af.recording_session_id = rs.id
          WHERE rs.guild_id = $1 AND rs.deletion_requested_at IS NULL AND af.reaped IS FALSE
          ORDER BY rs.started_at DESC, af.segment_index",
    )
    .bind(guild_id)
    .fetch_all(&state.pool)
    .await?;
    let sessions = session_rows
        .into_iter()
        .map(
            |(
                id,
                user_id,
                state,
                audio_file_id,
                file_name,
                channel_id,
                year,
                month,
                start,
                end,
            )| {
                SessionEntry {
                    id: id.to_string(),
                    user_id: user_id.to_string(),
                    channel_id: channel_id.to_string(),
                    state,
                    audio_file_id: audio_file_id.to_string(),
                    file_name,
                    year,
                    month,
                    start_ms: start.unwrap_or_default(),
                    end_ms: end,
                    restricted: restricted.contains(&channel_id),
                }
            },
        )
        .collect();

    let clip_rows: Vec<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT clip_id, channel_id, recording_session_id FROM clips
          WHERE guild_id = $1 AND deleted_at IS NULL ORDER BY created_at DESC",
    )
    .bind(guild_id)
    .fetch_all(&state.pool)
    .await?;
    let clips = clip_rows
        .into_iter()
        .map(|(clip_id, channel_id, session_id)| ClipEntry {
            clip_id,
            channel_id: channel_id.unwrap_or_default().to_string(),
            recording_session_id: session_id.map(|id| id.to_string()),
            restricted: channel_id.is_some_and(|id| restricted.contains(&id)),
        })
        .collect();

    let strings = |ids: Vec<i64>| ids.into_iter().map(|id| id.to_string()).collect();
    Ok(Some(Catalog {
        state: "ready",
        fixture,
        guild_id: guild_id.to_string(),
        owner_id: owner.to_string(),
        channels: channel_rows
            .into_iter()
            .map(|(id, name, restricted)| ChannelEntry {
                id: id.to_string(),
                name: name.unwrap_or_default(),
                restricted,
            })
            .collect(),
        members: strings(members),
        insiders: strings(insiders),
        moderators: strings(moderators),
        sessions,
        clips,
        live: state.sims.summaries(guild_id),
    }))
}
