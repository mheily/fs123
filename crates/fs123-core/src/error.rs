//! Error types for the fs123 client library.

use thiserror::Error;

/// Internal error type for the fs123 client.
#[derive(Error, Debug)]
pub enum Fs123Error {
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("HTTP error {status}: {message}")]
    HttpError { status: u16, message: String },

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    #[error("Filesystem error {errno}: {message}")]
    FilesystemError { errno: i32, message: String },

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("Operation on closed handle")]
    Closed,

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
}

impl Fs123Error {
    /// Get the filesystem errno if this is a filesystem error.
    pub fn errno(&self) -> Option<i32> {
        match self {
            Fs123Error::FilesystemError { errno, .. } => Some(*errno),
            _ => None,
        }
    }

    /// Convert to a libc errno value for FUSE operations.
    pub fn to_errno(&self) -> i32 {
        match self {
            Fs123Error::FilesystemError { errno, .. } => *errno,
            Fs123Error::ConnectionFailed(_) => libc::EIO,
            Fs123Error::HttpError { status, .. } => {
                if *status >= 500 {
                    libc::EIO
                } else if *status == 404 {
                    libc::ENOENT
                } else if *status == 403 {
                    libc::EACCES
                } else {
                    libc::EIO
                }
            }
            Fs123Error::InvalidResponse(_) => libc::EIO,
            Fs123Error::ProtocolError(_) => libc::EIO,
            Fs123Error::InvalidUrl(_) => libc::EINVAL,
            Fs123Error::InvalidArgument(_) => libc::EINVAL,
            Fs123Error::Closed => libc::EBADF,
            Fs123Error::IoError(_) => libc::EIO,
        }
    }
}

pub type Result<T> = std::result::Result<T, Fs123Error>;
