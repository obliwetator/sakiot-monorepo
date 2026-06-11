//! Release identifiers, the manifest, artifact reuse, and release garbage
//! collection, ported from ops/lib/common.sh and deploy-release.sh.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::components::Component;
use crate::config::Mode;
use crate::log;
use crate::systemctl::Systemctl;

/// deploy-release.sh lines 124-131.
pub fn release_id(mode: Mode, tag: &str, sha: &str, timestamp: &str) -> String {
    let short_sha = &sha[..12.min(sha.len())];
    match mode {
        Mode::Rollback => format!("{tag}-{short_sha}-rollback-{timestamp}"),
        Mode::Stage => format!("staging-{short_sha}-{timestamp}"),
        Mode::Release => format!("{tag}-{short_sha}-{timestamp}"),
    }
}

/// manifest.json. Field order matches the jq template in deploy-release.sh so
/// output stays byte-compatible (serde_json preserves struct order).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub target: String,
    pub mode: String,
    pub tag: String,
    pub sha: String,
    pub previous_tag: String,
    pub previous_sha: String,
    pub release_id: String,
    pub components: Vec<String>,
    pub changed_paths: Vec<String>,
    pub database: ManifestDatabase,
    pub reused: ManifestReused,
    pub deployed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestDatabase {
    pub migrations_ran: bool,
    pub migration_head: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestReused {
    pub bot: bool,
    pub web: bool,
    pub frontend: bool,
}

impl Manifest {
    /// jq pretty-prints with two-space indent and a trailing newline.
    pub fn write(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self).context("failed to render manifest")?;
        std::fs::write(path, format!("{json}\n"))
            .with_context(|| format!("failed to write {}", path.display()))
    }
}

/// Bot unit name for a release dir, or None when the release has no bot
/// artifact.
pub fn release_bot_unit(release_dir: &Path, bot_unit_prefix: &str) -> Option<String> {
    if !release_dir.join("fbi-agent").is_dir() {
        return None;
    }
    let name = release_dir.file_name()?.to_string_lossy();
    Some(format!("{bot_unit_prefix}{name}.service"))
}

fn stamp_in(name: &str) -> Option<String> {
    // grep -oE '[0-9]{8}T[0-9]{6}Z' | tail -n 1
    let bytes = name.as_bytes();
    let mut last = None;
    for start in 0..bytes.len() {
        let candidate = &bytes[start..];
        if candidate.len() < 16 {
            break;
        }
        let digits8 = candidate[..8].iter().all(u8::is_ascii_digit);
        let digits6 = candidate[9..15].iter().all(u8::is_ascii_digit);
        if digits8 && candidate[8] == b'T' && digits6 && candidate[15] == b'Z' {
            last = Some(name[start..start + 16].to_string());
        }
    }
    last
}

/// Release dirs newest -> oldest, ordered by the trailing release timestamp
/// in the id, falling back to directory mtime when absent.
pub fn releases_newest_first(release_root_abs: &Path) -> Result<Vec<PathBuf>> {
    let mut lines: Vec<(String, PathBuf)> = Vec::new();
    let entries = match std::fs::read_dir(release_root_abs) {
        Ok(entries) => entries,
        Err(_) => return Ok(Vec::new()),
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir = release_root_abs.join(entry.file_name());
        let name = entry.file_name().to_string_lossy().into_owned();
        let stamp = stamp_in(&name)
            .or_else(|| mtime_stamp(&dir))
            .unwrap_or_else(|| "00000000T000000Z".to_string());
        lines.push((format!("{stamp}\t{}", dir.display()), dir));
    }
    // `sort -r` over "stamp\tdir" lines.
    lines.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(lines.into_iter().map(|(_, dir)| dir).collect())
}

fn mtime_stamp(dir: &Path) -> Option<String> {
    let modified = std::fs::metadata(dir).ok()?.modified().ok()?;
    let moment = time::OffsetDateTime::from(modified);
    crate::clock::compact_timestamp(moment).ok()
}

/// Find a prior release dir whose manifest SHA matches `target_sha` and that
/// holds a usable artifact for the component. Returns the newest match.
/// `exclude_dir` (the release being built now) is skipped.
pub fn reusable_artifact(
    release_root: &Path,
    target_sha: &str,
    component: Component,
    exclude_dir: &Path,
) -> Result<Option<PathBuf>> {
    if !release_root.is_dir() {
        return Ok(None);
    }
    let release_root_abs = release_root
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", release_root.display()))?;

    for dir in releases_newest_first(&release_root_abs)? {
        if dir == exclude_dir {
            continue;
        }
        let manifest_path = dir.join("manifest.json");
        if !manifest_path.is_file() {
            continue;
        }
        let manifest_sha = std::fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
            .and_then(|json| {
                json.get("sha")
                    .and_then(|sha| sha.as_str().map(String::from))
            })
            .unwrap_or_default();
        if manifest_sha != target_sha {
            continue;
        }

        let usable = match component {
            Component::Bot => file_non_empty(&dir.join("fbi-agent/fbi_agent")),
            Component::Web => file_non_empty(&dir.join("web/web_server")),
            Component::Frontend => dir_non_empty(&dir.join("frontend/dist")),
            Component::Database => return Ok(None),
        };
        if usable {
            return Ok(Some(dir));
        }
    }
    Ok(None)
}

fn file_non_empty(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.len() > 0)
        .unwrap_or(false)
}

fn dir_non_empty(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

/// Remove old release directories, keeping the newest `keep` and never
/// touching a release that is still in use (current web symlink target,
/// current manifest, current bot, or any release whose bot unit is active
/// such as a draining old instance).
pub fn prune_old_releases(
    systemctl: &Systemctl,
    release_root: &Path,
    current_root: &Path,
    state_dir: &Path,
    keep: &str,
    bot_unit_prefix: &str,
) -> Result<()> {
    if !release_root.is_dir() {
        return Ok(());
    }
    // Bash guards with ^[0-9]+$; integer parse alone would also accept "+5".
    let all_digits = !keep.is_empty() && keep.bytes().all(|b| b.is_ascii_digit());
    let Ok(keep) = keep
        .parse::<usize>()
        .map_err(|_| ())
        .and_then(|n| if all_digits { Ok(n) } else { Err(()) })
    else {
        log(format!(
            "invalid SAKIOT_KEEP_RELEASES '{keep}'; skipping prune"
        ));
        return Ok(());
    };

    let release_root_abs = release_root
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", release_root.display()))?;

    let mut protected: HashSet<PathBuf> = HashSet::new();
    if let Ok(target) = std::fs::read_link(current_root.join("web"))
        && let Some(parent) = target.parent()
    {
        protected.insert(parent.to_path_buf());
    }
    if let Some(manifest) = crate::fsx::read_line(&state_dir.join("current.manifest"))
        && !manifest.is_empty()
        && let Some(parent) = Path::new(&manifest).parent()
    {
        protected.insert(parent.to_path_buf());
    }
    if let Some(unit) = crate::fsx::read_line(&state_dir.join("current-bot.unit"))
        && !unit.is_empty()
    {
        let unit_release = unit.strip_prefix(bot_unit_prefix).unwrap_or(&unit);
        let unit_release = unit_release
            .strip_suffix(".service")
            .unwrap_or(unit_release);
        protected.insert(release_root_abs.join(unit_release));
    }

    let ordered = releases_newest_first(&release_root_abs)?;
    for (index, dir) in ordered.iter().enumerate() {
        if index < keep {
            continue;
        }
        if protected.contains(dir) {
            continue;
        }

        let unit = release_bot_unit(dir, bot_unit_prefix);
        if let Some(unit) = &unit
            && systemctl.run_ok(&["is-active", "--quiet", unit])
        {
            log(format!("keeping in-use release {}", dir.display()));
            continue;
        }

        // Path safety: only ever remove a direct child of release_root.
        if !dir.starts_with(&release_root_abs) {
            log(format!(
                "refusing to prune path outside release root: {}",
                dir.display()
            ));
            continue;
        }

        if let Some(unit) = &unit {
            let _ = systemctl.run_ok(&["disable", unit]);
        }
        log(format!("pruning old release {}", dir.display()));
        std::fs::remove_dir_all(dir)
            .with_context(|| format!("failed to remove {}", dir.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_ids_match_bash_format() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let ts = "20260115T120000Z";
        assert_eq!(
            release_id(Mode::Release, "v1.2.3", sha, ts),
            "v1.2.3-0123456789ab-20260115T120000Z"
        );
        assert_eq!(
            release_id(Mode::Rollback, "v1.2.3", sha, ts),
            "v1.2.3-0123456789ab-rollback-20260115T120000Z"
        );
        assert_eq!(
            release_id(Mode::Stage, "main", sha, ts),
            "staging-0123456789ab-20260115T120000Z"
        );
    }

    #[test]
    fn stamp_extraction_takes_last_match() {
        assert_eq!(
            stamp_in("v1.0.0-abc-20260101T000000Z").as_deref(),
            Some("20260101T000000Z")
        );
        assert_eq!(
            stamp_in("v1-20250101T000000Z-rollback-20260101T010101Z").as_deref(),
            Some("20260101T010101Z")
        );
        assert_eq!(stamp_in("no-stamp-here"), None);
    }

    #[test]
    fn manifest_serialization_matches_jq_layout() {
        let manifest = Manifest {
            target: "production".into(),
            mode: "release".into(),
            tag: "v1.0.0".into(),
            sha: "a".repeat(40),
            previous_tag: String::new(),
            previous_sha: String::new(),
            release_id: "v1.0.0-aaaaaaaaaaaa-20260101T000000Z".into(),
            components: vec!["database".into(), "bot".into()],
            changed_paths: vec![],
            database: ManifestDatabase {
                migrations_ran: true,
                migration_head: "0007_x.sql".into(),
            },
            reused: ManifestReused {
                bot: false,
                web: false,
                frontend: false,
            },
            deployed_at: "2026-01-01T00:00:00Z".into(),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        manifest.write(&path).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();

        // jq -n field order, two-space indent, trailing newline.
        let expected_order = [
            "\"target\"",
            "\"mode\"",
            "\"tag\"",
            "\"sha\"",
            "\"previous_tag\"",
            "\"previous_sha\"",
            "\"release_id\"",
            "\"components\"",
            "\"changed_paths\"",
            "\"database\"",
            "\"reused\"",
            "\"deployed_at\"",
        ];
        let mut cursor = 0;
        for key in expected_order {
            let position = written[cursor..].find(key).expect(key);
            cursor += position;
        }
        assert!(written.ends_with("}\n"));
        assert!(written.contains("  \"target\": \"production\""));

        let parsed: Manifest = serde_json::from_str(&written).unwrap();
        assert_eq!(parsed.sha, "a".repeat(40));
    }
}
