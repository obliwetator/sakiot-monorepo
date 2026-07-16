mod cli;
mod metrics;
mod repository;
mod serve;
mod worker;

use std::{path::Path, sync::Arc};

use sakiot_storage::{Archive, ArchiveConfig, ArchiveMode, FileDigest, StorageErrorKind};
use sqlx::{Pool, Postgres};

pub use cli::run_media_command;
pub(crate) use repository::recording_id;
pub use serve::RemoteDisposition;
pub use worker::{spawn_archive_worker, spawn_local_cleanup};

use crate::errors::AppError;

#[derive(Clone)]
pub struct MediaArchive {
    inner: Option<Arc<Archive>>,
    config: Option<ArchiveConfig>,
}

impl MediaArchive {
    pub fn disabled() -> Self {
        Self {
            inner: None,
            config: None,
        }
    }

    pub async fn from_env() -> Result<Self, sakiot_storage::ConfigError> {
        match ArchiveMode::from_env()? {
            ArchiveMode::Disabled => Ok(Self::disabled()),
            ArchiveMode::Enabled(config) => {
                let archive = Archive::new(&config).await;
                Ok(Self {
                    inner: Some(Arc::new(archive)),
                    config: Some(config),
                })
            }
        }
    }

    pub fn enabled(&self) -> bool {
        self.inner.is_some()
    }

    pub fn archive(&self) -> Option<&Arc<Archive>> {
        self.inner.as_ref()
    }

    pub fn config(&self) -> Option<&ArchiveConfig> {
        self.config.as_ref()
    }

    pub async fn ensure_recording_local(
        &self,
        pool: &Pool<Postgres>,
        audio_file_id: i64,
        path: &Path,
    ) -> Result<(), AppError> {
        self.ensure_local(pool, repository::SourceId::Recording(audio_file_id), path)
            .await
    }

    pub async fn ensure_clip_local(
        &self,
        pool: &Pool<Postgres>,
        clip_id: &str,
        path: &Path,
    ) -> Result<(), AppError> {
        self.ensure_local(pool, repository::SourceId::Clip(clip_id.to_owned()), path)
            .await
    }

    async fn ensure_local(
        &self,
        pool: &Pool<Postgres>,
        source: repository::SourceId,
        path: &Path,
    ) -> Result<(), AppError> {
        if tokio::fs::try_exists(path).await.unwrap_or(false) {
            return Ok(());
        }
        let archive = self.archive().ok_or(AppError::FileNotFound)?;
        let object = repository::available_object(pool, &source)
            .await?
            .ok_or(AppError::FileNotFound)?;
        let started = std::time::Instant::now();
        let result = archive
            .download_verified(
                &object.object_key,
                path,
                &FileDigest {
                    bytes: object.bytes,
                    sha256: object.sha256.clone(),
                },
            )
            .await;
        match result {
            Ok(()) => {
                if let Err(error) = repository::reset_local_retention(
                    pool,
                    object.id,
                    self.config()
                        .map_or(7, |config| config.local_retention_days),
                )
                .await
                {
                    let _ = tokio::fs::remove_file(path).await;
                    return Err(error);
                }
                metrics::remote_read_success(object.bytes, started.elapsed());
                Ok(())
            }
            Err(error) => {
                metrics::remote_read_failure();
                if error.kind() == StorageErrorKind::NotFound {
                    repository::mark_remote_missing(pool, object.id, &error.to_string()).await?;
                    Err(AppError::FileNotFound)
                } else if error.kind() == StorageErrorKind::Integrity {
                    repository::mark_verification_conflict(pool, object.id, &error.to_string())
                        .await?;
                    Err(AppError::ServiceUnavailable(
                        "archived media failed integrity validation".to_owned(),
                    ))
                } else {
                    Err(AppError::ServiceUnavailable(
                        "media archive unavailable".to_owned(),
                    ))
                }
            }
        }
    }

    pub async fn serve_recording(
        &self,
        request: &actix_web::HttpRequest,
        pool: &Pool<Postgres>,
        audio_file_id: i64,
        disposition: RemoteDisposition,
    ) -> Result<actix_web::HttpResponse, AppError> {
        serve::serve_remote(
            self,
            request,
            pool,
            repository::SourceId::Recording(audio_file_id),
            disposition,
        )
        .await
    }

    pub async fn serve_clip(
        &self,
        request: &actix_web::HttpRequest,
        pool: &Pool<Postgres>,
        clip_id: &str,
        disposition: RemoteDisposition,
    ) -> Result<actix_web::HttpResponse, AppError> {
        serve::serve_remote(
            self,
            request,
            pool,
            repository::SourceId::Clip(clip_id.to_owned()),
            disposition,
        )
        .await
    }
}

pub(crate) fn clip_local_path(saved_file_name: &str) -> Result<std::path::PathBuf, AppError> {
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
        return Err(AppError::InternalError);
    }
    Ok(sakiot_paths::DataRoots::from_env().clips.join(relative))
}
