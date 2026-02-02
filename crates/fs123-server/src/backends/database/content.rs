//! Content fetchers for retrieving file data from URLs

use crate::backends::types::BackendError;
use async_trait::async_trait;
use std::path::Path;
use url::Url;

/// Trait for fetching file content from URLs
#[async_trait]
pub trait ContentFetcher: Send + Sync {
    /// Fetch bytes from the given URL
    ///
    /// # Arguments
    /// * `url` - The content URL (file://, s3://, http://, etc.)
    /// * `offset` - Byte offset to start reading from
    /// * `length` - Maximum number of bytes to read
    ///
    /// # Returns
    /// The requested bytes, or an error
    async fn fetch(&self, url: &str, offset: u64, length: usize) -> Result<Vec<u8>, BackendError>;
}

/// Content fetcher that only supports file:// URLs
///
/// This is the simplest implementation - it reads from the local filesystem.
pub struct LocalContentFetcher;

impl LocalContentFetcher {
    pub fn new() -> Self {
        LocalContentFetcher
    }
}

impl Default for LocalContentFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ContentFetcher for LocalContentFetcher {
    async fn fetch(&self, url: &str, offset: u64, length: usize) -> Result<Vec<u8>, BackendError> {
        let parsed = Url::parse(url)
            .map_err(|e| BackendError::new(libc::EINVAL, format!("Invalid URL: {}", e)))?;

        match parsed.scheme() {
            "file" => {
                let path = parsed.path();
                self.read_local_file(path, offset, length).await
            }
            scheme => Err(BackendError::new(
                libc::ENOTSUP,
                format!("Unsupported URL scheme: {}", scheme),
            )),
        }
    }
}

impl LocalContentFetcher {
    async fn read_local_file(
        &self,
        path: &str,
        offset: u64,
        length: usize,
    ) -> Result<Vec<u8>, BackendError> {
        use std::io::{Read, Seek, SeekFrom};

        let path = Path::new(path);

        // Open the file
        let mut file = std::fs::File::open(path).map_err(|e| BackendError::from_io_error(&e))?;

        // Seek to offset
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| BackendError::from_io_error(&e))?;

        // Read the requested bytes
        let mut buffer = vec![0u8; length];
        let bytes_read = file
            .read(&mut buffer)
            .map_err(|e| BackendError::from_io_error(&e))?;

        buffer.truncate(bytes_read);
        Ok(buffer)
    }
}

/// Multi-scheme content fetcher that delegates to appropriate handlers
///
/// Supports file://, and can be extended to support s3://, http://, etc.
pub struct MultiSchemeContentFetcher {
    local: LocalContentFetcher,
}

impl MultiSchemeContentFetcher {
    pub fn new() -> Self {
        MultiSchemeContentFetcher {
            local: LocalContentFetcher::new(),
        }
    }
}

impl Default for MultiSchemeContentFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ContentFetcher for MultiSchemeContentFetcher {
    async fn fetch(&self, url: &str, offset: u64, length: usize) -> Result<Vec<u8>, BackendError> {
        let parsed = Url::parse(url)
            .map_err(|e| BackendError::new(libc::EINVAL, format!("Invalid URL: {}", e)))?;

        match parsed.scheme() {
            "file" => self.local.fetch(url, offset, length).await,

            // Future: Add S3 support
            // "s3" => self.s3.fetch(url, offset, length).await,

            // Future: Add HTTP support
            // "http" | "https" => self.http.fetch(url, offset, length).await,
            scheme => Err(BackendError::new(
                libc::ENOTSUP,
                format!("Unsupported URL scheme: {}", scheme),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_local_content_fetcher() {
        // Create a temp file with known content
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"Hello, World!").unwrap();
        file.flush().unwrap();

        let url = format!("file://{}", file.path().display());
        let fetcher = LocalContentFetcher::new();

        // Read entire file
        let data = fetcher.fetch(&url, 0, 100).await.unwrap();
        assert_eq!(data, b"Hello, World!");

        // Read with offset
        let data = fetcher.fetch(&url, 7, 100).await.unwrap();
        assert_eq!(data, b"World!");

        // Read partial
        let data = fetcher.fetch(&url, 0, 5).await.unwrap();
        assert_eq!(data, b"Hello");
    }

    #[tokio::test]
    async fn test_unsupported_scheme() {
        let fetcher = LocalContentFetcher::new();
        let result = fetcher.fetch("s3://bucket/key", 0, 100).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().errno, libc::ENOTSUP);
    }
}
