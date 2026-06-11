//! Deploy mutual exclusion: flock(2) on the same lock file the bash engine
//! uses, so both engines exclude each other during the transition window.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::Path;

use anyhow::{Context, Result, bail};

pub struct DeployLock {
    _file: File,
}

impl DeployLock {
    /// Blocks until the exclusive lock is acquired (bash `flock 9`). The lock
    /// is held for the life of the returned guard.
    pub fn acquire(path: &Path) -> Result<DeployLock> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .with_context(|| format!("failed to open lock file {}", path.display()))?;
        // SAFETY: flock on a valid owned fd; no memory is involved.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc != 0 {
            bail!(
                "failed to lock {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            );
        }
        Ok(DeployLock { _file: file })
    }

    #[cfg(test)]
    fn try_acquire(path: &Path) -> Result<Option<DeployLock>> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        // SAFETY: flock on a valid owned fd.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            return Ok(None);
        }
        Ok(Some(DeployLock { _file: file }))
    }
}

#[cfg(test)]
mod tests {
    //! Ported from ops/tests/lock_test.sh: a held lock blocks a second taker.

    use super::*;

    #[test]
    fn second_taker_blocks_until_release() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deploy.lock");

        let held = DeployLock::acquire(&path).unwrap();
        assert!(
            DeployLock::try_acquire(&path).unwrap().is_none(),
            "lock should be contended while held"
        );
        drop(held);
        assert!(
            DeployLock::try_acquire(&path).unwrap().is_some(),
            "lock should be free after release"
        );
    }
}
