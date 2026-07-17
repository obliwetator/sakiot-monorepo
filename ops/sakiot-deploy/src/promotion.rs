//! Immutable production bundles prepared by a successful staging deployment.
//! Staging and production run on the same Debian host, so promotion preserves
//! runtime ABI compatibility while avoiding a second test/build cycle.

use std::fmt::Write as _;
use std::fs;
use std::io::Read as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const FORMAT_VERSION: u8 = 1;
const MANIFEST_FILE: &str = "promotion.json";

#[derive(Debug, Serialize, Deserialize)]
struct PromotionManifest {
    format_version: u8,
    tag: String,
    sha: String,
    artifacts: ArtifactDigests,
}

#[derive(Debug, Serialize, Deserialize)]
struct ArtifactDigests {
    bot: String,
    web: String,
    frontend: String,
}

/// Fully assembled bundle held under a temporary directory. Publishing is
/// one directory rename, so production never observes a partial bundle.
pub struct PreparedPromotion {
    temporary: tempfile::TempDir,
    promotion_root: PathBuf,
    tag: String,
    sha: String,
}

impl PreparedPromotion {
    pub fn new(promotion_root: &Path, tag: &str, sha: &str) -> Result<Self> {
        crate::fsx::ensure_dir_mode(promotion_root, 0o750)?;
        let temporary = tempfile::Builder::new()
            .prefix("promotion.")
            .tempdir_in(promotion_root)
            .context("failed to create temporary promotion directory")?;
        Ok(Self {
            temporary,
            promotion_root: promotion_root.to_path_buf(),
            tag: tag.to_string(),
            sha: sha.to_string(),
        })
    }

    pub fn root(&self) -> &Path {
        self.temporary.path()
    }

    pub fn bot_dir(&self) -> PathBuf {
        self.root().join("fbi-agent")
    }

    pub fn web_dir(&self) -> PathBuf {
        self.root().join("web")
    }

    pub fn frontend_dir(&self) -> PathBuf {
        self.root().join("frontend")
    }

    pub fn publish(self) -> Result<PathBuf> {
        let root = self.root();
        let bot = root.join("fbi-agent/fbi_agent");
        let web = root.join("web/web_server");
        let frontend = root.join("frontend/dist");
        require_non_empty_file(&bot, "FBI Agent")?;
        require_non_empty_file(&web, "web server")?;
        require_non_empty_dir(&frontend, "frontend")?;

        let manifest = PromotionManifest {
            format_version: FORMAT_VERSION,
            tag: self.tag.clone(),
            sha: self.sha.clone(),
            artifacts: ArtifactDigests {
                bot: digest_file(&bot)?,
                web: digest_file(&web)?,
                frontend: digest_tree(&frontend)?,
            },
        };
        let json = serde_json::to_string_pretty(&manifest)
            .context("failed to render promotion manifest")?;
        fs::write(root.join(MANIFEST_FILE), format!("{json}\n"))
            .context("failed to write promotion manifest")?;

        let target = bundle_path(&self.promotion_root, &self.tag, &self.sha);
        let old = self
            .promotion_root
            .join(format!(".replaced.{}", std::process::id()));
        if old.exists() {
            fs::remove_dir_all(&old)
                .with_context(|| format!("failed to remove stale {}", old.display()))?;
        }
        if target.exists() {
            fs::rename(&target, &old).with_context(|| {
                format!(
                    "failed to move existing promotion {} aside",
                    target.display()
                )
            })?;
        }

        let temporary = self.temporary.keep();
        if let Err(error) = fs::rename(&temporary, &target) {
            if old.exists() {
                let _ = fs::rename(&old, &target);
            }
            return Err(error)
                .with_context(|| format!("failed to publish promotion {}", target.display()));
        }
        if old.exists() {
            fs::remove_dir_all(&old)
                .with_context(|| format!("failed to remove replaced {}", old.display()))?;
        }
        Ok(target)
    }
}

pub fn bundle_path(root: &Path, tag: &str, sha: &str) -> PathBuf {
    root.join(format!("{tag}-{sha}"))
}

/// Return verified promotion. Presence with invalid manifest or digest is an
/// error, never a silent source-build fallback.
pub fn verified(root: &Path, tag: &str, sha: &str) -> Result<Option<PathBuf>> {
    let bundle = bundle_path(root, tag, sha);
    if !bundle.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(bundle.join(MANIFEST_FILE))
        .with_context(|| format!("failed to read promotion {}", bundle.display()))?;
    let manifest: PromotionManifest =
        serde_json::from_str(&content).context("invalid promotion manifest")?;
    if manifest.format_version != FORMAT_VERSION {
        bail!("unsupported promotion manifest version");
    }
    if manifest.tag != tag || manifest.sha != sha {
        bail!("promotion identity does not match requested tag and SHA");
    }

    verify_digest(
        &bundle.join("fbi-agent/fbi_agent"),
        &manifest.artifacts.bot,
        false,
    )?;
    verify_digest(
        &bundle.join("web/web_server"),
        &manifest.artifacts.web,
        false,
    )?;
    verify_digest(
        &bundle.join("frontend/dist"),
        &manifest.artifacts.frontend,
        true,
    )?;
    Ok(Some(bundle))
}

fn verify_digest(path: &Path, expected: &str, tree: bool) -> Result<()> {
    let actual = if tree {
        digest_tree(path)?
    } else {
        digest_file(path)?
    };
    if actual != expected {
        bail!("promotion digest mismatch for {}", path.display());
    }
    Ok(())
}

fn require_non_empty_file(path: &Path, label: &str) -> Result<()> {
    let valid = fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false);
    if !valid {
        bail!("prepared {label} artifact is missing or empty");
    }
    Ok(())
}

fn require_non_empty_dir(path: &Path, label: &str) -> Result<()> {
    let valid = fs::read_dir(path)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false);
    if !valid {
        bail!("prepared {label} artifact is missing or empty");
    }
    Ok(())
}

fn digest_file(path: &Path) -> Result<String> {
    require_non_empty_file(path, "file")?;
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open artifact {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to hash artifact {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(hasher.finalize()))
}

fn digest_tree(root: &Path) -> Result<String> {
    require_non_empty_dir(root, "directory")?;
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();

    let mut hasher = Sha256::new();
    for relative in files {
        let path = root.join(&relative);
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        if !metadata.is_file() {
            bail!("promotion tree contains non-file {}", path.display());
        }
        let relative = relative
            .to_str()
            .context("promotion tree contains a non-UTF-8 path")?;
        hasher.update(relative.len().to_le_bytes());
        hasher.update(relative.as_bytes());
        hasher.update((metadata.permissions().mode() & 0o7777).to_le_bytes());
        hasher.update(metadata.len().to_le_bytes());
        let mut file = fs::File::open(&path)?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
    }
    Ok(hex(hasher.finalize()))
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read promotion tree {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files(root, &path, files)?;
        } else if file_type.is_file() {
            files.push(path.strip_prefix(root)?.to_path_buf());
        } else {
            bail!(
                "promotion tree contains unsupported entry {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(promotion: &PreparedPromotion) {
        fs::create_dir_all(promotion.bot_dir()).unwrap();
        fs::write(promotion.bot_dir().join("fbi_agent"), "bot").unwrap();
        fs::create_dir_all(promotion.web_dir()).unwrap();
        fs::write(promotion.web_dir().join("web_server"), "web").unwrap();
        fs::create_dir_all(promotion.frontend_dir().join("dist/assets")).unwrap();
        fs::write(promotion.frontend_dir().join("dist/index.html"), "index").unwrap();
        fs::write(promotion.frontend_dir().join("dist/assets/app.js"), "app").unwrap();
    }

    #[test]
    fn published_bundle_verifies_and_detects_tampering() {
        let temp = tempfile::tempdir().unwrap();
        let promotion = PreparedPromotion::new(temp.path(), "v1.2.3", &"a".repeat(40)).unwrap();
        fill(&promotion);
        let path = promotion.publish().unwrap();

        assert_eq!(
            verified(temp.path(), "v1.2.3", &"a".repeat(40))
                .unwrap()
                .as_deref(),
            Some(path.as_path())
        );
        fs::write(path.join("web/web_server"), "changed").unwrap();
        assert!(verified(temp.path(), "v1.2.3", &"a".repeat(40)).is_err());
    }

    #[test]
    fn missing_bundle_is_not_an_error() {
        let temp = tempfile::tempdir().unwrap();
        assert!(
            verified(temp.path(), "v1.2.3", &"b".repeat(40))
                .unwrap()
                .is_none()
        );
    }
}
