//! Per-guild listening consent: who may hear the recordings of a member's
//! voice. Shared by default; a row in `recording_private` keeps them to the
//! member alone, whatever anyone else holds.

use sqlx::{Pool, Postgres};

use super::DbResult;

/// Sets whether only `user_id` may hear the recordings of their voice in
/// `guild_id`, returning `false` when it already was so.
pub async fn set_private(
    pool: &Pool<Postgres>,
    guild_id: i64,
    user_id: i64,
    private: bool,
) -> DbResult<bool> {
    let result = if private {
        sqlx::query!(
            "INSERT INTO recording_private (guild_id, user_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
            guild_id,
            user_id
        )
        .execute(pool)
        .await?
    } else {
        sqlx::query!(
            "DELETE FROM recording_private WHERE guild_id = $1 AND user_id = $2",
            guild_id,
            user_id
        )
        .execute(pool)
        .await?
    };
    Ok(result.rows_affected() > 0)
}

/// The guilds holding recordings of `user_id`, with their names. Someone who
/// has left a guild still decides who hears their voice there.
pub async fn recorded_guilds(pool: &Pool<Postgres>, user_id: i64) -> DbResult<Vec<(i64, String)>> {
    let rows = sqlx::query!(
        r#"SELECT g.id, COALESCE(g.name, g.id::text) AS "name!"
             FROM guilds g
            WHERE EXISTS (SELECT 1
                            FROM recording_sessions rs
                           WHERE rs.guild_id = g.id AND rs.user_id = $1)"#,
        user_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|row| (row.id, row.name)).collect())
}

pub async fn is_private(pool: &Pool<Postgres>, guild_id: i64, user_id: i64) -> DbResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1 FROM recording_private WHERE guild_id = $1 AND user_id = $2
           ) AS "private!""#,
        guild_id,
        user_id
    )
    .fetch_one(pool)
    .await?)
}
