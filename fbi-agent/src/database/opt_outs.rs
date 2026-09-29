//! Per-guild recording opt-outs: members who asked the bot not to record their
//! voice. Recording is on by default; a row in `recording_opt_outs` turns it
//! off for one user in one guild.

use sqlx::{PgExecutor, Pool, Postgres};

use super::DbResult;

/// Sets whether `user_id` is opted out of recording in `guild_id`, returning
/// `false` when the user was already in that state.
///
/// Takes the guild lock that `create_fragment_in` holds while it checks the
/// opt-out, so no fragment can open for a user after their opt-out commits.
pub async fn set_opted_out(
    pool: &Pool<Postgres>,
    guild_id: i64,
    user_id: i64,
    opted_out: bool,
) -> DbResult<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query!("SELECT pg_advisory_xact_lock($1)", guild_id)
        .execute(&mut *tx)
        .await?;
    let result = if opted_out {
        sqlx::query!(
            "INSERT INTO recording_opt_outs (guild_id, user_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
            guild_id,
            user_id
        )
        .execute(&mut *tx)
        .await?
    } else {
        sqlx::query!(
            "DELETE FROM recording_opt_outs WHERE guild_id = $1 AND user_id = $2",
            guild_id,
            user_id
        )
        .execute(&mut *tx)
        .await?
    };
    tx.commit().await?;
    Ok(result.rows_affected() > 0)
}

/// Takes a pool, or a transaction's connection when the answer must hold for
/// the rest of that transaction.
pub async fn is_opted_out<'e>(
    executor: impl PgExecutor<'e>,
    guild_id: i64,
    user_id: i64,
) -> DbResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1 FROM recording_opt_outs WHERE guild_id = $1 AND user_id = $2
           ) AS "opted_out!""#,
        guild_id,
        user_id
    )
    .fetch_one(executor)
    .await?)
}

/// The members of `user_ids` who are opted out of recording in `guild_id`.
pub async fn opted_out_among(
    pool: &Pool<Postgres>,
    guild_id: i64,
    user_ids: &[i64],
) -> DbResult<Vec<i64>> {
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_scalar!(
        "SELECT user_id FROM recording_opt_outs WHERE guild_id = $1 AND user_id = ANY($2)",
        guild_id,
        user_ids
    )
    .fetch_all(pool)
    .await?)
}
