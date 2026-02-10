/// Backend trait definitions

use async_trait::async_trait;
use super::types::{AttributeInfo, BackendError, BackendResult, DirectoryListing, FileContent, StatfsInfo};

/// Backend trait for abstracting filesystem operations
///
/// This trait allows the fs123 server to serve files from different sources:
/// - Local filesystem (FileBackend)
/// - Future: S3, databases, HTTP proxies, etc.
#[async_trait]
pub trait Backend: Send + Sync {
    /// Get file/directory attributes
    ///
    /// Returns stat-like information for the given path.
    /// Uses symlink_metadata semantics (does not follow symlinks).
    async fn get_attributes(&self, path: &str) -> BackendResult<AttributeInfo>;

    /// Read file content
    ///
    /// Reads `length` bytes starting at `offset` from the file at `path`.
    /// Returns the data along with validator and estalecookie.
    async fn read_file(&self, path: &str, offset: u64, length: usize) -> BackendResult<FileContent>;

    /// Read directory contents
    ///
    /// Returns up to `max_bytes` worth of directory entries, starting after `start_after`.
    /// If `start_after` is empty, starts from the beginning.
    async fn read_directory(
        &self,
        path: &str,
        max_bytes: usize,
        start_after: &str,
    ) -> BackendResult<DirectoryListing>;

    /// Read symbolic link target
    ///
    /// Returns the target path of the symlink at `path`.
    async fn read_symlink(&self, path: &str) -> BackendResult<String>;

    /// Get filesystem statistics
    ///
    /// Returns statvfs-like information for the filesystem containing `path`.
    async fn statfs(&self, path: &str) -> BackendResult<StatfsInfo>;

    /// Get extended attribute value
    ///
    /// Returns the value of the extended attribute `name` on `path`,
    /// up to `max_size` bytes.
    async fn get_xattr(&self, path: &str, name: &str, max_size: usize) -> BackendResult<Vec<u8>>;

    /// Return a human-readable description of this backend
    ///
    /// Used for logging and the /n stats endpoint.
    fn describe(&self) -> String;
}

/// Trait for backends that support write operations (v8 protocol).
///
/// All methods have default implementations returning ENOSYS.
/// Backends that support writing should override the relevant methods.
#[async_trait]
pub trait WritableBackend: Send + Sync {
    async fn mkdir(&self, _path: &str, _mode: u32) -> BackendResult<()> {
        Err(BackendError::new(libc::ENOSYS, "Not implemented"))
    }

    async fn rmdir(&self, _path: &str) -> BackendResult<()> {
        Err(BackendError::new(libc::ENOSYS, "Not implemented"))
    }

    async fn chmod(&self, _path: &str, _mode: u32) -> BackendResult<()> {
        Err(BackendError::new(libc::ENOSYS, "Not implemented"))
    }

    async fn chown(&self, _path: &str, _uid: u32, _gid: u32) -> BackendResult<()> {
        Err(BackendError::new(libc::ENOSYS, "Not implemented"))
    }

    async fn utimens(
        &self,
        _path: &str,
        _atime: (i64, i64),
        _mtime: (i64, i64),
    ) -> BackendResult<()> {
        Err(BackendError::new(libc::ENOSYS, "Not implemented"))
    }

    async fn symlink(&self, _target: &str, _linkpath: &str) -> BackendResult<()> {
        Err(BackendError::new(libc::ENOSYS, "Not implemented"))
    }

    async fn link(&self, _oldpath: &str, _newpath: &str) -> BackendResult<()> {
        Err(BackendError::new(libc::ENOSYS, "Not implemented"))
    }

    async fn unlink(&self, _path: &str) -> BackendResult<()> {
        Err(BackendError::new(libc::ENOSYS, "Not implemented"))
    }

    async fn rename(&self, _from: &str, _to: &str) -> BackendResult<()> {
        Err(BackendError::new(libc::ENOSYS, "Not implemented"))
    }

    async fn setxattr(
        &self,
        _path: &str,
        _name: &str,
        _value: &[u8],
        _flags: u32,
    ) -> BackendResult<()> {
        Err(BackendError::new(libc::ENOSYS, "Not implemented"))
    }

    async fn removexattr(&self, _path: &str, _name: &str) -> BackendResult<()> {
        Err(BackendError::new(libc::ENOSYS, "Not implemented"))
    }

    async fn check_access(&self, _path: &str, _mask: u32) -> BackendResult<()> {
        Err(BackendError::new(libc::ENOSYS, "Not implemented"))
    }
}
