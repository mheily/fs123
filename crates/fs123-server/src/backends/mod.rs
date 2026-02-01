/// Backend abstraction layer for fs123-server
///
/// This module provides a `Backend` trait that abstracts filesystem operations,
/// enabling the server to serve files from sources other than the local filesystem.

mod file;
mod traits;
mod types;

pub use file::FileBackend;
pub use traits::Backend;
pub use types::{
    AttributeInfo, BackendError, BackendResult, DirEntry, DirectoryListing, FileContent,
    StatfsInfo,
};

use std::path::PathBuf;
use std::sync::Arc;

/// Create a backend from a URL string
///
/// Supported URL schemes:
/// - `file:///path` or `/path` - Local filesystem backend
///
/// If no scheme is provided, defaults to `file://` scheme.
pub fn create_backend(url: &str) -> Result<Arc<dyn Backend>, String> {
    // Parse the URL to determine the backend type
    let (scheme, path) = if let Some(rest) = url.strip_prefix("file://") {
        ("file", rest)
    } else if url.starts_with('/') {
        // Bare path, treat as file://
        ("file", url)
    } else if url.contains("://") {
        // Unknown scheme
        let scheme = url.split("://").next().unwrap_or("");
        return Err(format!("Unsupported backend scheme: {}", scheme));
    } else {
        // Relative path, treat as file://
        ("file", url)
    };

    match scheme {
        "file" => {
            let path_buf = PathBuf::from(path);
            Ok(Arc::new(FileBackend::new(path_buf)))
        }
        _ => Err(format!("Unsupported backend scheme: {}", scheme)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_backend_file_url() {
        let backend = create_backend("file:///tmp/test").unwrap();
        assert_eq!(backend.describe(), "file:///tmp/test");
    }

    #[test]
    fn test_create_backend_bare_path() {
        let backend = create_backend("/tmp/test").unwrap();
        assert_eq!(backend.describe(), "file:///tmp/test");
    }

    #[test]
    fn test_create_backend_relative_path() {
        let backend = create_backend("./test").unwrap();
        assert_eq!(backend.describe(), "file://./test");
    }

    #[test]
    fn test_create_backend_unsupported_scheme() {
        let result = create_backend("s3://bucket/path");
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.contains("Unsupported backend scheme"));
    }
}
