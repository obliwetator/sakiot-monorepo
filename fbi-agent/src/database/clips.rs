use sqlx::{Pool, Postgres};

use crate::database::DbResult;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PlayableClip {
    pub saved_file_name: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClipChoice {
    pub name: String,
    pub clip_id: String,
}

pub async fn playable_clip(
    pool: &Pool<Postgres>,
    guild_id: i64,
    clip_id: &str,
) -> DbResult<Option<PlayableClip>> {
    let row = sqlx::query!(
        "SELECT saved_file_name, name
           FROM clips
          WHERE guild_id = $1
            AND clip_id = $2
            AND deleted_at IS NULL",
        guild_id,
        clip_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|record| PlayableClip {
        display_name: record.name.unwrap_or_else(|| clip_id.to_string()),
        saved_file_name: record
            .saved_file_name
            .unwrap_or_else(|| format!("{}.ogg", clip_id)),
    }))
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

pub async fn autocomplete_clip_choices(
    pool: &Pool<Postgres>,
    guild_id: i64,
    query: &str,
) -> DbResult<Vec<ClipChoice>> {
    let query_wildcard = format!("%{}%", query);

    let rows = sqlx::query!(
        "SELECT name, clip_id
           FROM clips
          WHERE guild_id = $1
            AND name ILIKE $2
            AND deleted_at IS NULL
          LIMIT 25",
        guild_id,
        query_wildcard
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            row.name.map(|name| ClipChoice {
                name,
                clip_id: row.clip_id,
            })
        })
        .collect())
}
