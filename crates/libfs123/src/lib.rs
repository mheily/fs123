//! libfs123 — C-compatible library for fs123 filesystem access.
//!
//! Provides POSIX-like functions that translate filesystem operations into
//! fs123 HTTP protocol requests.  Designed to be called via FFI from Python,
//! C, or any language with a C FFI.
//!
//! # URL format
//!
//! All functions accept a URL of the form:
//!
//! ```text
//! http://host[:port]/path
//! ```
//!
//! The path component is the filesystem path on the fs123 server.
//! The protocol version defaults to 7.3 and can be changed with
//! [`fs123_set_proto`].

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;

use fs123_core::{
    types::{DirEntryData, Fs123StatResult},
    Fs123Error, Fs123Function, Fs123HttpClient,
};

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

// ── URL parsing ───────────────────────────────────────────────────

struct ParsedUrl {
    host: String,
    port: u16,
    path: String,
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

    let (host_port, path) = match rest.find('/') {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, "/"),
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

    Ok(ParsedUrl {
        host,
        port,
        path: if path.is_empty() {
            "/".to_string()
        } else {
            path.to_string()
        },
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

struct FileHandle {
    client: Fs123HttpClient,
    path: String,
    position: u64,
    file_size: i64,
}

struct DirHandle {
    entries: Vec<DirEntryData>,
    index: usize,
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
/// If never called, defaults to "7.3".
#[no_mangle]
pub extern "C" fn fs123_set_proto(proto: *const c_char) {
    if proto.is_null() {
        return;
    }
    if let Ok(s) = unsafe { CStr::from_ptr(proto) }.to_str() {
        DEFAULT_PROTO.with(|p| *p.borrow_mut() = s.to_string());
    }
}

// ── C API: stat ───────────────────────────────────────────────────

/// Get file/directory attributes for the given fs123 URL.
///
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn fs123_stat(url: *const c_char, buf: *mut fs123_stat_t) -> c_int {
    clear_error();

    let result = (|| -> Result<Fs123StatResult, Fs123Error> {
        let url_str = unsafe { cstr_to_str(url)? };
        if buf.is_null() {
            return Err(Fs123Error::InvalidArgument("NULL buffer".to_string()));
        }

        let parsed = parse_fs123_url(url_str)?;
        let proto = get_default_proto();
        let client = Fs123HttpClient::new(&parsed.host, parsed.port, &proto);
        let response = client.request_raw(Fs123Function::Stat, &parsed.path, None)?;

        if let Some(errno) = response.errno() {
            if errno != 0 {
                return Err(Fs123Error::FilesystemError {
                    errno,
                    message: format!("stat: {}", parsed.path),
                });
            }
        }

        let content = response.get_str("content").ok_or_else(|| {
            Fs123Error::InvalidResponse("Missing content in stat response".to_string())
        })?;

        Fs123StatResult::from_str(&content).ok_or_else(|| {
            Fs123Error::InvalidResponse(format!("Cannot parse stat: {}", content))
        })
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
pub extern "C" fn fs123_opendir(url: *const c_char) -> *mut c_void {
    clear_error();

    let result = (|| -> Result<DirHandle, Fs123Error> {
        let url_str = unsafe { cstr_to_str(url)? };
        let parsed = parse_fs123_url(url_str)?;
        let proto = get_default_proto();
        let client = Fs123HttpClient::new(&parsed.host, parsed.port, &proto);

        let mut all_entries = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let params = match &cursor {
                Some(c) => vec!["64".to_string(), c.clone()],
                None => vec!["64".to_string(), String::new()],
            };
            let params_ref: Vec<&str> = params.iter().map(|s| s.as_str()).collect();

            let response =
                client.request(Fs123Function::Readdir, &parsed.path, Some(&params_ref))?;

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
        ptr::copy_nonoverlapping(name_bytes.as_ptr(), out.name.as_mut_ptr() as *mut u8, copy_len);
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
pub extern "C" fn fs123_open(url: *const c_char, mode: *const c_char) -> *mut c_void {
    clear_error();

    let result = (|| -> Result<FileHandle, Fs123Error> {
        let url_str = unsafe { cstr_to_str(url)? };

        // Validate mode
        if !mode.is_null() {
            let mode_str = unsafe { cstr_to_str(mode)? };
            if !mode_str.starts_with('r') {
                return Err(Fs123Error::InvalidArgument(
                    "fs123 is read-only; mode must start with 'r'".to_string(),
                ));
            }
        }

        let parsed = parse_fs123_url(url_str)?;
        let proto = get_default_proto();
        let client = Fs123HttpClient::new(&parsed.host, parsed.port, &proto);

        // Stat to learn the file size (needed for EOF and SEEK_END).
        let response = client.request_raw(Fs123Function::Stat, &parsed.path, None)?;
        if let Some(errno) = response.errno() {
            if errno != 0 {
                return Err(Fs123Error::FilesystemError {
                    errno,
                    message: format!("open: {}", parsed.path),
                });
            }
        }

        let file_size = response
            .get_str("content")
            .and_then(|c| Fs123StatResult::from_str(&c))
            .map(|s| s.st_size)
            .unwrap_or(-1);

        Ok(FileHandle {
            client,
            path: parsed.path,
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

    // EOF check
    if handle.file_size >= 0 && handle.position >= handle.file_size as u64 {
        return 0;
    }

    // The fs123 /f endpoint uses KiB-aligned offset and length.
    let offset = handle.position as i64;
    let offset_kib = offset / 1024;
    let end_byte = offset + count as i64;
    let end_kib = (end_byte + 1023) / 1024;
    let len_kib = end_kib - offset_kib;

    // Protocol: /f/path?Len;Offset  (length first, offset second, both in KiB)
    let params = [len_kib.to_string(), offset_kib.to_string()];
    let params_ref: Vec<&str> = params.iter().map(|s| s.as_str()).collect();

    let response = match handle
        .client
        .request(Fs123Function::Read, &handle.path, Some(&params_ref))
    {
        Ok(r) => r,
        Err(e) => {
            set_error(&e);
            return -1;
        }
    };

    let content = response.content().unwrap_or(&[]);

    // Trim to the exact byte range requested
    let skip = (offset % 1024) as usize;
    let available = content.len().saturating_sub(skip);
    let to_copy = available.min(count);

    if to_copy > 0 {
        unsafe {
            ptr::copy_nonoverlapping(content[skip..skip + to_copy].as_ptr(), buf as *mut u8, to_copy);
        }
        handle.position += to_copy as u64;
    }

    to_copy as isize
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

    let new_pos: i64 = match whence {
        0 => offset, // SEEK_SET
        1 => handle.position as i64 + offset, // SEEK_CUR
        2 => {
            // SEEK_END
            if handle.file_size < 0 {
                set_raw_error(libc::ESPIPE, "Unknown file size; cannot SEEK_END");
                return -1;
            }
            handle.file_size + offset
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

    handle.position = new_pos as u64;
    new_pos
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
pub extern "C" fn fs123_readlink(url: *const c_char, buf: *mut c_char, bufsiz: usize) -> isize {
    clear_error();

    if buf.is_null() || bufsiz == 0 {
        set_raw_error(libc::EINVAL, "NULL or zero-size buffer");
        return -1;
    }

    let result = (|| -> Result<String, Fs123Error> {
        let url_str = unsafe { cstr_to_str(url)? };
        let parsed = parse_fs123_url(url_str)?;
        let proto = get_default_proto();
        let client = Fs123HttpClient::new(&parsed.host, parsed.port, &proto);
        let response = client.request(Fs123Function::Readlink, &parsed.path, None)?;
        response
            .content_str()
            .ok_or_else(|| Fs123Error::InvalidResponse("Missing readlink target".to_string()))
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

    #[test]
    fn test_parse_url_basic() {
        let u = parse_fs123_url("http://example.com/some/path").unwrap();
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, 80);
        assert_eq!(u.path, "/some/path");
    }

    #[test]
    fn test_parse_url_with_port() {
        let u = parse_fs123_url("http://localhost:8080/dir").unwrap();
        assert_eq!(u.host, "localhost");
        assert_eq!(u.port, 8080);
        assert_eq!(u.path, "/dir");
    }

    #[test]
    fn test_parse_url_https() {
        let u = parse_fs123_url("https://secure.example.com/data").unwrap();
        assert_eq!(u.host, "secure.example.com");
        assert_eq!(u.port, 443);
        assert_eq!(u.path, "/data");
    }

    #[test]
    fn test_parse_url_no_path() {
        let u = parse_fs123_url("http://host.example.com").unwrap();
        assert_eq!(u.host, "host.example.com");
        assert_eq!(u.path, "/");
    }

    #[test]
    fn test_parse_url_bad_scheme() {
        assert!(parse_fs123_url("ftp://example.com/foo").is_err());
    }

    #[test]
    fn test_parse_url_empty_host() {
        assert!(parse_fs123_url("http:///path").is_err());
    }

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

    #[test]
    fn test_stat_null_url() {
        let mut buf = unsafe { std::mem::zeroed::<fs123_stat_t>() };
        assert_eq!(fs123_stat(ptr::null(), &mut buf), -1);
        assert_eq!(fs123_errno(), libc::EINVAL);
    }

    #[test]
    fn test_stat_null_buf() {
        let url = CString::new("http://localhost/test").unwrap();
        assert_eq!(fs123_stat(url.as_ptr(), ptr::null_mut()), -1);
        assert_eq!(fs123_errno(), libc::EINVAL);
    }

    #[test]
    fn test_open_bad_mode() {
        let url = CString::new("http://localhost/test").unwrap();
        let mode = CString::new("w").unwrap();
        let fh = fs123_open(url.as_ptr(), mode.as_ptr());
        assert!(fh.is_null());
        assert_eq!(fs123_errno(), libc::EINVAL);
    }

    #[test]
    fn test_opendir_bad_url() {
        let url = CString::new("ftp://nope").unwrap();
        let dh = fs123_opendir(url.as_ptr());
        assert!(dh.is_null());
        assert_eq!(fs123_errno(), libc::EINVAL);
    }

    #[test]
    fn test_readdir_null() {
        let mut entry = unsafe { std::mem::zeroed::<fs123_dirent_t>() };
        assert_eq!(fs123_readdir(ptr::null_mut(), &mut entry), -1);
    }

    #[test]
    fn test_close_null() {
        // Should not crash
        assert_eq!(fs123_close(ptr::null_mut()), 0);
        fs123_closedir(ptr::null_mut());
    }

    #[test]
    fn test_seek_null() {
        assert_eq!(fs123_seek(ptr::null_mut(), 0, 0), -1);
    }

    #[test]
    fn test_readlink_null_buf() {
        let url = CString::new("http://localhost/link").unwrap();
        assert_eq!(fs123_readlink(url.as_ptr(), ptr::null_mut(), 256), -1);
    }
}
