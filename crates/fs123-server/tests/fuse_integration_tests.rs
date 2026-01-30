//! Integration tests for fs123 protocol through FUSE mount.
//!
//! These tests:
//! 1. Create a temporary directory with test fixtures
//! 2. Start fs123-server serving the directory
//! 3. Start fs123-mount (FUSE client) mounting the server
//! 4. Test file operations through the FUSE mount
//! 5. Clean up (kill processes, remove temp directories)
//!
//! ## Requirements
//!
//! ### For HTTP tests (test_http_*)
//! - Binaries must be built: `cargo build --workspace`
//! - No sandbox restrictions (actix-web uses nice() for thread priorities)
//!
//! ### For FUSE tests (all others)
//! - All HTTP test requirements, plus:
//! - FUSE support (macFUSE on macOS, libfuse on Linux)
//! - Appropriate FUSE permissions (may require root or fuse group membership)
//!
//! ## Running Tests
//!
//! ```bash
//! # Build binaries first
//! cargo build --workspace
//!
//! # Run HTTP-level tests (no FUSE required)
//! cargo test -p fs123-server --test fuse_integration_tests -- test_http --ignored
//!
//! # Run FUSE tests (requires FUSE support)
//! cargo test -p fs123-server --test fuse_integration_tests -- --ignored --test-threads=1
//!
//! # Run all tests
//! cargo test -p fs123-server --test fuse_integration_tests -- --include-ignored --test-threads=1
//! ```
//!
//! ## Docker
//!
//! These tests are designed to run in the project's Docker environment which has
//! FUSE support and no sandbox restrictions:
//!
//! ```bash
//! docker-compose exec client cargo test -p fs123-server --test fuse_integration_tests -- --include-ignored
//! ```

use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Atomic counter for unique port assignment
static PORT_COUNTER: AtomicU16 = AtomicU16::new(10000);

/// Find the project root by looking for Cargo.toml with workspace
fn find_project_root() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while path.parent().is_some() {
        let cargo_toml = path.join("Cargo.toml");
        if cargo_toml.exists() {
            if let Ok(content) = fs::read_to_string(&cargo_toml) {
                if content.contains("[workspace]") {
                    return path;
                }
            }
        }
        path = path.parent().unwrap().to_path_buf();
    }
    panic!("Could not find workspace root");
}

/// Get path to the server binary
fn server_binary() -> PathBuf {
    let root = find_project_root();
    let debug_path = root.join("target/debug/fs123-server");
    let release_path = root.join("target/release/fs123-server");

    if release_path.exists() {
        release_path
    } else if debug_path.exists() {
        debug_path
    } else {
        panic!(
            "Server binary not found. Run `cargo build --workspace` first.\n\
             Looked for:\n  - {}\n  - {}",
            debug_path.display(),
            release_path.display()
        );
    }
}

/// Get path to the client binary
fn client_binary() -> PathBuf {
    let root = find_project_root();
    let debug_path = root.join("target/debug/fs123-mount");
    let release_path = root.join("target/release/fs123-mount");

    if release_path.exists() {
        release_path
    } else if debug_path.exists() {
        debug_path
    } else {
        panic!(
            "Client binary not found. Run `cargo build --workspace` first.\n\
             Looked for:\n  - {}\n  - {}",
            debug_path.display(),
            release_path.display()
        );
    }
}

/// Get a unique port for testing
fn get_unique_port() -> u16 {
    // Use atomic counter to ensure unique ports across tests
    // Also add process ID to reduce conflicts when running tests in parallel
    let base = PORT_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid_offset = (std::process::id() % 1000) as u16;
    10000 + base + pid_offset
}

/// Wait for server to be ready by checking if it accepts connections
fn wait_for_server(host: &str, port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    let addr = format!("{}:{}", host, port);

    while start.elapsed() < timeout {
        if std::net::TcpStream::connect(&addr).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Wait for mount to be ready by checking if a test file is accessible
fn wait_for_mount(mountpoint: &Path, test_file: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    let test_path = mountpoint.join(test_file);

    while start.elapsed() < timeout {
        // Try to access the test file - this confirms the mount is working
        // and the server is responding
        if test_path.exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Unmount a FUSE filesystem
fn unmount(mountpoint: &Path) {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("umount")
            .arg(mountpoint)
            .status();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("fusermount")
            .arg("-u")
            .arg(mountpoint)
            .status();
    }
}

/// Test harness that manages server and client processes
struct TestHarness {
    export_dir: TempDir,
    mount_dir: TempDir,
    server_process: Option<Child>,
    client_process: Option<Child>,
    port: u16,
}

impl TestHarness {
    /// Create a new test harness
    fn new() -> Self {
        let export_dir = TempDir::new().expect("Failed to create export temp dir");
        let mount_dir = TempDir::new().expect("Failed to create mount temp dir");
        let port = get_unique_port();

        Self {
            export_dir,
            mount_dir,
            server_process: None,
            client_process: None,
            port,
        }
    }

    /// Get the export directory path
    fn export_path(&self) -> &Path {
        self.export_dir.path()
    }

    /// Get the mount directory path
    fn mount_path(&self) -> &Path {
        self.mount_dir.path()
    }

    /// Start the server
    fn start_server(&mut self) -> Result<(), String> {
        let server_bin = server_binary();
        let bind_addr = format!("127.0.0.1:{}", self.port);

        let child = Command::new(&server_bin)
            .arg("--bind")
            .arg(&bind_addr)
            .arg("--export-root")
            .arg(self.export_path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start server: {}", e))?;

        self.server_process = Some(child);

        // Wait for server to be ready
        if !wait_for_server("127.0.0.1", self.port, Duration::from_secs(10)) {
            // Try to get error output
            if let Some(ref mut proc) = self.server_process {
                if let Some(stderr) = proc.stderr.take() {
                    let reader = BufReader::new(stderr);
                    let lines: Vec<_> = reader.lines().take(10).filter_map(|l| l.ok()).collect();
                    return Err(format!(
                        "Server failed to start within timeout. Stderr:\n{}",
                        lines.join("\n")
                    ));
                }
            }
            return Err("Server failed to start within timeout".to_string());
        }

        Ok(())
    }

    /// Start the FUSE client
    fn start_client(&mut self) -> Result<(), String> {
        // Create a sentinel file to verify the mount is working
        let sentinel = ".fs123_test_ready";
        fs::write(self.export_path().join(sentinel), b"ready").map_err(|e| {
            format!("Failed to create sentinel file: {}", e)
        })?;

        let client_bin = client_binary();

        let child = Command::new(&client_bin)
            .arg("-H")
            .arg("127.0.0.1")
            .arg("-p")
            .arg(self.port.to_string())
            .arg("--auto-unmount")
            .arg("--cache=false")  // Disable caching for predictable test results
            .arg(self.mount_path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start client: {}", e))?;

        self.client_process = Some(child);

        // Wait for mount to be ready by checking for sentinel file
        if !wait_for_mount(self.mount_path(), sentinel, Duration::from_secs(10)) {
            // Try to get error output
            if let Some(ref mut proc) = self.client_process {
                if let Some(stderr) = proc.stderr.take() {
                    let reader = BufReader::new(stderr);
                    let lines: Vec<_> = reader.lines().take(10).filter_map(|l| l.ok()).collect();
                    return Err(format!(
                        "Mount failed within timeout. Stderr:\n{}",
                        lines.join("\n")
                    ));
                }
            }
            return Err("Mount failed within timeout".to_string());
        }

        Ok(())
    }

    /// Start both server and client
    fn start(&mut self) -> Result<(), String> {
        self.start_server()?;
        self.start_client()?;
        Ok(())
    }

    /// Stop processes and cleanup
    fn stop(&mut self) {
        // Unmount first
        unmount(self.mount_path());

        // Kill client
        if let Some(ref mut child) = self.client_process {
            let _ = signal::kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM);
            let _ = child.wait();
        }
        self.client_process = None;

        // Kill server
        if let Some(ref mut child) = self.server_process {
            let _ = signal::kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM);
            let _ = child.wait();
        }
        self.server_process = None;
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        self.stop();
    }
}

// ============================================================================
// Test Fixtures
// ============================================================================

/// Create standard test fixtures in the export directory
fn create_test_fixtures(export_path: &Path) {
    // Regular file with known content
    fs::write(export_path.join("hello.txt"), b"Hello, fs123!").unwrap();

    // Binary file
    let binary_content: Vec<u8> = (0u8..=255).collect();
    fs::write(export_path.join("binary.dat"), &binary_content).unwrap();

    // Large file (> 1 KiB for offset testing)
    let large_content: Vec<u8> = (0..10240).map(|i| (i % 256) as u8).collect();
    fs::write(export_path.join("large.dat"), &large_content).unwrap();

    // Empty file
    fs::write(export_path.join("empty.txt"), b"").unwrap();

    // Subdirectory with files
    let subdir = export_path.join("subdir");
    fs::create_dir(&subdir).unwrap();
    fs::write(subdir.join("nested.txt"), b"Nested content").unwrap();
    fs::write(subdir.join("another.txt"), b"Another file").unwrap();

    // Deeply nested directory
    let deep_dir = export_path.join("a/b/c/d");
    fs::create_dir_all(&deep_dir).unwrap();
    fs::write(deep_dir.join("deep.txt"), b"Deep file").unwrap();

    // Symbolic link (relative)
    #[cfg(unix)]
    std::os::unix::fs::symlink("hello.txt", export_path.join("link_to_hello")).unwrap();

    // Symbolic link (absolute path in symlink target)
    #[cfg(unix)]
    {
        let absolute_target = export_path.join("hello.txt");
        std::os::unix::fs::symlink(&absolute_target, export_path.join("absolute_link")).unwrap();
    }

    // Symbolic link to directory
    #[cfg(unix)]
    std::os::unix::fs::symlink("subdir", export_path.join("link_to_subdir")).unwrap();

    // File with special characters in name
    fs::write(export_path.join("file with spaces.txt"), b"Has spaces").unwrap();

    // File with permissions
    let restricted_file = export_path.join("restricted.txt");
    fs::write(&restricted_file, b"Restricted content").unwrap();
    let mut perms = fs::metadata(&restricted_file).unwrap().permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&restricted_file, perms).unwrap();

    // Unicode filename
    fs::write(export_path.join("日本語.txt"), b"Japanese filename").unwrap();
}

// ============================================================================
// Tests
// ============================================================================

/// Test: /a endpoint - Get file attributes
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_attributes_regular_file() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    // Test attributes of a regular file
    let mount_file = harness.mount_path().join("hello.txt");
    let export_file = harness.export_path().join("hello.txt");

    let mount_meta = fs::metadata(&mount_file).expect("Failed to stat file through mount");
    let export_meta = fs::metadata(&export_file).expect("Failed to stat export file");

    // Verify key attributes match
    assert_eq!(mount_meta.len(), export_meta.len(), "File size mismatch");
    assert!(mount_meta.is_file(), "Should be a regular file");
    // Mode bits (at least the type bits should match)
    assert_eq!(
        mount_meta.mode() & libc::S_IFMT as u32,
        libc::S_IFREG as u32,
        "Should be regular file type"
    );
}

/// Test: /a endpoint - Get directory attributes
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_attributes_directory() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let mount_dir = harness.mount_path().join("subdir");
    let mount_meta = fs::metadata(&mount_dir).expect("Failed to stat directory through mount");

    assert!(mount_meta.is_dir(), "Should be a directory");
    assert_eq!(
        mount_meta.mode() & libc::S_IFMT as u32,
        libc::S_IFDIR as u32,
        "Should be directory type"
    );
}

/// Test: /a endpoint - Non-existent file returns ENOENT
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_attributes_nonexistent() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let nonexistent = harness.mount_path().join("nonexistent.txt");
    let result = fs::metadata(&nonexistent);

    assert!(result.is_err(), "Should fail for non-existent file");
    let err = result.unwrap_err();
    assert_eq!(
        err.raw_os_error(),
        Some(libc::ENOENT),
        "Should return ENOENT"
    );
}

/// Test: /a endpoint - Symlink attributes (using symlink_metadata)
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_attributes_symlink() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let mount_link = harness.mount_path().join("link_to_hello");
    let mount_meta = fs::symlink_metadata(&mount_link).expect("Failed to stat symlink through mount");

    assert!(mount_meta.file_type().is_symlink(), "Should be a symlink");
}

/// Test: /f endpoint - Read file content
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_file_read_basic() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let mount_file = harness.mount_path().join("hello.txt");
    let content = fs::read_to_string(&mount_file).expect("Failed to read file through mount");

    assert_eq!(content, "Hello, fs123!", "Content should match");
}

/// Test: /f endpoint - Read binary file
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_file_read_binary() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let mount_file = harness.mount_path().join("binary.dat");
    let content = fs::read(&mount_file).expect("Failed to read binary file through mount");

    let expected: Vec<u8> = (0u8..=255).collect();
    assert_eq!(content, expected, "Binary content should match");
}

/// Test: /f endpoint - Read large file with offset
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_file_read_large() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let mount_file = harness.mount_path().join("large.dat");
    let content = fs::read(&mount_file).expect("Failed to read large file through mount");

    let expected: Vec<u8> = (0..10240).map(|i| (i % 256) as u8).collect();
    assert_eq!(content.len(), expected.len(), "Content length should match");
    assert_eq!(content, expected, "Content should match");
}

/// Test: /f endpoint - Read empty file
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_file_read_empty() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let mount_file = harness.mount_path().join("empty.txt");
    let content = fs::read(&mount_file).expect("Failed to read empty file through mount");

    assert!(content.is_empty(), "Empty file should have no content");
}

/// Test: /f endpoint - Read nested file
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_file_read_nested() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let mount_file = harness.mount_path().join("subdir/nested.txt");
    let content = fs::read_to_string(&mount_file).expect("Failed to read nested file");

    assert_eq!(content, "Nested content", "Content should match");
}

/// Test: /f endpoint - Read deeply nested file
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_file_read_deep_nesting() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let mount_file = harness.mount_path().join("a/b/c/d/deep.txt");
    let content = fs::read_to_string(&mount_file).expect("Failed to read deeply nested file");

    assert_eq!(content, "Deep file", "Content should match");
}

/// Test: /d endpoint - List directory contents
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_directory_listing_root() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let entries: Vec<_> = fs::read_dir(harness.mount_path())
        .expect("Failed to read directory")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    // Check for expected files
    assert!(entries.contains(&"hello.txt".to_string()), "Should contain hello.txt");
    assert!(entries.contains(&"binary.dat".to_string()), "Should contain binary.dat");
    assert!(entries.contains(&"subdir".to_string()), "Should contain subdir");
    assert!(entries.contains(&"link_to_hello".to_string()), "Should contain link_to_hello");
}

/// Test: /d endpoint - List subdirectory contents
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_directory_listing_subdir() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let subdir = harness.mount_path().join("subdir");
    let entries: Vec<_> = fs::read_dir(&subdir)
        .expect("Failed to read subdirectory")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    assert!(entries.contains(&"nested.txt".to_string()), "Should contain nested.txt");
    assert!(entries.contains(&"another.txt".to_string()), "Should contain another.txt");
    assert_eq!(entries.len(), 2, "Should have exactly 2 entries");
}

/// Test: /d endpoint - List empty directory
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_directory_listing_empty() {
    let mut harness = TestHarness::new();

    // Create an empty subdirectory
    let empty_dir = harness.export_path().join("empty_dir");
    fs::create_dir(&empty_dir).unwrap();

    harness.start().expect("Failed to start test environment");

    let mount_empty = harness.mount_path().join("empty_dir");
    let entries: Vec<_> = fs::read_dir(&mount_empty)
        .expect("Failed to read empty directory")
        .filter_map(|e| e.ok())
        .collect();

    assert!(entries.is_empty(), "Empty directory should have no entries");
}

/// Test: /d endpoint - Directory with many files
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_directory_listing_many_files() {
    let mut harness = TestHarness::new();

    // Create directory with many files
    let many_dir = harness.export_path().join("many_files");
    fs::create_dir(&many_dir).unwrap();
    for i in 0..100 {
        fs::write(many_dir.join(format!("file_{:03}.txt", i)), format!("Content {}", i)).unwrap();
    }

    harness.start().expect("Failed to start test environment");

    let mount_many = harness.mount_path().join("many_files");
    let entries: Vec<_> = fs::read_dir(&mount_many)
        .expect("Failed to read directory with many files")
        .filter_map(|e| e.ok())
        .collect();

    assert_eq!(entries.len(), 100, "Should have 100 files");
}

/// Test: /l endpoint - Read symbolic link target
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_symlink_readlink() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let mount_link = harness.mount_path().join("link_to_hello");
    let target = fs::read_link(&mount_link).expect("Failed to read symlink");

    assert_eq!(target.to_string_lossy(), "hello.txt", "Symlink target should match");
}

/// Test: /l endpoint - Read content through symlink (follow)
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_symlink_follow() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let mount_link = harness.mount_path().join("link_to_hello");
    let content = fs::read_to_string(&mount_link).expect("Failed to read through symlink");

    assert_eq!(content, "Hello, fs123!", "Content through symlink should match");
}

/// Test: /l endpoint - Symlink to directory
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_symlink_to_directory() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let mount_link = harness.mount_path().join("link_to_subdir");

    // Read link target
    let target = fs::read_link(&mount_link).expect("Failed to read symlink");
    assert_eq!(target.to_string_lossy(), "subdir", "Symlink target should be subdir");

    // List through symlink
    let entries: Vec<_> = fs::read_dir(&mount_link)
        .expect("Failed to read directory through symlink")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    assert!(entries.contains(&"nested.txt".to_string()), "Should see nested.txt through symlink");
}

/// Test: /l endpoint - Broken symlink
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_symlink_broken() {
    let mut harness = TestHarness::new();

    // Create a broken symlink
    #[cfg(unix)]
    std::os::unix::fs::symlink("nonexistent_target", harness.export_path().join("broken_link")).unwrap();

    harness.start().expect("Failed to start test environment");

    let mount_link = harness.mount_path().join("broken_link");

    // readlink should succeed
    let target = fs::read_link(&mount_link).expect("Failed to read broken symlink");
    assert_eq!(target.to_string_lossy(), "nonexistent_target");

    // But following should fail with ENOENT
    let result = fs::read_to_string(&mount_link);
    assert!(result.is_err(), "Following broken symlink should fail");
}

/// Test: /s endpoint - Filesystem statistics (via statfs)
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_statfs() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    // Use nix::sys::statvfs to get filesystem stats
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let mount_path_cstr = CString::new(harness.mount_path().to_string_lossy().as_bytes()).unwrap();

    unsafe {
        let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
        let ret = libc::statvfs(mount_path_cstr.as_ptr(), stat.as_mut_ptr());

        assert_eq!(ret, 0, "statvfs should succeed");

        let stat = stat.assume_init();
        assert!(stat.f_bsize > 0, "Block size should be positive");
        assert!(stat.f_blocks > 0, "Total blocks should be positive");
    }
}

/// Test: Read-only filesystem (write operations should fail)
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_readonly_write_fails() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let new_file = harness.mount_path().join("new_file.txt");
    let result = fs::write(&new_file, b"test");

    assert!(result.is_err(), "Write should fail on read-only filesystem");
    let err = result.unwrap_err();
    // Should return EROFS (read-only file system)
    assert_eq!(
        err.raw_os_error(),
        Some(libc::EROFS),
        "Should return EROFS for write attempt"
    );
}

/// Test: Read-only filesystem (mkdir should fail)
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_readonly_mkdir_fails() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let new_dir = harness.mount_path().join("new_dir");
    let result = fs::create_dir(&new_dir);

    assert!(result.is_err(), "mkdir should fail on read-only filesystem");
}

/// Test: Read-only filesystem (unlink should fail)
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_readonly_unlink_fails() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let file = harness.mount_path().join("hello.txt");
    let result = fs::remove_file(&file);

    assert!(result.is_err(), "unlink should fail on read-only filesystem");
}

/// Test: File with spaces in name
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_file_with_spaces() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let file_with_spaces = harness.mount_path().join("file with spaces.txt");
    let content = fs::read_to_string(&file_with_spaces).expect("Failed to read file with spaces");

    assert_eq!(content, "Has spaces", "Content should match");
}

/// Test: Unicode filename
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_unicode_filename() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let unicode_file = harness.mount_path().join("日本語.txt");
    let content = fs::read_to_string(&unicode_file).expect("Failed to read unicode filename");

    assert_eq!(content, "Japanese filename", "Content should match");
}

/// Test: access() call (permission checking)
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_access_readable() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    use std::os::unix::fs::PermissionsExt;

    let file = harness.mount_path().join("hello.txt");
    let metadata = fs::metadata(&file).expect("Failed to stat file");

    // File should be readable
    let perms = metadata.permissions();
    assert!(perms.mode() & 0o444 != 0, "File should have read permissions");
}

/// Test: Multiple sequential reads
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_multiple_reads() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let file = harness.mount_path().join("hello.txt");

    // Read multiple times
    for _ in 0..10 {
        let content = fs::read_to_string(&file).expect("Failed to read file");
        assert_eq!(content, "Hello, fs123!", "Content should match on repeated reads");
    }
}

/// Test: Concurrent reads from multiple threads
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_concurrent_reads() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    let mount_path = harness.mount_path().to_path_buf();

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let path = mount_path.clone();
            thread::spawn(move || {
                for _ in 0..10 {
                    let file = path.join("hello.txt");
                    let content = fs::read_to_string(&file).expect("Failed to read file");
                    assert_eq!(content, "Hello, fs123!");
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread panicked");
    }
}

/// Test: Directory entry types are correct
#[test]
#[ignore = "Requires FUSE support (macFUSE on macOS, libfuse on Linux)"]
fn test_directory_entry_types() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start().expect("Failed to start test environment");

    for entry in fs::read_dir(harness.mount_path()).expect("Failed to read directory") {
        let entry = entry.expect("Failed to get entry");
        let name = entry.file_name().to_string_lossy().to_string();
        let ft = entry.file_type().expect("Failed to get file type");

        match name.as_str() {
            "hello.txt" | "binary.dat" | "large.dat" | "empty.txt" | "restricted.txt" | "file with spaces.txt" | "日本語.txt" => {
                assert!(ft.is_file(), "{} should be a file", name);
            }
            "subdir" | "a" => {
                assert!(ft.is_dir(), "{} should be a directory", name);
            }
            "link_to_hello" | "absolute_link" | "link_to_subdir" => {
                assert!(ft.is_symlink(), "{} should be a symlink", name);
            }
            _ => {} // Ignore other entries
        }
    }
}

// ============================================================================
// HTTP-level tests (without FUSE, test server directly)
// ============================================================================

/// Test: Server responds to /a endpoint via HTTP
#[test]
#[ignore = "Requires subprocess spawning without sandbox restrictions"]
fn test_http_attributes() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start_server().expect("Failed to start server");

    let url = format!("http://127.0.0.1:{}/fs123/7/3/a/hello.txt", harness.port);
    let response = reqwest::blocking::get(&url).expect("HTTP request failed");

    assert_eq!(response.status(), 200);
    let body = response.text().expect("Failed to read body");
    assert!(body.contains("errno"), "Response should contain errno");
}

/// Test: Server responds to /f endpoint via HTTP
#[test]
#[ignore = "Requires subprocess spawning without sandbox restrictions"]
fn test_http_file_read() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start_server().expect("Failed to start server");

    let url = format!("http://127.0.0.1:{}/fs123/7/3/f/hello.txt?1;0", harness.port);
    let response = reqwest::blocking::get(&url).expect("HTTP request failed");

    assert_eq!(response.status(), 200);
    let body = response.text().expect("Failed to read body");
    assert!(body.contains("Hello, fs123!"), "Response should contain file content");
}

/// Test: Server responds to /d endpoint via HTTP
#[test]
#[ignore = "Requires subprocess spawning without sandbox restrictions"]
fn test_http_directory() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start_server().expect("Failed to start server");

    let url = format!("http://127.0.0.1:{}/fs123/7/3/d/?64", harness.port);
    let response = reqwest::blocking::get(&url).expect("HTTP request failed");

    assert_eq!(response.status(), 200);
    let body = response.text().expect("Failed to read body");
    assert!(body.contains("errno"), "Response should contain errno");
}

/// Test: Server responds to /l endpoint via HTTP
#[test]
#[ignore = "Requires subprocess spawning without sandbox restrictions"]
fn test_http_symlink() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start_server().expect("Failed to start server");

    let url = format!("http://127.0.0.1:{}/fs123/7/3/l/link_to_hello", harness.port);
    let response = reqwest::blocking::get(&url).expect("HTTP request failed");

    assert_eq!(response.status(), 200);
    let body = response.text().expect("Failed to read body");
    assert!(body.contains("hello.txt"), "Response should contain symlink target");
}

/// Test: Server responds to /s endpoint via HTTP
#[test]
#[ignore = "Requires subprocess spawning without sandbox restrictions"]
fn test_http_statfs() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start_server().expect("Failed to start server");

    let url = format!("http://127.0.0.1:{}/fs123/7/3/s/", harness.port);
    let response = reqwest::blocking::get(&url).expect("HTTP request failed");

    assert_eq!(response.status(), 200);
    let body = response.text().expect("Failed to read body");
    assert!(body.contains("errno"), "Response should contain errno");
}

/// Test: Server responds to /n endpoint via HTTP
#[test]
#[ignore = "Requires subprocess spawning without sandbox restrictions"]
fn test_http_server_stats() {
    let mut harness = TestHarness::new();

    harness.start_server().expect("Failed to start server");

    let url = format!("http://127.0.0.1:{}/fs123/7/3/n/", harness.port);
    let response = reqwest::blocking::get(&url).expect("HTTP request failed");

    assert_eq!(response.status(), 200);
    let body = response.text().expect("Failed to read body");
    assert!(body.contains("fs123-server"), "Response should contain server info");
}

/// Test: Server returns 501 for /p endpoint via HTTP
#[test]
#[ignore = "Requires subprocess spawning without sandbox restrictions"]
fn test_http_passthrough_not_implemented() {
    let mut harness = TestHarness::new();

    harness.start_server().expect("Failed to start server");

    let url = format!("http://127.0.0.1:{}/fs123/7/3/p/anything", harness.port);
    let response = reqwest::blocking::get(&url).expect("HTTP request failed");

    assert_eq!(response.status(), 501, "Passthrough should return 501");
}

/// Test: Server returns error for non-existent file
#[test]
#[ignore = "Requires subprocess spawning without sandbox restrictions"]
fn test_http_nonexistent_file() {
    let mut harness = TestHarness::new();

    harness.start_server().expect("Failed to start server");

    let url = format!("http://127.0.0.1:{}/fs123/7/3/a/nonexistent.txt", harness.port);
    let response = reqwest::blocking::get(&url).expect("HTTP request failed");

    assert_eq!(response.status(), 200); // HTTP 200 but with errno in body
    let body = response.text().expect("Failed to read body");
    // Should contain non-zero errno (ENOENT = 2)
    assert!(body.contains("errno"), "Response should contain errno");
}

/// Test: Server handles missing query params for /f
#[test]
#[ignore = "Requires subprocess spawning without sandbox restrictions"]
fn test_http_file_read_missing_params() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start_server().expect("Failed to start server");

    // Missing length and offset parameters
    let url = format!("http://127.0.0.1:{}/fs123/7/3/f/hello.txt", harness.port);
    let response = reqwest::blocking::get(&url).expect("HTTP request failed");

    assert_eq!(response.status(), 400, "Should return 400 for missing params");
}

/// Test: Server handles invalid query params for /f
#[test]
#[ignore = "Requires subprocess spawning without sandbox restrictions"]
fn test_http_file_read_invalid_params() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start_server().expect("Failed to start server");

    // Invalid (non-numeric) parameters
    let url = format!("http://127.0.0.1:{}/fs123/7/3/f/hello.txt?abc;def", harness.port);
    let response = reqwest::blocking::get(&url).expect("HTTP request failed");

    assert_eq!(response.status(), 400, "Should return 400 for invalid params");
}

/// Test: Server handles read with offset beyond file size
#[test]
#[ignore = "Requires subprocess spawning without sandbox restrictions"]
fn test_http_file_read_past_eof() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start_server().expect("Failed to start server");

    // Offset 1000 KiB is way past the 13-byte hello.txt
    let url = format!("http://127.0.0.1:{}/fs123/7/3/f/hello.txt?1;1000", harness.port);
    let response = reqwest::blocking::get(&url).expect("HTTP request failed");

    assert_eq!(response.status(), 200);
    // Content should be empty since offset is past EOF
}

/// Test: Protocol version in URL
#[test]
#[ignore = "Requires subprocess spawning without sandbox restrictions"]
fn test_http_protocol_version() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start_server().expect("Failed to start server");

    // Version 7.2 should also work
    let url = format!("http://127.0.0.1:{}/fs123/7/2/a/hello.txt", harness.port);
    let response = reqwest::blocking::get(&url).expect("HTTP request failed");

    assert_eq!(response.status(), 200);
}

/// Test: URL with selector prefix
#[test]
#[ignore = "Requires subprocess spawning without sandbox restrictions"]
fn test_http_with_selector() {
    let mut harness = TestHarness::new();
    create_test_fixtures(harness.export_path());

    harness.start_server().expect("Failed to start server");

    // URL with selector prefix
    let url = format!("http://127.0.0.1:{}/sel/ector/fs123/7/3/a/hello.txt", harness.port);
    let response = reqwest::blocking::get(&url).expect("HTTP request failed");

    assert_eq!(response.status(), 200);
}
