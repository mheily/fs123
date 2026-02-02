//! Application-level cache with stale-while-revalidate semantics.
//!
//! This cache sits above the kernel-level FUSE caching and implements
//! background refresh to achieve eventual consistency while prioritizing
//! performance over freshness.

use crossbeam_channel::{Receiver, Sender};
use fs123_core::{
    types::{DirEntryData, Fs123StatResult},
    Fs123HttpClient,
};
use log::{debug, error};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// State of a cache entry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheState {
    /// Data is fresh (within max-age)
    Fresh,
    /// Data is stale but usable, background revalidation in progress
    StaleRevalidating,
    /// Data is stale, no revalidation in progress
    Stale,
    /// Data is expired (beyond stale-while-revalidate window)
    Expired,
}

/// Metadata for cache invalidation and freshness
#[derive(Debug, Clone)]
pub struct CacheMetadata {
    /// Validator from server (changes when content changes)
    #[allow(dead_code)]
    pub validator: u64,
    /// ESTALE cookie (changes when inode changes)
    #[allow(dead_code)]
    pub estalecookie: u64,
    /// When this entry was cached
    pub cached_at: Instant,
    /// TTL from server (max-age) - how long data is "fresh"
    pub max_age: Duration,
    /// Stale-while-revalidate window
    pub stale_while_revalidate: Duration,
}

impl CacheMetadata {
    /// Calculate the current state of this cache entry
    pub fn state(&self) -> CacheState {
        let age = self.cached_at.elapsed();

        if age <= self.max_age {
            CacheState::Fresh
        } else if age <= self.max_age + self.stale_while_revalidate {
            CacheState::Stale
        } else {
            CacheState::Expired
        }
    }

    /// Check if data should be served (fresh or stale-revalidatable)
    #[allow(dead_code)]
    pub fn is_servable(&self) -> bool {
        matches!(
            self.state(),
            CacheState::Fresh | CacheState::Stale | CacheState::StaleRevalidating
        )
    }

    /// Check if background revalidation should be triggered
    #[allow(dead_code)]
    pub fn needs_revalidation(&self) -> bool {
        matches!(self.state(), CacheState::Stale)
    }
}

/// Generic cache entry wrapper
#[derive(Debug, Clone)]
pub struct CacheEntry<T> {
    /// The cached data
    pub data: T,
    /// Cache metadata for freshness/validation
    pub metadata: CacheMetadata,
    /// Whether revalidation is currently in progress
    pub revalidating: bool,
}

impl<T: Clone> CacheEntry<T> {
    pub fn new(data: T, metadata: CacheMetadata) -> Self {
        CacheEntry {
            data,
            metadata,
            revalidating: false,
        }
    }

    /// Get the current state of this cache entry
    /// Considers both time-based staleness and revalidation status
    pub fn state(&self) -> CacheState {
        let base_state = self.metadata.state();
        // If data is Stale and revalidation is in progress, return StaleRevalidating
        if base_state == CacheState::Stale && self.revalidating {
            CacheState::StaleRevalidating
        } else {
            base_state
        }
    }
}

/// Cached attribute data (from /a endpoint)
#[derive(Debug, Clone)]
pub struct CachedAttributes {
    pub stat: Fs123StatResult,
    pub validator: u64,
    pub estalecookie: u64,
}

/// Cached directory listing
#[derive(Debug, Clone)]
pub struct CachedDirectory {
    pub entries: Vec<DirEntryData>,
    pub estalecookie: u64,
}

/// Cached symlink target
#[derive(Debug, Clone)]
pub struct CachedSymlink {
    pub target: String,
}

/// Configuration for the cache
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of attribute entries
    pub max_attr_entries: usize,
    /// Maximum number of directory entries
    pub max_dir_entries: usize,
    /// Maximum number of symlink entries
    pub max_symlink_entries: usize,
    /// Default TTL when server doesn't provide Cache-Control
    pub default_max_age: Duration,
    /// Default stale-while-revalidate window
    pub default_swr: Duration,
    /// Enable background refresh
    pub enable_background_refresh: bool,
    /// Number of background refresh threads
    pub refresh_threads: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        CacheConfig {
            max_attr_entries: 10_000,
            max_dir_entries: 1_000,
            max_symlink_entries: 5_000,
            default_max_age: Duration::from_secs(60),
            default_swr: Duration::from_secs(300),
            enable_background_refresh: true,
            refresh_threads: 2,
        }
    }
}

/// Request for background revalidation
#[derive(Debug, Clone)]
pub enum RevalidateRequest {
    Attributes {
        path: String,
        current_validator: u64,
        current_cookie: u64,
    },
    Directory {
        path: String,
        current_cookie: u64,
    },
    Symlink {
        path: String,
    },
}

/// The main cache manager
pub struct Fs123Cache {
    /// Attribute cache: path -> entry
    attrs: RwLock<HashMap<String, CacheEntry<CachedAttributes>>>,
    /// Directory cache: path -> entry
    dirs: RwLock<HashMap<String, CacheEntry<CachedDirectory>>>,
    /// Symlink cache: path -> entry
    symlinks: RwLock<HashMap<String, CacheEntry<CachedSymlink>>>,
    /// Configuration
    pub config: CacheConfig,
    /// Channel to send revalidation requests to background thread
    revalidate_tx: Sender<RevalidateRequest>,
}

impl Fs123Cache {
    pub fn new(config: CacheConfig) -> (Self, Receiver<RevalidateRequest>) {
        let (tx, rx) = crossbeam_channel::unbounded();

        (
            Fs123Cache {
                attrs: RwLock::new(HashMap::new()),
                dirs: RwLock::new(HashMap::new()),
                symlinks: RwLock::new(HashMap::new()),
                config,
                revalidate_tx: tx,
            },
            rx,
        )
    }

    /// Get cached attributes, triggering background refresh if stale
    pub fn get_attr(&self, path: &str) -> Option<CachedAttributes> {
        let attrs = self.attrs.read();
        if let Some(entry) = attrs.get(path) {
            match entry.state() {
                CacheState::Fresh => {
                    debug!("Cache hit (fresh): attr for {}", path);
                    return Some(entry.data.clone());
                }
                CacheState::Stale | CacheState::StaleRevalidating => {
                    debug!("Cache hit (stale): attr for {}", path);
                    let data = entry.data.clone();
                    let needs_revalidation = !entry.revalidating;
                    drop(attrs);
                    // Trigger background revalidation if not already in progress
                    if needs_revalidation {
                        self.trigger_attr_revalidation(path, &data);
                    }
                    return Some(data);
                }
                CacheState::Expired => {
                    debug!("Cache miss (expired): attr for {}", path);
                    return None;
                }
            }
        }
        debug!("Cache miss: attr for {}", path);
        None
    }

    /// Store attributes in cache
    pub fn put_attr(&self, path: &str, attrs: CachedAttributes, metadata: CacheMetadata) {
        let mut cache = self.attrs.write();

        // Check for out-of-order validator (indicates stale data from cache)
        if let Some(existing) = cache.get(path) {
            if attrs.validator < existing.data.validator {
                debug!(
                    "Rejecting out-of-order attr response for {}: {} < {}",
                    path, attrs.validator, existing.data.validator
                );
                return;
            }
            // ESTALE cookie change means file was replaced
            if attrs.estalecookie != existing.data.estalecookie
                && existing.data.estalecookie != 0
                && attrs.estalecookie != 0
            {
                debug!("ESTALE cookie changed for {}, invalidating caches", path);
            }
        }

        // Evict if at capacity (simple random eviction for now)
        if cache.len() >= self.config.max_attr_entries && !cache.contains_key(path) {
            self.evict_random_attr(&mut cache);
        }

        cache.insert(path.to_string(), CacheEntry::new(attrs, metadata));
        debug!("Cached attr for {}", path);
    }

    /// Trigger background revalidation for attributes
    fn trigger_attr_revalidation(&self, path: &str, current: &CachedAttributes) {
        // Mark as revalidating
        if let Some(entry) = self.attrs.write().get_mut(path) {
            entry.revalidating = true;
        }

        let _ = self.revalidate_tx.send(RevalidateRequest::Attributes {
            path: path.to_string(),
            current_validator: current.validator,
            current_cookie: current.estalecookie,
        });
        debug!("Queued attr revalidation for {}", path);
    }

    /// Clear revalidating flag for attributes
    pub fn clear_revalidating_attr(&self, path: &str) {
        if let Some(entry) = self.attrs.write().get_mut(path) {
            entry.revalidating = false;
        }
    }

    /// Get cached directory listing
    pub fn get_dir(&self, path: &str) -> Option<CachedDirectory> {
        let dirs = self.dirs.read();
        if let Some(entry) = dirs.get(path) {
            match entry.state() {
                CacheState::Fresh => {
                    debug!("Cache hit (fresh): dir for {}", path);
                    return Some(entry.data.clone());
                }
                CacheState::Stale | CacheState::StaleRevalidating => {
                    debug!("Cache hit (stale): dir for {}", path);
                    let data = entry.data.clone();
                    let needs_revalidation = !entry.revalidating;
                    drop(dirs);
                    if needs_revalidation {
                        self.trigger_dir_revalidation(path, &data);
                    }
                    return Some(data);
                }
                CacheState::Expired => {
                    debug!("Cache miss (expired): dir for {}", path);
                    return None;
                }
            }
        }
        debug!("Cache miss: dir for {}", path);
        None
    }

    /// Store directory listing in cache
    pub fn put_dir(&self, path: &str, dir: CachedDirectory, metadata: CacheMetadata) {
        let mut cache = self.dirs.write();

        if cache.len() >= self.config.max_dir_entries && !cache.contains_key(path) {
            self.evict_random_dir(&mut cache);
        }

        cache.insert(path.to_string(), CacheEntry::new(dir, metadata));
        debug!("Cached dir for {}", path);
    }

    /// Trigger background revalidation for directory
    fn trigger_dir_revalidation(&self, path: &str, current: &CachedDirectory) {
        if let Some(entry) = self.dirs.write().get_mut(path) {
            entry.revalidating = true;
        }

        let _ = self.revalidate_tx.send(RevalidateRequest::Directory {
            path: path.to_string(),
            current_cookie: current.estalecookie,
        });
        debug!("Queued dir revalidation for {}", path);
    }

    /// Clear revalidating flag for directory
    pub fn clear_revalidating_dir(&self, path: &str) {
        if let Some(entry) = self.dirs.write().get_mut(path) {
            entry.revalidating = false;
        }
    }

    /// Get cached symlink target
    pub fn get_symlink(&self, path: &str) -> Option<String> {
        let symlinks = self.symlinks.read();
        if let Some(entry) = symlinks.get(path) {
            match entry.state() {
                CacheState::Fresh => {
                    debug!("Cache hit (fresh): symlink for {}", path);
                    return Some(entry.data.target.clone());
                }
                CacheState::Stale | CacheState::StaleRevalidating => {
                    debug!("Cache hit (stale): symlink for {}", path);
                    let target = entry.data.target.clone();
                    let needs_revalidation = !entry.revalidating;
                    drop(symlinks);
                    if needs_revalidation {
                        self.trigger_symlink_revalidation(path);
                    }
                    return Some(target);
                }
                CacheState::Expired => {
                    debug!("Cache miss (expired): symlink for {}", path);
                    return None;
                }
            }
        }
        debug!("Cache miss: symlink for {}", path);
        None
    }

    /// Store symlink target in cache
    pub fn put_symlink(&self, path: &str, target: String, metadata: CacheMetadata) {
        let mut cache = self.symlinks.write();

        if cache.len() >= self.config.max_symlink_entries && !cache.contains_key(path) {
            self.evict_random_symlink(&mut cache);
        }

        let symlink = CachedSymlink { target };
        cache.insert(path.to_string(), CacheEntry::new(symlink, metadata));
        debug!("Cached symlink for {}", path);
    }

    /// Trigger background revalidation for symlink
    fn trigger_symlink_revalidation(&self, path: &str) {
        if let Some(entry) = self.symlinks.write().get_mut(path) {
            entry.revalidating = true;
        }

        let _ = self.revalidate_tx.send(RevalidateRequest::Symlink {
            path: path.to_string(),
        });
        debug!("Queued symlink revalidation for {}", path);
    }

    /// Clear revalidating flag for symlink
    pub fn clear_revalidating_symlink(&self, path: &str) {
        if let Some(entry) = self.symlinks.write().get_mut(path) {
            entry.revalidating = false;
        }
    }

    /// Simple random eviction for attribute cache
    fn evict_random_attr(&self, cache: &mut HashMap<String, CacheEntry<CachedAttributes>>) {
        if let Some(key) = cache.keys().next().cloned() {
            cache.remove(&key);
            debug!("Evicted attr cache entry: {}", key);
        }
    }

    /// Simple random eviction for directory cache
    fn evict_random_dir(&self, cache: &mut HashMap<String, CacheEntry<CachedDirectory>>) {
        if let Some(key) = cache.keys().next().cloned() {
            cache.remove(&key);
            debug!("Evicted dir cache entry: {}", key);
        }
    }

    /// Simple random eviction for symlink cache
    fn evict_random_symlink(&self, cache: &mut HashMap<String, CacheEntry<CachedSymlink>>) {
        if let Some(key) = cache.keys().next().cloned() {
            cache.remove(&key);
            debug!("Evicted symlink cache entry: {}", key);
        }
    }

    /// Invalidate all caches for a path (when estalecookie changes)
    pub fn invalidate_path(&self, path: &str) {
        self.attrs.write().remove(path);
        self.dirs.write().remove(path);
        self.symlinks.write().remove(path);
        debug!("Invalidated all caches for {}", path);
    }
}

/// Background refresher that processes revalidation requests
pub struct BackgroundRefresher {
    // Thread handles kept alive to maintain background workers
    // Threads exit automatically when the channel is closed
    _handles: Vec<thread::JoinHandle<()>>,
}

impl BackgroundRefresher {
    pub fn new(
        receiver: Receiver<RevalidateRequest>,
        client: Arc<Mutex<Fs123HttpClient>>,
        cache: Arc<Fs123Cache>,
        num_threads: usize,
    ) -> Self {
        let mut handles = Vec::with_capacity(num_threads);

        for i in 0..num_threads {
            let rx = receiver.clone();
            let client = Arc::clone(&client);
            let cache = Arc::clone(&cache);

            let handle = thread::Builder::new()
                .name(format!("cache-refresh-{}", i))
                .spawn(move || {
                    Self::worker_loop(rx, client, cache);
                })
                .expect("Failed to spawn background refresh thread");

            handles.push(handle);
        }

        BackgroundRefresher {
            _handles: handles,
        }
    }

    fn worker_loop(
        rx: Receiver<RevalidateRequest>,
        client: Arc<Mutex<Fs123HttpClient>>,
        cache: Arc<Fs123Cache>,
    ) {
        while let Ok(request) = rx.recv() {
            match request {
                RevalidateRequest::Attributes {
                    path,
                    current_validator,
                    current_cookie,
                } => {
                    Self::revalidate_attr(&client, &cache, &path, current_validator, current_cookie);
                }
                RevalidateRequest::Directory { path, current_cookie } => {
                    Self::revalidate_dir(&client, &cache, &path, current_cookie);
                }
                RevalidateRequest::Symlink { path } => {
                    Self::revalidate_symlink(&client, &cache, &path);
                }
            }
        }
        debug!("Background refresh thread exiting");
    }

    fn revalidate_attr(
        client: &Arc<Mutex<Fs123HttpClient>>,
        cache: &Arc<Fs123Cache>,
        path: &str,
        current_validator: u64,
        current_cookie: u64,
    ) {
        let client_guard = client.lock().unwrap();

        match client_guard.request_raw("a", path, None) {
            Ok(response) => {
                if let Some(errno) = response.errno() {
                    if errno != 0 {
                        debug!("Revalidation failed for {}: errno={}", path, errno);
                        cache.clear_revalidating_attr(path);
                        return;
                    }
                }

                let new_validator = response.validator().unwrap_or(0);
                let new_cookie = response.estalecookie().unwrap_or(0);

                // Check for out-of-order (stale proxy response)
                if new_validator < current_validator {
                    debug!(
                        "Rejecting out-of-order revalidation for {}: {} < {}",
                        path, new_validator, current_validator
                    );
                    cache.clear_revalidating_attr(path);
                    return;
                }

                // Check for ESTALE condition
                if new_cookie != 0 && current_cookie != 0 && new_cookie != current_cookie {
                    debug!("ESTALE detected during revalidation for {}", path);
                    cache.invalidate_path(path);
                    return;
                }

                // Parse and update cache
                if let Some(content) = response.content_str() {
                    if let Some(stat) = Fs123StatResult::from_str(&content) {
                        let attrs = CachedAttributes {
                            stat,
                            validator: new_validator,
                            estalecookie: new_cookie,
                        };
                        let metadata = CacheMetadata {
                            validator: new_validator,
                            estalecookie: new_cookie,
                            cached_at: Instant::now(),
                            max_age: cache.config.default_max_age,
                            stale_while_revalidate: cache.config.default_swr,
                        };
                        cache.put_attr(path, attrs, metadata);
                        debug!("Background refresh completed for {}", path);
                    } else {
                        error!("Failed to parse stat result during revalidation for {}", path);
                        cache.clear_revalidating_attr(path);
                    }
                } else {
                    error!("Missing content during revalidation for {}", path);
                    cache.clear_revalidating_attr(path);
                }
            }
            Err(e) => {
                error!("Revalidation request failed for {}: {}", path, e);
                cache.clear_revalidating_attr(path);
            }
        }
    }

    fn revalidate_dir(
        client: &Arc<Mutex<Fs123HttpClient>>,
        cache: &Arc<Fs123Cache>,
        path: &str,
        _current_cookie: u64,
    ) {
        let client_guard = client.lock().unwrap();

        let mut all_entries = Vec::new();
        let mut nextstart: Option<String> = None;

        loop {
            let params = match &nextstart {
                Some(start) => vec!["64".to_string(), start.clone()],
                None => vec!["64".to_string(), String::new()],
            };
            let params_ref: Vec<&str> = params.iter().map(|s| s.as_str()).collect();

            match client_guard.request("d", path, Some(&params_ref)) {
                Ok(response) => {
                    if let Some(content) = response.content() {
                        let entries = DirEntryData::parse_entries(content);
                        all_entries.extend(entries);
                    }

                    if response.has_more_entries() {
                        if let Some(next) = response.nextstart() {
                            nextstart = Some(String::from_utf8_lossy(&next).to_string());
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                Err(e) => {
                    error!("Directory revalidation failed for {}: {}", path, e);
                    cache.clear_revalidating_dir(path);
                    return;
                }
            }
        }

        let cached_dir = CachedDirectory {
            entries: all_entries,
            estalecookie: 0,
        };

        let metadata = CacheMetadata {
            validator: 0,
            estalecookie: 0,
            cached_at: Instant::now(),
            max_age: cache.config.default_max_age,
            stale_while_revalidate: cache.config.default_swr,
        };

        cache.put_dir(path, cached_dir, metadata);
        debug!("Background refresh completed for directory {}", path);
    }

    fn revalidate_symlink(
        client: &Arc<Mutex<Fs123HttpClient>>,
        cache: &Arc<Fs123Cache>,
        path: &str,
    ) {
        let client_guard = client.lock().unwrap();

        match client_guard.request("l", path, None) {
            Ok(response) => {
                if let Some(target) = response.content_str() {
                    let metadata = CacheMetadata {
                        validator: 0,
                        estalecookie: 0,
                        cached_at: Instant::now(),
                        max_age: cache.config.default_max_age,
                        stale_while_revalidate: cache.config.default_swr,
                    };
                    cache.put_symlink(path, target, metadata);
                    debug!("Background refresh completed for symlink {}", path);
                } else {
                    error!("Missing content during symlink revalidation for {}", path);
                    cache.clear_revalidating_symlink(path);
                }
            }
            Err(e) => {
                error!("Symlink revalidation failed for {}: {}", path, e);
                cache.clear_revalidating_symlink(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_state_transitions() {
        let metadata = CacheMetadata {
            validator: 1,
            estalecookie: 1,
            cached_at: Instant::now() - Duration::from_secs(30),
            max_age: Duration::from_secs(60),
            stale_while_revalidate: Duration::from_secs(300),
        };

        assert_eq!(metadata.state(), CacheState::Fresh);
        assert!(metadata.is_servable());
        assert!(!metadata.needs_revalidation());

        // Simulate time passing to stale
        let metadata_stale = CacheMetadata {
            validator: 1,
            estalecookie: 1,
            cached_at: Instant::now() - Duration::from_secs(70),
            max_age: Duration::from_secs(60),
            stale_while_revalidate: Duration::from_secs(300),
        };

        assert_eq!(metadata_stale.state(), CacheState::Stale);
        assert!(metadata_stale.is_servable());
        assert!(metadata_stale.needs_revalidation());

        // Simulate time passing to expired
        let metadata_expired = CacheMetadata {
            validator: 1,
            estalecookie: 1,
            cached_at: Instant::now() - Duration::from_secs(400),
            max_age: Duration::from_secs(60),
            stale_while_revalidate: Duration::from_secs(300),
        };

        assert_eq!(metadata_expired.state(), CacheState::Expired);
        assert!(!metadata_expired.is_servable());
    }

    #[test]
    fn test_cache_put_get_attr() {
        let config = CacheConfig::default();
        let (cache, _rx) = Fs123Cache::new(config);

        let stat = Fs123StatResult {
            st_mode: 0o100644,
            st_nlink: 1,
            st_uid: 1000,
            st_gid: 1000,
            st_size: 1024,
            st_mtime: 1234567890,
            st_ctime: 1234567890,
            st_atime: 1234567890,
            st_ino: 12345,
            st_mtime_nsec: 0,
            st_ctime_nsec: 0,
            st_atime_nsec: 0,
            st_dev: 0,
            st_blocks: 8,
            st_blksize: 4096,
            st_rdev: 0,
        };

        let attrs = CachedAttributes {
            stat: stat.clone(),
            validator: 1,
            estalecookie: 1,
        };

        let metadata = CacheMetadata {
            validator: 1,
            estalecookie: 1,
            cached_at: Instant::now(),
            max_age: Duration::from_secs(60),
            stale_while_revalidate: Duration::from_secs(300),
        };

        // Put and get
        cache.put_attr("/test/file", attrs.clone(), metadata);
        let retrieved = cache.get_attr("/test/file");

        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.stat.st_size, 1024);
        assert_eq!(retrieved.validator, 1);
        assert_eq!(retrieved.estalecookie, 1);

        // Cache miss
        assert!(cache.get_attr("/nonexistent").is_none());
    }

    #[test]
    fn test_cache_eviction() {
        let config = CacheConfig {
            max_attr_entries: 2,
            ..Default::default()
        };
        let (cache, _rx) = Fs123Cache::new(config);

        let stat = Fs123StatResult {
            st_mode: 0o100644,
            st_nlink: 1,
            st_uid: 1000,
            st_gid: 1000,
            st_size: 1024,
            st_mtime: 1234567890,
            st_ctime: 1234567890,
            st_atime: 1234567890,
            st_ino: 12345,
            st_mtime_nsec: 0,
            st_ctime_nsec: 0,
            st_atime_nsec: 0,
            st_dev: 0,
            st_blocks: 8,
            st_blksize: 4096,
            st_rdev: 0,
        };

        let metadata = CacheMetadata {
            validator: 1,
            estalecookie: 1,
            cached_at: Instant::now(),
            max_age: Duration::from_secs(60),
            stale_while_revalidate: Duration::from_secs(300),
        };

        // Fill cache to capacity
        for i in 0..3 {
            let path = format!("/test/file{}", i);
            let attrs = CachedAttributes {
                stat: stat.clone(),
                validator: i,
                estalecookie: i,
            };
            cache.put_attr(&path, attrs, metadata.clone());
        }

        // Cache should have at most max_attr_entries
        let cache_size = cache.attrs.read().len();
        assert!(cache_size <= 2);
    }

    #[test]
    fn test_out_of_order_validator_rejection() {
        let config = CacheConfig::default();
        let (cache, _rx) = Fs123Cache::new(config);

        let stat = Fs123StatResult {
            st_mode: 0o100644,
            st_nlink: 1,
            st_uid: 1000,
            st_gid: 1000,
            st_size: 1024,
            st_mtime: 1234567890,
            st_ctime: 1234567890,
            st_atime: 1234567890,
            st_ino: 12345,
            st_mtime_nsec: 0,
            st_ctime_nsec: 0,
            st_atime_nsec: 0,
            st_dev: 0,
            st_blocks: 8,
            st_blksize: 4096,
            st_rdev: 0,
        };

        // Cache entry with validator 10
        let attrs1 = CachedAttributes {
            stat: stat.clone(),
            validator: 10,
            estalecookie: 1,
        };

        let metadata1 = CacheMetadata {
            validator: 10,
            estalecookie: 1,
            cached_at: Instant::now(),
            max_age: Duration::from_secs(60),
            stale_while_revalidate: Duration::from_secs(300),
        };

        cache.put_attr("/test/file", attrs1, metadata1);

        // Try to cache entry with validator 5 (out-of-order)
        let attrs2 = CachedAttributes {
            stat: stat.clone(),
            validator: 5,
            estalecookie: 1,
        };

        let metadata2 = CacheMetadata {
            validator: 5,
            estalecookie: 1,
            cached_at: Instant::now(),
            max_age: Duration::from_secs(60),
            stale_while_revalidate: Duration::from_secs(300),
        };

        cache.put_attr("/test/file", attrs2, metadata2);

        // Should still have validator 10
        let retrieved = cache.get_attr("/test/file").unwrap();
        assert_eq!(retrieved.validator, 10);
    }

    #[test]
    fn test_symlink_cache() {
        let config = CacheConfig::default();
        let (cache, _rx) = Fs123Cache::new(config);

        let metadata = CacheMetadata {
            validator: 0,
            estalecookie: 0,
            cached_at: Instant::now(),
            max_age: Duration::from_secs(60),
            stale_while_revalidate: Duration::from_secs(300),
        };

        cache.put_symlink("/test/link", "/target/path".to_string(), metadata);

        let target = cache.get_symlink("/test/link");
        assert_eq!(target, Some("/target/path".to_string()));

        // Cache miss
        assert!(cache.get_symlink("/nonexistent").is_none());
    }

    #[test]
    fn test_directory_cache() {
        let config = CacheConfig::default();
        let (cache, _rx) = Fs123Cache::new(config);

        let entries = vec![
            DirEntryData {
                name: "file1.txt".to_string(),
                d_type: 8, // DT_REG
                estalecookie: 1,
            },
            DirEntryData {
                name: "dir1".to_string(),
                d_type: 4, // DT_DIR
                estalecookie: 2,
            },
        ];

        let dir = CachedDirectory {
            entries: entries.clone(),
            estalecookie: 1,
        };

        let metadata = CacheMetadata {
            validator: 0,
            estalecookie: 1,
            cached_at: Instant::now(),
            max_age: Duration::from_secs(60),
            stale_while_revalidate: Duration::from_secs(300),
        };

        cache.put_dir("/test/dir", dir, metadata);

        let retrieved = cache.get_dir("/test/dir");
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.entries.len(), 2);
        assert_eq!(retrieved.entries[0].name, "file1.txt");

        // Cache miss
        assert!(cache.get_dir("/nonexistent").is_none());
    }
}
