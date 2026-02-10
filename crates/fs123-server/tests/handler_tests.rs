/// Tests for handler functions with query parameter parsing

#[test]
fn test_query_param_parsing_in_context() {
    use fs123_core::parse_url;

    // Test file read with query parameters
    let url = "/fs123/7/3/f/testfile.txt?1;0";
    let req = parse_url(url).unwrap();

    assert_eq!(req.function, fs123_core::Fs123Function::Read);
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

    assert_eq!(req.function, fs123_core::Fs123Function::Readdir);
    assert_eq!(req.query_params.len(), 2);
    assert_eq!(req.query_params[0], "64");
    assert_eq!(req.query_params[1], "lastfile");
}

#[test]
fn test_xattr_query_params() {
    use fs123_core::parse_url;

    let url = "/fs123/7/3/x/file?128;user.myattr;";
    let req = parse_url(url).unwrap();

    assert_eq!(req.function, fs123_core::Fs123Function::Xattr);
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
    assert_eq!(req.function, fs123_core::Fs123Function::Read);
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

#[test]
fn test_v8_protocol_parsing() {
    use fs123_core::parse_url;

    // Test v8 stat endpoint
    let req = parse_url("/fs123/8/0/stat/testfile.txt").unwrap();
    assert_eq!(req.major_version, 8);
    assert_eq!(req.minor_version, 0);
    assert_eq!(req.function, fs123_core::Fs123Function::Stat);
    assert_eq!(req.path, "/testfile.txt");

    // Test v8 read endpoint
    let req = parse_url("/fs123/8/0/read/testfile.txt?1;0").unwrap();
    assert_eq!(req.function, fs123_core::Fs123Function::Read);
    assert_eq!(req.query_params, vec!["1", "0"]);

    // Test v8 readdir endpoint
    let req = parse_url("/fs123/8/0/readdir/mydir?64").unwrap();
    assert_eq!(req.function, fs123_core::Fs123Function::Readdir);
    assert_eq!(req.query_params, vec!["64"]);

    // Test v8 readlink endpoint
    let req = parse_url("/fs123/8/0/readlink/mysymlink").unwrap();
    assert_eq!(req.function, fs123_core::Fs123Function::Readlink);

    // Test v8 statvfs endpoint
    let req = parse_url("/fs123/8/0/statvfs/").unwrap();
    assert_eq!(req.function, fs123_core::Fs123Function::Statvfs);

    // Test v8 getxattr endpoint
    let req = parse_url("/fs123/8/0/getxattr/file?128;user.attr;").unwrap();
    assert_eq!(req.function, fs123_core::Fs123Function::Getxattr);
    assert_eq!(req.query_params, vec!["128", "user.attr", ""]);

    // Test v8 listxattr endpoint
    let req = parse_url("/fs123/8/0/listxattr/file?128").unwrap();
    assert_eq!(req.function, fs123_core::Fs123Function::Listxattr);
    assert_eq!(req.query_params, vec!["128"]);
}

#[test]
fn test_v7_and_v8_compatibility() {
    use fs123_core::parse_url;

    // Verify both v7 and v8 can be parsed
    let v7_req = parse_url("/fs123/7/3/a/file").unwrap();
    let v8_req = parse_url("/fs123/8/0/stat/file").unwrap();

    // Both should parse the same path
    assert_eq!(v7_req.path, v8_req.path);

    // Both map to the same enum variant
    assert_eq!(v7_req.function, fs123_core::Fs123Function::Stat);
    assert_eq!(v8_req.function, fs123_core::Fs123Function::Stat);

    // Different protocol versions
    assert_eq!(v7_req.major_version, 7);
    assert_eq!(v8_req.major_version, 8);
}
