//! Filesystem helpers replicating the coreutils idioms used by the bash
//! engine: `install -d -m`, `install -m`, atomic symlink swap, and the
//! write-then-rename state files.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

/// `install -d -m <mode> <dir>`: create parents as needed and apply the mode
/// to the named directory even when it already exists.
pub fn ensure_dir_mode(dir: &Path, mode: u32) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(mode))
        .with_context(|| format!("failed to chmod {}", dir.display()))?;
    Ok(())
}

/// `install -m <mode> <src> <dst>`: replace dst with a fresh copy of src.
/// Unlinks first so a running binary at dst never causes ETXTBSY.
pub fn install_file(src: &Path, dst: &Path, mode: u32) -> Result<()> {
    if dst.exists() {
        fs::remove_file(dst).with_context(|| format!("failed to remove {}", dst.display()))?;
    }
    fs::copy(src, dst)
        .with_context(|| format!("failed to copy {} to {}", src.display(), dst.display()))?;
    fs::set_permissions(dst, fs::Permissions::from_mode(mode))
        .with_context(|| format!("failed to chmod {}", dst.display()))?;
    Ok(())
}

/// Mirrors atomic_symlink() from ops/lib/common.sh: create a temporary link
/// then rename over the destination so readers never see a missing link.
pub fn atomic_symlink(target: &Path, link: &Path) -> Result<()> {
    let temporary = link.with_file_name(format!(
        "{}.new.{}",
        link.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id()
    ));
    let _ = fs::remove_file(&temporary);
    std::os::unix::fs::symlink(target, &temporary).with_context(|| {
        format!(
            "failed to create symlink {} -> {}",
            temporary.display(),
            target.display()
        )
    })?;
    fs::rename(&temporary, link)
        .with_context(|| format!("failed to replace symlink {}", link.display()))?;
    Ok(())
}

/// `printf '%s\n' value > path`.
pub fn write_line(path: &Path, value: &str) -> Result<()> {
    fs::write(path, format!("{value}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

/// State-file update via `> path.new && mv path.new path`.
pub fn write_line_atomic(path: &Path, value: &str) -> Result<()> {
    let temporary = path.with_file_name(format!(
        "{}.new",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    write_line(&temporary, value)?;
    fs::rename(&temporary, path)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

/// `cat path 2>/dev/null || true`, minus the trailing newline.
pub fn read_line(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|content| content.trim_end_matches('\n').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_dir_mode_applies_to_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o777)).unwrap();
        ensure_dir_mode(&path, 0o750).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o750);
    }

    #[test]
    fn atomic_symlink_replaces_existing() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("current");
        atomic_symlink(Path::new("/first"), &link).unwrap();
        atomic_symlink(Path::new("/second"), &link).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("/second"));
    }

    #[test]
    fn state_lines_round_trip_with_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("current.sha");
        write_line_atomic(&path, "abc").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "abc\n");
        assert_eq!(read_line(&path).unwrap(), "abc");
        assert_eq!(read_line(&dir.path().join("missing")), None);
    }
}
