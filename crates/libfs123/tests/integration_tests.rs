//! Integration tests for the libfs123 C API.
//!
//! These tests require a running fs123-server. The test runner script
//! (`scripts/run_integration_tests.py`) handles server lifecycle.
//!
//! Environment variables:
//!   FS123_TEST_SERVER_URL  - e.g. http://127.0.0.1:18423
//!   FS123_TEST_MOUNTPOINT  - e.g. /fs123_test
//!
//! Run via: cargo test -p libfs123 --test integration_tests -- --ignored --test-threads=1

use std::env;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::Once;

use fs123::{
    fs123_close, fs123_closedir, fs123_errno, fs123_fsync, fs123_mount, fs123_open,
    fs123_opendir, fs123_read, fs123_readdir, fs123_readlink, fs123_seek, fs123_set_proto,
    fs123_stat, fs123_strerror, fs123_umount,
};
use fs123::{fs123_dirent_t, fs123_stat_t};

const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;

static MOUNT_ONCE: Once = Once::new();

fn mountpoint() -> String {
    env::var("FS123_TEST_MOUNTPOINT").expect("FS123_TEST_MOUNTPOINT not set")
}

fn setup() {
    MOUNT_ONCE.call_once(|| {
        let url = env::var("FS123_TEST_SERVER_URL").expect("FS123_TEST_SERVER_URL not set");
        let mp = mountpoint();
        let proto = CString::new("7.3").unwrap();
        fs123_set_proto(proto.as_ptr());
        let url_c = CString::new(url).unwrap();
        let mp_c = CString::new(mp).unwrap();
        let rc = fs123_mount(url_c.as_ptr(), mp_c.as_ptr(), std::ptr::null());
        assert_eq!(rc, 0, "fs123_mount failed");
    });
}

fn make_path(rel: &str) -> CString {
    let mp = mountpoint();
    if rel.is_empty() {
        CString::new(mp).unwrap()
    } else {
        CString::new(format!("{}/{}", mp, rel)).unwrap()
    }
}

fn do_stat(rel: &str) -> fs123_stat_t {
    setup();
    let path = make_path(rel);
    let mut buf = std::mem::MaybeUninit::<fs123_stat_t>::zeroed();
    let rc = fs123_stat(path.as_ptr(), buf.as_mut_ptr());
    assert_eq!(rc, 0, "fs123_stat failed for {}", rel);
    unsafe { buf.assume_init() }
}

fn read_dir_names(rel: &str) -> Vec<String> {
    setup();
    let path = make_path(rel);
    let dp = fs123_opendir(path.as_ptr());
    assert!(!dp.is_null(), "fs123_opendir failed for {}", rel);
    let mut names = Vec::new();
    loop {
        let mut ent = std::mem::MaybeUninit::<fs123_dirent_t>::zeroed();
        let rc = fs123_readdir(dp, ent.as_mut_ptr());
        if rc == 0 {
            break;
        }
        assert_eq!(rc, 1, "fs123_readdir returned error");
        let ent = unsafe { ent.assume_init() };
        let name = unsafe { CStr::from_ptr(ent.name.as_ptr() as *const c_char) };
        let name_str = name.to_str().unwrap().to_string();
        if name_str != "." && name_str != ".." {
            names.push(name_str);
        }
    }
    fs123_closedir(dp);
    names
}

fn open_and_read_all(rel: &str) -> Vec<u8> {
    setup();
    let path = make_path(rel);
    let mode = CString::new("rb").unwrap();
    let fh = fs123_open(path.as_ptr(), mode.as_ptr());
    assert!(!fh.is_null(), "fs123_open failed for {}", rel);
    let mut result = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs123_read(fh, buf.as_mut_ptr() as *mut c_void, buf.len());
        assert!(n >= 0, "fs123_read returned error");
        if n == 0 {
            break;
        }
        result.extend_from_slice(&buf[..n as usize]);
    }
    fs123_close(fh);
    result
}

// ── Mount / Unmount ─────────────────────────────────────────────────

#[test]
#[ignore]
fn test_mount_umount() {
    let url = env::var("FS123_TEST_SERVER_URL").expect("FS123_TEST_SERVER_URL not set");
    let mp = "/fs123_test_mount_umount";
    let proto = CString::new("7.3").unwrap();
    fs123_set_proto(proto.as_ptr());
    let url_c = CString::new(url).unwrap();
    let mp_c = CString::new(mp).unwrap();

    let rc = fs123_mount(url_c.as_ptr(), mp_c.as_ptr(), std::ptr::null());
    assert_eq!(rc, 0, "mount should succeed");

    let rc = fs123_umount(mp_c.as_ptr());
    assert_eq!(rc, 0, "umount should succeed");

    // Double umount should fail
    let rc = fs123_umount(mp_c.as_ptr());
    assert_eq!(rc, -1, "double umount should fail");
}

#[test]
#[ignore]
fn test_set_proto() {
    let proto = CString::new("7.3").unwrap();
    fs123_set_proto(proto.as_ptr());
    setup();
    let st = do_stat("hello.txt");
    assert!(st.st_size > 0);
}

// ── Error handling ──────────────────────────────────────────────────

#[test]
#[ignore]
fn test_errno_strerror_on_success() {
    setup();
    let _ = do_stat("hello.txt");
    assert_eq!(fs123_errno(), 0);
    assert!(fs123_strerror().is_null());
}

#[test]
#[ignore]
fn test_errno_strerror_on_failure() {
    setup();
    let path = make_path("nonexistent_file_xyz");
    let mut buf = std::mem::MaybeUninit::<fs123_stat_t>::zeroed();
    let rc = fs123_stat(path.as_ptr(), buf.as_mut_ptr());
    assert_eq!(rc, -1);
    let errno = fs123_errno();
    assert_eq!(errno, libc::ENOENT, "expected ENOENT, got {}", errno);
    let msg = fs123_strerror();
    assert!(!msg.is_null(), "strerror should be non-null after error");
}

// ── Stat ────────────────────────────────────────────────────────────

#[test]
#[ignore]
fn test_stat_regular_file() {
    let st = do_stat("hello.txt");
    assert_eq!(st.st_size, 13, "hello.txt should be 13 bytes");
    assert_eq!(st.st_mode & S_IFMT, S_IFREG, "should be regular file");
    assert!(st.st_nlink >= 1);
}

#[test]
#[ignore]
fn test_stat_directory() {
    let st = do_stat("subdir");
    assert_eq!(st.st_mode & S_IFMT, S_IFDIR, "should be directory");
}

#[test]
#[ignore]
fn test_stat_symlink() {
    let st = do_stat("link_to_hello");
    assert_eq!(st.st_size, 13, "symlink target hello.txt should be 13 bytes");
}

#[test]
#[ignore]
fn test_stat_nonexistent() {
    setup();
    let path = make_path("nonexistent_file_xyz");
    let mut buf = std::mem::MaybeUninit::<fs123_stat_t>::zeroed();
    let rc = fs123_stat(path.as_ptr(), buf.as_mut_ptr());
    assert_eq!(rc, -1, "stat on nonexistent should fail");
}

// ── Directory operations ────────────────────────────────────────────

#[test]
#[ignore]
fn test_opendir_readdir_closedir() {
    let names = read_dir_names("");
    assert!(names.contains(&"hello.txt".to_string()));
    assert!(names.contains(&"subdir".to_string()));
    assert!(names.contains(&"binary.dat".to_string()));
    assert!(names.contains(&"empty.txt".to_string()));
}

#[test]
#[ignore]
fn test_readdir_subdir() {
    let names = read_dir_names("subdir");
    assert!(names.contains(&"nested.txt".to_string()));
    assert!(names.contains(&"another.txt".to_string()));
    assert_eq!(names.len(), 2);
}

#[test]
#[ignore]
fn test_opendir_nonexistent() {
    setup();
    let path = make_path("nonexistent_dir_xyz");
    let dp = fs123_opendir(path.as_ptr());
    assert!(dp.is_null(), "opendir on nonexistent should return NULL");
    assert_ne!(fs123_errno(), 0);
}

// ── File read ───────────────────────────────────────────────────────

#[test]
#[ignore]
fn test_open_read_close() {
    let data = open_and_read_all("hello.txt");
    assert_eq!(data, b"Hello, fs123!");
}

#[test]
#[ignore]
fn test_read_binary() {
    let data = open_and_read_all("binary.dat");
    let expected: Vec<u8> = (0u8..=255).collect();
    assert_eq!(data, expected);
}

#[test]
#[ignore]
fn test_read_large_file() {
    let data = open_and_read_all("large.dat");
    assert_eq!(data.len(), 10240);
    let expected: Vec<u8> = (0..10240).map(|i| (i % 256) as u8).collect();
    assert_eq!(data, expected);
}

#[test]
#[ignore]
fn test_read_empty_file() {
    let data = open_and_read_all("empty.txt");
    assert!(data.is_empty());
}

// ── Seek ────────────────────────────────────────────────────────────

#[test]
#[ignore]
fn test_read_with_seek() {
    setup();
    let path = make_path("large.dat");
    let mode = CString::new("rb").unwrap();
    let fh = fs123_open(path.as_ptr(), mode.as_ptr());
    assert!(!fh.is_null());

    let pos = fs123_seek(fh, 1024, 0); // SEEK_SET
    assert_eq!(pos, 1024);

    let mut buf = [0u8; 16];
    let n = fs123_read(fh, buf.as_mut_ptr() as *mut c_void, buf.len());
    assert_eq!(n, 16);
    let expected: Vec<u8> = (1024..1040).map(|i| (i % 256) as u8).collect();
    assert_eq!(&buf[..], &expected[..]);

    fs123_close(fh);
}

#[test]
#[ignore]
fn test_seek_end() {
    setup();
    let path = make_path("hello.txt");
    let mode = CString::new("rb").unwrap();
    let fh = fs123_open(path.as_ptr(), mode.as_ptr());
    assert!(!fh.is_null());

    let pos = fs123_seek(fh, 0, 2); // SEEK_END
    assert_eq!(pos, 13);

    fs123_close(fh);
}

#[test]
#[ignore]
fn test_seek_cur() {
    setup();
    let path = make_path("hello.txt");
    let mode = CString::new("rb").unwrap();
    let fh = fs123_open(path.as_ptr(), mode.as_ptr());
    assert!(!fh.is_null());

    let mut buf = [0u8; 5];
    let n = fs123_read(fh, buf.as_mut_ptr() as *mut c_void, 5);
    assert_eq!(n, 5);

    let pos = fs123_seek(fh, 0, 1); // SEEK_CUR
    assert_eq!(pos, 5);

    fs123_close(fh);
}

#[test]
#[ignore]
fn test_open_nonexistent() {
    setup();
    let path = make_path("nonexistent_file_xyz");
    let mode = CString::new("rb").unwrap();
    let fh = fs123_open(path.as_ptr(), mode.as_ptr());
    assert!(fh.is_null(), "open on nonexistent should return NULL");
    assert_eq!(fs123_errno(), libc::ENOENT);
}

// ── Symlinks ────────────────────────────────────────────────────────

#[test]
#[ignore]
fn test_readlink() {
    setup();
    let path = make_path("link_to_hello");
    let mut buf = [0i8; 4096];
    let n = fs123_readlink(path.as_ptr(), buf.as_mut_ptr(), buf.len());
    assert!(n > 0, "readlink should return positive length");
    let target = unsafe { CStr::from_ptr(buf.as_ptr()) };
    assert_eq!(target.to_str().unwrap(), "hello.txt");
}

#[test]
#[ignore]
fn test_readlink_nonexistent() {
    setup();
    let path = make_path("nonexistent_link_xyz");
    let mut buf = [0i8; 4096];
    let n = fs123_readlink(path.as_ptr(), buf.as_mut_ptr(), buf.len());
    assert_eq!(n, -1, "readlink on nonexistent should fail");
}

// ── Special filenames ───────────────────────────────────────────────

#[test]
#[ignore]
fn test_file_with_spaces() {
    let st = do_stat("file with spaces.txt");
    assert_eq!(st.st_mode & S_IFMT, S_IFREG);
    let data = open_and_read_all("file with spaces.txt");
    assert_eq!(data, b"Has spaces");
}

#[test]
#[ignore]
fn test_unicode_filename() {
    let st = do_stat("日本語.txt");
    assert_eq!(st.st_mode & S_IFMT, S_IFREG);
    let data = open_and_read_all("日本語.txt");
    assert_eq!(data, b"Japanese filename");
}

#[test]
#[ignore]
fn test_deep_nesting() {
    let st = do_stat("a/b/c/d/deep.txt");
    assert_eq!(st.st_mode & S_IFMT, S_IFREG);
    let data = open_and_read_all("a/b/c/d/deep.txt");
    assert_eq!(data, b"Deep file");
}

// ── fsync ───────────────────────────────────────────────────────────

#[test]
#[ignore]
fn test_fsync() {
    setup();
    let path = make_path("hello.txt");
    let rc = fs123_fsync(path.as_ptr());
    assert_eq!(rc, 0, "fsync should succeed");
}
