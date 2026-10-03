//! Writes for the member and voice presence projections (`guild_members`,
//! `voice_presence`), fenced by `guild_projection_state`.
//!
//! Every write runs in one transaction that first locks the guild's
//! `guild_projection_state` row and checks that this instance owns it at the
//! generation it claimed. The caller then reads serenity's cache and writes
//! that current state, so writes are order-insensitive: whichever lands last
//! stores what the cache holds now. Ownership passes only when a replacement
//! with a complete snapshot claims the guild by bumping the generation; the
//! incumbent, even while draining, keeps writing until then.
//!
//! This is deliberately not the recording-control advisory lock
//! (`pg_advisory_xact_lock(guild_id)`) nor a voice lease: a draining bot keeps
//! its leases, and presence must not wait on recording control.

use crate::cast::ToI64;
use crate::database::DbResult;
use serenity::model::guild::{Guild, Member};
use serenity::model::voice::VoiceState;
use sqlx::PgConnection;

/// One roster row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemberRow {
    pub user_id: i64,
    pub username: String,
    pub global_name: Option<String>,
    pub nickname: Option<String>,
    pub is_bot: bool,
}

impl MemberRow {
    pub fn from_member(member: &Member) -> Self {
        Self {
            user_id: member.user.id.to_i64(),
            username: member.user.name.clone(),
            global_name: member.user.global_name.clone(),
            nickname: member.nick.clone(),
            is_bot: member.user.bot,
        }
    }
}

/// One voice state, for a user who is in a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PresenceRow {
    pub user_id: i64,
    pub channel_id: i64,
    pub self_mute: bool,
    pub self_deaf: bool,
    pub server_mute: bool,
    pub server_deaf: bool,
    pub streaming: bool,
    pub video: bool,
}

impl PresenceRow {
    /// `None` when the user is not in a channel.
    pub fn from_voice_state(state: &VoiceState) -> Option<Self> {
        Some(Self {
            user_id: state.user_id.to_i64(),
            channel_id: state.channel_id?.to_i64(),
            self_mute: state.self_mute,
            self_deaf: state.self_deaf,
            server_mute: state.mute,
            server_deaf: state.deaf,
            streaming: state.self_stream.unwrap_or(false),
            video: state.self_video,
        })
    }
}

/// A guild's projections as one cache read sees them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuildSnapshot {
    pub guild_id: i64,
    pub members: Vec<MemberRow>,
    pub presence: Vec<PresenceRow>,
    /// Whether `members` is the whole roster. Discord sends a large guild's
    /// members only partially until a full chunk set arrives; absence from
    /// an incomplete roster is never a departure.
    pub roster_complete: bool,
}

impl GuildSnapshot {
    pub fn of(guild: &Guild) -> Self {
        let members: Vec<MemberRow> = guild.members.values().map(MemberRow::from_member).collect();
        let roster_complete =
            u64::try_from(members.len()).unwrap_or(u64::MAX) >= guild.member_count;
        Self {
            guild_id: guild.id.to_i64(),
            members,
            presence: guild
                .voice_states
                .values()
                .filter_map(PresenceRow::from_voice_state)
                .collect(),
            roster_complete,
        }
    }
}

/// Claims a guild's projections for this instance at a new generation and
/// returns it. Locks the row until the transaction ends. Only call with a
/// complete snapshot to write in the same transaction (`write_snapshot`).
pub(crate) async fn claim(
    connection: &mut PgConnection,
    guild_id: i64,
    instance_id: &str,
) -> DbResult<i64> {
    let generation = sqlx::query_scalar!(
        "INSERT INTO guild_projection_state
            (guild_id, owner_instance_id, generation, roster_complete_at, presence_synced_at)
         VALUES ($1, $2, 1, now(), now())
         ON CONFLICT (guild_id) DO UPDATE SET
             owner_instance_id = EXCLUDED.owner_instance_id,
             generation = guild_projection_state.generation + 1,
             roster_complete_at = now(),
             presence_synced_at = now(),
             updated_at = now()
         RETURNING generation",
        guild_id,
        instance_id
    )
    .fetch_one(&mut *connection)
    .await?;
    Ok(generation)
}

/// Locks the guild's row and reports whether this instance still owns it at
/// `generation`. On `false` the caller must write nothing.
pub(crate) async fn fence(
    connection: &mut PgConnection,
    guild_id: i64,
    instance_id: &str,
    generation: i64,
) -> DbResult<bool> {
    let owned = sqlx::query_scalar!(
        "SELECT 1 AS \"owned!\"
           FROM guild_projection_state
          WHERE guild_id = $1
            AND owner_instance_id = $2
            AND generation = $3
            FOR UPDATE",
        guild_id,
        instance_id,
        generation
    )
    .fetch_optional(&mut *connection)
    .await?;
    Ok(owned.is_some())
}

/// Writes a whole guild's projections, only the differences. The roster is
/// pruned only when the snapshot is complete. The fence (or claim) must be
/// held.
pub(crate) async fn write_snapshot(
    connection: &mut PgConnection,
    snapshot: &GuildSnapshot,
) -> DbResult<()> {
    upsert_members(connection, snapshot.guild_id, &snapshot.members).await?;
    if snapshot.roster_complete {
        let keep: Vec<i64> = snapshot
            .members
            .iter()
            .map(|member| member.user_id)
            .collect();
        sqlx::query!(
            "DELETE FROM guild_members WHERE guild_id = $1 AND NOT (user_id = ANY($2))",
            snapshot.guild_id,
            &keep
        )
        .execute(&mut *connection)
        .await?;
        sqlx::query!(
            "UPDATE guild_projection_state
                SET roster_complete_at = now(), updated_at = now()
              WHERE guild_id = $1",
            snapshot.guild_id
        )
        .execute(&mut *connection)
        .await?;
    }

    upsert_presence(connection, snapshot.guild_id, &snapshot.presence).await?;
    let in_voice: Vec<i64> = snapshot.presence.iter().map(|row| row.user_id).collect();
    sqlx::query!(
        "DELETE FROM voice_presence WHERE guild_id = $1 AND NOT (user_id = ANY($2))",
        snapshot.guild_id,
        &in_voice
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query!(
        "UPDATE guild_projection_state
            SET presence_synced_at = now(), updated_at = now()
          WHERE guild_id = $1",
        snapshot.guild_id
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

/// One user's current state: their roster row when the cache has them, and
/// their voice state (`None` removes it). The fence must be held.
pub(crate) async fn write_user(
    connection: &mut PgConnection,
    guild_id: i64,
    member: Option<&MemberRow>,
    user_id: i64,
    presence: Option<&PresenceRow>,
) -> DbResult<()> {
    if let Some(member) = member {
        upsert_members(connection, guild_id, std::slice::from_ref(member)).await?;
    }
    match presence {
        Some(row) => upsert_presence(connection, guild_id, std::slice::from_ref(row)).await?,
        None => {
            sqlx::query!(
                "DELETE FROM voice_presence WHERE guild_id = $1 AND user_id = $2",
                guild_id,
                user_id
            )
            .execute(&mut *connection)
            .await?;
        }
    }
    Ok(())
}

/// A member left the guild (a member-remove event, never mere absence from
/// the cache). The fence must be held.
pub(crate) async fn remove_member(
    connection: &mut PgConnection,
    guild_id: i64,
    user_id: i64,
) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM voice_presence WHERE guild_id = $1 AND user_id = $2",
        guild_id,
        user_id
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query!(
        "DELETE FROM guild_members WHERE guild_id = $1 AND user_id = $2",
        guild_id,
        user_id
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

/// A stopping instance gives up the guilds nobody took over, so dashboards
/// show presence as unknown instead of frozen.
pub(crate) async fn release_all(
    pool: &sqlx::Pool<sqlx::Postgres>,
    instance_id: &str,
) -> DbResult<u64> {
    Ok(sqlx::query!(
        "UPDATE guild_projection_state
            SET owner_instance_id = NULL, updated_at = now()
          WHERE owner_instance_id = $1",
        instance_id
    )
    .execute(pool)
    .await?
    .rows_affected())
}

async fn upsert_members(
    connection: &mut PgConnection,
    guild_id: i64,
    members: &[MemberRow],
) -> DbResult<()> {
    if members.is_empty() {
        return Ok(());
    }
    let user_ids: Vec<i64> = members.iter().map(|member| member.user_id).collect();
    let usernames: Vec<String> = members
        .iter()
        .map(|member| member.username.clone())
        .collect();
    let global_names: Vec<Option<String>> = members
        .iter()
        .map(|member| member.global_name.clone())
        .collect();
    let nicknames: Vec<Option<String>> = members
        .iter()
        .map(|member| member.nickname.clone())
        .collect();
    let bots: Vec<bool> = members.iter().map(|member| member.is_bot).collect();
    sqlx::query!(
        "INSERT INTO guild_members (guild_id, user_id, username, global_name, nickname, is_bot)
         SELECT $1, * FROM UNNEST($2::bigint[], $3::text[], $4::text[], $5::text[], $6::bool[])
         ON CONFLICT (guild_id, user_id) DO UPDATE SET
             username = EXCLUDED.username,
             global_name = EXCLUDED.global_name,
             nickname = EXCLUDED.nickname,
             is_bot = EXCLUDED.is_bot
          WHERE (guild_members.username, guild_members.global_name,
                 guild_members.nickname, guild_members.is_bot)
                IS DISTINCT FROM
                (EXCLUDED.username, EXCLUDED.global_name, EXCLUDED.nickname, EXCLUDED.is_bot)",
        guild_id,
        &user_ids,
        &usernames,
        &global_names as &[Option<String>],
        &nicknames as &[Option<String>],
        &bots
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

async fn upsert_presence(
    connection: &mut PgConnection,
    guild_id: i64,
    rows: &[PresenceRow],
) -> DbResult<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let column = |pick: fn(&PresenceRow) -> bool| rows.iter().map(pick).collect::<Vec<bool>>();
    let user_ids: Vec<i64> = rows.iter().map(|row| row.user_id).collect();
    let channel_ids: Vec<i64> = rows.iter().map(|row| row.channel_id).collect();
    sqlx::query!(
        "INSERT INTO voice_presence
            (guild_id, user_id, channel_id, self_mute, self_deaf,
             server_mute, server_deaf, streaming, video)
         SELECT $1, * FROM UNNEST($2::bigint[], $3::bigint[], $4::bool[], $5::bool[],
                                  $6::bool[], $7::bool[], $8::bool[], $9::bool[])
         ON CONFLICT (guild_id, user_id) DO UPDATE SET
             channel_id = EXCLUDED.channel_id,
             self_mute = EXCLUDED.self_mute,
             self_deaf = EXCLUDED.self_deaf,
             server_mute = EXCLUDED.server_mute,
             server_deaf = EXCLUDED.server_deaf,
             streaming = EXCLUDED.streaming,
             video = EXCLUDED.video
          WHERE (voice_presence.channel_id, voice_presence.self_mute, voice_presence.self_deaf,
                 voice_presence.server_mute, voice_presence.server_deaf,
                 voice_presence.streaming, voice_presence.video)
                IS DISTINCT FROM
                (EXCLUDED.channel_id, EXCLUDED.self_mute, EXCLUDED.self_deaf,
                 EXCLUDED.server_mute, EXCLUDED.server_deaf,
                 EXCLUDED.streaming, EXCLUDED.video)",
        guild_id,
        &user_ids,
        &channel_ids,
        &column(|row| row.self_mute),
        &column(|row| row.self_deaf),
        &column(|row| row.server_mute),
        &column(|row| row.server_deaf),
        &column(|row| row.streaming),
        &column(|row| row.video)
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}
