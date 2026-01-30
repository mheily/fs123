# Caching in fs123-fuse

## Overview

The fs123 FUSE client implements an application-level cache with stale-while-revalidate semantics. This cache sits above the kernel-level FUSE caching and implements background refresh to achieve eventual consistency while prioritizing performance over freshness.

## Architecture

The caching system consists of three main components:

1. **Fs123Cache**: Main cache manager that stores attributes, directories, and symlinks
2. **CacheMetadata**: Tracks freshness state, validators, and ESTALE cookies
3. **BackgroundRefresher**: Worker threads that asynchronously revalidate stale entries

## What is Cached

The client caches three types of data:

- **Attributes** (from `/a` endpoint): File/directory metadata including stat information
- **Directory listings** (from `/d` endpoint): Complete directory contents
- **Symlink targets** (from `/l` endpoint): Symbolic link destinations

**Note**: File content is NOT cached at the application level. The kernel page cache already handles this effectively.

## Cache States

Each cache entry can be in one of four states:

1. **Fresh**: Within max-age, served immediately
2. **Stale**: Beyond max-age but within stale-while-revalidate window, served immediately with background refresh
3. **StaleRevalidating**: Background refresh in progress, stale data still served
4. **Expired**: Beyond stale-while-revalidate window, must fetch fresh data

## Stale-While-Revalidate

This caching strategy prioritizes performance:

1. When a cache entry is **fresh** (age ≤ max-age): Return immediately
2. When a cache entry is **stale** (age > max-age but ≤ max-age + swr):
   - Return the stale data immediately (non-blocking)
   - Queue a background revalidation request
   - Background thread fetches fresh data for next request
3. When a cache entry is **expired** (age > max-age + swr): Block and fetch fresh

This ensures requests are never delayed by cache revalidation while still achieving eventual consistency.

## Revalidation Strategy

Background revalidation uses lightweight GET requests to the `/a` (attributes) endpoint to check if cached data is still valid:

1. GET `/a/{path}` to fetch current validator and estalecookie
2. Compare with cached values:
   - **Validator unchanged**: Cached content is still valid, no action needed
   - **Validator increased**: Content changed, fetch fresh data
   - **Validator decreased**: Out-of-order response from stale proxy, ignore
   - **Estalecookie changed**: File was replaced, invalidate all caches for path

## Cache Invalidation

The cache uses two mechanisms for invalidation:

### Validators

64-bit values that change when file content changes. Used to detect stale data:
- If validator in response < cached validator: Reject (out-of-order response)
- If validator in response > cached validator: Content changed, update cache

### ESTALE Cookies

64-bit identifiers that change when the inode at a path changes (file unlinked/renamed):
- If estalecookie changes: Invalidate all cached data for that path
- Ensures POSIX semantics for file replacement

## Cache Eviction

The cache uses simple random eviction when reaching capacity:
- Attributes: 10,000 entries (default)
- Directories: 1,000 entries (default)
- Symlinks: 5,000 entries (default)

No LRU tracking is implemented for simplicity.

## Configuration

Cache behavior is controlled via CLI flags:

```bash
fs123-mount --help

Cache Options:
  --cache <BOOL>                     Enable caching [default: true]
  --cache-max-attrs <NUM>            Max attribute cache entries [default: 10000]
  --cache-max-dirs <NUM>             Max directory cache entries [default: 1000]
  --cache-max-links <NUM>            Max symlink cache entries [default: 5000]
  --cache-ttl <SECS>                 Default TTL/max-age [default: 60]
  --cache-swr <SECS>                 Stale-while-revalidate window [default: 300]
  --cache-background-refresh <BOOL>  Enable background refresh [default: true]
  --cache-refresh-threads <NUM>      Background thread count [default: 2]
```

### Example Usage

**Aggressive caching** (5 minute TTL, 1 hour stale):
```bash
fs123-mount -H server -p 8123 --cache-ttl 300 --cache-swr 3600 /mnt
```

**Disable caching**:
```bash
fs123-mount -H server -p 8123 --cache false /mnt
```

**Large cache** for many files:
```bash
fs123-mount -H server -p 8123 --cache-max-attrs 100000 /mnt
```

## Performance Considerations

### When Caching Helps

- High latency connections to server
- Repeated access to same files/directories
- Workloads with good temporal locality
- Read-heavy workloads

### When Caching May Not Help

- First-time access to files (cold cache)
- Rapidly changing server data
- Low-latency local network
- Workloads with poor locality

### Tuning Guidelines

- **Increase TTL**: If data changes infrequently and staleness is acceptable
- **Increase SWR**: To serve more requests without blocking on revalidation
- **Increase cache sizes**: If you have many files and sufficient memory
- **Increase refresh threads**: If revalidation queue backs up (check logs)

## Monitoring

The cache logs activity at DEBUG level:

```bash
# Enable debug logging
fs123-mount -H server -p 8123 --debug /mnt

# Logged events:
# - Cache hits (fresh/stale)
# - Cache misses
# - Background refresh operations
# - Evictions
# - Invalidations
```

## Consistency Guarantees

The cache provides **eventual consistency**:

1. Stale data may be served for up to `max-age + stale-while-revalidate` seconds
2. Out-of-order responses from stale proxies are detected and rejected
3. ESTALE cookies ensure file replacement is detected
4. Background refresh ensures cache converges to server state

This matches the consistency model described in the fs123 protocol documentation (see `legacy/docs/Fs123Consistency`).

## Thread Safety

The cache is fully thread-safe:
- Uses `parking_lot::RwLock` for concurrent read access
- Background refresh threads coordinate via channels
- Multiple FUSE threads can access cache simultaneously

## Implementation Details

- **Language**: Rust
- **Dependencies**: `parking_lot` (RwLock), `crossbeam-channel` (refresh queue)
- **Location**: `crates/fs123-client/src/cache.rs`
- **Tests**: Comprehensive unit tests in cache module

## See Also

- Protocol documentation: `legacy/docs/Fs123Protocol`
- Consistency model: `legacy/docs/Fs123Consistency`
- Cache control: `legacy/docs/Fs123CacheControl`
