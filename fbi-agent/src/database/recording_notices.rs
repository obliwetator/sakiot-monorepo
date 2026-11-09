//! The one-time notice telling a member their voice is recorded in a guild.
//!
//! [`queue`] runs in the transaction that opens a recording, so nobody is
//! recorded without a row here. `crate::recording_notice` delivers them, and
//! the row stays as the record of what was sent.

use sqlx::{PgExecutor, Pool, Postgres};

use super::DbResult;

/// Attempts before a notice that keeps failing transiently is given up on.
pub const MAX_ATTEMPTS: i32 = 8;

/// Where a notice goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Dm,
    Channel,
}

impl Delivery {
    fn parse(value: &str) -> Self {
        if value == "channel" {
            Self::Channel
        } else {
            Self::Dm
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Dm => "dm",
            Self::Channel => "channel",
        }
    }
}

/// A notice claimed for delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DueNotice {
    pub guild_id: i64,
    pub user_id: i64,
    pub channel_id: i64,
    /// Includes this one.
    pub attempts: i32,
    /// The guild's preference.
    pub delivery: Delivery,
}

/// Queues the notice unless the member already has one. A failed notice is
/// queued again once it has been failed for a day, so someone the bot could
/// not reach is retried for as long as they keep being recorded.
pub async fn queue<'e>(
    executor: impl PgExecutor<'e>,
    guild_id: i64,
    channel_id: i64,
    user_id: i64,
) -> DbResult<()> {
    sqlx::query!(
        "INSERT INTO recording_notices (guild_id, user_id, channel_id)
         VALUES ($1, $2, $3)
         ON CONFLICT (guild_id, user_id) DO UPDATE
            SET status = 'pending',
                channel_id = EXCLUDED.channel_id,
                attempts = 0,
                next_attempt_at = now()
          WHERE recording_notices.status = 'failed'
            AND recording_notices.next_attempt_at < now() - interval '1 day'",
        guild_id,
        user_id,
        channel_id
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Claims the notice due longest ago. Pushing `next_attempt_at` back is the
/// claim: another instance won't take it, and if this one dies mid-send the
/// notice comes due again. The delay doubles from a minute up to an hour.
pub async fn claim_due(pool: &Pool<Postgres>) -> DbResult<Option<DueNotice>> {
    let row = sqlx::query!(
        r#"WITH due AS (
             SELECT guild_id, user_id
               FROM recording_notices
              WHERE status = 'pending' AND next_attempt_at <= now()
              ORDER BY next_attempt_at, guild_id, user_id
              LIMIT 1
                FOR UPDATE SKIP LOCKED
           )
           UPDATE recording_notices n
              SET attempts = n.attempts + 1,
                  next_attempt_at =
                      now() + make_interval(mins => LEAST(power(2, n.attempts)::int, 60))
             FROM due
            WHERE n.guild_id = due.guild_id AND n.user_id = due.user_id
           RETURNING n.guild_id AS "guild_id!",
                     n.user_id AS "user_id!",
                     n.channel_id AS "channel_id!",
                     n.attempts AS "attempts!",
                     COALESCE(
                         (SELECT p.notice_delivery
                            FROM guild_recording_policy p
                           WHERE p.guild_id = n.guild_id),
                         'dm'
                     ) AS "delivery!""#
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| DueNotice {
        guild_id: row.guild_id,
        user_id: row.user_id,
        channel_id: row.channel_id,
        attempts: row.attempts,
        delivery: Delivery::parse(&row.delivery),
    }))
}

pub async fn mark_sent(
    pool: &Pool<Postgres>,
    notice: &DueNotice,
    via: Delivery,
    message_id: i64,
) -> DbResult<()> {
    sqlx::query!(
        "UPDATE recording_notices
            SET status = 'sent', sent_at = now(), delivered_via = $3, message_id = $4,
                last_error = NULL
          WHERE guild_id = $1 AND user_id = $2 AND status = 'pending'",
        notice.guild_id,
        notice.user_id,
        via.as_str(),
        message_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// There is nobody to tell: a bot, or a member who already opted out.
pub async fn mark_skipped(pool: &Pool<Postgres>, notice: &DueNotice) -> DbResult<()> {
    sqlx::query!(
        "UPDATE recording_notices SET status = 'skipped'
          WHERE guild_id = $1 AND user_id = $2 AND status = 'pending'",
        notice.guild_id,
        notice.user_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Records why an attempt failed. Unless `give_up`, the notice stays pending
/// and comes due again when its claim runs out.
pub async fn record_failure(
    pool: &Pool<Postgres>,
    notice: &DueNotice,
    error: &str,
    give_up: bool,
) -> DbResult<()> {
    sqlx::query!(
        "UPDATE recording_notices
            SET last_error = $3,
                status = CASE WHEN $4 THEN 'failed' ELSE status END
          WHERE guild_id = $1 AND user_id = $2 AND status = 'pending'",
        notice.guild_id,
        notice.user_id,
        error,
        give_up
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
