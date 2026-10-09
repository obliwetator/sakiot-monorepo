//! Removing a deleted session's files from the local media roots.

use super::*;

pub(super) async fn purge_job_outputs(root: &Path, id: &str) -> Result<(), AppError> {
    let mut entries = match tokio::fs::read_dir(root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let prefix = format!("{id}-");
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".ogg") {
            remove_file_if_exists(&entry.path()).await?;
        }
    }
    Ok(())
}

pub(super) async fn remove_file_if_exists(path: &Path) -> Result<(), AppError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) async fn remove_dir_if_exists(path: &Path) -> Result<(), AppError> {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) async fn purge_fragment(
    roots: &DataRoots,
    fragment: &Fragment,
) -> Result<(), AttemptError> {
    if !matches!(
        Path::new(&fragment.file_name)
            .components()
            .collect::<Vec<_>>()
            .as_slice(),
        [std::path::Component::Normal(_)]
    ) {
        return Err(AttemptError::failed(
            ErrorKind::UnsafeMediaReference,
            format!("recording fragment {} has an unsafe file name", fragment.id),
        ));
    }
    let key = RecordingKey::new(
        fragment.guild_id,
        fragment.channel_id,
        fragment.year,
        fragment.month as u32,
        fragment.file_name.clone(),
    );
    remove_file_if_exists(&key.recording_path(&roots.recordings_str())).await?;
    let no_silence = key.no_silence_path(&roots.no_silence_str());
    remove_file_if_exists(&no_silence).await?;
    if let Some(parent) = no_silence.parent() {
        purge_cached_variants(
            parent,
            &format!("{}{}.", sakiot_paths::NO_SILENCE_PREFIX, fragment.file_name),
            &[".tmp.ogg"],
        )
        .await?;
    }
    remove_file_if_exists(&key.waveform_path(&roots.waveforms_str())).await?;
    remove_file_if_exists(&roots.waveforms.join(format!(
        "{}{}.dat",
        sakiot_paths::NO_SILENCE_PREFIX,
        fragment.file_name
    )))
    .await?;
    for prefix in [
        format!("{}.", fragment.file_name),
        format!("{}{}.", sakiot_paths::NO_SILENCE_PREFIX, fragment.file_name),
    ] {
        purge_cached_variants(&roots.waveforms, &prefix, &[".tmp.dat"]).await?;
    }
    remove_dir_if_exists(&key.live_dir(&roots.recordings_str())).await?;
    Ok(())
}

pub(super) async fn purge_session_cache(
    roots: &DataRoots,
    guild_id: i64,
    id: i64,
    channel: i64,
    started: DateTime<Utc>,
    ended: Option<DateTime<Utc>>,
    fragments: &[Fragment],
) -> Result<(), AppError> {
    let started_ms = started.timestamp_millis();
    if let Some(ended) = ended {
        let session_dir = roots.no_silence.join("logical_sessions");
        remove_file_if_exists(&session_dir.join(format!(
            "{id}-{started_ms}-{}.ogg",
            ended.timestamp_millis()
        )))
        .await?;
        purge_cached_variants(
            &session_dir,
            &format!("{id}-{started_ms}-{}.", ended.timestamp_millis()),
            &[".tmp.ogg"],
        )
        .await?;
    }
    for suffix in ["", "-silence-free"] {
        remove_file_if_exists(
            &roots
                .waveforms
                .join(format!("logical-session-{id}{suffix}.dat")),
        )
        .await?;
        purge_cached_variants(
            &roots.waveforms,
            &format!("logical-session-{id}{suffix}."),
            &[".tmp.dat"],
        )
        .await?;
    }
    purge_cached_variants(
        &roots.waveforms,
        &format!("logical-session-{id}-"),
        &[".ogg"],
    )
    .await?;
    let mut directories = HashSet::new();
    directories.insert((channel, started.year(), started.month()));
    for f in fragments {
        directories.insert((f.channel_id, f.year, f.month as u32));
    }
    for (ch, year, month) in directories {
        let key = SessionKey::new(guild_id, ch, year, month, started_ms);
        // The all-recordings mix is a cache and can contain these bytes too.
        remove_dir_if_exists(&key.mix_dir(&roots.recordings_str())).await?;
    }
    Ok(())
}

pub(super) async fn purge_clip_waveforms(root: &Path, clip_id: &str) -> Result<(), AppError> {
    purge_cached_variants(root, &format!("clip-{clip_id}-"), &[".dat"]).await
}

async fn purge_cached_variants(
    root: &Path,
    prefix: &str,
    endings: &[&str],
) -> Result<(), AppError> {
    let mut entries = match tokio::fs::read_dir(root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(prefix) && endings.iter().any(|ending| name.ends_with(ending)) {
            remove_file_if_exists(&entry.path()).await?;
        }
    }
    Ok(())
}
