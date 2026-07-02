//! Background sweep that deletes stale per-recording HLS caches.
//!
//! HLS (`{recordings_root}/{guild}/{ch}/{y}/{m}/hls-{stem}/`) is only used
//! while a recording is live; finished recordings play back from the `.ogg`
//! blob directly. So once a recording's live period is well past, its `hls-*`
//! dir is dead weight and can be removed — it would be rebuilt on demand if a
//! request ever needed it again. Live dirs keep a fresh mtime (segments are
//! written every couple seconds), so they're never caught by the age check.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tracing::{error, info, warn};

use super::paths::recording_path;

/// Delete `hls-*` dirs whose mtime is older than this.
const HLS_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// How often the sweep runs.
const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Spawn the periodic HLS reaper. Runs an initial sweep at startup, then every
/// `SWEEP_INTERVAL`. Failures are logged, never fatal.
pub fn spawn_hls_reaper() {
    tokio::spawn(async move {
        let root = recording_path();
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            ticker.tick().await;
            match sweep_hls(&root).await {
                Ok(removed) if removed > 0 => info!(removed, "hls reaper: removed stale dirs"),
                Ok(_) => {}
                Err(e) => error!("hls reaper sweep failed: {:?}", e),
            }
        }
    });
}

/// Walk the recordings tree and remove `hls-*` dirs older than `HLS_MAX_AGE`.
/// Returns the number of dirs removed.
async fn sweep_hls(root: &str) -> std::io::Result<u32> {
    let now = SystemTime::now();
    let mut stack: Vec<PathBuf> = vec![PathBuf::from(root)];
    let mut removed = 0u32;

    while let Some(dir) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            // Dir vanished or is unreadable mid-sweep — skip it.
            Err(_) => continue,
        };

        while let Some(entry) = entries.next_entry().await? {
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }

            let path = entry.path();
            let name = entry.file_name();
            if name.to_string_lossy().starts_with("hls-") {
                // Don't descend into HLS dirs; reap the whole thing if stale.
                if is_older_than(&path, now, HLS_MAX_AGE).await {
                    match tokio::fs::remove_dir_all(&path).await {
                        Ok(()) => {
                            removed += 1;
                            info!(path = %path.display(), "hls reaper: removed");
                        }
                        Err(e) => {
                            warn!(path = %path.display(), "hls reaper: remove failed: {:?}", e);
                        }
                    }
                }
            } else {
                // Recurse into guild/channel/year/month dirs.
                stack.push(path);
            }
        }
    }

    Ok(removed)
}

/// Whether `path`'s mtime is more than `max_age` in the past. Unreadable
/// metadata or a future mtime counts as "not old" (left alone).
async fn is_older_than(path: &Path, now: SystemTime, max_age: Duration) -> bool {
    let Ok(meta) = tokio::fs::metadata(path).await else {
        return false;
    };
    let Ok(mtime) = meta.modified() else {
        return false;
    };
    now.duration_since(mtime)
        .map(|age| age > max_age)
        .unwrap_or(false)
}
