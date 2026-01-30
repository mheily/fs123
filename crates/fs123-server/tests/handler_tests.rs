/// Tests for handler functions with query parameter parsing
use std::fs;
use std::path::PathBuf;

// Helper to create test files
fn setup_test_dir() -> PathBuf {
    let test_dir = PathBuf::from("/tmp/fs123-handler-test");
    if test_dir.exists() {
        fs::remove_dir_all(&test_dir).ok();
    }
    fs::create_dir_all(&test_dir).unwrap();

    // Create test file
    fs::write(test_dir.join("testfile.txt"), b"Hello, World! This is a test file with some content.").unwrap();

    // Create subdirectory
    let subdir = test_dir.join("subdir");
    fs::create_dir_all(&subdir).unwrap();
    fs::write(subdir.join("file1.txt"), b"File 1").unwrap();
    fs::write(subdir.join("file2.txt"), b"File 2").unwrap();

    test_dir
}

#[test]
fn test_query_param_parsing_in_context() {
    use fs123_core::parse_url;

    // Test file read with query parameters
    let url = "/fs123/7/3/f/testfile.txt?1;0";
    let req = parse_url(url).unwrap();

    assert_eq!(req.function, "f");
    assert_eq!(req.path, "/testfile.txt");
    assert_eq!(req.query_params.len(), 2);
    assert_eq!(req.query_params[0], "1");
    assert_eq!(req.query_params[1], "0");

    // Verify we can parse the parameters
    let len_kib: usize = req.query_params[0].parse().unwrap();
    let offset_kib: u64 = req.query_params[1].parse().unwrap();
    assert_eq!(len_kib, 1);
    assert_eq!(offset_kib, 0);
}

#[test]
fn test_directory_query_params() {
    use fs123_core::parse_url;

    let url = "/fs123/7/3/d/mydir?64;lastfile";
    let req = parse_url(url).unwrap();

    assert_eq!(req.function, "d");
    assert_eq!(req.query_params.len(), 2);
    assert_eq!(req.query_params[0], "64");
    assert_eq!(req.query_params[1], "lastfile");
}

#[test]
fn test_xattr_query_params() {
    use fs123_core::parse_url;

    let url = "/fs123/7/3/x/file?128;user.myattr;";
    let req = parse_url(url).unwrap();

    assert_eq!(req.function, "x");
    assert_eq!(req.query_params.len(), 3);
    assert_eq!(req.query_params[0], "128");
    assert_eq!(req.query_params[1], "user.myattr");
    assert_eq!(req.query_params[2], "");
}

#[test]
fn test_numeric_params() {
    use fs123_core::parse_url;

    // Test various numeric values
    let test_cases = vec![
        ("/fs123/7/3/f/file?123;45", vec!["123", "45"]),
        ("/fs123/7/3/f/file?0;0", vec!["0", "0"]),
        ("/fs123/7/3/f/file?1024;2048", vec!["1024", "2048"]),
        ("/fs123/7/3/d/dir?64;", vec!["64", ""]),
    ];

    for (url, expected) in test_cases {
        let req = parse_url(url).unwrap();
        assert_eq!(req.query_params, expected, "Failed for URL: {}", url);
    }
}

#[test]
fn test_full_request_pipeline() {
    use fs123_core::parse_url;

    // Simulate a complete request cycle
    let url = "/fs123/7/3/f/testfile.txt?1;0";

    // Step 1: Parse URL
    let req = parse_url(url).unwrap();
    assert_eq!(req.function, "f");
    assert_eq!(req.path, "/testfile.txt");

    // Step 2: Validate query parameters (like handler does)
    assert!(req.query_params.len() >= 2, "Missing length and offset parameters");

    // Step 3: Parse parameters as handler does
    let len_kib: usize = req.query_params[0].parse()
        .expect("Invalid length parameter");
    let offset_kib: u64 = req.query_params[1].parse()
        .expect("Invalid offset parameter");

    // Step 4: Verify parsed values
    assert_eq!(len_kib, 1);
    assert_eq!(offset_kib, 0);

    // Verify byte conversion (as handler does)
    let len_bytes = len_kib * 1024;
    let offset_bytes = offset_kib * 1024;
    assert_eq!(len_bytes, 1024);
    assert_eq!(offset_bytes, 0);
}

#[test]
fn test_edge_cases() {
    use fs123_core::parse_url;

    // Large offsets
    let req = parse_url("/fs123/7/3/f/file?1024;999999").unwrap();
    let offset: u64 = req.query_params[1].parse().unwrap();
    assert_eq!(offset, 999999);

    // Zero values
    let req = parse_url("/fs123/7/3/f/file?0;0").unwrap();
    assert_eq!(req.query_params[0], "0");
    assert_eq!(req.query_params[1], "0");

    // Complex path with query
    let req = parse_url("/fs123/7/3/f/deep/nested/path/file.txt?128;64").unwrap();
    assert_eq!(req.path, "/deep/nested/path/file.txt");
    assert_eq!(req.query_params.len(), 2);
}
