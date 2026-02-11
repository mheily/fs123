/// Backend abstraction layer for fs123-server
///
/// This module provides a `Backend` trait that abstracts filesystem operations,
/// enabling the server to serve files from sources other than the local filesystem.

#[cfg(feature = "database")]
pub mod database;
mod file;
mod traits;
mod types;

#[cfg(feature = "database")]
pub use database::DatabaseBackend;
pub use file::{EstaleCookieSource, FileBackend};
pub use traits::{Backend, WritableBackend};
pub use types::{
    AttributeInfo, BackendError, BackendResult, DirEntry, DirectoryListing, FileContent,
    StatfsInfo, UploadSession,
};

use std::path::PathBuf;
use std::sync::Arc;
use url::Url;

/// Parse ESTALE cookie source from string
fn parse_estale_cookie_source(s: &str) -> Result<EstaleCookieSource, String> {
    match s.to_lowercase().as_str() {
        "ioc_getversion" | "getversion" | "ioctl" => Ok(EstaleCookieSource::GetVersionIoctl),
        "xattr" | "extendedattribute" | "extended_attribute" => {
            Ok(EstaleCookieSource::ExtendedAttribute)
        }
        "inode" | "st_ino" => Ok(EstaleCookieSource::Inode),
        "none" | "disabled" | "0" => Ok(EstaleCookieSource::None),
        _ => Err(format!(
            "Invalid estalecookie value: '{}'. Valid values: ioc_getversion, xattr, inode, none",
            s
        )),
    }
}

/// Create a backend from a parsed URL
///
/// Supported URL schemes:
/// - `file://` - Local filesystem backend
/// - `sqlite://` - SQLite database backend (requires "database" feature)
///
/// # URL Parameters (for file:// URLs)
/// - `estalecookie=<strategy>` - ESTALE cookie strategy:
///   - `ioc_getversion` (default) - Use FS_IOC_GETVERSION ioctl
///   - `xattr` - Use extended attributes (not yet implemented)
///   - `inode` - Use inode number (not recommended)
///   - `none` - Disabled (return 0)
///
/// # Examples
/// ```rust,no_run
/// # async fn example() {
/// use url::Url;
/// use fs123_server::backends::create_backend;
///
/// let url = Url::parse("file:///srv/data").unwrap();
/// let backend = create_backend(&url).await.unwrap();
///
/// // With database feature:
/// let url = Url::parse("sqlite:///var/lib/fs123/metadata.db").unwrap();
/// let backend = create_backend(&url).await.unwrap();
/// # }
/// ```
pub async fn create_backend(url: &Url) -> Result<Arc<dyn Backend>, String> {
    // Parse query parameters
    let mut estalecookie_src = EstaleCookieSource::GetVersionIoctl; // Default
    for (key, value) in url.query_pairs() {
        if key == "estalecookie" {
            estalecookie_src = parse_estale_cookie_source(value.as_ref())?;
        }
    }

    // Check the scheme
    match url.scheme() {
        "file" => {
            // For file:// URLs, the path() method returns the path
            let path_buf = PathBuf::from(url.path());
            let backend = FileBackend::with_estale_strategy(path_buf, estalecookie_src);
            Ok(Arc::new(backend))
        }

        #[cfg(feature = "database")]
        "sqlite" => {
            // For sqlite:// URLs, create a database backend
            // Convert sqlite:// to sqlite: format expected by sqlx
            let db_url = url.as_str();
            let backend = DatabaseBackend::new(db_url)
                .await
                .map_err(|e| format!("Failed to create database backend: {}", e))?;
            backend
                .initialize_schema()
                .await
                .map_err(|e| format!("Failed to initialize database schema: {}", e))?;
            Ok(Arc::new(backend))
        }

        #[cfg(not(feature = "database"))]
        "sqlite" => Err(
            "SQLite backend requires the 'database' feature. \
             Recompile with: cargo build --features database"
                .to_string(),
        ),

        scheme => Err(format!("Unsupported backend scheme: {}", scheme)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_backend_file_url() {
        let url = Url::parse("file:///tmp/test").unwrap();
        let backend = create_backend(&url).await.unwrap();
        assert_eq!(backend.describe(), "file:///tmp/test");
    }

    #[tokio::test]
    async fn test_create_backend_unsupported_scheme() {
        let url = Url::parse("http://example.com/path").unwrap();
        let result = create_backend(&url).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.contains("Unsupported backend scheme"));
    }

    #[tokio::test]
    async fn test_create_backend_with_estale_url_param() {
        // Test inode strategy via URL parameter
        let url = Url::parse("file:///tmp/test?estalecookie=inode").unwrap();
        let backend = create_backend(&url).await.unwrap();
        assert_eq!(backend.describe(), "file:///tmp/test");

        // Test none strategy
        let url = Url::parse("file:///tmp/test?estalecookie=none").unwrap();
        let backend = create_backend(&url).await.unwrap();
        assert_eq!(backend.describe(), "file:///tmp/test");

        // Test default (no parameter)
        let url = Url::parse("file:///tmp/test").unwrap();
        let backend = create_backend(&url).await.unwrap();
        assert_eq!(backend.describe(), "file:///tmp/test");
    }

    #[tokio::test]
    async fn test_create_backend_invalid_estale_param() {
        let url = Url::parse("file:///tmp/test?estalecookie=invalid").unwrap();
        let result = create_backend(&url).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.contains("Invalid estalecookie value"));
    }
}
