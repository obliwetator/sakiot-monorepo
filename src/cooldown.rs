use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use sqlx::{Pool, Postgres};
use tracing::warn;

#[derive(Clone)]
pub struct JamCooldown {
    last: Arc<DashMap<(i64, i64), Instant>>,
}

pub enum CheckResult {
    Allowed,
    OnCooldown { remaining_secs: u32 },
}

impl JamCooldown {
    pub fn new() -> Self {
        Self {
            last: Arc::new(DashMap::new()),
        }
    }

    pub async fn check_and_record(
        &self,
        pool: &Pool<Postgres>,
        guild_id: i64,
        user_id: i64,
    ) -> CheckResult {
        let cooldown_secs = match resolve_cooldown(pool, guild_id, user_id).await {
            Ok(secs) => secs,
            Err(e) => {
                warn!(
                    "Cooldown lookup failed (guild={}, user={}): {}. Allowing.",
                    guild_id, user_id, e
                );
                0
            }
        };

        if cooldown_secs == 0 {
            self.last.insert((guild_id, user_id), Instant::now());
            return CheckResult::Allowed;
        }

        let now = Instant::now();
        let cooldown = Duration::from_secs(cooldown_secs as u64);

        if let Some(prev) = self.last.get(&(guild_id, user_id)) {
            let elapsed = now.duration_since(*prev);
            if elapsed < cooldown {
                let remaining = cooldown - elapsed;
                let remaining_secs = remaining.as_secs() as u32 + 1;
                return CheckResult::OnCooldown { remaining_secs };
            }
        }

        self.last.insert((guild_id, user_id), now);
        CheckResult::Allowed
    }
}

impl Default for JamCooldown {
    fn default() -> Self {
        Self::new()
    }
}

// No caching: every jam attempt does up to two DB round-trips (override + guild base).
// Acceptable at current scale and means admin edits take effect immediately.
// Add a short-TTL cache here if playback volume grows enough to make this hurt.
async fn resolve_cooldown(
    pool: &Pool<Postgres>,
    guild_id: i64,
    user_id: i64,
) -> Result<i32, sqlx::Error> {
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
