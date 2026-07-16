use std::path::Path;

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::StorageError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDigest {
    pub bytes: u64,
    pub sha256: String,
}

pub async fn hash_file(path: &Path) -> Result<FileDigest, StorageError> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok(FileDigest {
        bytes,
        sha256: hex::encode(hasher.finalize()),
    })
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncWriteExt;

    use super::*;

    #[tokio::test]
    async fn hashes_streamed_file() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("audio.ogg");
        let mut file = tokio::fs::File::create(&path).await?;
        file.write_all(b"abc").await?;
        file.flush().await?;
        let digest = hash_file(&path).await?;
        assert_eq!(digest.bytes, 3);
        assert_eq!(
            digest.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        Ok(())
    }
}
