# fs123-server Testing Documentation

## Test Coverage

The fs123 project has comprehensive test coverage across multiple crates.

### Unit Tests

#### fs123-core (31 tests)

Tests for shared protocol components are in `crates/fs123-core/src/`:

**Netstring Module (`netstring.rs` - 6 tests)**:
- `test_encode` - Verifies netstring encoding for strings and binary data
- `test_encode_str` - String encoding helper
- `test_decode` - Verifies netstring decoding with correct byte count
- `test_decode_invalid` - Verifies rejection of malformed netstrings
- `test_decode_str` - UTF-8 string decoding
- `test_parse_response` - Parse netstring key-value response body

**Protocol Module (`protocol.rs` - 22 tests)**:
- URL parsing tests (basic, with selector, root path, versions)
- Query parameter parsing (semicolon-separated, URL encoding, special chars)
- HTTP client and response handling

**Types Module (`types.rs` - 3 tests)**:
- Stat result serialization
- Directory entry parsing
- Statvfs result handling

#### fs123-server (3 tests)

Tests in `crates/fs123-server/src/`:

**Response Builder Module (`response.rs` - 3 tests)**:
- `test_simple_response` - Basic response with errno and content
- `test_error_response` - Error response with non-zero errno
- `test_with_validator` - Response with validator and estalecookie

### Integration Tests (48 tests)

Tests in `crates/fs123-server/tests/`:

#### Handler Tests (`handler_tests.rs` - 6 tests)
- `test_query_param_parsing_in_context` - End-to-end query parsing for file reads
- `test_directory_query_params` - Directory listing query validation
- `test_xattr_query_params` - Extended attribute query validation
- `test_numeric_params` - Various numeric parameter combinations
- `test_full_request_pipeline` - Complete request cycle simulation
- `test_edge_cases` - Large offsets, zero values, complex paths

#### Setup Tests (`integration_test.rs` - 1 test)
- `test_server_setup` - Verify test directory creation

#### FUSE Integration Tests (`fuse_integration_tests.rs` - 41 tests)

Full end-to-end tests that start the server and FUSE client:

**Attribute Tests (`/a` endpoint)**:
- `test_attributes_regular_file` - File stat
- `test_attributes_directory` - Directory stat
- `test_attributes_nonexistent` - ENOENT handling
- `test_attributes_symlink` - Symlink metadata

**File Read Tests (`/f` endpoint)**:
- `test_file_read_basic` - Simple file content
- `test_file_read_binary` - Binary data (all byte values)
- `test_file_read_large` - Large files with offset handling
- `test_file_read_empty` - Empty file
- `test_file_read_nested` - Nested directory files
- `test_file_read_deep_nesting` - Deeply nested paths

**Directory Tests (`/d` endpoint)**:
- `test_directory_listing_root` - Root directory listing
- `test_directory_listing_subdir` - Subdirectory listing
- `test_directory_listing_empty` - Empty directory
- `test_directory_listing_many_files` - Directory with 100 files
- `test_directory_entry_types` - Correct file types in listings

**Symlink Tests (`/l` endpoint)**:
- `test_symlink_readlink` - Read link target
- `test_symlink_follow` - Follow symlink to read content
- `test_symlink_to_directory` - Directory symlinks
- `test_symlink_broken` - Broken symlink handling

**Statfs Test (`/s` endpoint)**:
- `test_statfs` - Filesystem statistics

**Read-only Enforcement**:
- `test_readonly_write_fails` - Write returns EROFS
- `test_readonly_mkdir_fails` - Mkdir fails
- `test_readonly_unlink_fails` - Unlink fails

**Edge Cases**:
- `test_file_with_spaces` - Filenames with spaces
- `test_unicode_filename` - Unicode filenames (日本語.txt)
- `test_access_readable` - Permission checking
- `test_multiple_reads` - Sequential reads
- `test_concurrent_reads` - Multi-threaded access

**HTTP-level Tests** (no FUSE required):
- `test_http_attributes` - `/a` endpoint
- `test_http_file_read` - `/f` endpoint
- `test_http_directory` - `/d` endpoint
- `test_http_symlink` - `/l` endpoint
- `test_http_statfs` - `/s` endpoint
- `test_http_server_stats` - `/n` endpoint
- `test_http_passthrough_not_implemented` - `/p` returns 501
- `test_http_nonexistent_file` - ENOENT in response
- `test_http_file_read_missing_params` - 400 for missing params
- `test_http_file_read_invalid_params` - 400 for invalid params
- `test_http_file_read_past_eof` - Read past end of file
- `test_http_protocol_version` - Version 7.2 compatibility
- `test_http_with_selector` - URL selector prefix

## Running Tests

```bash
# Run all unit tests (no special requirements)
cargo test --workspace

# Run only fs123-core tests
cargo test -p fs123-core

# Run only fs123-server unit tests
cargo test -p fs123-server --lib

# Run handler integration tests
cargo test -p fs123-server --test handler_tests

# Run HTTP-level tests (requires no sandbox restrictions)
cargo test -p fs123-server --test fuse_integration_tests -- test_http --ignored

# Run FUSE tests (requires FUSE support + no sandbox)
cargo test -p fs123-server --test fuse_integration_tests -- --ignored --test-threads=1

# Run all tests including integration tests
cargo test -p fs123-server --test fuse_integration_tests -- --include-ignored --test-threads=1
```

## Test Statistics

- **Total Tests:** ~82
- **Unit Tests:** 34 (31 in fs123-core, 3 in fs123-server)
- **Integration Tests:** 48 (6 handler + 1 setup + 41 FUSE)
- **Coverage Areas:** URL parsing, netstring encoding, response building, query parameters, all protocol endpoints, read-only enforcement, edge cases
