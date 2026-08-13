//! Managed fixture manifests and safe media cleanup.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::environment::atomic_write_private;

pub const FIXTURE_MANIFEST: &str = ".dev-fixtures.list";
pub const RECORDINGS_MANIFEST: &str = ".dev-fixture-recordings.list";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManagedManifest {
    pub files: BTreeSet<PathBuf>,
    pub recordings: BTreeSet<String>,
}

impl ManagedManifest {
    pub fn load(data_dir: &Path) -> Result<Self> {
        let files_path = data_dir.join(FIXTURE_MANIFEST);
        let recordings_path = data_dir.join(RECORDINGS_MANIFEST);
        let files = read_entries(&files_path)?;
        let recordings = if recordings_path.is_file() {
            read_entries(&recordings_path)?
                .into_iter()
                .filter_map(|path| path.to_str().map(str::to_string))
                .collect()
        } else {
            legacy_recordings(&files)
        };
        Ok(Self { files, recordings })
    }

    pub fn count(&self) -> usize {
        self.files.len()
    }

    pub fn clear(&self, data_dir: &Path) -> Result<usize> {
        let mut removed = 0;
        let mut parents = BTreeSet::new();
        for relative in &self.files {
            let absolute = safe_join(data_dir, relative)?;
            if absolute.is_file() {
                fs::remove_file(&absolute).with_context(|| {
                    format!("could not remove managed fixture {}", absolute.display())
                })?;
                removed += 1;
                if let Some(parent) = absolute.parent() {
                    parents.insert(parent.to_path_buf());
                }
            }
        }
        for parent in parents {
            remove_empty_ancestors(&parent, data_dir)?;
        }
        for path in [
            data_dir.join(FIXTURE_MANIFEST),
            data_dir.join(RECORDINGS_MANIFEST),
        ] {
            if path.is_file() {
                fs::remove_file(path)?;
            }
        }
        Ok(removed)
    }

    pub fn store(&self, data_dir: &Path) -> Result<()> {
        fs::create_dir_all(data_dir)
            .with_context(|| format!("could not create {}", data_dir.display()))?;
        let files = lines(&self.files);
        let recordings = self
            .recordings
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        let files_content = if files.is_empty() {
            String::new()
        } else {
            format!("{files}\n")
        };
        let recordings_content = if recordings.is_empty() {
            String::new()
        } else {
            format!("{recordings}\n")
        };
        atomic_write_private(&data_dir.join(FIXTURE_MANIFEST), files_content.as_bytes())?;
        atomic_write_private(
            &data_dir.join(RECORDINGS_MANIFEST),
            recordings_content.as_bytes(),
        )?;
        Ok(())
    }
}

pub fn safe_join(base: &Path, relative: &Path) -> Result<PathBuf> {
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!(
            "managed fixture path escapes the data directory: {}",
            relative.display()
        )
    }
    Ok(base.join(relative))
}

fn read_entries(path: &Path) -> Result<BTreeSet<PathBuf>> {
    if !path.is_file() {
        return Ok(BTreeSet::new());
    }
    let content = fs::read_to_string(path)
        .with_context(|| format!("could not read fixture manifest {}", path.display()))?;
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let relative = PathBuf::from(line.trim());
            validate_manifest_entry(&relative)?;
            Ok(relative)
        })
        .collect()
}

fn validate_manifest_entry(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        bail!("empty managed fixture path")
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        bail!("unsafe managed fixture path: {}", path.display())
    }
    Ok(())
}

fn legacy_recordings(files: &BTreeSet<PathBuf>) -> BTreeSet<String> {
    files
        .iter()
        .filter_map(|path| {
            let components = path.components().collect::<Vec<_>>();
            if components.len() == 6
                && components[0].as_os_str() == "voice_recordings"
                && components[5]
                    .as_os_str()
                    .to_string_lossy()
                    .ends_with(".ogg")
            {
                Some(
                    components[5]
                        .as_os_str()
                        .to_string_lossy()
                        .trim_end_matches(".ogg")
                        .to_string(),
                )
            } else {
                None
            }
        })
        .collect()
}

fn lines(paths: &BTreeSet<PathBuf>) -> String {
    paths
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join("\n")
}

fn remove_empty_ancestors(start: &Path, stop: &Path) -> Result<()> {
    let mut current = start.to_path_buf();
    while current != stop && current.starts_with(stop) {
        if !current.is_dir() || fs::read_dir(&current)?.next().is_some() {
            break;
        }
        fs::remove_dir(&current)?;
        if !current.pop() {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn legacy_manifest_is_understood_and_unrelated_media_survives() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        fs::create_dir_all(data.join("voice_recordings/1/2/2026/07")).unwrap();
        fs::write(
            data.join(FIXTURE_MANIFEST),
            "voice_recordings/1/2/2026/07/a.ogg\nclips/a.ogg\n",
        )
        .unwrap();
        fs::write(data.join("voice_recordings/1/2/2026/07/a.ogg"), b"x").unwrap();
        fs::create_dir_all(data.join("clips")).unwrap();
        fs::write(data.join("clips/a.ogg"), b"x").unwrap();
        fs::write(data.join("unrelated.ogg"), b"keep").unwrap();

        let manifest = ManagedManifest::load(data).unwrap();
        assert!(manifest.recordings.contains("a"));
        assert_eq!(manifest.clear(data).unwrap(), 2);
        assert!(data.join("unrelated.ogg").is_file());
    }

    #[test]
    fn traversal_is_rejected() {
        assert!(safe_join(Path::new("/tmp/data"), Path::new("../secret")).is_err());
        assert!(validate_manifest_entry(Path::new("/tmp/secret")).is_err());
    }

    #[test]
    fn manifest_files_are_private_and_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let mut manifest = ManagedManifest::default();
        manifest.files.insert(PathBuf::from("clips/a.ogg"));
        manifest.recordings.insert("a".into());
        manifest.store(dir.path()).unwrap();
        assert_eq!(
            fs::metadata(dir.path().join(FIXTURE_MANIFEST))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
