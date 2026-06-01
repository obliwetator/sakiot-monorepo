use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use sqlx::{Pool, Postgres};

use crate::database::{DbResult, clips};

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
    ) -> DbResult<CheckResult> {
        let cooldown_secs = clips::resolve_jam_cooldown(pool, guild_id, user_id).await?;

        if cooldown_secs == 0 {
            self.last.insert((guild_id, user_id), Instant::now());
            return Ok(CheckResult::Allowed);
        }

        let now = Instant::now();
        let cooldown = Duration::from_secs(cooldown_secs as u64);

        if let Some(prev) = self.last.get(&(guild_id, user_id)) {
            let elapsed = now.duration_since(*prev);
            if elapsed < cooldown {
                let remaining = cooldown - elapsed;
                let remaining_secs = remaining.as_secs() as u32 + 1;
                return Ok(CheckResult::OnCooldown { remaining_secs });
            }
        }

        self.last.insert((guild_id, user_id), now);
        Ok(CheckResult::Allowed)
    }
}

impl Default for JamCooldown {
    fn default() -> Self {
        Self::new()
    }
}
