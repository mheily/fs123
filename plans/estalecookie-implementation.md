# ESTALE Cookie Implementation - Summary

## Overview

The Rust fs123-server now supports multiple ESTALE cookie strategies similar to the C++ implementation, configured via URL parameters.

## Implementation Details

### New Types

**`EstaleCookieSource` enum** (in `crates/fs123-server/src/backends/file.rs`):
```rust
pub enum EstaleCookieSource {
    GetVersionIoctl,      // Use FS_IOC_GETVERSION ioctl (recommended)
    ExtendedAttribute,    // Use xattr (not yet implemented)
    Inode,                // Use st_ino (not recommended)
    None,                 // Return 0 (disabled)
}
```

### Strategy Implementations

#### 1. **GetVersionIoctl** (Default, Recommended)
- Uses `ioctl(fd, FS_IOC_GETVERSION, &generation)` on Linux
- Returns filesystem-maintained generation counter
- Unique per inode lifetime (changes when inode is reused)
- Supported on ext3/ext4/xfs/btrfs
- Falls back to inode number if ioctl fails or on non-Linux platforms

#### 2. **ExtendedAttribute**
- Placeholder for future implementation
- Would read/write `user.fs123.estalecookie` xattr
- Currently falls back to inode strategy

#### 3. **Inode**
- Uses `metadata.ino()` directly
- Simple but NOT recommended (inodes can be reused)
- Matches old Rust behavior

#### 4. **None**
- Always returns 0
- Indicates immutable data (inode never changes)
- Only for write-once filesystems

### Configuration

Configured via URL parameters in the `--export-root` argument:

```bash
# Default (uses GetVersionIoctl)
fs123-server --export-root /srv/data

# Explicit strategy via URL parameter
fs123-server --export-root "file:///srv/data?estalecookie=ioc_getversion"
fs123-server --export-root "/srv/data?estalecookie=inode"
fs123-server --export-root "/srv/data?estalecookie=none"

# With other URL parameters
fs123-server --export-root "/srv/data?estalecookie=inode&other_param=value"
```

**Valid parameter values**:
- `ioc_getversion`, `getversion`, `ioctl` → GetVersionIoctl
- `xattr`, `extendedattribute`, `extended_attribute` → ExtendedAttribute
- `inode`, `st_ino` → Inode
- `none`, `disabled`, `0` → None

### Code Changes

**Modified files**:
1. `crates/fs123-server/src/backends/file.rs`:
   - Added `EstaleCookieSource` enum
   - Added `estalecookie_src` field to `FileBackend`
   - Rewrote `get_estalecookie()` as instance method with strategy dispatch
   - Updated all callers to pass path and use instance method

2. `crates/fs123-server/src/backends/mod.rs`:
   - Exported `EstaleCookieSource`
   - Added `parse_estale_cookie_source()` helper
   - Modified `create_backend()` to parse URL parameters
   - Added tests for URL parameter parsing

3. `crates/fs123-server/src/main.rs`:
   - Updated documentation for `--export-root` to describe URL parameters

### Comparison to C++

| Aspect | C++ | Rust |
|--------|-----|------|
| Strategy selection | `--fs123-estale-cookie-src` CLI flag | URL parameter `?estalecookie=` |
| Default | IOC_GETVERSION | IOC_GETVERSION (new) |
| Strategies | 4 (all implemented) | 4 (3 implemented, 1 stub) |
| Fallback behavior | Returns 0 on error | Falls back to inode on ioctl error |
| Platform support | Linux-specific ioctl | Linux ioctl + cross-platform fallback |

### Testing

All existing tests pass. New tests added:
- `test_create_backend_with_estale_url_param` - URL parameter parsing
- `test_create_backend_invalid_estale_param` - Error handling

### Known Limitations

1. **ExtendedAttribute strategy not implemented** - Currently falls back to inode
2. **FS_IOC_GETVERSION constant hardcoded** - Should use libc const when available
3. **No validation of filesystem support** - Silently falls back on unsupported filesystems
4. **Debug output to stderr** - Should use proper logging

### Future Work

- Implement ExtendedAttribute strategy using `fgetxattr`/`fsetxattr`
- Use proper logging instead of `eprintln!`
- Add integration tests verifying cookie behavior
- Consider adding CLI flag as alternative to URL parameter
- Add runtime detection of filesystem support for IOC_GETVERSION
