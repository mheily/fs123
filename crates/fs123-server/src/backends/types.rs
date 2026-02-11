/// Shared types for backend implementations

/// Attribute information for a file or directory
#[derive(Debug, Clone)]
pub struct AttributeInfo {
    pub mode: u32,
    pub nlink: u64,
    pub uid: u32,
    pub gid: u32,
    pub size: i64,
    pub mtime: i64,
    pub ctime: i64,
    pub atime: i64,
    pub ino: u64,
    pub mtime_nsec: i64,
    pub ctime_nsec: i64,
    pub atime_nsec: i64,
    pub dev: u64,
    pub blocks: i64,
    pub blksize: i64,
    pub rdev: u64,
    pub validator: u64,
    pub estalecookie: u64,
    /// Whether this is a regular file
    pub is_file: bool,
    /// Whether this is a directory
    pub is_dir: bool,
}

/// A single directory entry
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub d_type: u8,
    pub estalecookie: u64,
}

/// Result of reading a directory
#[derive(Debug, Clone)]
pub struct DirectoryListing {
    pub entries: Vec<DirEntry>,
    pub nextstart: String,
    pub estalecookie: u64,
}

/// Result of reading file content
#[derive(Debug, Clone)]
pub struct FileContent {
    pub data: Vec<u8>,
    pub validator: u64,
    pub estalecookie: u64,
}

/// Filesystem statistics
#[derive(Debug, Clone)]
pub struct StatfsInfo {
    pub bsize: u64,
    pub frsize: u64,
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub files: u64,
    pub ffree: u64,
    pub favail: u64,
    pub fsid: u64,
    pub flag: u64,
    pub namemax: u64,
}

/// Backend error with errno
#[derive(Debug, Clone)]
pub struct BackendError {
    pub errno: i32,
    pub message: String,
}

impl BackendError {
    pub fn new(errno: i32, message: impl Into<String>) -> Self {
        BackendError {
            errno,
            message: message.into(),
        }
    }

    pub fn from_io_error(err: &std::io::Error) -> Self {
        BackendError {
            errno: err.raw_os_error().unwrap_or(libc::EIO),
            message: err.to_string(),
        }
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BackendError(errno={}, {})", self.errno, self.message)
    }
}

impl std::error::Error for BackendError {}

/// Result type for backend operations
pub type BackendResult<T> = Result<T, BackendError>;

/// An in-progress upload session
#[derive(Debug, Clone)]
pub struct UploadSession {
    pub upload_id: String,
    pub path: String,
    pub mode: u32,
    pub next_part: u32,
    pub created_at: std::time::Instant,
    pub temp_path: std::path::PathBuf,
}
