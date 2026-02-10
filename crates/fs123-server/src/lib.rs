/// fs123-server library

pub mod backends;
pub mod handlers;
pub mod response;

// Re-export from core for convenience
pub use fs123_core::{parse_url, Fs123Request};

use std::sync::Arc;
use backends::{Backend, WritableBackend};

/// Configuration for the fs123 server
#[derive(Clone)]
pub struct ServerConfig {
    /// Backend for filesystem operations
    pub backend: Arc<dyn Backend>,
    /// Optional writable backend for v8 write operations.
    /// If None, all write operations return EROFS.
    pub writable_backend: Option<Arc<dyn WritableBackend>>,
    /// Default max-age for Cache-Control header (seconds)
    pub default_max_age: u32,
    /// Default stale-while-revalidate for Cache-Control header (seconds)
    pub default_stale_while_revalidate: u32,
}
