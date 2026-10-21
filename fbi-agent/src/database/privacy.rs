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
