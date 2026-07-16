use std::{path::Path, time::Duration};

use chrono::Utc;
use futures_util::future::join_all;
use sakiot_storage::{
    FileDigest, StorageError, StorageErrorKind, clip_object_key, hash_file, recording_object_key,
};
use sqlx::{Pool, Postgres};
use tracing::{error, info, warn};

use super::{MediaArchive, metrics, repository};
use repository::{SourceId, WorkItem};

const RECONCILE_INTERVAL: Duration = Duration::from_secs(15);
const IDLE_INTERVAL: Duration = Duration::from_secs(1);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);
const LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(60);
const UPLOAD_CONCURRENCY: i64 = 2;

pub fn spawn_archive_worker(pool: Pool<Postgres>, media: MediaArchive) {
    if !media.enabled() {
        info!("media archive disabled; filesystem-only mode active");
        return;
    }
    let reconcile_pool = pool.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(RECONCILE_INTERVAL);
        loop {
            ticker.tick().await;
            match repository::reconcile(&reconcile_pool).await {
                Ok(inserted) if inserted > 0 => {
                    info!(inserted, "media archive reconciliation queued objects");
                }
                Ok(_) => {}
                Err(error) => error!(?error, "media archive reconciliation failed"),
            }
        }
    });
    tokio::spawn(async move {
        let owner = lease_owner();
        loop {
            let work = match repository::claim_batch(&pool, &owner, UPLOAD_CONCURRENCY).await {
                Ok(work) => work,
                Err(error) => {
                    error!(?error, "media archive claim failed");
                    tokio::time::sleep(IDLE_INTERVAL).await;
                    continue;
                }
            };
            if work.is_empty() {
                record_status(&pool).await;
                tokio::time::sleep(IDLE_INTERVAL).await;
                continue;
            }

            join_all(
                work.into_iter()
                    .map(|item| process_item(&pool, &media, &owner, item)),
            )
            .await;
            record_status(&pool).await;
        }
    });
}

pub fn spawn_local_cleanup(pool: Pool<Postgres>, media: MediaArchive) {
    let Some(config) = media.config().cloned() else {
        return;
    };
    if !config.local_prune_enabled {
        info!("media local pruning disabled");
    }
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(CLEANUP_INTERVAL);
        loop {
            ticker.tick().await;
            if config.local_prune_enabled {
                match cleanup_once(&pool, config.local_cache_max_bytes).await {
                    Ok((removed, remaining)) => {
                        metrics::record_evictable_cache_bytes(remaining);
                        if removed > 0 {
                            info!(removed, remaining, "media local cleanup complete");
                        }
                    }
                    Err(error) => error!(?error, "media local cleanup failed"),
                }
            } else {
                match cache_bytes(&pool).await {
                    Ok(bytes) => metrics::record_evictable_cache_bytes(bytes),
                    Err(error) => error!(?error, "media local cache measurement failed"),
                }
            }
        }
    });
}

pub(crate) async fn process_available_batch(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    owner: &str,
) -> Result<usize, crate::errors::AppError> {
    let work = repository::claim_batch(pool, owner, UPLOAD_CONCURRENCY).await?;
    let count = work.len();
    join_all(
        work.into_iter()
            .map(|item| process_item(pool, media, owner, item)),
    )
    .await;
    Ok(count)
}

async fn process_item(pool: &Pool<Postgres>, media: &MediaArchive, owner: &str, item: WorkItem) {
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let lease_keeper = tokio::spawn(keep_lease(pool.clone(), item.id, owner.to_owned(), stop_rx));
    let result = process_item_inner(pool, media, owner, &item).await;
    let _ = stop_tx.send(true);
    let _ = lease_keeper.await;
    if let Err(error) = result {
        error!(
            media_object_id = item.id,
            ?error,
            "media archive item processing failed"
        );
        if let Err(db_error) =
            repository::mark_pending(pool, &item, owner, &error.to_string()).await
        {
            error!(
                media_object_id = item.id,
                ?db_error,
                "media retry state update failed"
            );
        }
    }
}

async fn keep_lease(
    pool: Pool<Postgres>,
    id: i64,
    owner: String,
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    let mut ticker = tokio::time::interval(LEASE_RENEW_INTERVAL);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                match repository::renew_lease(&pool, id, &owner).await {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(error) => warn!(media_object_id = id, ?error, "media lease renewal failed"),
                }
            }
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return;
                }
            }
        }
    }
}

async fn process_item_inner(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    owner: &str,
    item: &WorkItem,
) -> Result<(), ProcessingError> {
    let archive = media
        .archive()
        .ok_or_else(|| ProcessingError::Local("archive disabled".to_owned()))?;

    let local_exists = tokio::fs::try_exists(&item.path)
        .await
        .map_err(|error| ProcessingError::Local(error.to_string()))?;
    if !local_exists {
        let Some((key, digest)) = existing_digest(item) else {
            repository::mark_missing(pool, item.id, owner, "local source file is missing").await?;
            warn!(
                media_object_id = item.id,
                path = %item.path.display(),
                "archive source missing before verification"
            );
            return Ok(());
        };
        let started = std::time::Instant::now();
        match archive.verify_object(key, &digest).await {
            Ok(head) => {
                repository::mark_uploaded(pool, item.id, owner, head.etag.as_deref()).await?;
                finish_available(pool, media, owner, item, head.etag.as_deref()).await?;
                metrics::transfer_success("verify", digest.bytes, started.elapsed());
                return Ok(());
            }
            Err(error) if error.kind() == StorageErrorKind::Integrity => {
                metrics::verification_failure();
                repository::mark_conflict(pool, item.id, owner, &error.to_string()).await?;
                return Ok(());
            }
            Err(error) if error.kind() == StorageErrorKind::NotFound => {
                repository::mark_missing(
                    pool,
                    item.id,
                    owner,
                    "local and archived source files are missing",
                )
                .await?;
                return Ok(());
            }
            Err(error) => return Err(ProcessingError::Storage(error)),
        }
    }

    let digest = hash_file(&item.path).await?;
    let Some(key) = object_key(&item.source, &digest.sha256) else {
        repository::mark_conflict(
            pool,
            item.id,
            owner,
            "source id cannot be represented in opaque object key",
        )
        .await?;
        return Ok(());
    };
    if !repository::record_prepared(pool, item, owner, &key, digest.bytes, &digest.sha256).await? {
        repository::mark_conflict(
            pool,
            item.id,
            owner,
            "local bytes changed after archive metadata was recorded",
        )
        .await?;
        return Ok(());
    }

    let existing = archive.head(&key).await?;
    let etag = if let Some(head) = existing {
        repository::mark_uploaded(pool, item.id, owner, head.etag.as_deref()).await?;
        head.etag
    } else {
        let started = std::time::Instant::now();
        let uploaded = match archive.upload_file(&key, &item.path, &digest).await {
            Ok(uploaded) => uploaded,
            Err(error) => {
                metrics::upload_failure();
                return Err(ProcessingError::Storage(error));
            }
        };
        metrics::transfer_success("upload", digest.bytes, started.elapsed());
        repository::mark_uploaded(pool, item.id, owner, uploaded.etag.as_deref()).await?;
        uploaded.etag
    };

    if !repository::renew_lease(pool, item.id, owner).await? {
        return Err(ProcessingError::Local(
            "archive lease lost before verification".to_owned(),
        ));
    }
    let started = std::time::Instant::now();
    match archive.verify_object(&key, &digest).await {
        Ok(head) => {
            metrics::transfer_success("verify", digest.bytes, started.elapsed());
            finish_available(
                pool,
                media,
                owner,
                item,
                head.etag.as_deref().or(etag.as_deref()),
            )
            .await
        }
        Err(error) if error.kind() == StorageErrorKind::Integrity => {
            metrics::verification_failure();
            repository::mark_conflict(pool, item.id, owner, &error.to_string()).await?;
            Ok(())
        }
        Err(error) => {
            metrics::verification_failure();
            Err(ProcessingError::Storage(error))
        }
    }
}

async fn finish_available(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    owner: &str,
    item: &WorkItem,
    etag: Option<&str>,
) -> Result<(), ProcessingError> {
    let retention_days = media
        .config()
        .map_or(7, |config| config.local_retention_days);
    if !repository::mark_available(pool, item.id, owner, etag, retention_days).await? {
        return Err(ProcessingError::Local(
            "archive lease lost while marking verification complete".to_owned(),
        ));
    }
    info!(
        media_object_id = item.id,
        source = ?item.source,
        "media archive object fully verified"
    );
    Ok(())
}

fn existing_digest(item: &WorkItem) -> Option<(&str, FileDigest)> {
    Some((
        item.object_key.as_deref()?,
        FileDigest {
            bytes: item.bytes?,
            sha256: item.sha256.clone()?,
        },
    ))
}

fn object_key(source: &SourceId, sha256: &str) -> Option<String> {
    match source {
        SourceId::Recording(id) => recording_object_key(*id, sha256),
        SourceId::Clip(id) => clip_object_key(id, sha256),
    }
}

async fn cleanup_once(
    pool: &Pool<Postgres>,
    max_bytes: u64,
) -> Result<(u64, u64), crate::errors::AppError> {
    let mut objects = repository::list_available(pool).await?;
    let now = Utc::now();
    let mut removed = 0u64;

    for object in &objects {
        if object.local_delete_after <= now && remove_verified_file(&object.path).await? {
            removed += 1;
        }
    }

    let mut cached = Vec::new();
    let mut total = 0u64;
    for object in objects.drain(..) {
        let Ok(metadata) = tokio::fs::metadata(&object.path).await else {
            continue;
        };
        let bytes = metadata.len();
        total = total.saturating_add(bytes);
        cached.push(CacheEntry {
            order_ms: object.local_delete_after.timestamp_millis(),
            path: object.path,
            bytes,
            kind: CacheEntryKind::File,
        });
    }
    let roots = sakiot_paths::DataRoots::from_env();
    collect_file_cache(&roots.no_silence, &mut cached, &mut total).await?;
    collect_file_cache(&roots.waveforms, &mut cached, &mut total).await?;
    collect_hls_cache(&roots.recordings, &mut cached, &mut total).await?;

    let (cap_removed, remaining) = enforce_cache_cap(cached, total, max_bytes).await?;
    removed = removed.saturating_add(cap_removed);
    total = remaining;
    Ok((removed, total))
}

async fn cache_bytes(pool: &Pool<Postgres>) -> Result<u64, crate::errors::AppError> {
    let objects = repository::list_available(pool).await?;
    let mut total = 0u64;
    let mut entries = Vec::new();
    for object in objects {
        if let Ok(metadata) = tokio::fs::metadata(object.path).await {
            total = total.saturating_add(metadata.len());
        }
    }
    let roots = sakiot_paths::DataRoots::from_env();
    collect_file_cache(&roots.no_silence, &mut entries, &mut total).await?;
    collect_file_cache(&roots.waveforms, &mut entries, &mut total).await?;
    collect_hls_cache(&roots.recordings, &mut entries, &mut total).await?;
    Ok(total)
}

#[derive(Clone, Copy)]
enum CacheEntryKind {
    File,
    Directory,
}

struct CacheEntry {
    order_ms: i64,
    path: std::path::PathBuf,
    bytes: u64,
    kind: CacheEntryKind,
}

async fn collect_file_cache(
    root: &Path,
    entries: &mut Vec<CacheEntry>,
    total: &mut u64,
) -> Result<(), crate::errors::AppError> {
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let mut children = match tokio::fs::read_dir(directory).await {
            Ok(children) => children,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        while let Some(child) = children.next_entry().await? {
            let file_type = child.file_type().await?;
            if file_type.is_dir() {
                directories.push(child.path());
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let metadata = child.metadata().await?;
            let bytes = metadata.len();
            *total = total.saturating_add(bytes);
            entries.push(CacheEntry {
                order_ms: modified_ms(&metadata),
                path: child.path(),
                bytes,
                kind: CacheEntryKind::File,
            });
        }
    }
    Ok(())
}

async fn collect_hls_cache(
    root: &Path,
    entries: &mut Vec<CacheEntry>,
    total: &mut u64,
) -> Result<(), crate::errors::AppError> {
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let mut children = match tokio::fs::read_dir(directory).await {
            Ok(children) => children,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        while let Some(child) = children.next_entry().await? {
            if !child.file_type().await?.is_dir() {
                continue;
            }
            let name = child.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("hls-") || name.starts_with("mix-") {
                let metadata = child.metadata().await?;
                let bytes = directory_bytes(&child.path()).await?;
                *total = total.saturating_add(bytes);
                entries.push(CacheEntry {
                    order_ms: modified_ms(&metadata),
                    path: child.path(),
                    bytes,
                    kind: CacheEntryKind::Directory,
                });
            } else {
                directories.push(child.path());
            }
        }
    }
    Ok(())
}

async fn directory_bytes(path: &Path) -> Result<u64, crate::errors::AppError> {
    let mut total = 0u64;
    let mut directories = vec![path.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let mut children = tokio::fs::read_dir(directory).await?;
        while let Some(child) = children.next_entry().await? {
            let file_type = child.file_type().await?;
            if file_type.is_dir() {
                directories.push(child.path());
            } else if file_type.is_file() {
                total = total.saturating_add(child.metadata().await?.len());
            }
        }
    }
    Ok(total)
}

fn modified_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}

async fn remove_cache_entry(entry: &CacheEntry) -> Result<bool, crate::errors::AppError> {
    let result = match entry.kind {
        CacheEntryKind::File => tokio::fs::remove_file(&entry.path).await,
        CacheEntryKind::Directory => tokio::fs::remove_dir_all(&entry.path).await,
    };
    match result {
        Ok(()) => {
            info!(path = %entry.path.display(), bytes = entry.bytes, "evicted local media cache");
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

async fn enforce_cache_cap(
    mut entries: Vec<CacheEntry>,
    mut total: u64,
    max_bytes: u64,
) -> Result<(u64, u64), crate::errors::AppError> {
    entries.sort_by_key(|entry| entry.order_ms);
    let mut removed = 0u64;
    for entry in entries {
        if total <= max_bytes {
            break;
        }
        if remove_cache_entry(&entry).await? {
            total = total.saturating_sub(entry.bytes);
            removed = removed.saturating_add(1);
        }
    }
    Ok((removed, total))
}

async fn remove_verified_file(path: &Path) -> Result<bool, crate::errors::AppError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {
            info!(path = %path.display(), "evicted fully verified local media cache");
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

async fn record_status(pool: &Pool<Postgres>) {
    match repository::status(pool).await {
        Ok(status) => metrics::record_status(&status),
        Err(error) => warn!(?error, "media archive metrics query failed"),
    }
}

pub(crate) fn lease_owner() -> String {
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_owned());
    format!("{host}:{}:{}", std::process::id(), uuid::Uuid::new_v4())
}

#[derive(Debug, thiserror::Error)]
enum ProcessingError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    App(#[from] crate::errors::AppError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("{0}")]
    Local(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cache_cap_evicts_oldest_entries_first() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let oldest = directory.path().join("oldest.cache");
        let newest = directory.path().join("newest.cache");
        tokio::fs::write(&oldest, b"12345").await?;
        tokio::fs::write(&newest, b"6789").await?;
        let entries = vec![
            CacheEntry {
                order_ms: 1,
                path: oldest.clone(),
                bytes: 5,
                kind: CacheEntryKind::File,
            },
            CacheEntry {
                order_ms: 2,
                path: newest.clone(),
                bytes: 4,
                kind: CacheEntryKind::File,
            },
        ];

        assert_eq!(enforce_cache_cap(entries, 9, 4).await?, (1, 4));
        assert!(!oldest.exists());
        assert!(newest.exists());
        Ok(())
    }

    #[test]
    fn clip_key_component_rejects_path_structure() {
        let digest = "a".repeat(64);
        assert!(
            object_key(
                &SourceId::Clip("550e8400-e29b-41d4-a716-446655440000".to_owned()),
                &digest,
            )
            .is_some()
        );
        assert!(object_key(&SourceId::Clip("../clip".to_owned()), &digest).is_none());
        assert!(object_key(&SourceId::Clip("guild/clip".to_owned()), &digest).is_none());
    }
}
