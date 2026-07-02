//! Startup zombie reaper for `audio_files`.
//!
//! Recording end is written by the leave/disconnect path in
//! `events/voice_receiver.rs`. If the bot crashed or was killed before that
//! ran, rows stay `end_ts IS NULL` forever and pollute "live" detection in
//! the web UI. A NULL end timestamp only means "not finalized"; it is live
//! only while the row's recording heartbeat is fresh and owned by a fresh,
//! non-stopped bot instance.
//!
//! Default mode: row stays, `end_ts = start_ts`, `reaped = TRUE`. Files on
//! disk are left alone — they may still be partially playable. Audit them
//! with:
//!
//! ```sql
//! SELECT * FROM audio_files WHERE reaped = TRUE ORDER BY start_ts DESC;
//! ```
//!
//! Purge mode (`REAPER_PURGE=1`): one-shot wipe — delete the `.ogg` and any
//! `hls-{stem}/` cache dir, then `DELETE FROM audio_files` for those rows.
//! Use after a long zombie buildup when you don't want to inspect each one.
//!
//! `bot_reaper_state.last_reap_ts` records when the startup reaper last ran.
//! We still rescan all unfinished rows because a row can be skipped while its
//! voice lease is live and become reapable later.

use chrono::{DateTime, Datelike, Utc};
use sakiot_paths::{DataRoots, RecordingKey};
use sqlx::{Pool, Postgres};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

use crate::database::DbResult;

pub async fn reap_zombie_recordings(pool: &Pool<Postgres>) -> DbResult<()> {
    let purge = std::env::var("REAPER_PURGE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let now_ms = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => {
            error!("reaper: clock before epoch: {}", e);
            0
        }
    };

    let last_reap_ts = crate::database::recordings::last_reap_ts(pool).await?;
    let zombies = crate::database::recordings::zombie_recordings(pool).await?;

    if zombies.is_empty() {
        info!(last_reap_ts, "reaper: no zombies");
    }

    let mut deleted_files = 0usize;
    let mut missing_files = 0usize;

    if purge {
        warn!("REAPER_PURGE=1 — deleting zombie files from disk");
        for z in &zombies {
            let Some(start_ts) = z.start_ts else {
                warn!(file_name = %z.file_name, "reaper: NULL start_ts, skipping fs delete");
                continue;
            };
            let Some(dt) = DateTime::<Utc>::from_timestamp_millis(start_ts) else {
                warn!(file_name = %z.file_name, start_ts, "reaper: bad start_ts, skipping fs delete");
                continue;
            };
            let key = RecordingKey::new(
                z.guild_id,
                z.channel_id,
                dt.year(),
                dt.month(),
                z.file_name.clone(),
            );
            let recording_root = DataRoots::from_env().recordings_str();
            let path = key.recording_path(&recording_root);
            match std::fs::remove_file(&path) {
                Ok(_) => {
                    deleted_files += 1;
                    info!(path = %path.display(), "reaper: deleted zombie recording");
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    missing_files += 1;
                }
                Err(e) => {
                    error!(path = %path.display(), error = %e, "reaper: delete failed");
                }
            }
            let hls = key.live_dir(&recording_root);
            if hls.exists()
                && let Err(e) = std::fs::remove_dir_all(&hls)
            {
                warn!(path = %hls.display(), error = %e, "reaper: hls cleanup failed");
            }
        }
    }

    let rows_changed = if purge {
        crate::database::recordings::delete_zombie_recordings(pool).await?
    } else {
        crate::database::recordings::mark_zombie_recordings_reaped(pool).await?
    };

    crate::database::recordings::bump_last_reap_ts(pool, now_ms).await?;

    info!(
        purge,
        zombies = zombies.len(),
        rows_changed,
        deleted_files,
        missing_files,
        last_reap_ts,
        now_ms,
        "startup zombie reaper done"
    );
    Ok(())
}
