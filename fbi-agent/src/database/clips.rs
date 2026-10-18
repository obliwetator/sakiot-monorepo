use sqlx::{Pool, Postgres};

use crate::database::DbResult;
use crate::permissions::Listening;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PlayableClip {
    pub clip_id: String,
    pub saved_file_name: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClipChoice {
    pub name: String,
    pub clip_id: String,
}

/// The clip `clip_id` (an id, or else a name) of the guild, if `listening`
/// may hear it: it holds only their own voice, or every channel its audio
/// came from is one of theirs.
pub async fn playable_clip(
    pool: &Pool<Postgres>,
    guild_id: i64,
    clip_id: &str,
    listening: &Listening,
) -> DbResult<Option<PlayableClip>> {
    let row = sqlx::query!(
        "SELECT clips.clip_id, clips.saved_file_name, clips.name
           FROM clips
          WHERE clips.guild_id = $1
            AND (clips.clip_id = $2 OR clips.name = $2)
            AND clips.deleted_at IS NULL
            AND ((cardinality(clips.speaker_ids) > 0
                  AND clips.speaker_ids <@ ARRAY[$4::bigint])
                 OR (cardinality(clips.channel_ids) > 0
                     AND clips.channel_ids <@ $3::bigint[]))
          ORDER BY (clips.clip_id = $2) DESC, clips.created_at, clips.clip_id
          LIMIT 1",
        guild_id,
        clip_id,
        &listening.channels,
        listening.own
    )
    .fetch_optional(pool)
    .await?;

    let clip = row
        .map(|record| {
            Ok::<PlayableClip, sqlx::Error>(PlayableClip {
                display_name: record
                    .name
                    .clone()
                    .unwrap_or_else(|| record.clip_id.clone()),
                saved_file_name: record
                    .saved_file_name
                    .clone()
                    .unwrap_or_else(|| format!("{}.ogg", record.clip_id)),
                clip_id: record.clip_id,
            })
        })
        .transpose()?;
    Ok(clip)
}

pub async fn record_jam_invocation(
    pool: &Pool<Postgres>,
    user_id: i64,
    guild_id: i64,
    clip_id: &str,
) -> DbResult<()> {
    sqlx::query!(
        "INSERT INTO jam_invocations (user_id, guild_id, clip_id)
         VALUES ($1, $2, $3)",
        user_id,
        guild_id,
        clip_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn resolve_jam_cooldown(
    pool: &Pool<Postgres>,
    guild_id: i64,
    user_id: i64,
) -> DbResult<i32> {
    let row = sqlx::query!(
        r#"
        SELECT COALESCE(
            (SELECT cooldown_seconds FROM user_jam_cooldown_overrides WHERE guild_id = $1 AND user_id = $2),
            (SELECT cooldown_seconds FROM guild_jam_cooldowns WHERE guild_id = $1),
            0
        ) AS "cooldown_seconds!"
        "#,
        guild_id,
        user_id
    )
    .fetch_one(pool)
    .await?;

    Ok(row.cooldown_seconds)
}

/// Up to 25 of the guild's clips named like `query` that `listening` may
/// hear (`playable_clip`).
pub async fn autocomplete_clip_choices(
    pool: &Pool<Postgres>,
    guild_id: i64,
    query: &str,
    listening: &Listening,
) -> DbResult<Vec<ClipChoice>> {
    let query_wildcard = format!("%{}%", query);

    let rows = sqlx::query!(
        "SELECT clips.name, clips.clip_id
           FROM clips
          WHERE clips.guild_id = $1
            AND clips.name ILIKE $2
            AND clips.deleted_at IS NULL
            AND ((cardinality(clips.speaker_ids) > 0
                  AND clips.speaker_ids <@ ARRAY[$4::bigint])
                 OR (cardinality(clips.channel_ids) > 0
                     AND clips.channel_ids <@ $3::bigint[]))
          ORDER BY clips.name, clips.clip_id
          LIMIT 25",
        guild_id,
        query_wildcard,
        &listening.channels,
        listening.own
    )
    .fetch_all(pool)
    .await?;

    let mut choices = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(name) = row.name {
            choices.push(ClipChoice {
                name,
                clip_id: row.clip_id,
            });
        }
    }
    Ok(choices)
}
