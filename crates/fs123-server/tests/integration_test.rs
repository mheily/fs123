/// Integration tests for fs123 server
use std::fs;
use tempfile::TempDir;

#[test]
fn test_server_setup() {
    // Create test directory structure using tempfile (respects TMPDIR)
    let test_dir = TempDir::new().expect("Failed to create temp dir");

    // Create test files
    fs::write(test_dir.path().join("test.txt"), b"Hello from fs123!").unwrap();

    let subdir = test_dir.path().join("testdir");
    fs::create_dir_all(&subdir).unwrap();
    fs::write(subdir.join("file2.txt"), b"Another file").unwrap();

    // Verify files exist
    assert!(test_dir.path().join("test.txt").exists());
    assert!(subdir.join("file2.txt").exists());
}

// Note: Full server integration tests would require spawning the server
// and making HTTP requests. For now, we rely on unit tests of individual
// components (protocol parsing, netstring encoding, handlers, etc.)
