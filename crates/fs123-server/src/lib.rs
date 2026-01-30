/// fs123-server library

pub mod response;
pub mod handlers;

// Re-export from core for convenience
pub use fs123_core::{parse_url, Fs123Request};

use std::path::PathBuf;

/// Configuration for the fs123 server
#[derive(Clone)]
pub struct ServerConfig {
    /// Root directory to export
    pub export_root: PathBuf,
    /// Default max-age for Cache-Control header (seconds)
    pub default_max_age: u32,
    /// Default stale-while-revalidate for Cache-Control header (seconds)
    pub default_stale_while_revalidate: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            export_root: PathBuf::from("/tmp/fs123-export"),
            default_max_age: 300,
            default_stale_while_revalidate: 60,
        }
    }
}
