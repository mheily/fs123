//! libfs123 — C-compatible library for fs123 filesystem access.
//!
//! Provides POSIX-like functions that translate filesystem operations into
//! fs123 HTTP protocol requests.  Designed to be called via FFI from Python,
//! C, or any language with a C FFI.
//!
//! # Usage
//!
//! First mount one or more fs123 servers into a virtual namespace:
//!
//! ```c
//! fs123_mount("http://server1:8080/exports/data", "/mnt/data", NULL);
//! fs123_mount("http://server2:8080", "/mnt/logs", "cache_ttl_secs=60");
//! ```
//!
//! Then use local-looking paths with every other call:
//!
//! ```c
//! fs123_stat("/mnt/data/file.txt", &sb);
//! void *fh = fs123_open("/mnt/logs/app.log", "r");
//! ```
//!
//! The library resolves each path through the mount table (longest-prefix
//! match), strips the mount prefix, and sends the remainder as the fs123
//! filesystem path to the corresponding server.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fs123_core::{
    types::{DirEntryData, Fs123StatResult},
    Fs123Error, Fs123Function, Fs123HttpClient,
};

use std::io::{Read as _, Seek as _, SeekFrom, Write as _};

// ── In-process cache ──────────────────────────────────────────────
//
// Simple per-mount TTL cache for stat results, directory listings,
// and readlink targets.  Avoids redundant HTTP round-trips within a
// single process (e.g. stat then open on the same file).
//
// TODO: add support for redb if multi-process caching becomes necessary.

const DEFAULT_CACHE_TTL_SECS: u64 = 30;
const DEFAULT_CACHE_MAX_ENTRIES: usize = 10_000;

#[derive(Clone)]
struct CacheEntry<T: Clone> {
    value: T,
    expires: Instant,
}

struct Cache {
    ttl: Duration,
    max_entries: usize,
    stats: Mutex<HashMap<String, CacheEntry<Fs123StatResult>>>,
    dirs: Mutex<HashMap<String, CacheEntry<Vec<DirEntryData>>>>,
    links: Mutex<HashMap<String, CacheEntry<String>>>,
}

impl Cache {
    fn new(ttl: Duration, max_entries: usize) -> Self {
        Cache {
            ttl,
            max_entries,
            stats: Mutex::new(HashMap::new()),
            dirs: Mutex::new(HashMap::new()),
            links: Mutex::new(HashMap::new()),
        }
    }

    fn get_stat(&self, path: &str) -> Option<Fs123StatResult> {
        let map = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        map.get(path)
            .filter(|e| e.expires > Instant::now())
            .map(|e| e.value.clone())
    }

    fn put_stat(&self, path: &str, value: Fs123StatResult) {
        let mut map = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= self.max_entries {
            Self::evict_expired(&mut map);
        }
        map.insert(
            path.to_string(),
            CacheEntry {
                value,
                expires: Instant::now() + self.ttl,
            },
        );
    }

    fn get_dir(&self, path: &str) -> Option<Vec<DirEntryData>> {
        let map = self.dirs.lock().unwrap_or_else(|e| e.into_inner());
        map.get(path)
            .filter(|e| e.expires > Instant::now())
            .map(|e| e.value.clone())
    }

    fn put_dir(&self, path: &str, value: Vec<DirEntryData>) {
        let mut map = self.dirs.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= self.max_entries {
            Self::evict_expired(&mut map);
        }
        map.insert(
            path.to_string(),
            CacheEntry {
                value,
                expires: Instant::now() + self.ttl,
            },
        );
    }

    fn get_link(&self, path: &str) -> Option<String> {
        let map = self.links.lock().unwrap_or_else(|e| e.into_inner());
        map.get(path)
            .filter(|e| e.expires > Instant::now())
            .map(|e| e.value.clone())
    }

    fn put_link(&self, path: &str, value: String) {
        let mut map = self.links.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= self.max_entries {
            Self::evict_expired(&mut map);
        }
        map.insert(
            path.to_string(),
            CacheEntry {
                value,
                expires: Instant::now() + self.ttl,
            },
        );
    }

    fn evict_expired<T: Clone>(map: &mut HashMap<String, CacheEntry<T>>) {
        let now = Instant::now();
        map.retain(|_, e| e.expires > now);
    }
}

/// Per-mount configuration parsed from the options string.
struct MountOptions {
    cache_ttl_secs: u64,
    cache_max_entries: usize,
    mirror: bool,
}

impl Default for MountOptions {
    fn default() -> Self {
        MountOptions {
            cache_ttl_secs: DEFAULT_CACHE_TTL_SECS,
            cache_max_entries: DEFAULT_CACHE_MAX_ENTRIES,
            mirror: false,
        }
    }
}

/// Parse a comma-separated `key=value` options string.
///
/// Recognized keys:
///   - `cache_ttl_secs`     — cache entry lifetime in seconds (default 30)
///   - `cache_max_entries`   — max entries per cache map (default 10000)
///   - `mirror`             — download files to local disk (default false)
///
/// Unknown keys are silently ignored so callers can pass through
/// application-specific options without breaking.
fn parse_mount_options(opts: &str) -> Result<MountOptions, Fs123Error> {
    let mut mo = MountOptions::default();

    for token in opts.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let (key, value) = token.split_once('=').ok_or_else(|| {
            Fs123Error::InvalidArgument(format!("Bad mount option (expected key=value): {}", token))
        })?;
        match key.trim() {
            "cache_ttl_secs" => {
                mo.cache_ttl_secs = value.trim().parse().map_err(|_| {
                    Fs123Error::InvalidArgument(format!("Invalid cache_ttl_secs: {}", value))
                })?;
            }
            "cache_max_entries" => {
                mo.cache_max_entries = value.trim().parse().map_err(|_| {
                    Fs123Error::InvalidArgument(format!("Invalid cache_max_entries: {}", value))
                })?;
            }
            "mirror" => {
                mo.mirror = matches!(value.trim(), "true" | "1" | "yes");
            }
            _ => {} // ignore unknown keys
        }
    }

    Ok(mo)
}

// ── Thread-local state ────────────────────────────────────────────

thread_local! {
    static LAST_ERRNO: RefCell<c_int> = RefCell::new(0);
    static LAST_ERROR: RefCell<Option<CString>> = RefCell::new(None);
    static DEFAULT_PROTO: RefCell<String> = RefCell::new("7.3".to_string());
}

fn set_error(err: &Fs123Error) {
    let errno = err.to_errno();
    let msg = CString::new(err.to_string()).unwrap_or_default();
    LAST_ERRNO.with(|e| *e.borrow_mut() = errno);
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(msg));
}

fn set_raw_error(errno: c_int, msg: &str) {
    let msg = CString::new(msg).unwrap_or_default();
    LAST_ERRNO.with(|e| *e.borrow_mut() = errno);
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(msg));
}

fn clear_error() {
    LAST_ERRNO.with(|e| *e.borrow_mut() = 0);
    LAST_ERROR.with(|e| *e.borrow_mut() = None);
}

fn get_default_proto() -> String {
    DEFAULT_PROTO.with(|p| p.borrow().clone())
}

// ── Mount table ───────────────────────────────────────────────────

struct MountEntry {
    mount_point: String,
    client: Arc<Fs123HttpClient>,
    cache: Arc<Cache>,
    mirror: bool,
}

static MOUNT_TABLE: Mutex<Vec<MountEntry>> = Mutex::new(Vec::new());

/// Lock the mount table, recovering from a poisoned mutex if necessary.
fn lock_mount_table() -> std::sync::MutexGuard<'static, Vec<MountEntry>> {
    MOUNT_TABLE.lock().unwrap_or_else(|e| e.into_inner())
}

// ── Fstab support ────────────────────────────────────────────────
//
// Two fstab files are consulted, in order:
//   1. /etc/fs123/fstab                    — system-wide defaults
//   2. $XDG_CONFIG_HOME/fs123/fstab        — per-user overrides
//      (falls back to ~/.config/fs123/fstab when XDG_CONFIG_HOME is unset)
//
// The user file is processed *after* the system file so that entries
// with the same mountpoint silently replace the system-wide entry.
// Callers invoke fs123_mountall() to read (or re-read) these files.

const FS123_SYSTEM_FSTAB: &str = "/etc/fs123/fstab";

/// Return the path to the per-user fstab file.
///
/// Uses `$XDG_CONFIG_HOME/fs123/fstab` if the variable is set and
/// non-empty, otherwise falls back to `$HOME/.config/fs123/fstab`.
fn user_fstab_path() -> Option<std::path::PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(std::path::PathBuf::from(xdg).join("fs123/fstab"));
        }
    }
    std::env::var("HOME")
        .ok()
        .map(|home| std::path::PathBuf::from(home).join(".config/fs123/fstab"))
}

/// Read and process a single fstab file.  Errors on individual lines
/// are silently ignored so that one bad entry does not prevent the
/// rest from mounting.
fn process_fstab_file(path: &str) {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return, // file missing or unreadable — nothing to do
    };

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 2 {
            continue; // need at least url and mountpoint
        }

        let url = fields[0];
        let mountpoint = fields[1];
        let options = if fields.len() >= 3 {
            let opt = fields[2];
            if opt == "-" || opt == "none" {
                None
            } else {
                Some(opt)
            }
        } else {
            None
        };

        let _ = mount_internal(url, mountpoint, options);
    }
}

/// Shared mount implementation used by both `fs123_mount` and fstab
/// processing.
fn mount_internal(url_str: &str, mount_str: &str, options: Option<&str>) -> Result<(), Fs123Error> {
    let opts = match options {
        Some(s) => parse_mount_options(s)?,
        None => MountOptions::default(),
    };

    if !mount_str.starts_with('/') {
        return Err(Fs123Error::InvalidArgument(
            "Mount point must be an absolute path".to_string(),
        ));
    }

    let parsed = parse_fs123_url(url_str)?;
    let proto = get_default_proto();
    let client = Arc::new(Fs123HttpClient::new_with_selector(
        &parsed.host,
        parsed.port,
        &proto,
        &parsed.selector,
    ));

    let mp = mount_str.trim_end_matches('/');
    let mp = if mp.is_empty() {
        "/".to_string()
    } else {
        mp.to_string()
    };

    let mut table = lock_mount_table();

    let cache = Arc::new(Cache::new(
        Duration::from_secs(opts.cache_ttl_secs),
        opts.cache_max_entries,
    ));

    if let Some(existing) = table.iter_mut().find(|e| e.mount_point == mp) {
        existing.client = client;
        existing.cache = cache;
        existing.mirror = opts.mirror;
    } else {
        table.push(MountEntry {
            mount_point: mp,
            client,
            cache,
            mirror: opts.mirror,
        });
    }

    Ok(())
}

/// Result of resolving a local path through the mount table.
struct ResolvedPath {
    client: Arc<Fs123HttpClient>,
    cache: Arc<Cache>,
    fs_path: String,
    mount_point: String,
    mirror: bool,
}

/// Resolve a local path via the mount table (longest-prefix match).
fn resolve_path(path: &str) -> Result<ResolvedPath, Fs123Error> {
    // Normalize: strip trailing slash unless path is exactly "/"
    let path = if path.len() > 1 && path.ends_with('/') {
        &path[..path.len() - 1]
    } else {
        path
    };

    let table = lock_mount_table();

    let mut best: Option<&MountEntry> = None;
    for entry in table.iter() {
        let is_match = if entry.mount_point == "/" {
            path.starts_with('/')
        } else {
            path == entry.mount_point || path.starts_with(&format!("{}/", entry.mount_point))
        };

        if is_match && (best.is_none() || entry.mount_point.len() > best.unwrap().mount_point.len())
        {
            best = Some(entry);
        }
    }

    let mount = best
        .ok_or_else(|| Fs123Error::InvalidArgument(format!("No fs123 mount for path: {}", path)))?;

    let fs_path = if path.len() <= mount.mount_point.len() {
        "/".to_string()
    } else if mount.mount_point == "/" {
        path.to_string()
    } else {
        path[mount.mount_point.len()..].to_string()
    };

    Ok(ResolvedPath {
        client: Arc::clone(&mount.client),
        cache: Arc::clone(&mount.cache),
        fs_path,
        mount_point: mount.mount_point.clone(),
        mirror: mount.mirror,
    })
}

// ── URL parsing (used by fs123_mount) ─────────────────────────────

struct ParsedUrl {
    host: String,
    port: u16,
    /// URL path component used as selector prefix in fs123 requests.
    selector: String,
}

fn parse_fs123_url(url: &str) -> Result<ParsedUrl, Fs123Error> {
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("http://") {
        ("http", rest)
    } else if let Some(rest) = url.strip_prefix("https://") {
        ("https", rest)
    } else {
        return Err(Fs123Error::InvalidUrl(format!(
            "Unsupported scheme: {}",
            url
        )));
    };

    let default_port: u16 = if scheme == "https" { 443 } else { 80 };

    let (host_port, raw_path) = match rest.find('/') {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, ""),
    };

    let (host, port) = match host_port.rfind(':') {
        Some(pos) => {
            let port: u16 = host_port[pos + 1..]
                .parse()
                .map_err(|_| Fs123Error::InvalidUrl(format!("Invalid port in: {}", url)))?;
            (host_port[..pos].to_string(), port)
        }
        None => (host_port.to_string(), default_port),
    };

    if host.is_empty() {
        return Err(Fs123Error::InvalidUrl("Empty host".to_string()));
    }

    // Normalize: "/" → "", "/foo/" → "/foo", "" → ""
    let selector = if raw_path.is_empty() || raw_path == "/" {
        String::new()
    } else {
        raw_path.trim_end_matches('/').to_string()
    };

    Ok(ParsedUrl {
        host,
        port,
        selector,
    })
}

/// Extract a `&str` from a C string pointer.
///
/// # Safety
/// `ptr` must point to a valid, null-terminated C string.
unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> Result<&'a str, Fs123Error> {
    if ptr.is_null() {
        return Err(Fs123Error::InvalidArgument(
            "NULL string pointer".to_string(),
        ));
    }
    CStr::from_ptr(ptr)
        .to_str()
        .map_err(|_| Fs123Error::InvalidArgument("Invalid UTF-8".to_string()))
}

// ── C-compatible types ────────────────────────────────────────────

/// File stat information (mirrors POSIX `struct stat` fields).
#[repr(C)]
pub struct fs123_stat_t {
    pub st_mode: u32,
    pub st_nlink: u64,
    pub st_uid: u32,
    pub st_gid: u32,
    pub st_size: i64,
    pub st_mtime: i64,
    pub st_ctime: i64,
    pub st_atime: i64,
    pub st_ino: u64,
    pub st_mtime_nsec: i64,
    pub st_ctime_nsec: i64,
    pub st_atime_nsec: i64,
    pub st_dev: u64,
    pub st_blocks: i64,
    pub st_blksize: i64,
    pub st_rdev: u64,
}

/// Directory entry.
#[repr(C)]
pub struct fs123_dirent_t {
    /// Null-terminated entry name (max 255 bytes + NUL).
    pub name: [c_char; 256],
    /// Entry type (DT_REG, DT_DIR, etc.).
    pub d_type: u8,
}

// ── Internal handle types ─────────────────────────────────────────

enum FileHandle {
    Remote {
        client: Arc<Fs123HttpClient>,
        path: String,
        position: u64,
        file_size: i64,
    },
    Local {
        file: std::fs::File,
    },
}

struct DirHandle {
    entries: Vec<DirEntryData>,
    index: usize,
}

// ── Mirror helpers ───────────────────────────────────────────────

/// Stat a local file and convert the result to `Fs123StatResult`.
fn stat_local_file(path: &str) -> Result<Fs123StatResult, Fs123Error> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::metadata(path)?;
    Ok(Fs123StatResult {
        st_mode: metadata.mode(),
        st_nlink: metadata.nlink(),
        st_uid: metadata.uid(),
        st_gid: metadata.gid(),
        st_size: metadata.size() as i64,
        st_mtime: metadata.mtime(),
        st_ctime: metadata.ctime(),
        st_atime: metadata.atime(),
        st_ino: metadata.ino(),
        st_mtime_nsec: metadata.mtime_nsec(),
        st_ctime_nsec: metadata.ctime_nsec(),
        st_atime_nsec: metadata.atime_nsec(),
        st_dev: metadata.dev(),
        st_blocks: metadata.blocks() as i64,
        st_blksize: metadata.blksize() as i64,
        st_rdev: metadata.rdev(),
    })
}

/// Download a remote file to local disk.
///
/// Downloads in 128 KiB chunks to a temporary file, then atomically
/// renames it to the final path.  Creates parent directories as needed.
fn download_file(
    client: &Fs123HttpClient,
    fs_path: &str,
    local_path: &str,
    file_size: i64,
) -> Result<(), Fs123Error> {
    let local = std::path::Path::new(local_path);

    if let Some(parent) = local.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp_path = format!("{}.fs123dl", local_path);
    let mut tmp_file = std::fs::File::create(&tmp_path)?;

    if file_size > 0 {
        let total_bytes = file_size as u64;
        let chunk_kib: i64 = 128;
        let total_kib = ((file_size as i64) + 1023) / 1024;
        let mut offset_kib: i64 = 0;

        while offset_kib < total_kib {
            let len_kib = chunk_kib.min(total_kib - offset_kib);
            let params = [len_kib.to_string(), offset_kib.to_string()];
            let params_ref: Vec<&str> = params.iter().map(|s| s.as_str()).collect();

            let response = match client.request(Fs123Function::Read, fs_path, Some(&params_ref)) {
                Ok(r) => r,
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp_path);
                    return Err(e);
                }
            };

            let content = response.content().unwrap_or(&[]);
            if let Err(e) = tmp_file.write_all(content) {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(e.into());
            }

            offset_kib += len_kib;
        }

        // The last chunk may contain trailing bytes; truncate to exact size.
        if let Err(e) = tmp_file.set_len(total_bytes) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e.into());
        }
    }

    drop(tmp_file);

    if let Err(e) = std::fs::rename(&tmp_path, local_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    Ok(())
}

/// Stat a remote file and return its size, using the cache if available.
fn remote_file_size(
    client: &Fs123HttpClient,
    cache: &Cache,
    fs_path: &str,
) -> Result<i64, Fs123Error> {
    if let Some(cached) = cache.get_stat(fs_path) {
        return Ok(cached.st_size);
    }

    let response = client.request_raw(Fs123Function::Stat, fs_path, None)?;
    if let Some(errno) = response.errno() {
        if errno != 0 {
            return Err(Fs123Error::FilesystemError {
                errno,
                message: format!("stat: {}", fs_path),
            });
        }
    }

    let stat = response
        .get_str("content")
        .and_then(|c| Fs123StatResult::from_str(&c));

    if let Some(ref s) = stat {
        cache.put_stat(fs_path, s.clone());
    }

    Ok(stat.map(|s| s.st_size).unwrap_or(-1))
}

// ── C API: Error handling ─────────────────────────────────────────

/// Get the errno from the last failed libfs123 call on this thread.
#[no_mangle]
pub extern "C" fn fs123_errno() -> c_int {
    LAST_ERRNO.with(|e| *e.borrow())
}

/// Get a human-readable error message from the last failed call.
///
/// Returns NULL if no error has occurred.  The returned pointer is valid
/// until the next libfs123 call on the same thread.
#[no_mangle]
pub extern "C" fn fs123_strerror() -> *const c_char {
    LAST_ERROR.with(|e| match &*e.borrow() {
        Some(msg) => msg.as_ptr(),
        None => ptr::null(),
    })
}

// ── C API: Configuration ──────────────────────────────────────────

/// Set the default fs123 protocol version string (e.g. "7.3", "8.0").
///
/// Affects subsequent `fs123_mount` calls.  If never called, defaults to
/// "7.3".  Thread-local.
#[no_mangle]
pub extern "C" fn fs123_set_proto(proto: *const c_char) {
    if proto.is_null() {
        return;
    }
    if let Ok(s) = unsafe { CStr::from_ptr(proto) }.to_str() {
        DEFAULT_PROTO.with(|p| *p.borrow_mut() = s.to_string());
    }
}

// ── C API: Mount / Unmount ────────────────────────────────────────

/// Mount an fs123 server URL at a local path prefix.
///
/// `url` — the server address, e.g. `"http://server:8080/exports/data"`.
/// The path component (if any) is sent as a URL prefix (selector) in
/// every request to this server.
///
/// `mountpoint` — an absolute path prefix used to resolve subsequent
/// calls, e.g. `"/mnt/data"`.  If a mount already exists at this point
/// it is silently replaced.
///
/// `options` — optional comma-separated `key=value` pairs, or NULL for
/// defaults.  Recognized keys:
///   - `cache_ttl_secs=N`     — per-entry TTL in seconds (default 30)
///   - `cache_max_entries=N`  — max entries per cache map (default 10000)
///
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn fs123_mount(
    url: *const c_char,
    mountpoint: *const c_char,
    options: *const c_char,
) -> c_int {
    clear_error();

    let result = (|| -> Result<(), Fs123Error> {
        let url_str = unsafe { cstr_to_str(url)? };
        let mount_str = unsafe { cstr_to_str(mountpoint)? };

        let opts_str = if options.is_null() {
            None
        } else {
            Some(unsafe { cstr_to_str(options)? })
        };

        mount_internal(url_str, mount_str, opts_str)
    })();

    match result {
        Ok(()) => 0,
        Err(e) => {
            set_error(&e);
            -1
        }
    }
}

/// Unmount a previously mounted fs123 server.
///
/// Returns 0 on success, -1 if the mount point was not found.
#[no_mangle]
pub extern "C" fn fs123_umount(mountpoint: *const c_char) -> c_int {
    clear_error();

    let result = (|| -> Result<(), Fs123Error> {
        let mount_str = unsafe { cstr_to_str(mountpoint)? };
        let mp = mount_str.trim_end_matches('/');
        let mp = if mp.is_empty() { "/" } else { mp };

        let mut table = lock_mount_table();

        let before = table.len();
        table.retain(|e| e.mount_point != mp);

        if table.len() == before {
            return Err(Fs123Error::InvalidArgument(format!("Not mounted: {}", mp)));
        }

        Ok(())
    })();

    match result {
        Ok(()) => 0,
        Err(e) => {
            set_error(&e);
            -1
        }
    }
}

// ── C API: Mountall ──────────────────────────────────────────────

/// Read (or re-read) the fs123 fstab files and mount every entry.
///
/// Two files are consulted, in order:
///   1. `/etc/fs123/fstab`                    — system-wide defaults
///   2. `$XDG_CONFIG_HOME/fs123/fstab`        — per-user overrides
///      (falls back to `~/.config/fs123/fstab` when `XDG_CONFIG_HOME`
///      is unset)
///
/// The per-user file is processed after the system file so that
/// entries at the same mountpoint silently replace the system-wide
/// entry.  May be called multiple times to pick up configuration
/// changes; each call re-reads both files.
///
/// Returns 0 on success.  (Individual lines that fail to parse are
/// silently skipped, so the return value is always 0 today.)
#[no_mangle]
pub extern "C" fn fs123_mountall() -> c_int {
    clear_error();

    // 1. System-wide
    process_fstab_file(FS123_SYSTEM_FSTAB);

    // 2. Per-user (overrides system entries at same mountpoint)
    if let Some(user_path) = user_fstab_path() {
        if let Some(s) = user_path.to_str() {
            process_fstab_file(s);
        }
    }

    0
}

// ── C API: fsync (per-file mirror) ───────────────────────────────

/// Download a single remote file to local disk, regardless of whether
/// `mirror=true` is set on the mount.
///
/// The file is stored at the local path corresponding to the mount
/// point (e.g. if `http://srv` is mounted at `/mnt` and `path` is
/// `/mnt/foo/bar`, the file is written to `/mnt/foo/bar`).  Parent
/// directories are created as needed using the current umask.
///
/// If the file already exists locally it is not re-downloaded.
///
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn fs123_fsync(path: *const c_char) -> c_int {
    clear_error();

    let result = (|| -> Result<(), Fs123Error> {
        let path_str = unsafe { cstr_to_str(path)? };
        let resolved = resolve_path(path_str)?;
        let local_path = format!("{}{}", resolved.mount_point, resolved.fs_path);

        if std::path::Path::new(&local_path).exists() {
            return Ok(());
        }

        let file_size = remote_file_size(&resolved.client, &resolved.cache, &resolved.fs_path)?;
        download_file(&resolved.client, &resolved.fs_path, &local_path, file_size)
    })();

    match result {
        Ok(()) => 0,
        Err(e) => {
            set_error(&e);
            -1
        }
    }
}

// ── C API: stat ───────────────────────────────────────────────────

/// Get file/directory attributes for a path in the mount namespace.
///
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn fs123_stat(path: *const c_char, buf: *mut fs123_stat_t) -> c_int {
    clear_error();

    let result = (|| -> Result<Fs123StatResult, Fs123Error> {
        let path_str = unsafe { cstr_to_str(path)? };
        if buf.is_null() {
            return Err(Fs123Error::InvalidArgument("NULL buffer".to_string()));
        }

        let resolved = resolve_path(path_str)?;

        // Mirror mode: if the file already exists locally, stat it directly.
        if resolved.mirror {
            let local_path = format!("{}{}", resolved.mount_point, resolved.fs_path);
            if let Ok(stat) = stat_local_file(&local_path) {
                return Ok(stat);
            }
            // Not mirrored yet — fall through to remote stat.
        }

        if let Some(cached) = resolved.cache.get_stat(&resolved.fs_path) {
            return Ok(cached);
        }

        let response = resolved
            .client
            .request_raw(Fs123Function::Stat, &resolved.fs_path, None)?;

        if let Some(errno) = response.errno() {
            if errno != 0 {
                return Err(Fs123Error::FilesystemError {
                    errno,
                    message: format!("stat: {}", resolved.fs_path),
                });
            }
        }

        let content = response.get_str("content").ok_or_else(|| {
            Fs123Error::InvalidResponse("Missing content in stat response".to_string())
        })?;

        let stat = Fs123StatResult::from_str(&content).ok_or_else(|| {
            Fs123Error::InvalidResponse(format!("Cannot parse stat: {}", content))
        })?;

        resolved.cache.put_stat(&resolved.fs_path, stat.clone());
        Ok(stat)
    })();

    match result {
        Ok(stat) => {
            let out = unsafe { &mut *buf };
            out.st_mode = stat.st_mode;
            out.st_nlink = stat.st_nlink;
            out.st_uid = stat.st_uid;
            out.st_gid = stat.st_gid;
            out.st_size = stat.st_size;
            out.st_mtime = stat.st_mtime;
            out.st_ctime = stat.st_ctime;
            out.st_atime = stat.st_atime;
            out.st_ino = stat.st_ino;
            out.st_mtime_nsec = stat.st_mtime_nsec;
            out.st_ctime_nsec = stat.st_ctime_nsec;
            out.st_atime_nsec = stat.st_atime_nsec;
            out.st_dev = stat.st_dev;
            out.st_blocks = stat.st_blocks;
            out.st_blksize = stat.st_blksize;
            out.st_rdev = stat.st_rdev;
            0
        }
        Err(e) => {
            set_error(&e);
            -1
        }
    }
}

// ── C API: Directory operations ───────────────────────────────────

/// Open a directory for iteration.
///
/// Returns an opaque handle, or NULL on error.  The entire directory listing
/// is fetched eagerly (with pagination) so that subsequent `fs123_readdir`
/// calls are purely local.
#[no_mangle]
pub extern "C" fn fs123_opendir(path: *const c_char) -> *mut c_void {
    clear_error();

    let result = (|| -> Result<DirHandle, Fs123Error> {
        let path_str = unsafe { cstr_to_str(path)? };
        let resolved = resolve_path(path_str)?;

        if let Some(cached) = resolved.cache.get_dir(&resolved.fs_path) {
            return Ok(DirHandle {
                entries: cached,
                index: 0,
            });
        }

        let mut all_entries = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let params = match &cursor {
                Some(c) => vec!["64".to_string(), c.clone()],
                None => vec!["64".to_string(), String::new()],
            };
            let params_ref: Vec<&str> = params.iter().map(|s| s.as_str()).collect();

            let response = resolved.client.request(
                Fs123Function::Readdir,
                &resolved.fs_path,
                Some(&params_ref),
            )?;

            if let Some(content) = response.content() {
                all_entries.extend(DirEntryData::parse_entries(content));
            }

            if response.has_more_entries() {
                if let Some(next) = response.nextstart() {
                    cursor = Some(String::from_utf8_lossy(&next).to_string());
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        resolved
            .cache
            .put_dir(&resolved.fs_path, all_entries.clone());

        Ok(DirHandle {
            entries: all_entries,
            index: 0,
        })
    })();

    match result {
        Ok(handle) => Box::into_raw(Box::new(handle)) as *mut c_void,
        Err(e) => {
            set_error(&e);
            ptr::null_mut()
        }
    }
}

/// Read the next directory entry.
///
/// Returns 1 if an entry was stored in `*entry`, 0 when the listing is
/// exhausted, or -1 on error.
#[no_mangle]
pub extern "C" fn fs123_readdir(dir: *mut c_void, entry: *mut fs123_dirent_t) -> c_int {
    clear_error();

    if dir.is_null() || entry.is_null() {
        set_raw_error(libc::EINVAL, "NULL argument");
        return -1;
    }

    let handle = unsafe { &mut *(dir as *mut DirHandle) };

    if handle.index >= handle.entries.len() {
        return 0; // done
    }

    let src = &handle.entries[handle.index];
    handle.index += 1;

    let out = unsafe { &mut *entry };

    // Copy name, null-terminated, truncated to fit
    let name_bytes = src.name.as_bytes();
    let copy_len = name_bytes.len().min(255);
    unsafe {
        ptr::copy_nonoverlapping(
            name_bytes.as_ptr(),
            out.name.as_mut_ptr() as *mut u8,
            copy_len,
        );
        out.name[copy_len] = 0;
    }
    out.d_type = src.d_type;

    1
}

/// Close a directory handle and free its resources.
#[no_mangle]
pub extern "C" fn fs123_closedir(dir: *mut c_void) {
    if !dir.is_null() {
        unsafe {
            drop(Box::from_raw(dir as *mut DirHandle));
        }
    }
}

// ── C API: File operations ────────────────────────────────────────

/// Open a remote file for reading.
///
/// `mode` should start with `'r'` (e.g. `"r"`, `"rb"`).  fs123 is read-only
/// so write modes are rejected.  The text/binary distinction is a no-op at
/// this layer; callers handle encoding.
///
/// Returns an opaque handle, or NULL on error.
#[no_mangle]
pub extern "C" fn fs123_open(path: *const c_char, mode: *const c_char) -> *mut c_void {
    clear_error();

    let result = (|| -> Result<FileHandle, Fs123Error> {
        let path_str = unsafe { cstr_to_str(path)? };

        // Validate mode
        if !mode.is_null() {
            let mode_str = unsafe { cstr_to_str(mode)? };
            if !mode_str.starts_with('r') {
                return Err(Fs123Error::InvalidArgument(
                    "fs123 is read-only; mode must start with 'r'".to_string(),
                ));
            }
        }

        let resolved = resolve_path(path_str)?;

        // Mirror mode: download the file to local disk, then open locally.
        if resolved.mirror {
            let local_path = format!("{}{}", resolved.mount_point, resolved.fs_path);

            if !std::path::Path::new(&local_path).exists() {
                // Stat remote file to learn the file size for download.
                let file_size =
                    remote_file_size(&resolved.client, &resolved.cache, &resolved.fs_path)?;
                download_file(&resolved.client, &resolved.fs_path, &local_path, file_size)?;
            }

            let file = std::fs::File::open(&local_path)?;
            return Ok(FileHandle::Local { file });
        }

        // Non-mirror: remote file handle.
        // Stat to learn the file size (needed for EOF and SEEK_END).
        let file_size = remote_file_size(&resolved.client, &resolved.cache, &resolved.fs_path)?;

        Ok(FileHandle::Remote {
            client: resolved.client,
            path: resolved.fs_path,
            position: 0,
            file_size,
        })
    })();

    match result {
        Ok(handle) => Box::into_raw(Box::new(handle)) as *mut c_void,
        Err(e) => {
            set_error(&e);
            ptr::null_mut()
        }
    }
}

/// Read up to `count` bytes from an open file into `buf`.
///
/// Returns the number of bytes actually read (may be less than `count`),
/// 0 at end-of-file, or -1 on error.
#[no_mangle]
pub extern "C" fn fs123_read(file: *mut c_void, buf: *mut c_void, count: usize) -> isize {
    clear_error();

    if file.is_null() || buf.is_null() {
        set_raw_error(libc::EINVAL, "NULL argument");
        return -1;
    }
    if count == 0 {
        return 0;
    }

    let handle = unsafe { &mut *(file as *mut FileHandle) };

    match handle {
        FileHandle::Local { file } => {
            let buf_slice = unsafe { std::slice::from_raw_parts_mut(buf as *mut u8, count) };
            match file.read(buf_slice) {
                Ok(n) => n as isize,
                Err(e) => {
                    set_error(&Fs123Error::IoError(e));
                    -1
                }
            }
        }
        FileHandle::Remote {
            client,
            path,
            position,
            file_size,
        } => {
            // EOF check
            if *file_size >= 0 && *position >= *file_size as u64 {
                return 0;
            }

            // The fs123 /f endpoint uses KiB-aligned offset and length.
            let offset = *position as i64;
            let offset_kib = offset / 1024;
            let end_byte = offset + count as i64;
            let end_kib = (end_byte + 1023) / 1024;
            let len_kib = end_kib - offset_kib;

            let params = [len_kib.to_string(), offset_kib.to_string()];
            let params_ref: Vec<&str> = params.iter().map(|s| s.as_str()).collect();

            let response = match client.request(Fs123Function::Read, path, Some(&params_ref)) {
                Ok(r) => r,
                Err(e) => {
                    set_error(&e);
                    return -1;
                }
            };

            let content = response.content().unwrap_or(&[]);

            let skip = (offset % 1024) as usize;
            let available = content.len().saturating_sub(skip);
            let to_copy = available.min(count);

            if to_copy > 0 {
                unsafe {
                    ptr::copy_nonoverlapping(
                        content[skip..skip + to_copy].as_ptr(),
                        buf as *mut u8,
                        to_copy,
                    );
                }
                *position += to_copy as u64;
            }

            to_copy as isize
        }
    }
}

/// Reposition the read offset of an open file.
///
/// `whence`: 0 = SEEK_SET, 1 = SEEK_CUR, 2 = SEEK_END.
/// Returns the new absolute byte offset, or -1 on error.
#[no_mangle]
pub extern "C" fn fs123_seek(file: *mut c_void, offset: i64, whence: c_int) -> i64 {
    clear_error();

    if file.is_null() {
        set_raw_error(libc::EINVAL, "NULL file handle");
        return -1;
    }

    let handle = unsafe { &mut *(file as *mut FileHandle) };

    match handle {
        FileHandle::Local { file } => {
            let seek_from = match whence {
                0 => SeekFrom::Start(offset as u64),
                1 => SeekFrom::Current(offset),
                2 => SeekFrom::End(offset),
                _ => {
                    set_raw_error(libc::EINVAL, "Invalid whence");
                    return -1;
                }
            };
            match file.seek(seek_from) {
                Ok(pos) => pos as i64,
                Err(e) => {
                    set_error(&Fs123Error::IoError(e));
                    -1
                }
            }
        }
        FileHandle::Remote {
            position,
            file_size,
            ..
        } => {
            let new_pos: i64 = match whence {
                0 => offset,                    // SEEK_SET
                1 => *position as i64 + offset, // SEEK_CUR
                2 => {
                    if *file_size < 0 {
                        set_raw_error(libc::ESPIPE, "Unknown file size; cannot SEEK_END");
                        return -1;
                    }
                    *file_size + offset
                }
                _ => {
                    set_raw_error(libc::EINVAL, "Invalid whence");
                    return -1;
                }
            };

            if new_pos < 0 {
                set_raw_error(libc::EINVAL, "Seek to negative offset");
                return -1;
            }

            *position = new_pos as u64;
            new_pos
        }
    }
}

/// Close a file handle and free its resources.
///
/// Returns 0.
#[no_mangle]
pub extern "C" fn fs123_close(file: *mut c_void) -> c_int {
    if !file.is_null() {
        unsafe {
            drop(Box::from_raw(file as *mut FileHandle));
        }
    }
    0
}

// ── C API: Symlinks ───────────────────────────────────────────────

/// Read the target of a symbolic link.
///
/// Writes a null-terminated string into `buf` (at most `bufsiz - 1`
/// characters plus NUL).  Returns the number of bytes written (excluding
/// NUL), or -1 on error.
#[no_mangle]
pub extern "C" fn fs123_readlink(path: *const c_char, buf: *mut c_char, bufsiz: usize) -> isize {
    clear_error();

    if buf.is_null() || bufsiz == 0 {
        set_raw_error(libc::EINVAL, "NULL or zero-size buffer");
        return -1;
    }

    let result = (|| -> Result<String, Fs123Error> {
        let path_str = unsafe { cstr_to_str(path)? };
        let resolved = resolve_path(path_str)?;

        if let Some(cached) = resolved.cache.get_link(&resolved.fs_path) {
            return Ok(cached);
        }

        let response = resolved
            .client
            .request(Fs123Function::Readlink, &resolved.fs_path, None)?;
        let target = response
            .content_str()
            .ok_or_else(|| Fs123Error::InvalidResponse("Missing readlink target".to_string()))?;

        resolved.cache.put_link(&resolved.fs_path, target.clone());
        Ok(target)
    })();

    match result {
        Ok(target) => {
            let bytes = target.as_bytes();
            let copy_len = bytes.len().min(bufsiz - 1);
            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, copy_len);
                *buf.add(copy_len) = 0;
            }
            copy_len as isize
        }
        Err(e) => {
            set_error(&e);
            -1
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── URL parsing ───────────────────────────────────────────────

    #[test]
    fn test_parse_url_basic() {
        let u = parse_fs123_url("http://example.com/some/path").unwrap();
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, 80);
        assert_eq!(u.selector, "/some/path");
    }

    #[test]
    fn test_parse_url_with_port() {
        let u = parse_fs123_url("http://localhost:8080/dir").unwrap();
        assert_eq!(u.host, "localhost");
        assert_eq!(u.port, 8080);
        assert_eq!(u.selector, "/dir");
    }

    #[test]
    fn test_parse_url_https() {
        let u = parse_fs123_url("https://secure.example.com/data").unwrap();
        assert_eq!(u.host, "secure.example.com");
        assert_eq!(u.port, 443);
        assert_eq!(u.selector, "/data");
    }

    #[test]
    fn test_parse_url_no_path() {
        let u = parse_fs123_url("http://host.example.com").unwrap();
        assert_eq!(u.host, "host.example.com");
        assert_eq!(u.selector, "");
    }

    #[test]
    fn test_parse_url_root_path() {
        let u = parse_fs123_url("http://host.example.com/").unwrap();
        assert_eq!(u.selector, "");
    }

    #[test]
    fn test_parse_url_trailing_slash() {
        let u = parse_fs123_url("http://host.example.com/sel/").unwrap();
        assert_eq!(u.selector, "/sel");
    }

    #[test]
    fn test_parse_url_bad_scheme() {
        assert!(parse_fs123_url("ftp://example.com/foo").is_err());
    }

    #[test]
    fn test_parse_url_empty_host() {
        assert!(parse_fs123_url("http:///path").is_err());
    }

    // ── Mount table ───────────────────────────────────────────────
    //
    // Each test uses a unique mount-point prefix so tests can run in
    // parallel without interfering with the shared global mount table.

    #[test]
    fn test_mount_and_resolve() {
        let url = CString::new("http://server1:8080/exports").unwrap();
        let mp = CString::new("/fs123_test_mount_and_resolve/data").unwrap();
        assert_eq!(fs123_mount(url.as_ptr(), mp.as_ptr(), ptr::null()), 0);

        let ResolvedPath { fs_path, .. } =
            resolve_path("/fs123_test_mount_and_resolve/data/foo/bar.txt").unwrap();
        assert_eq!(fs_path, "/foo/bar.txt");

        let ResolvedPath { fs_path, .. } =
            resolve_path("/fs123_test_mount_and_resolve/data").unwrap();
        assert_eq!(fs_path, "/");
    }

    #[test]
    fn test_mount_longest_prefix() {
        let url1 = CString::new("http://server1:8080").unwrap();
        let mp1 = CString::new("/fs123_test_longest_prefix/mnt").unwrap();
        let url2 = CString::new("http://server2:8080").unwrap();
        let mp2 = CString::new("/fs123_test_longest_prefix/mnt/deep").unwrap();

        fs123_mount(url1.as_ptr(), mp1.as_ptr(), ptr::null());
        fs123_mount(url2.as_ptr(), mp2.as_ptr(), ptr::null());

        // /fs123_test_longest_prefix/mnt/deep/file → resolves to server2, path /file
        let ResolvedPath {
            client, fs_path, ..
        } = resolve_path("/fs123_test_longest_prefix/mnt/deep/file").unwrap();
        assert_eq!(fs_path, "/file");
        assert_eq!(client.port(), 8080);

        // /fs123_test_longest_prefix/mnt/other → resolves to server1, path /other
        let ResolvedPath {
            client, fs_path, ..
        } = resolve_path("/fs123_test_longest_prefix/mnt/other").unwrap();
        assert_eq!(fs_path, "/other");
        assert_eq!(client.port(), 8080);
    }

    #[test]
    fn test_mount_root() {
        let url = CString::new("http://server:80").unwrap();
        let mp = CString::new("/fs123_test_mount_root").unwrap();
        assert_eq!(fs123_mount(url.as_ptr(), mp.as_ptr(), ptr::null()), 0);

        let ResolvedPath { fs_path, .. } = resolve_path("/fs123_test_mount_root/any/path").unwrap();
        assert_eq!(fs_path, "/any/path");

        let ResolvedPath { fs_path, .. } = resolve_path("/fs123_test_mount_root").unwrap();
        assert_eq!(fs_path, "/");
    }

    #[test]
    fn test_mount_replace() {
        let mp = CString::new("/fs123_test_mount_replace").unwrap();
        let url1 = CString::new("http://old:80").unwrap();
        let url2 = CString::new("http://new:80").unwrap();

        fs123_mount(url1.as_ptr(), mp.as_ptr(), ptr::null());
        fs123_mount(url2.as_ptr(), mp.as_ptr(), ptr::null());

        let table = lock_mount_table();
        let entry = table
            .iter()
            .find(|e| e.mount_point == "/fs123_test_mount_replace")
            .unwrap();
        assert_eq!(entry.client.host(), "new");
    }

    #[test]
    fn test_umount() {
        let url = CString::new("http://server:80").unwrap();
        let mp = CString::new("/fs123_test_umount").unwrap();
        fs123_mount(url.as_ptr(), mp.as_ptr(), ptr::null());

        assert_eq!(fs123_umount(mp.as_ptr()), 0);
        assert!(resolve_path("/fs123_test_umount/foo").is_err());
    }

    #[test]
    fn test_umount_not_found() {
        let mp = CString::new("/fs123_test_umount_not_found").unwrap();
        assert_eq!(fs123_umount(mp.as_ptr()), -1);
        assert_eq!(fs123_errno(), libc::EINVAL);
    }

    #[test]
    fn test_process_fstab_basic() {
        let contents = "\
# comment line
http://server1:8080/exports   /fs123_test_fstab_basic/data   cache_ttl_secs=120

http://server2:9090            /fs123_test_fstab_basic/logs   -
";
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 2 {
                continue;
            }
            let url = fields[0];
            let mountpoint = fields[1];
            let options = if fields.len() >= 3 {
                let opt = fields[2];
                if opt == "-" || opt == "none" {
                    None
                } else {
                    Some(opt)
                }
            } else {
                None
            };
            mount_internal(url, mountpoint, options).unwrap();
        }

        let table = lock_mount_table();

        // server1 at /fs123_test_fstab_basic/data with custom TTL
        let e1 = table
            .iter()
            .find(|e| e.mount_point == "/fs123_test_fstab_basic/data")
            .unwrap();
        assert_eq!(e1.client.host(), "server1");
        assert_eq!(e1.client.port(), 8080);
        assert_eq!(e1.cache.ttl, Duration::from_secs(120));

        // server2 at /fs123_test_fstab_basic/logs with default TTL
        let e2 = table
            .iter()
            .find(|e| e.mount_point == "/fs123_test_fstab_basic/logs")
            .unwrap();
        assert_eq!(e2.client.host(), "server2");
        assert_eq!(e2.client.port(), 9090);
        assert_eq!(e2.cache.ttl, Duration::from_secs(DEFAULT_CACHE_TTL_SECS));
    }

    #[test]
    fn test_mount_with_options() {
        let url = CString::new("http://server:80").unwrap();
        let mp = CString::new("/fs123_test_mount_with_options").unwrap();
        let opts = CString::new("cache_ttl_secs=120,cache_max_entries=500").unwrap();
        assert_eq!(fs123_mount(url.as_ptr(), mp.as_ptr(), opts.as_ptr()), 0);

        let table = lock_mount_table();
        let entry = table
            .iter()
            .find(|e| e.mount_point == "/fs123_test_mount_with_options")
            .unwrap();
        assert_eq!(entry.cache.ttl, Duration::from_secs(120));
        assert_eq!(entry.cache.max_entries, 500);
    }

    // ── Cache ─────────────────────────────────────────────────────

    #[test]
    fn test_cache_stat_hit() {
        let cache = Cache::new(Duration::from_secs(60), 100);
        let stat = Fs123StatResult {
            st_mode: 0o100644,
            st_size: 42,
            ..Default::default()
        };

        assert!(cache.get_stat("/file").is_none());
        cache.put_stat("/file", stat.clone());
        let cached = cache.get_stat("/file").unwrap();
        assert_eq!(cached.st_size, 42);
    }

    #[test]
    fn test_cache_stat_expiry() {
        let cache = Cache::new(Duration::from_millis(1), 100);
        let stat = Fs123StatResult::default();

        cache.put_stat("/file", stat);
        std::thread::sleep(Duration::from_millis(5));
        assert!(cache.get_stat("/file").is_none());
    }

    #[test]
    fn test_cache_eviction_on_full() {
        let cache = Cache::new(Duration::from_secs(60), 2);
        let stat = Fs123StatResult::default();

        cache.put_stat("/a", stat.clone());
        cache.put_stat("/b", stat.clone());
        // Third insert triggers eviction of expired entries.
        // Since none are expired, all 3 will exist (evict_expired is a no-op).
        cache.put_stat("/c", stat);
        // At least the newest entry survives.
        assert!(cache.get_stat("/c").is_some());
    }

    #[test]
    fn test_cache_dir_and_link() {
        let cache = Cache::new(Duration::from_secs(60), 100);

        assert!(cache.get_dir("/d").is_none());
        cache.put_dir("/d", vec![]);
        assert!(cache.get_dir("/d").unwrap().is_empty());

        assert!(cache.get_link("/l").is_none());
        cache.put_link("/l", "/target".to_string());
        assert_eq!(cache.get_link("/l").unwrap(), "/target");
    }

    // ── Error handling ────────────────────────────────────────────

    #[test]
    fn test_thread_local_error() {
        clear_error();
        assert_eq!(fs123_errno(), 0);
        assert!(fs123_strerror().is_null());

        set_raw_error(libc::ENOENT, "not found");
        assert_eq!(fs123_errno(), libc::ENOENT);
        assert!(!fs123_strerror().is_null());

        clear_error();
        assert_eq!(fs123_errno(), 0);
    }

    // ── API null-safety ───────────────────────────────────────────

    #[test]
    fn test_stat_null_path() {
        let mut buf = unsafe { std::mem::zeroed::<fs123_stat_t>() };
        assert_eq!(fs123_stat(ptr::null(), &mut buf), -1);
        assert_eq!(fs123_errno(), libc::EINVAL);
    }

    #[test]
    fn test_stat_null_buf() {
        let url = CString::new("http://x:80").unwrap();
        let mp = CString::new("/fs123_test_stat_null_buf").unwrap();
        fs123_mount(url.as_ptr(), mp.as_ptr(), ptr::null());

        let p = CString::new("/fs123_test_stat_null_buf/file").unwrap();
        assert_eq!(fs123_stat(p.as_ptr(), ptr::null_mut()), -1);
        assert_eq!(fs123_errno(), libc::EINVAL);
    }

    #[test]
    fn test_stat_no_mount() {
        let p = CString::new("/fs123_test_stat_no_mount").unwrap();
        let mut buf = unsafe { std::mem::zeroed::<fs123_stat_t>() };
        assert_eq!(fs123_stat(p.as_ptr(), &mut buf), -1);
        assert_eq!(fs123_errno(), libc::EINVAL);
    }

    #[test]
    fn test_open_bad_mode() {
        let url = CString::new("http://x:80").unwrap();
        let mp = CString::new("/fs123_test_open_bad_mode").unwrap();
        fs123_mount(url.as_ptr(), mp.as_ptr(), ptr::null());

        let p = CString::new("/fs123_test_open_bad_mode/file").unwrap();
        let mode = CString::new("w").unwrap();
        let fh = fs123_open(p.as_ptr(), mode.as_ptr());
        assert!(fh.is_null());
        assert_eq!(fs123_errno(), libc::EINVAL);
    }

    #[test]
    fn test_readdir_null() {
        let mut entry = unsafe { std::mem::zeroed::<fs123_dirent_t>() };
        assert_eq!(fs123_readdir(ptr::null_mut(), &mut entry), -1);
    }

    #[test]
    fn test_close_null() {
        assert_eq!(fs123_close(ptr::null_mut()), 0);
        fs123_closedir(ptr::null_mut());
    }

    #[test]
    fn test_seek_null() {
        assert_eq!(fs123_seek(ptr::null_mut(), 0, 0), -1);
    }

    #[test]
    fn test_readlink_null_buf() {
        let url = CString::new("http://x:80").unwrap();
        let mp = CString::new("/fs123_test_readlink_null_buf").unwrap();
        fs123_mount(url.as_ptr(), mp.as_ptr(), ptr::null());

        let p = CString::new("/fs123_test_readlink_null_buf/link").unwrap();
        assert_eq!(fs123_readlink(p.as_ptr(), ptr::null_mut(), 256), -1);
    }

    #[test]
    fn test_mount_relative_path_rejected() {
        let url = CString::new("http://x:80").unwrap();
        let mp = CString::new("relative").unwrap();
        assert_eq!(fs123_mount(url.as_ptr(), mp.as_ptr(), ptr::null()), -1);
        assert_eq!(fs123_errno(), libc::EINVAL);
    }

    // ── Mirror option parsing ─────────────────────────────────────

    #[test]
    fn test_parse_mount_options_mirror_true() {
        let opts = parse_mount_options("mirror=true").unwrap();
        assert!(opts.mirror);
    }

    #[test]
    fn test_parse_mount_options_mirror_yes() {
        let opts = parse_mount_options("mirror=yes").unwrap();
        assert!(opts.mirror);
    }

    #[test]
    fn test_parse_mount_options_mirror_one() {
        let opts = parse_mount_options("mirror=1").unwrap();
        assert!(opts.mirror);
    }

    #[test]
    fn test_parse_mount_options_mirror_false() {
        let opts = parse_mount_options("mirror=false").unwrap();
        assert!(!opts.mirror);
    }

    #[test]
    fn test_parse_mount_options_mirror_default() {
        let opts = parse_mount_options("cache_ttl_secs=10").unwrap();
        assert!(!opts.mirror);
    }

    #[test]
    fn test_mount_stores_mirror_flag() {
        mount_internal(
            "http://srv:80",
            "/fs123_test_mirror_flag",
            Some("mirror=true"),
        )
        .unwrap();
        let table = lock_mount_table();
        let entry = table
            .iter()
            .find(|e| e.mount_point == "/fs123_test_mirror_flag")
            .unwrap();
        assert!(entry.mirror);
    }

    #[test]
    fn test_resolve_path_returns_mirror() {
        mount_internal(
            "http://srv:80",
            "/fs123_test_resolve_mirror",
            Some("mirror=true"),
        )
        .unwrap();
        let r = resolve_path("/fs123_test_resolve_mirror/file").unwrap();
        assert!(r.mirror);
        assert_eq!(r.mount_point, "/fs123_test_resolve_mirror");
        assert_eq!(r.fs_path, "/file");

        mount_internal("http://srv:80", "/fs123_test_resolve_nomirror", None).unwrap();
        let r = resolve_path("/fs123_test_resolve_nomirror/file").unwrap();
        assert!(!r.mirror);
    }

    // ── Mirror: local file handle ─────────────────────────────────

    #[test]
    fn test_local_file_handle_read_seek() {
        use std::io::Write;

        // Create a temp file with known content
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("testfile");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"hello world").unwrap();
        }

        // Open as a Local FileHandle
        let file = std::fs::File::open(&path).unwrap();
        let mut handle = FileHandle::Local { file };

        // Read via the handle
        if let FileHandle::Local { ref mut file } = handle {
            let mut buf = [0u8; 5];
            let n = file.read(&mut buf).unwrap();
            assert_eq!(n, 5);
            assert_eq!(&buf, b"hello");

            // Seek back to start
            file.seek(SeekFrom::Start(0)).unwrap();
            let mut buf2 = [0u8; 11];
            let n = file.read(&mut buf2).unwrap();
            assert_eq!(n, 11);
            assert_eq!(&buf2, b"hello world");

            // Seek to end
            let pos = file.seek(SeekFrom::End(0)).unwrap();
            assert_eq!(pos, 11);
        }
    }

    #[test]
    fn test_stat_local_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stattest");
        std::fs::write(&path, "test data").unwrap();

        let stat = stat_local_file(path.to_str().unwrap()).unwrap();
        assert_eq!(stat.st_size, 9); // "test data" = 9 bytes
        assert!(stat.st_mode & 0o100000 != 0); // regular file
    }

    #[test]
    fn test_download_file_creates_parents() {
        // We can't test a real download without a server, but we can
        // verify that download_file creates parent directories and
        // handles a zero-byte file correctly.
        let dir = tempfile::tempdir().unwrap();
        let local_path = dir.path().join("a/b/c/empty");

        // Create a dummy client (won't be used for a zero-byte file)
        let client = Fs123HttpClient::new("localhost", 1, "7.3");
        download_file(&client, "/dummy", local_path.to_str().unwrap(), 0).unwrap();

        assert!(local_path.exists());
        assert_eq!(std::fs::read(&local_path).unwrap().len(), 0);
    }
}
