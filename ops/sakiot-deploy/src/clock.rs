//! Wall clock and pacing behind a trait so readiness polls are instant and
//! deterministic in tests.

use std::time::Duration;

use anyhow::{Context, Result};
use time::OffsetDateTime;
use time::format_description::FormatItem;
use time::macros::format_description;

const COMPACT: &[FormatItem<'_>] =
    format_description!("[year][month][day]T[hour][minute][second]Z");
const RFC3339_SECONDS: &[FormatItem<'_>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

pub trait Clock {
    fn now_utc(&self) -> OffsetDateTime;
    fn sleep(&self, duration: Duration);
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_utc(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// `date -u +%Y%m%dT%H%M%SZ` — release id timestamps.
pub fn compact_timestamp(now: OffsetDateTime) -> Result<String> {
    now.format(&COMPACT).context("failed to format timestamp")
}

/// `date -u +%Y-%m-%dT%H:%M:%SZ` — manifest deployed_at.
pub fn rfc3339_timestamp(now: OffsetDateTime) -> Result<String> {
    now.format(&RFC3339_SECONDS)
        .context("failed to format timestamp")
}

pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|name| name.trim().to_string())
        .unwrap_or_else(|_| "localhost".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_formats_match_date_u() {
        let moment = OffsetDateTime::from_unix_timestamp(1_768_478_400).unwrap();
        assert_eq!(compact_timestamp(moment).unwrap(), "20260115T120000Z");
        assert_eq!(rfc3339_timestamp(moment).unwrap(), "2026-01-15T12:00:00Z");
    }
}
