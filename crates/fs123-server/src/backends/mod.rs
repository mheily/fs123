/// Backend abstraction layer for fs123-server
///
/// This module provides a `Backend` trait that abstracts filesystem operations,
/// enabling the server to serve files from sources other than the local filesystem.

mod file;
mod traits;
mod types;

pub use file::{EstaleCookieSource, FileBackend};
pub use traits::Backend;
pub use types::{
    AttributeInfo, BackendError, BackendResult, DirEntry, DirectoryListing, FileContent,
    StatfsInfo,
};

use std::path::PathBuf;
use std::sync::Arc;

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

/// Create a backend from a URL string
///
/// Supported URL schemes:
/// - `file:///path` or `/path` - Local filesystem backend
///
/// If no scheme is provided, defaults to `file://` scheme.
///
/// # URL Parameters (for file:// URLs)
/// - `estalecookie=<strategy>` - ESTALE cookie strategy:
///   - `ioc_getversion` (default) - Use FS_IOC_GETVERSION ioctl
///   - `xattr` - Use extended attributes (not yet implemented)
///   - `inode` - Use inode number (not recommended)
///   - `none` - Disabled (return 0)
///
/// # Examples
/// - `file:///srv/data` - Use default (ioc_getversion)
/// - `file:///srv/data?estalecookie=inode` - Use inode strategy
/// - `/srv/data?estalecookie=none` - Bare path with disabled cookies
pub fn create_backend(url: &str) -> Result<Arc<dyn Backend>, String> {
    // Split URL into path and query parts
    let (url_base, query_str) = url.split_once('?').unwrap_or((url, ""));

    // Parse query parameters
    let mut estalecookie_src = EstaleCookieSource::GetVersionIoctl; // Default
    if !query_str.is_empty() {
        for param in query_str.split('&') {
            if let Some((key, value)) = param.split_once('=') {
                if key == "estalecookie" {
                    estalecookie_src = parse_estale_cookie_source(value)?;
                }
            }
        }
    }

    // Parse the URL base to determine the backend type
    let (scheme, path) = if let Some(rest) = url_base.strip_prefix("file://") {
        ("file", rest)
    } else if url_base.starts_with('/') {
        // Bare path, treat as file://
        ("file", url_base)
    } else if url_base.contains("://") {
        // Unknown scheme
        let scheme = url_base.split("://").next().unwrap_or("");
        return Err(format!("Unsupported backend scheme: {}", scheme));
    } else {
        // Relative path, treat as file://
        ("file", url_base)
    };

    match scheme {
        "file" => {
            let path_buf = PathBuf::from(path);
            let backend = FileBackend::with_estale_strategy(path_buf, estalecookie_src);
            Ok(Arc::new(backend))
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

    #[test]
    fn test_create_backend_with_estale_url_param() {
        // Test inode strategy via URL parameter
        let backend = create_backend("/tmp/test?estalecookie=inode").unwrap();
        assert_eq!(backend.describe(), "file:///tmp/test");

        // Test none strategy
        let backend = create_backend("file:///tmp/test?estalecookie=none").unwrap();
        assert_eq!(backend.describe(), "file:///tmp/test");

        // Test default (no parameter)
        let backend = create_backend("/tmp/test").unwrap();
        assert_eq!(backend.describe(), "file:///tmp/test");
    }

    #[test]
    fn test_create_backend_invalid_estale_param() {
        let result = create_backend("/tmp/test?estalecookie=invalid");
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.contains("Invalid estalecookie value"));
    }
}
