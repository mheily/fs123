//! Database backend for fs123-server
//!
//! This backend stores filesystem metadata in a SQL database (SQLite by default)
//! while keeping file content external, referenced by URLs.
//!
//! # Schema
//!
//! The database uses a single `files` table with denormalized data for efficient
//! queries without JOINs. See [`schema::SCHEMA`] for the full schema definition.
//!
//! # Content URLs
//!
//! File content is stored externally and referenced by URL:
//! - `file:///path/to/file` - Local filesystem
//! - `s3://bucket/key` - S3 object storage (future)
//! - `http://example.com/file` - HTTP URL (future)

mod content;
mod schema;

pub use content::{ContentFetcher, LocalContentFetcher, MultiSchemeContentFetcher};
pub use schema::{DirEntryRow, FileRow, SCHEMA};

use crate::backends::traits::Backend;
use crate::backends::types::{
    AttributeInfo, BackendError, BackendResult, DirEntry, DirectoryListing, FileContent,
    StatfsInfo,
};
use async_trait::async_trait;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::sync::Arc;

/// Database backend that stores filesystem metadata in SQLite
pub struct DatabaseBackend {
    pool: SqlitePool,
    content_fetcher: Arc<dyn ContentFetcher>,
    db_url: String,
}

impl DatabaseBackend {
    /// Create a new DatabaseBackend from a SQLite database URL
    ///
    /// # Arguments
    /// * `db_url` - SQLite database URL (e.g., "sqlite:///path/to/db.sqlite")
    ///
    /// # Example
    /// ```ignore
    /// let backend = DatabaseBackend::new("sqlite:///var/lib/fs123/metadata.db").await?;
    /// ```
    pub async fn new(db_url: &str) -> Result<Self, BackendError> {
        Self::with_content_fetcher(db_url, Arc::new(MultiSchemeContentFetcher::new())).await
    }

    /// Create a DatabaseBackend with a custom content fetcher
    pub async fn with_content_fetcher(
        db_url: &str,
        content_fetcher: Arc<dyn ContentFetcher>,
    ) -> Result<Self, BackendError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(db_url)
            .await
            .map_err(|e| BackendError::new(libc::EIO, format!("Failed to connect to database: {}", e)))?;

        Ok(DatabaseBackend {
            pool,
            content_fetcher,
            db_url: db_url.to_string(),
        })
    }

    /// Initialize the database schema
    ///
    /// Creates the `files` table if it doesn't exist.
    pub async fn initialize_schema(&self) -> Result<(), BackendError> {
        sqlx::query(SCHEMA)
            .execute(&self.pool)
            .await
            .map_err(|e| BackendError::new(libc::EIO, format!("Failed to initialize schema: {}", e)))?;
        Ok(())
    }

    /// Get the file row for a path
    async fn get_file(&self, path: &str) -> BackendResult<FileRow> {
        sqlx::query_as::<_, FileRow>("SELECT * FROM files WHERE path = ?")
            .bind(path)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| BackendError::new(libc::EIO, format!("Database error: {}", e)))?
            .ok_or_else(|| BackendError::new(libc::ENOENT, "No such file or directory"))
    }
}

#[async_trait]
impl Backend for DatabaseBackend {
    async fn get_attributes(&self, path: &str) -> BackendResult<AttributeInfo> {
        let row = self.get_file(path).await?;

        Ok(AttributeInfo {
            mode: row.mode as u32,
            nlink: row.nlink as u64,
            uid: row.uid as u32,
            gid: row.gid as u32,
            size: row.size,
            mtime: row.mtime_sec,
            ctime: row.ctime_sec,
            atime: row.atime_sec,
            ino: row.estalecookie as u64, // Use estalecookie as synthetic inode
            mtime_nsec: row.mtime_nsec,
            ctime_nsec: row.ctime_nsec,
            atime_nsec: row.atime_nsec,
            dev: 0,      // Synthetic device
            blocks: (row.size + 511) / 512,
            blksize: 4096,
            rdev: 0,
            validator: row.validator as u64,
            estalecookie: row.estalecookie as u64,
            is_file: row.is_file(),
            is_dir: row.is_dir(),
        })
    }

    async fn read_file(&self, path: &str, offset: u64, length: usize) -> BackendResult<FileContent> {
        let row = self.get_file(path).await?;

        if !row.is_file() {
            return Err(BackendError::new(libc::EINVAL, "Not a regular file"));
        }

        let content_url = row.content_url.ok_or_else(|| {
            BackendError::new(libc::EIO, "File has no content URL")
        })?;

        let data = self.content_fetcher.fetch(&content_url, offset, length).await?;

        Ok(FileContent {
            data,
            validator: row.validator as u64,
            estalecookie: row.estalecookie as u64,
        })
    }

    async fn read_directory(
        &self,
        path: &str,
        max_bytes: usize,
        start_after: &str,
    ) -> BackendResult<DirectoryListing> {
        // First verify the path is a directory
        let dir_row = self.get_file(path).await?;
        if !dir_row.is_dir() {
            return Err(BackendError::new(libc::ENOTDIR, "Not a directory"));
        }

        // Query directory entries
        let rows: Vec<DirEntryRow> = sqlx::query_as(
            "SELECT name, file_type, estalecookie FROM files
             WHERE parent_path = ? AND name > ?
             ORDER BY name
             LIMIT 1000"
        )
        .bind(path)
        .bind(start_after)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| BackendError::new(libc::EIO, format!("Database error: {}", e)))?;

        // Build entries list respecting max_bytes
        let mut entries = Vec::new();
        let mut current_size = 0;
        let mut last_name = String::new();

        for row in rows {
            // Estimate entry size (name + overhead)
            let entry_size = row.name.len() + 20;

            if current_size + entry_size > max_bytes && !entries.is_empty() {
                break;
            }

            let d_type = row.d_type();
            last_name = row.name.clone();
            entries.push(DirEntry {
                name: row.name,
                d_type,
                estalecookie: row.estalecookie as u64,
            });
            current_size += entry_size;
        }

        let nextstart = if entries.is_empty() {
            String::new()
        } else {
            last_name
        };

        Ok(DirectoryListing {
            entries,
            nextstart,
            estalecookie: dir_row.estalecookie as u64,
        })
    }

    async fn read_symlink(&self, path: &str) -> BackendResult<String> {
        let row = self.get_file(path).await?;

        if !row.is_symlink() {
            return Err(BackendError::new(libc::EINVAL, "Not a symbolic link"));
        }

        row.symlink_target.ok_or_else(|| {
            BackendError::new(libc::EIO, "Symlink has no target")
        })
    }

    async fn statfs(&self, _path: &str) -> BackendResult<StatfsInfo> {
        // Count files and calculate total size
        let stats: (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM files"
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| BackendError::new(libc::EIO, format!("Database error: {}", e)))?;

        let file_count = stats.0 as u64;
        let total_size = stats.1 as u64;
        let block_size = 4096u64;
        let total_blocks = (total_size + block_size - 1) / block_size;

        Ok(StatfsInfo {
            bsize: block_size,
            frsize: block_size,
            blocks: total_blocks.max(1000), // Report at least some space
            bfree: 0,  // Read-only filesystem
            bavail: 0,
            files: file_count,
            ffree: 0,
            favail: 0,
            fsid: 0x46533132, // "FS12" in hex
            flag: 1, // ST_RDONLY
            namemax: 255,
        })
    }

    async fn get_xattr(&self, _path: &str, _name: &str, _max_size: usize) -> BackendResult<Vec<u8>> {
        // Extended attributes not supported in this schema
        Err(BackendError::new(libc::ENOTSUP, "Extended attributes not supported"))
    }

    fn describe(&self) -> String {
        format!("database://{}", self.db_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;

    async fn create_test_db() -> DatabaseBackend {
        let backend = DatabaseBackend::new("sqlite::memory:")
            .await
            .expect("Failed to create in-memory database");
        backend.initialize_schema().await.expect("Failed to initialize schema");
        backend
    }

    async fn insert_test_file(backend: &DatabaseBackend, path: &str, parent: &str, name: &str, content_url: Option<&str>) {
        let file_type = if content_url.is_some() { "file" } else { "directory" };
        sqlx::query(
            "INSERT INTO files (path, parent_path, name, file_type, mode, nlink, uid, gid, size,
             mtime_sec, mtime_nsec, atime_sec, atime_nsec, ctime_sec, ctime_nsec,
             content_url, symlink_target, validator, estalecookie)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(path)
        .bind(parent)
        .bind(name)
        .bind(file_type)
        .bind(if content_url.is_some() { 0o644i64 } else { 0o755i64 })
        .bind(1i64)
        .bind(1000i64)
        .bind(1000i64)
        .bind(100i64)
        .bind(1704067200i64)
        .bind(0i64)
        .bind(1704067200i64)
        .bind(0i64)
        .bind(1704067200i64)
        .bind(0i64)
        .bind(content_url)
        .bind(None::<String>)
        .bind(1704067200000i64)
        .bind(42i64)
        .execute(&backend.pool)
        .await
        .expect("Failed to insert test file");
    }

    #[tokio::test]
    async fn test_get_attributes() {
        let backend = create_test_db().await;
        insert_test_file(&backend, "/", "", "", None).await;
        insert_test_file(&backend, "/test.txt", "/", "test.txt", Some("file:///tmp/test.txt")).await;

        let attrs = backend.get_attributes("/test.txt").await.unwrap();
        assert_eq!(attrs.mode, 0o644);
        assert!(attrs.is_file);
        assert!(!attrs.is_dir);
    }

    #[tokio::test]
    async fn test_get_attributes_not_found() {
        let backend = create_test_db().await;
        let result = backend.get_attributes("/nonexistent").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().errno, libc::ENOENT);
    }

    #[tokio::test]
    async fn test_read_directory() {
        let backend = create_test_db().await;
        insert_test_file(&backend, "/", "", "", None).await;
        insert_test_file(&backend, "/a.txt", "/", "a.txt", Some("file:///tmp/a.txt")).await;
        insert_test_file(&backend, "/b.txt", "/", "b.txt", Some("file:///tmp/b.txt")).await;
        insert_test_file(&backend, "/c.txt", "/", "c.txt", Some("file:///tmp/c.txt")).await;

        let listing = backend.read_directory("/", 10000, "").await.unwrap();
        assert_eq!(listing.entries.len(), 3);
        assert_eq!(listing.entries[0].name, "a.txt");
        assert_eq!(listing.entries[1].name, "b.txt");
        assert_eq!(listing.entries[2].name, "c.txt");
    }

    #[tokio::test]
    async fn test_read_file() {
        let backend = create_test_db().await;

        // Create a temp file with content
        let mut temp = NamedTempFile::new().unwrap();
        temp.write_all(b"Hello, Database Backend!").unwrap();
        temp.flush().unwrap();

        let content_url = format!("file://{}", temp.path().display());
        insert_test_file(&backend, "/", "", "", None).await;

        // Insert file with content URL
        sqlx::query(
            "INSERT INTO files (path, parent_path, name, file_type, mode, nlink, uid, gid, size,
             mtime_sec, mtime_nsec, atime_sec, atime_nsec, ctime_sec, ctime_nsec,
             content_url, symlink_target, validator, estalecookie)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind("/test.txt")
        .bind("/")
        .bind("test.txt")
        .bind("file")
        .bind(0o644i64)
        .bind(1i64)
        .bind(1000i64)
        .bind(1000i64)
        .bind(24i64)
        .bind(1704067200i64)
        .bind(0i64)
        .bind(1704067200i64)
        .bind(0i64)
        .bind(1704067200i64)
        .bind(0i64)
        .bind(&content_url)
        .bind(None::<String>)
        .bind(1704067200000i64)
        .bind(42i64)
        .execute(&backend.pool)
        .await
        .unwrap();

        let content = backend.read_file("/test.txt", 0, 1000).await.unwrap();
        assert_eq!(content.data, b"Hello, Database Backend!");
    }

    #[tokio::test]
    async fn test_statfs() {
        let backend = create_test_db().await;
        insert_test_file(&backend, "/", "", "", None).await;
        insert_test_file(&backend, "/test.txt", "/", "test.txt", Some("file:///tmp/test.txt")).await;

        let stats = backend.statfs("/").await.unwrap();
        assert_eq!(stats.files, 2);
        assert_eq!(stats.namemax, 255);
    }
}
