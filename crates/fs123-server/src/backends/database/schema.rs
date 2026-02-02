//! SQL schema and row types for the database backend

use sqlx::FromRow;

/// SQL schema for creating the files table
pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS files (
    -- Path information
    path TEXT PRIMARY KEY,
    parent_path TEXT NOT NULL,
    name TEXT NOT NULL,

    -- File type: 'file', 'directory', 'symlink'
    file_type TEXT NOT NULL,

    -- Standard stat fields
    mode INTEGER NOT NULL,
    nlink INTEGER NOT NULL,
    uid INTEGER NOT NULL,
    gid INTEGER NOT NULL,
    size INTEGER NOT NULL,

    -- Timestamps
    mtime_sec INTEGER NOT NULL,
    mtime_nsec INTEGER NOT NULL,
    atime_sec INTEGER NOT NULL,
    atime_nsec INTEGER NOT NULL,
    ctime_sec INTEGER NOT NULL,
    ctime_nsec INTEGER NOT NULL,

    -- Content reference (NULL for directories)
    content_url TEXT,

    -- Symlink target (NULL for non-symlinks)
    symlink_target TEXT,

    -- fs123 protocol fields
    validator INTEGER NOT NULL,
    estalecookie INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_files_parent ON files(parent_path);
"#;

/// Row type for full file metadata
#[derive(Debug, Clone, FromRow)]
pub struct FileRow {
    pub path: String,
    pub parent_path: String,
    pub name: String,
    pub file_type: String,
    pub mode: i64,
    pub nlink: i64,
    pub uid: i64,
    pub gid: i64,
    pub size: i64,
    pub mtime_sec: i64,
    pub mtime_nsec: i64,
    pub atime_sec: i64,
    pub atime_nsec: i64,
    pub ctime_sec: i64,
    pub ctime_nsec: i64,
    pub content_url: Option<String>,
    pub symlink_target: Option<String>,
    pub validator: i64,
    pub estalecookie: i64,
}

/// Row type for directory entry (minimal fields for listing)
#[derive(Debug, Clone, FromRow)]
pub struct DirEntryRow {
    pub name: String,
    pub file_type: String,
    pub estalecookie: i64,
}

impl FileRow {
    /// Convert file_type string to d_type constant
    pub fn d_type(&self) -> u8 {
        match self.file_type.as_str() {
            "directory" => libc::DT_DIR,
            "file" => libc::DT_REG,
            "symlink" => libc::DT_LNK,
            _ => libc::DT_UNKNOWN,
        }
    }

    /// Check if this is a regular file
    pub fn is_file(&self) -> bool {
        self.file_type == "file"
    }

    /// Check if this is a directory
    pub fn is_dir(&self) -> bool {
        self.file_type == "directory"
    }

    /// Check if this is a symlink
    pub fn is_symlink(&self) -> bool {
        self.file_type == "symlink"
    }
}

impl DirEntryRow {
    /// Convert file_type string to d_type constant
    pub fn d_type(&self) -> u8 {
        match self.file_type.as_str() {
            "directory" => libc::DT_DIR,
            "file" => libc::DT_REG,
            "symlink" => libc::DT_LNK,
            _ => libc::DT_UNKNOWN,
        }
    }
}
