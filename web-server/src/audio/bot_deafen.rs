//! When the bot was deafened, for the timelines. A deafened bot hears nobody,
//! so this is why a stretch of a recording is silent. The bot writes it: see
//! `fbi-agent/src/events/voice/bot_log.rs`. Who did it stays in the log
//! managers read.

use sqlx::{Pool, Postgres};

use crate::errors::AppError;

/// A change to the bot's deafen within a timeline.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct BotDeafenEvent {
    pub event_type: &'static str,
    pub offset_ms: i64,
}

const DEAFENED: &str = "bot_deafened";

fn event_type(kind: &str) -> Result<&'static str, AppError> {
    match kind {
        "deafened" => Ok(DEAFENED),
        "undeafened" => Ok("bot_undeafened"),
        "deafen_clear_failed" => Ok("bot_deafen_clear_failed"),
        _ => Err(AppError::InternalError),
    }
}

/// The deafen changes in a guild between `start_ms` and `end_ms`, or now, as
/// offsets from `start_ms`. Sessions are per member, so a member can join when
/// the bot is already deafened: that deafen opens the list, at offset 0.
pub(crate) async fn bot_deafen_events(
    pool: &Pool<Postgres>,
    guild_id: i64,
    start_ms: i64,
    end_ms: Option<i64>,
) -> Result<Vec<BotDeafenEvent>, AppError> {
    let before = sqlx::query_scalar!(
        "SELECT kind
           FROM bot_voice_log
          WHERE guild_id = $1
            AND kind IN ('deafened', 'undeafened', 'deafen_clear_failed')
            AND occurred_at < to_timestamp($2::double precision / 1000.0)
          ORDER BY occurred_at DESC, id DESC
          LIMIT 1",
        guild_id,
        start_ms as f64
    )
    .fetch_optional(pool)
    .await?;
    let mut events = Vec::new();
    if before.is_some_and(|kind| kind != "undeafened") {
        events.push(BotDeafenEvent {
            event_type: DEAFENED,
            offset_ms: 0,
        });
    }

    let rows = sqlx::query!(
        r#"SELECT kind,
                  ((EXTRACT(EPOCH FROM occurred_at) * 1000)::bigint - $2) AS "offset_ms!"
             FROM bot_voice_log
            WHERE guild_id = $1
              AND kind IN ('deafened', 'undeafened', 'deafen_clear_failed')
              AND occurred_at >= to_timestamp($2::double precision / 1000.0)
              AND occurred_at <= COALESCE(to_timestamp($3::double precision / 1000.0), now())
            ORDER BY occurred_at, id"#,
        guild_id,
        start_ms,
        end_ms.map(|ms| ms as f64)
    )
    .fetch_all(pool)
    .await?;
    for row in rows {
        events.push(BotDeafenEvent {
            event_type: event_type(&row.kind)?,
            offset_ms: row.offset_ms,
        });
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::{BotDeafenEvent, bot_deafen_events};

    const GUILD: i64 = 1;
    const START_MS: i64 = 1_794_000_000_000;
    const END_MS: i64 = START_MS + 60_000;

    async fn log(pool: &PgPool, guild: i64, kind: &str, at_ms: i64) -> Result<(), sqlx::Error> {
        let to_channel = (kind == "moved").then_some(11_i64);
        sqlx::query(
            "INSERT INTO bot_voice_log
                (guild_id, kind, channel_id, to_channel_id, attribution, occurred_at)
             VALUES ($1, $2, 10, $3, 'unknown', to_timestamp($4::double precision / 1000.0))",
        )
        .bind(guild)
        .bind(kind)
        .bind(to_channel)
        .bind(at_ms as f64)
        .execute(pool)
        .await?;
        Ok(())
    }

    fn event(event_type: &'static str, offset_ms: i64) -> BotDeafenEvent {
        BotDeafenEvent {
            event_type,
            offset_ms,
        }
    }

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn lists_changes_inside_the_window_as_offsets(pool: PgPool) -> TestResult {
        log(&pool, GUILD, "deafened", START_MS + 1_000).await?;
        log(&pool, GUILD, "undeafened", START_MS + 5_000).await?;
        log(&pool, GUILD, "deafened", END_MS + 1_000).await?;
        assert_eq!(
            bot_deafen_events(&pool, GUILD, START_MS, Some(END_MS)).await?,
            [event("bot_deafened", 1_000), event("bot_undeafened", 5_000)]
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn carries_in_a_deafen_from_before_the_window(pool: PgPool) -> TestResult {
        log(&pool, GUILD, "deafened", START_MS - 60_000).await?;
        log(&pool, GUILD, "undeafened", START_MS + 5_000).await?;
        assert_eq!(
            bot_deafen_events(&pool, GUILD, START_MS, Some(END_MS)).await?,
            [event("bot_deafened", 0), event("bot_undeafened", 5_000)]
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn carries_in_a_failed_cleanup_too(pool: PgPool) -> TestResult {
        log(&pool, GUILD, "deafen_clear_failed", START_MS - 60_000).await?;
        assert_eq!(
            bot_deafen_events(&pool, GUILD, START_MS, Some(END_MS)).await?,
            [event("bot_deafened", 0)]
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn a_deafen_that_ended_before_the_window_is_history(pool: PgPool) -> TestResult {
        log(&pool, GUILD, "deafened", START_MS - 60_000).await?;
        log(&pool, GUILD, "undeafened", START_MS - 30_000).await?;
        assert_eq!(
            bot_deafen_events(&pool, GUILD, START_MS, Some(END_MS)).await?,
            []
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn ignores_other_guilds_and_other_kinds(pool: PgPool) -> TestResult {
        log(&pool, 2, "deafened", START_MS + 1_000).await?;
        log(&pool, GUILD, "moved", START_MS + 1_000).await?;
        log(&pool, GUILD, "disconnected", START_MS + 2_000).await?;
        assert_eq!(
            bot_deafen_events(&pool, GUILD, START_MS, Some(END_MS)).await?,
            []
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn an_open_recording_reads_up_to_now(pool: PgPool) -> TestResult {
        let start_ms = chrono::Utc::now().timestamp_millis() - 60_000;
        log(&pool, GUILD, "deafened", start_ms + 1_000).await?;
        assert_eq!(
            bot_deafen_events(&pool, GUILD, start_ms, None).await?,
            [event("bot_deafened", 1_000)]
        );
        Ok(())
    }
}
