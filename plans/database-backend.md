# Database Backend Design

## Overview

A new backend implementation that stores filesystem metadata in a SQL database while keeping file content external (referenced by URL). This enables:

- Filesystem metadata managed in a database (easy to query, update, replicate)
- File content stored separately (local files, S3, HTTP, etc.)
- Decoupled metadata from storage location

## Schema Design

### Goals
1. **Simple** - Single table, no JOINs for common operations
2. **Denormalized** - Each row contains all metadata needed
3. **No BLOBs** - Content stored externally via URLs
4. **Efficient directory listing** - Index on parent path

### Schema

```sql
-- Main files table - stores all filesystem entries
CREATE TABLE files (
    -- Path information
    path TEXT PRIMARY KEY,           -- Full absolute path: "/foo/bar.txt"
    parent_path TEXT NOT NULL,       -- Parent directory: "/foo"
    name TEXT NOT NULL,              -- Entry name: "bar.txt"

    -- File type
    file_type TEXT NOT NULL,         -- 'file', 'directory', 'symlink'

    -- Standard stat fields
    mode INTEGER NOT NULL,           -- Unix permission bits (e.g., 0o644)
    nlink INTEGER NOT NULL,          -- Number of hard links
    uid INTEGER NOT NULL,            -- Owner user ID
    gid INTEGER NOT NULL,            -- Owner group ID
    size INTEGER NOT NULL,           -- Size in bytes

    -- Timestamps (stored as separate sec/nsec for precision)
    mtime_sec INTEGER NOT NULL,
    mtime_nsec INTEGER NOT NULL,
    atime_sec INTEGER NOT NULL,
    atime_nsec INTEGER NOT NULL,
    ctime_sec INTEGER NOT NULL,
    ctime_nsec INTEGER NOT NULL,

    -- Content reference (NULL for directories)
    content_url TEXT,                -- "file:///path", "s3://bucket/key", etc.

    -- Symlink target (NULL for non-symlinks)
    symlink_target TEXT,

    -- fs123 protocol fields
    validator INTEGER NOT NULL,      -- Changes when content changes
    estalecookie INTEGER NOT NULL    -- Changes when inode changes
);

-- Index for efficient directory listings
CREATE INDEX idx_files_parent ON files(parent_path);

-- Optional: Index for content URL lookups (useful for garbage collection)
CREATE INDEX idx_files_content_url ON files(content_url) WHERE content_url IS NOT NULL;
```

### Example Data

```sql
-- Root directory
INSERT INTO files VALUES (
    '/', '', '', 'directory',
    16877, 2, 0, 0, 4096,
    1704067200, 0, 1704067200, 0, 1704067200, 0,
    NULL, NULL, 1704067200000000000, 1
);

-- A regular file
INSERT INTO files VALUES (
    '/docs/readme.txt', '/docs', 'readme.txt', 'file',
    33188, 1, 1000, 1000, 1234,
    1704067200, 0, 1704067200, 0, 1704067200, 0,
    'file:///var/fs123-content/abc123.txt', NULL,
    1704067200000000000, 42
);

-- A file on S3
INSERT INTO files VALUES (
    '/data/large-dataset.csv', '/data', 'large-dataset.csv', 'file',
    33188, 1, 1000, 1000, 1073741824,
    1704067200, 0, 1704067200, 0, 1704067200, 0,
    's3://my-bucket/datasets/large-dataset.csv', NULL,
    1704067200000000000, 43
);

-- A symlink
INSERT INTO files VALUES (
    '/link', '/', 'link', 'symlink',
    41471, 1, 1000, 1000, 11,
    1704067200, 0, 1704067200, 0, 1704067200, 0,
    NULL, '/docs/readme.txt',
    1704067200000000000, 44
);
```

## Query Patterns

### Get attributes for a path
```sql
SELECT * FROM files WHERE path = ?;
```

### List directory entries
```sql
SELECT name, file_type, estalecookie
FROM files
WHERE parent_path = ?
  AND name > ?           -- pagination cursor
ORDER BY name
LIMIT ?;
```

### Read symlink target
```sql
SELECT symlink_target FROM files WHERE path = ? AND file_type = 'symlink';
```

### Get content URL for file read
```sql
SELECT content_url, validator, estalecookie, size
FROM files
WHERE path = ? AND file_type = 'file';
```

## Content URL Schemes

The `content_url` field supports multiple schemes:

| Scheme | Example | Description |
|--------|---------|-------------|
| `file://` | `file:///var/content/abc.txt` | Local filesystem on server |
| `s3://` | `s3://bucket/key` | AWS S3 object storage |
| `gs://` | `gs://bucket/key` | Google Cloud Storage |
| `http://` | `http://cdn.example.com/file.bin` | HTTP(S) URL |
| `data:` | `data:text/plain;base64,...` | Inline small content |

## Backend Implementation

### Struct

```rust
pub struct DatabaseBackend {
    pool: sqlx::Pool<sqlx::Sqlite>,  // Or Postgres, MySQL
    content_fetcher: Arc<dyn ContentFetcher>,
}
```

### Content Fetcher Trait

```rust
#[async_trait]
pub trait ContentFetcher: Send + Sync {
    async fn fetch(&self, url: &str, offset: u64, length: usize) -> Result<Vec<u8>, BackendError>;
}
```

### Implementation Approach

1. **get_attributes**: Single SELECT by path
2. **read_directory**: SELECT by parent_path with ORDER BY and LIMIT
3. **read_symlink**: SELECT symlink_target by path
4. **read_file**:
   - SELECT content_url from database
   - Use ContentFetcher to retrieve actual bytes
5. **statfs**: Aggregate query or fixed values

## File Structure

```
crates/fs123-server/src/backends/
├── mod.rs           # Add database module
├── traits.rs        # Existing Backend trait
├── types.rs         # Existing types
├── file.rs          # Existing FileBackend
└── database/
    ├── mod.rs       # DatabaseBackend implementation
    ├── schema.rs    # SQL schema and queries
    └── content.rs   # ContentFetcher implementations
```

## Configuration

Database URL format following sqlx conventions:

```bash
# SQLite
fs123-server --export-root "sqlite:///var/lib/fs123/metadata.db"

# PostgreSQL
fs123-server --export-root "postgres://user:pass@localhost/fs123"

# With content fetcher config
fs123-server --export-root "sqlite:///path/db?content_base=/var/content"
```

## Advantages

1. **Flexible content storage** - Mix local files, S3, HTTP sources
2. **Queryable metadata** - SQL queries for filesystem analysis
3. **Easy updates** - Modify metadata without touching content
4. **Replication** - Database replication for metadata HA
5. **Content deduplication** - Multiple paths can reference same content URL

## Limitations

1. **Two-phase reads** - Metadata lookup + content fetch
2. **Consistency** - Content URL must remain valid
3. **No extended attributes** - Would need separate table (future work)

## Future Extensions

- Extended attributes table
- ACL support
- Content checksums for integrity
- Automatic content URL scheme detection
- Write support (for non-read-only use cases)
