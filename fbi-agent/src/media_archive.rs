use std::{
    path::Path,
    sync::{Arc, OnceLock},
    time::Instant,
};

use opentelemetry::metrics::{Counter, Histogram};
use sakiot_storage::{
    Archive, ArchiveConfig, ArchiveMode, FileDigest, StorageError, StorageErrorKind,
};
use serenity::prelude::TypeMapKey;
use sqlx::{Pool, Postgres, Row};

#[derive(Clone)]
pub struct MediaArchive {
    archive: Option<Arc<Archive>>,
    config: Option<ArchiveConfig>,
}

impl MediaArchive {
    pub async fn from_env() -> Result<Self, sakiot_storage::ConfigError> {
        match ArchiveMode::from_env()? {
            ArchiveMode::Disabled => Ok(Self {
                archive: None,
                config: None,
            }),
            ArchiveMode::Enabled(config) => Ok(Self {
                archive: Some(Arc::new(Archive::new(&config).await)),
                config: Some(config),
            }),
        }
    }

    pub async fn ensure_clip_local(
        &self,
        pool: &Pool<Postgres>,
        clip_id: &str,
        saved_file_name: &str,
    ) -> Result<std::path::PathBuf, MediaArchiveError> {
        let path = clip_path(saved_file_name)?;
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(path);
        }
        let archive = self.archive.as_ref().ok_or(MediaArchiveError::Missing)?;
        let row = sqlx::query(
            "SELECT id, object_key, bytes, sha256
               FROM media_objects
              WHERE clip_id = $1
                AND state = 'available'
                AND verified_at IS NOT NULL",
        )
        .bind(clip_id)
        .fetch_optional(pool)
        .await?
        .ok_or(MediaArchiveError::Missing)?;
        let id: i64 = row.try_get("id")?;
        let object_key: String = row.try_get("object_key")?;
        let bytes = u64::try_from(row.try_get::<i64, _>("bytes")?)
            .map_err(|_| MediaArchiveError::InvalidMetadata)?;
        let sha256: String = row.try_get("sha256")?;
        let started = Instant::now();
        if let Err(error) = archive
            .download_verified(&object_key, &path, &FileDigest { bytes, sha256 })
            .await
        {
            remote_read_failures().add(1, &[]);
            let state = match error.kind() {
                StorageErrorKind::NotFound => Some("missing"),
                StorageErrorKind::Integrity => Some("conflict"),
                _ => None,
            };
            if let Some(state) = state {
                sqlx::query(
                    "UPDATE media_objects
                        SET state = $2,
                            last_error = left($3, 4000),
                            updated_at = now()
                      WHERE id = $1 AND state = 'available'",
                )
                .bind(id)
                .bind(state)
                .bind(error.to_string())
                .execute(pool)
                .await?;
            }
            return Err(error.into());
        }
        remote_read_bytes().add(bytes, &[]);
        remote_read_duration().record(started.elapsed().as_secs_f64(), &[]);
        let retention_days = self
            .config
            .as_ref()
            .map_or(7, |config| config.local_retention_days);
        let retention_days =
            i64::try_from(retention_days).map_err(|_| MediaArchiveError::InvalidMetadata)?;
        let retention_update = sqlx::query(
            "UPDATE media_objects
                SET local_delete_after = now() + ($2::bigint * interval '1 day'),
                    updated_at = now()
              WHERE id = $1 AND state = 'available' AND verified_at IS NOT NULL",
        )
        .bind(id)
        .bind(retention_days)
        .execute(pool)
        .await;
        if let Err(error) = retention_update {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(error.into());
        }
        Ok(path)
    }
}

fn remote_read_failures() -> &'static Counter<u64> {
    static COUNTER: OnceLock<Counter<u64>> = OnceLock::new();
    COUNTER.get_or_init(|| {
        opentelemetry::global::meter(crate::config::SERVICE_NAME)
            .u64_counter("media_archive_remote_read_failures")
            .with_description("Failed FBI-agent runtime reads from B2")
            .build()
    })
}

fn remote_read_bytes() -> &'static Counter<u64> {
    static COUNTER: OnceLock<Counter<u64>> = OnceLock::new();
    COUNTER.get_or_init(|| {
        opentelemetry::global::meter(crate::config::SERVICE_NAME)
            .u64_counter("media_archive_transferred_bytes")
            .with_description("Bytes hydrated from B2 for FBI-agent playback")
            .with_unit("By")
            .build()
    })
}

fn remote_read_duration() -> &'static Histogram<f64> {
    static HISTOGRAM: OnceLock<Histogram<f64>> = OnceLock::new();
    HISTOGRAM.get_or_init(|| {
        opentelemetry::global::meter(crate::config::SERVICE_NAME)
            .f64_histogram("media_archive_transfer_duration")
            .with_description("B2 clip hydration duration")
            .with_unit("s")
            .build()
    })
}

pub struct MediaArchiveKey;

impl TypeMapKey for MediaArchiveKey {
    type Value = MediaArchive;
}

pub async fn from_ctx(ctx: &serenity::client::Context) -> Option<MediaArchive> {
    ctx.data.read().await.get::<MediaArchiveKey>().cloned()
}

fn clip_path(saved_file_name: &str) -> Result<std::path::PathBuf, MediaArchiveError> {
    let relative = Path::new(saved_file_name);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(MediaArchiveError::InvalidPath);
    }
    Ok(crate::events::voice_receiver::clips_file_path().join(relative))
}

#[derive(Debug, thiserror::Error)]
pub enum MediaArchiveError {
    #[error("clip media is unavailable")]
    Missing,
    #[error("clip archive metadata is invalid")]
    InvalidMetadata,
    #[error("clip saved path is invalid")]
    InvalidPath,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Storage(#[from] StorageError),
}
