use sqlx::{Pool, Postgres, Row};

use crate::database::DbResult;

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

pub async fn playable_clip(
    pool: &Pool<Postgres>,
    guild_id: i64,
    clip_id: &str,
    visible_channel_ids: &[i64],
) -> DbResult<Option<PlayableClip>> {
    let row = sqlx::query(
        "SELECT clips.clip_id, clips.saved_file_name, clips.name
           FROM clips
          CROSS JOIN LATERAL (
              SELECT CASE
                  WHEN clips.recording_session_id IS NOT NULL THEN COALESCE(
                      (
                          SELECT array_agg(DISTINCT audio_files.channel_id ORDER BY audio_files.channel_id)
                            FROM audio_files
                           WHERE audio_files.recording_session_id = clips.recording_session_id
                      ),
                      (
                          SELECT ARRAY[recording_sessions.starting_channel_id]::bigint[]
                            FROM recording_sessions
                           WHERE recording_sessions.id = clips.recording_session_id
                      ),
                      ARRAY[]::bigint[]
                  )
                  WHEN clips.channel_id IS NOT NULL THEN ARRAY[clips.channel_id]::bigint[]
                  ELSE ARRAY[]::bigint[]
              END AS channel_ids
          ) source
          WHERE clips.guild_id = $1
            AND (clips.clip_id = $2 OR clips.name = $2)
            AND clips.deleted_at IS NULL
            AND cardinality(source.channel_ids) > 0
            AND source.channel_ids <@ $3::bigint[]
          ORDER BY (clips.clip_id = $2) DESC, clips.created_at, clips.clip_id
          LIMIT 1",
    )
    .bind(guild_id)
    .bind(clip_id)
    .bind(visible_channel_ids)
    .fetch_optional(pool)
    .await?;

    let clip = row
        .map(|record| {
            let resolved_clip_id: String = record.try_get("clip_id")?;
            Ok::<PlayableClip, sqlx::Error>(PlayableClip {
                display_name: record
                    .try_get::<Option<String>, _>("name")?
                    .unwrap_or_else(|| resolved_clip_id.clone()),
                saved_file_name: record
                    .try_get::<Option<String>, _>("saved_file_name")?
                    .unwrap_or_else(|| format!("{resolved_clip_id}.ogg")),
                clip_id: resolved_clip_id,
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

pub async fn autocomplete_clip_choices(
    pool: &Pool<Postgres>,
    guild_id: i64,
    query: &str,
    visible_channel_ids: &[i64],
) -> DbResult<Vec<ClipChoice>> {
    let query_wildcard = format!("%{}%", query);

    let rows = sqlx::query(
        "SELECT clips.name, clips.clip_id
           FROM clips
          CROSS JOIN LATERAL (
              SELECT CASE
                  WHEN clips.recording_session_id IS NOT NULL THEN COALESCE(
                      (
                          SELECT array_agg(DISTINCT audio_files.channel_id ORDER BY audio_files.channel_id)
                            FROM audio_files
                           WHERE audio_files.recording_session_id = clips.recording_session_id
                      ),
                      (
                          SELECT ARRAY[recording_sessions.starting_channel_id]::bigint[]
                            FROM recording_sessions
                           WHERE recording_sessions.id = clips.recording_session_id
                      ),
                      ARRAY[]::bigint[]
                  )
                  WHEN clips.channel_id IS NOT NULL THEN ARRAY[clips.channel_id]::bigint[]
                  ELSE ARRAY[]::bigint[]
              END AS channel_ids
          ) source
          WHERE clips.guild_id = $1
            AND clips.name ILIKE $2
            AND clips.deleted_at IS NULL
            AND cardinality(source.channel_ids) > 0
            AND source.channel_ids <@ $3::bigint[]
          ORDER BY clips.name, clips.clip_id
          LIMIT 25",
    )
    .bind(guild_id)
    .bind(query_wildcard)
    .bind(visible_channel_ids)
    .fetch_all(pool)
    .await?;

    let mut choices = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(name) = row.try_get::<Option<String>, _>("name")? {
            choices.push(ClipChoice {
                name,
                clip_id: row.try_get("clip_id")?,
            });
        }
    }
    Ok(choices)
}
