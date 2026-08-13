//! RAII temporary export bundles and deterministic media path derivation.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sakiot_paths::RecordingKey;
use tempfile::TempDir;

#[derive(Debug)]
pub struct FixtureWorkspace {
    _tempdir: TempDir,
    pub root: PathBuf,
    pub media: PathBuf,
}

impl FixtureWorkspace {
    pub fn new() -> Result<Self> {
        let tempdir = tempfile::Builder::new()
            .prefix("sakiot-dev-fixtures-")
            .tempdir()
            .context("could not create temporary fixture workspace")?;
        let root = tempdir.path().to_path_buf();
        let media = root.join("media");
        fs::create_dir_all(&media)?;
        Ok(Self {
            _tempdir: tempdir,
            root,
            media,
        })
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub fn write(&self, name: &str, data: impl AsRef<[u8]>) -> Result<PathBuf> {
        let path = self.path(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, data)?;
        Ok(path)
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        fs::read(self.path(name)).with_context(|| format!("could not read fixture export {name}"))
    }

    pub fn read_text(&self, name: &str) -> Result<String> {
        String::from_utf8(self.read(name)?)
            .with_context(|| format!("fixture export {name} is not UTF-8"))
    }

    pub fn file_exists_in_media(&self, relative: &str) -> bool {
        self.media.join(relative).is_file()
    }
}

pub fn build_media_lists(workspace: &FixtureWorkspace) -> Result<()> {
    let audio = workspace.read_text("audio_files.tsv")?;
    let clip_meta = workspace.read_text("clip_meta.tsv").unwrap_or_default();
    let mut files = BTreeSet::new();
    let mut recordings = BTreeSet::new();

    for line in audio.lines().filter(|line| !line.is_empty()) {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() < 6 {
            bail!("audio_files export has fewer than six prefix columns")
        }
        let guild = parse_i64(fields[1], "audio_files.guild_id")?;
        let channel = parse_i64(fields[2], "audio_files.channel_id")?;
        let year = fields[4]
            .parse::<i32>()
            .with_context(|| format!("invalid audio_files.year {}", fields[4]))?;
        let month = fields[5]
            .parse::<u32>()
            .with_context(|| format!("invalid audio_files.month {}", fields[5]))?;
        let key = RecordingKey::new(guild, channel, year, month, fields[0]);
        files.insert(relative(&key.recording_path("voice_recordings"))?);
        files.insert(relative(
            &key.no_silence_path("no_silence_voice_recordings"),
        )?);
        files.insert(relative(&key.waveform_path("waveform_data"))?);
        recordings.insert(fields[0].to_string());
    }

    for line in clip_meta.lines().filter(|line| !line.is_empty()) {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() >= 5 && !fields[4].is_empty() {
            let saved = Path::new(fields[4]);
            if saved.is_absolute()
                || saved.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
            {
                bail!("unsafe clip saved_file_name {}", fields[4])
            }
            files.insert(PathBuf::from("clips").join(saved));
        }
    }

    workspace.write(
        "files.list",
        files
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n")
            + if files.is_empty() { "" } else { "\n" },
    )?;
    workspace.write(
        "new-recordings.list",
        recordings
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n")
            + if recordings.is_empty() { "" } else { "\n" },
    )?;
    workspace.write(
        "clip-media.list",
        clip_meta
            .lines()
            .filter_map(|line| {
                let fields = line.split('\t').collect::<Vec<_>>();
                (fields.len() >= 5 && !fields[4].is_empty()).then(|| format!("clips/{}", fields[4]))
            })
            .collect::<Vec<_>>()
            .join("\n")
            + if clip_meta.lines().any(|line| !line.is_empty()) {
                "\n"
            } else {
                ""
            },
    )?;
    Ok(())
}

pub fn available_media_files(workspace: &FixtureWorkspace) -> Result<Vec<String>> {
    let files = workspace.read_text("files.list")?;
    Ok(files
        .lines()
        .filter(|line| !line.is_empty() && workspace.file_exists_in_media(line))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

fn relative(path: &Path) -> Result<PathBuf> {
    let mut components = path.components();
    let first = components
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty media path"))?;
    if !matches!(first, std::path::Component::Normal(_)) {
        bail!("media path is not relative: {}", path.display())
    }
    Ok(path.to_path_buf())
}

fn parse_i64(value: &str, field: &str) -> Result<i64> {
    value
        .parse::<i64>()
        .with_context(|| format!("{field} must be an integer, got {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_paths_use_sakiot_paths_layout() {
        let workspace = FixtureWorkspace::new().unwrap();
        workspace
            .write(
                "audio_files.tsv",
                "1700000000000-42\t1\t2\t42\t2026\t7\tother\n",
            )
            .unwrap();
        workspace.write("clip_meta.tsv", "").unwrap();
        build_media_lists(&workspace).unwrap();
        assert_eq!(
            workspace.read_text("files.list").unwrap(),
            concat!(
                "no_silence_voice_recordings/1/2/2026/07/_no_silence_1700000000000-42.ogg\n",
                "voice_recordings/1/2/2026/07/1700000000000-42.ogg\n",
                "waveform_data/1700000000000-42.dat\n"
            )
        );
    }
}
