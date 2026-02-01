# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

fs123-rs is a Rust implementation of the fs123 protocol. Fs123 is a FUSE-based read-only distributed filesystem that translates filesystem operations into HTTP requests.

**Project Structure**:
```
fs123-rs/
├── Cargo.toml              # Workspace root
├── crates/
│   ├── fs123-core/         # Shared protocol library
│   ├── fs123-server/       # HTTP server (fs123-server binary)
│   └── fs123-client/       # FUSE client (fs123-mount binary)
```

## Technology Stack

**Shared Core (`crates/fs123-core`)**:
- Protocol types: `Fs123StatResult`, `Fs123StatvfsResult`, `DirEntryData`
- HTTP client: `Fs123HttpClient` using ureq (blocking)
- Netstring encoding/decoding
- Error types with errno conversion

**Server (`crates/fs123-server`)**:
- **Language**: Rust
- **Web Framework**: actix-web
- **Backend Abstraction**: Pluggable storage backends via async trait
- **Testing**: Unit tests using synchronous HTTP requests

**FUSE Client (`crates/fs123-client`)**:
- **Language**: Rust
- **FUSE Library**: fuser crate
- **CLI**: clap for argument parsing
- **Binary**: `fs123-mount`
- **Caching**: Application-level cache with stale-while-revalidate semantics

The Docker image includes:
- Rust toolchain
- FUSE libraries (`fuse`, `libfuse-dev`)
- Build essentials and OpenSSL

## Building

**Workspace Build** (builds all crates):
```bash
cargo build --workspace
```

**Individual Crates**:
```bash
# Core library
cargo build -p fs123-core

# Server
cargo build -p fs123-server

# FUSE client
cargo build -p fs123-client
```

**Release Builds**:
```bash
cargo build --release --workspace
```

## Architecture

### Shared Core Library (`crates/fs123-core`)

The core library provides shared functionality used by all clients:

**Modules**:
- `error.rs` - `Fs123Error` enum with `to_errno()` for FUSE/FFI error conversion
- `netstring.rs` - Netstring encoding/decoding for protocol responses
- `protocol.rs` - `Fs123HttpClient`, `Fs123Response`, URL building
- `types.rs` - `Fs123StatResult`, `Fs123StatvfsResult`, `DirEntryData`

### FUSE Client (`crates/fs123-client`)

A FUSE filesystem that mounts a remote fs123 server as a local read-only filesystem.

**Modules**:
- `main.rs` - CLI with clap, mount options handling
- `inode.rs` - Bidirectional path↔inode mapping with ESTALE cookie tracking
- `fs.rs` - `fuser::Filesystem` trait implementation
- `cache.rs` - Application-level cache with background refresh

**Usage**:
```bash
fs123-mount -H <host> -p <port> [--proto 7.3] <mountpoint> [options]

Options:
  -H, --host <HOST>                  Server hostname or IP address
  -p, --port <PORT>                  Server port [default: 80]
      --proto <PROTO>                Protocol version [default: 7.3]
      --allow-other                  Allow other users to access
      --allow-root                   Allow root to access
      --auto-unmount                 Auto unmount on exit
      --attr-timeout <SECS>          Kernel attribute cache timeout [default: 1.0]
      --entry-timeout <SECS>         Kernel entry cache timeout [default: 1.0]
  -d, --debug                        Enable debug logging

Cache Options:
      --cache <BOOL>                 Enable application-level caching [default: true]
      --cache-max-attrs <NUM>        Max attribute cache entries [default: 10000]
      --cache-max-dirs <NUM>         Max directory cache entries [default: 1000]
      --cache-max-links <NUM>        Max symlink cache entries [default: 5000]
      --cache-ttl <SECS>             Default TTL/max-age [default: 60]
      --cache-swr <SECS>             Stale-while-revalidate window [default: 300]
      --cache-background-refresh     Enable background refresh [default: true]
      --cache-refresh-threads <NUM>  Background thread count [default: 2]
```

**Implemented FUSE Operations**:
- `lookup`, `forget`, `getattr` - Inode and attribute management
- `open`, `read`, `release` - File reading
- `opendir`, `readdir`, `releasedir` - Directory listing
- `readlink` - Symlink resolution
- `statfs` - Filesystem statistics
- `getxattr`, `listxattr` - Extended attributes
- `access` - Permission checking (read-only enforced)

**Key Design Decisions**:
1. **Path-to-inode mapping**: fs123 is path-based but FUSE is inode-based; `InodeManager` maintains bidirectional mapping
2. **ESTALE handling**: Tracks estalecookies per inode to detect stale file handles
3. **Read-only**: Rejects all write operations with `EROFS`
4. **Two-level caching**:
   - Kernel-level: `--attr-timeout` and `--entry-timeout` control kernel cache behavior
   - Application-level: Stale-while-revalidate cache with background refresh for eventual consistency
5. **File content not cached**: Kernel page cache handles this effectively; only attributes, directories, and symlinks are cached

### Server Implementation

The server responds to HTTP GET requests following the fs123 protocol specification (version 7.3, with backward compatibility for 7.2).

**Modules**:
- `main.rs` - CLI with clap, actix-web server setup
- `lib.rs` - `ServerConfig` and module exports
- `handlers.rs` - Protocol endpoint handlers (`handle_attributes`, `handle_file_read`, etc.)
- `response.rs` - `Fs123ResponseBuilder` for constructing netstring responses
- `backends/` - Backend abstraction layer (see Backend Abstraction Layer section)

**Protocol Endpoints** - Each corresponds to a filesystem operation:
- `/a` - Attributes (returns stat information for a path)
- `/d` - Directory listing (returns directory entries)
- `/f` - File read (returns file content chunks)
- `/l` - Symbolic link (returns link target)
- `/s` - Statfs (returns filesystem statistics)
- `/x` - Extended attributes
- `/n` - Server statistics
- `/p` - Passthrough (server-specific extensions)

**URL Structure**:
```
http://host[:port]/SELECTOR/fs123/MAJOR/MINOR/FUNCTION/PATH?QUERY
```

Example:
```
http://server:8080/selector/fs123/7/3/f/path/to/file?128;0
```

Where:
- SELECTOR: Zero or more path components (server may use for routing/selection)
- `/fs123`: Literal sigil separator
- MAJOR/MINOR: Protocol version (e.g., 7/3)
- FUNCTION: Single letter endpoint (`a`, `d`, `f`, `l`, `s`, `x`, `n`, `p`)
- PATH: Path relative to export root
- QUERY: Semicolon-separated parameters (function-specific, not key-value pairs)

### Backend Abstraction Layer

The server uses a pluggable backend architecture that abstracts storage operations behind the `Backend` trait. This allows the server to serve files from sources other than the local filesystem.

**Architecture** (`src/backends/`):
```
backends/
├── mod.rs        # Module exports and create_backend() factory
├── traits.rs     # Backend trait definition
├── types.rs      # Shared types (AttributeInfo, DirEntry, etc.)
└── file.rs       # FileBackend implementation
```

**Backend Trait** (`Backend`):
All backend implementations must provide these async methods:
- `get_attributes(&self, path: &str) -> BackendResult<AttributeInfo>` - Get stat-like info
- `read_file(&self, path: &str, offset: u64, length: usize) -> BackendResult<FileContent>` - Read file content
- `read_directory(&self, path: &str, max_bytes: usize, start_after: &str) -> BackendResult<DirectoryListing>` - List directory
- `read_symlink(&self, path: &str) -> BackendResult<String>` - Read symlink target
- `statfs(&self, path: &str) -> BackendResult<StatfsInfo>` - Get filesystem statistics
- `get_xattr(&self, path: &str, name: &str, max_size: usize) -> BackendResult<Vec<u8>>` - Get extended attribute
- `describe(&self) -> String` - Return human-readable backend description

**Shared Types** (`types.rs`):
- `AttributeInfo` - File/directory stat information (mode, nlink, uid, gid, size, timestamps, validator, estalecookie)
- `DirEntry` - Single directory entry (name, d_type, estalecookie)
- `DirectoryListing` - Directory read result (entries, nextstart cursor, estalecookie)
- `FileContent` - File read result (data, validator, estalecookie)
- `StatfsInfo` - Filesystem statistics (blocks, files, namemax, etc.)
- `BackendError` - Error with errno (for POSIX error codes)
- `BackendResult<T>` - Result type alias

**FileBackend** (`file.rs`):
The local filesystem backend implementation:
- Serves files from a root directory on the local filesystem
- Uses `fs::symlink_metadata()` for attribute operations
- Implements validator using `mtime` nanoseconds
- Implements estalecookie using inode number (TODO: use `FS_IOC_GETVERSION`)
- Handles path resolution by joining request paths with the root directory

**Backend Factory** (`create_backend()`):
The `create_backend(url: &str)` factory function creates backends from URL strings:
- `file:///path` or `/path` → Creates `FileBackend` with the given path
- Bare paths default to `file://` scheme
- Returns `Result<Arc<dyn Backend>, String>` for error handling
- Extensible for future backends (e.g., `s3://`, `http://`)

**Server Configuration**:
`ServerConfig` holds an `Arc<dyn Backend>` instead of a direct filesystem path:
```rust
pub struct ServerConfig {
    pub backend: Arc<dyn Backend>,
    pub default_max_age: u32,
    pub default_stale_while_revalidate: u32,
}
```

**CLI Usage**:
```bash
fs123-server --export-root <URL> [options]

# Examples:
fs123-server --export-root /srv/data
fs123-server --export-root file:///srv/data
```

**Handler Integration**:
All protocol endpoint handlers (`handle_attributes`, `handle_file_read`, etc.) call the configured backend methods instead of direct filesystem operations. This keeps handler logic independent of storage implementation.

**Future Backends**:
The abstraction enables future storage backends such as:
- S3 or object storage backends
- Database backends
- HTTP proxy backends (chain multiple fs123 servers)
- Caching or overlay backends
- Virtual or synthetic filesystems

### Response Format

All successful responses (200, 304) contain netstring-encoded key-value pairs:

```
5:errno, 1:0,
7:content, 11:hello world,
```

**Required Keys** (varies by endpoint):
- `errno` - Error code as decimal integer (0 = success, non-zero = filesystem errno)
- `content` - The actual data (format depends on function)
- `validator` - 64-bit decimal unsigned integer that changes when content changes (required for `/a`, `/f`)
- `estalecookie` - 64-bit decimal unsigned integer that changes when inode changes (required for `/a`, `/d`, `/f`)
- `nextstart` - Opaque cursor for directory iteration (required for `/d`)

## Protocol Details

### Query Parameters

Parameters in QUERY are semicolon-separated and function-specific, NOT standard key=value CGI parameters:
- `/f/path?128;0` means: read 128 KiB starting at offset 0 KiB (Len;Offset)
- `/d/path?64;cursor` means: read 64 KiB of directory entries starting at cursor
- `/x/path?128;user.xattr;` means: read extended attribute "user.xattr" up to 128 KiB

**Important**: For `/f` (file read), the query is `Len;Offset` (length first, then offset).

### Content Units

Length and offset parameters are in **kibibytes (KiB)**, not bytes. Multiply by 1024 to get byte offsets.

### Netstring Format

`<length>:<data>,` - The comma is part of the format. No escaping inside data; it's raw binary.

### Error Handling

- 5xx: Internal server errors (client sees EIO)
- 4xx: Protocol errors (client sees EIO)
- 200 with non-zero errno: Filesystem-level semantic errors (ENOENT, EPERM, etc.)

### ESTALE Cookies

64-bit identifiers ensuring clients don't read stale data when files are unlinked/renamed:
- Must be unique per inode lifetime
- Use `ioctl(FS_IOC_GETVERSION)` if backed by ext3/ext4/xfs/btrfs
- Use 0 only for truly immutable/write-once filesystems
- Never use `st_ino` alone (inodes can be reused)

### Validators

Monotonically increasing 64-bit values that MUST change when file content changes. Used for cache invalidation. The value `0xffffffffffffffff` is reserved.

## Protocol Documentation

Essential reading in `legacy/docs/`:
- **Fs123Protocol**: Complete protocol specification (v7.3 and v7.2)
- **Fs123Consistency**: Consistency model, settled time, and ESTALE semantics
- **Fs123CacheControl**: Caching strategy and implications

## Testing

### Unit Tests

```bash
# Run all unit tests
cargo test --workspace

# Run tests for specific crate
cargo test -p fs123-core
cargo test -p fs123-server
cargo test -p fs123-client
```

### Integration Tests

Integration tests require special environments and are marked `#[ignore]` by default.

**HTTP-level tests** (test server directly, no FUSE required):
```bash
# Requires: no sandbox restrictions (actix-web uses nice() syscall)
cargo test -p fs123-server --test fuse_integration_tests -- test_http --ignored
```

**FUSE mount tests** (test full stack through mounted filesystem):
```bash
# Requires: FUSE support + no sandbox restrictions
# Use --test-threads=1 to avoid mount conflicts
cargo test -p fs123-server --test fuse_integration_tests -- --ignored --test-threads=1
```

**All integration tests**:
```bash
cargo test -p fs123-server --test fuse_integration_tests -- --include-ignored --test-threads=1
```

### Docker Testing

Docker is the recommended environment for integration tests (has FUSE support, no sandbox):

```bash
# Run all tests including integration tests
docker-compose exec client cargo test --workspace -- --include-ignored

# Run only FUSE integration tests
docker-compose exec client cargo test -p fs123-server --test fuse_integration_tests -- --ignored
```

### Test Coverage

The integration tests cover all protocol endpoints:
- `/a` - File/directory attributes
- `/d` - Directory listing
- `/f` - File reading (basic, binary, large files, offsets)
- `/l` - Symbolic links
- `/s` - Filesystem statistics (statfs)
- `/x` - Extended attributes (returns ENOTSUP)
- `/n` - Server statistics
- `/p` - Passthrough (returns 501)

Plus edge cases: unicode filenames, spaces in paths, concurrent access, read-only enforcement.
