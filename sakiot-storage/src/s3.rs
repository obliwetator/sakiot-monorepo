use std::{path::Path, time::Duration};

use aws_sdk_s3::{
    Client,
    config::{
        BehaviorVersion, Credentials, Region, RequestChecksumCalculation,
        ResponseChecksumValidation, retry::RetryConfig, timeout::TimeoutConfig,
    },
    error::ProvideErrorMetadata,
    primitives::ByteStream,
    types::{CompletedMultipartUpload, CompletedPart, ServerSideEncryption},
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{ArchiveConfig, ByteRange, FileDigest};

pub const MULTIPART_THRESHOLD_BYTES: u64 = 64 * 1024 * 1024;
pub const MULTIPART_PART_BYTES: usize = 64 * 1024 * 1024;
const MAX_ATTEMPTS: u32 = 5;
const STALE_PARTIAL_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone)]
pub struct Archive {
    client: Client,
    bucket: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectHead {
    pub bytes: u64,
    pub etag: Option<String>,
    pub content_type: Option<String>,
    pub sha256: Option<String>,
}

pub struct RemoteBody {
    pub head: ObjectHead,
    pub body: ByteStream,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadResult {
    pub etag: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageErrorKind {
    NotFound,
    Unavailable,
    Integrity,
    InvalidResponse,
    LocalIo,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct StorageError {
    kind: StorageErrorKind,
    message: String,
}

impl StorageError {
    pub fn kind(&self) -> StorageErrorKind {
        self.kind
    }

    fn unavailable(operation: &str, error: impl std::fmt::Display) -> Self {
        Self {
            kind: StorageErrorKind::Unavailable,
            message: format!("B2 {operation} failed: {error}"),
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: StorageErrorKind::InvalidResponse,
            message: message.into(),
        }
    }

    fn integrity(message: impl Into<String>) -> Self {
        Self {
            kind: StorageErrorKind::Integrity,
            message: message.into(),
        }
    }
}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        Self {
            kind: StorageErrorKind::LocalIo,
            message: error.to_string(),
        }
    }
}

impl Archive {
    pub async fn new(config: &ArchiveConfig) -> Self {
        let credentials = Credentials::new(
            config.access_key_id.clone(),
            config.secret_access_key.clone(),
            None,
            None,
            "sakiot-media-archive",
        );
        let s3_config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(config.region.clone()))
            .credentials_provider(credentials)
            .endpoint_url(&config.endpoint)
            .retry_config(RetryConfig::standard().with_max_attempts(MAX_ATTEMPTS))
            .timeout_config(
                TimeoutConfig::builder()
                    .connect_timeout(Duration::from_secs(5))
                    .read_timeout(Duration::from_secs(30))
                    .build(),
            )
            .force_path_style(true)
            // B2 does not require AWS's optional SDK checksum trailers. The
            // archive performs its own full streamed SHA-256 verification.
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
            .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
            .build();
        Self {
            client: Client::from_conf(s3_config),
            bucket: config.bucket.clone(),
        }
    }

    #[cfg(test)]
    pub fn from_client(client: Client, bucket: impl Into<String>) -> Self {
        Self {
            client,
            bucket: bucket.into(),
        }
    }

    pub async fn head(&self, key: &str) -> Result<Option<ObjectHead>, StorageError> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(output) => Ok(Some(ObjectHead {
                bytes: u64::try_from(output.content_length().unwrap_or_default())
                    .map_err(|_| StorageError::invalid("B2 returned negative content length"))?,
                etag: output.e_tag().map(str::to_owned),
                content_type: output.content_type().map(str::to_owned),
                sha256: output
                    .metadata()
                    .and_then(|metadata| metadata.get("sha256"))
                    .cloned(),
            })),
            Err(error) if is_not_found(error.as_service_error()) => Ok(None),
            Err(error) => Err(StorageError::unavailable("HEAD", error)),
        }
    }

    pub async fn get(
        &self,
        key: &str,
        range: Option<ByteRange>,
    ) -> Result<RemoteBody, StorageError> {
        let mut request = self.client.get_object().bucket(&self.bucket).key(key);
        if let Some(range) = range {
            request = request.range(range.request_header());
        }
        match request.send().await {
            Ok(output) => {
                let bytes = u64::try_from(output.content_length().unwrap_or_default())
                    .map_err(|_| StorageError::invalid("B2 returned negative content length"))?;
                Ok(RemoteBody {
                    head: ObjectHead {
                        bytes,
                        etag: output.e_tag().map(str::to_owned),
                        content_type: output.content_type().map(str::to_owned),
                        sha256: output
                            .metadata()
                            .and_then(|metadata| metadata.get("sha256"))
                            .cloned(),
                    },
                    body: output.body,
                })
            }
            Err(error) if is_not_found(error.as_service_error()) => Err(StorageError {
                kind: StorageErrorKind::NotFound,
                message: format!("B2 object not found: {key}"),
            }),
            Err(error) => Err(StorageError::unavailable("GET", error)),
        }
    }

    pub async fn upload_file(
        &self,
        key: &str,
        path: &Path,
        digest: &FileDigest,
    ) -> Result<UploadResult, StorageError> {
        if digest.bytes >= MULTIPART_THRESHOLD_BYTES {
            self.multipart_upload(key, path, digest).await
        } else {
            self.single_upload(key, path, digest).await
        }
    }

    async fn single_upload(
        &self,
        key: &str,
        path: &Path,
        digest: &FileDigest,
    ) -> Result<UploadResult, StorageError> {
        let body = ByteStream::read_from()
            .path(path)
            .buffer_size(1024 * 1024)
            .build()
            .await
            .map_err(|error| StorageError {
                kind: StorageErrorKind::LocalIo,
                message: error.to_string(),
            })?;
        let output = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type("audio/ogg")
            .server_side_encryption(ServerSideEncryption::Aes256)
            .metadata("sha256", &digest.sha256)
            .body(body)
            .send()
            .await
            .map_err(|error| StorageError::unavailable("PutObject", error))?;
        Ok(UploadResult {
            etag: output.e_tag().map(str::to_owned),
        })
    }

    async fn multipart_upload(
        &self,
        key: &str,
        path: &Path,
        digest: &FileDigest,
    ) -> Result<UploadResult, StorageError> {
        let created = self
            .client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .content_type("audio/ogg")
            .server_side_encryption(ServerSideEncryption::Aes256)
            .metadata("sha256", &digest.sha256)
            .send()
            .await
            .map_err(|error| StorageError::unavailable("CreateMultipartUpload", error))?;
        let upload_id = created
            .upload_id()
            .ok_or_else(|| StorageError::invalid("B2 omitted multipart upload id"))?
            .to_owned();

        let result = match self.upload_parts(key, path, &upload_id).await {
            Ok(parts) => {
                let upload = CompletedMultipartUpload::builder()
                    .set_parts(Some(parts))
                    .build();
                self.client
                    .complete_multipart_upload()
                    .bucket(&self.bucket)
                    .key(key)
                    .upload_id(&upload_id)
                    .multipart_upload(upload)
                    .send()
                    .await
                    .map_err(|error| StorageError::unavailable("CompleteMultipartUpload", error))
                    .map(|output| UploadResult {
                        etag: output.e_tag().map(str::to_owned),
                    })
            }
            Err(error) => Err(error),
        };

        match result {
            Ok(uploaded) => Ok(uploaded),
            Err(error) => {
                if let Err(abort_error) = self
                    .client
                    .abort_multipart_upload()
                    .bucket(&self.bucket)
                    .key(key)
                    .upload_id(&upload_id)
                    .send()
                    .await
                {
                    return Err(StorageError {
                        kind: error.kind,
                        message: format!(
                            "{error}; B2 AbortMultipartUpload also failed: {abort_error}"
                        ),
                    });
                }
                Err(error)
            }
        }
    }

    async fn upload_parts(
        &self,
        key: &str,
        path: &Path,
        upload_id: &str,
    ) -> Result<Vec<CompletedPart>, StorageError> {
        let mut file = tokio::fs::File::open(path).await?;
        let mut parts = Vec::new();
        let mut part_number = 1i32;
        loop {
            let mut bytes = vec![0u8; MULTIPART_PART_BYTES];
            let mut filled = 0usize;
            while filled < bytes.len() {
                let read = file.read(&mut bytes[filled..]).await?;
                if read == 0 {
                    break;
                }
                filled += read;
            }
            if filled == 0 {
                break;
            }
            bytes.truncate(filled);
            let output = self
                .client
                .upload_part()
                .bucket(&self.bucket)
                .key(key)
                .upload_id(upload_id)
                .part_number(part_number)
                .body(ByteStream::from(bytes))
                .send()
                .await
                .map_err(|error| StorageError::unavailable("UploadPart", error))?;
            let etag = output
                .e_tag()
                .ok_or_else(|| StorageError::invalid("B2 omitted multipart part ETag"))?;
            parts.push(
                CompletedPart::builder()
                    .part_number(part_number)
                    .e_tag(etag)
                    .build(),
            );
            part_number = part_number
                .checked_add(1)
                .ok_or_else(|| StorageError::invalid("multipart part number overflow"))?;
        }
        Ok(parts)
    }

    pub async fn verify_object(
        &self,
        key: &str,
        expected: &FileDigest,
    ) -> Result<ObjectHead, StorageError> {
        let Some(head) = self.head(key).await? else {
            return Err(StorageError {
                kind: StorageErrorKind::NotFound,
                message: format!("B2 object not found: {key}"),
            });
        };
        validate_head(&head, expected)?;
        let remote = self.get(key, None).await?;
        let mut reader = remote.body.into_async_read();
        let mut hasher = Sha256::new();
        let mut bytes = 0u64;
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .await
                .map_err(|error| StorageError::unavailable("GET body", error))?;
            if read == 0 {
                break;
            }
            bytes += read as u64;
            hasher.update(&buffer[..read]);
        }
        let sha256 = hex::encode(hasher.finalize());
        if bytes != expected.bytes || sha256 != expected.sha256 {
            return Err(StorageError::integrity(format!(
                "B2 full verification mismatch: expected {} bytes/{}, got {bytes}/{sha256}",
                expected.bytes, expected.sha256
            )));
        }
        Ok(head)
    }

    pub async fn download_verified(
        &self,
        key: &str,
        target: &Path,
        expected: &FileDigest,
    ) -> Result<(), StorageError> {
        let parent = target
            .parent()
            .ok_or_else(|| StorageError::invalid("target has no parent directory"))?;
        tokio::fs::create_dir_all(parent).await?;
        cleanup_stale_partials(parent, target).await;
        let temporary = parent.join(format!(
            ".{}.{}.partial",
            target
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("media"),
            uuid::Uuid::new_v4()
        ));
        let result = self.download_to_temporary(key, &temporary, expected).await;
        match result {
            Ok(()) => match tokio::fs::rename(&temporary, target).await {
                Ok(()) => Ok(()),
                Err(error) => {
                    let _ = tokio::fs::remove_file(&temporary).await;
                    Err(error.into())
                }
            },
            Err(error) => {
                let _ = tokio::fs::remove_file(&temporary).await;
                Err(error)
            }
        }
    }

    async fn download_to_temporary(
        &self,
        key: &str,
        temporary: &Path,
        expected: &FileDigest,
    ) -> Result<(), StorageError> {
        let remote = self.get(key, None).await?;
        validate_head(&remote.head, expected)?;
        let mut reader = remote.body.into_async_read();
        let mut output = tokio::fs::File::create(temporary).await?;
        let mut hasher = Sha256::new();
        let mut bytes = 0u64;
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .await
                .map_err(|error| StorageError::unavailable("GET body", error))?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read]).await?;
            hasher.update(&buffer[..read]);
            bytes += read as u64;
        }
        output.flush().await?;
        output.sync_all().await?;
        let sha256 = hex::encode(hasher.finalize());
        if bytes != expected.bytes || sha256 != expected.sha256 {
            return Err(StorageError::integrity(format!(
                "B2 download mismatch: expected {} bytes/{}, got {bytes}/{sha256}",
                expected.bytes, expected.sha256
            )));
        }
        Ok(())
    }
}

async fn cleanup_stale_partials(parent: &Path, target: &Path) {
    let target_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("media");
    let prefix = format!(".{target_name}.");
    let Ok(mut entries) = tokio::fs::read_dir(parent).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(&prefix) || !name.ends_with(".partial") {
            continue;
        }
        let stale = entry
            .metadata()
            .await
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= STALE_PARTIAL_AGE);
        if stale {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

fn validate_head(head: &ObjectHead, expected: &FileDigest) -> Result<(), StorageError> {
    if head.bytes != expected.bytes {
        return Err(StorageError::integrity(format!(
            "B2 HEAD size mismatch: expected {}, got {}",
            expected.bytes, head.bytes
        )));
    }
    if head.sha256.as_deref() != Some(expected.sha256.as_str()) {
        return Err(StorageError::integrity("B2 HEAD SHA-256 metadata mismatch"));
    }
    Ok(())
}

fn is_not_found(error: Option<&(impl ProvideErrorMetadata + ?Sized)>) -> bool {
    error
        .and_then(ProvideErrorMetadata::code)
        .is_some_and(|code| matches!(code, "NotFound" | "NoSuchKey" | "404"))
}
