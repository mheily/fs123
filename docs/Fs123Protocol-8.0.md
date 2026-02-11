# Fs123 Protocol 8.0

This document describes protocol version 8.0. For protocol 7.3, see
`legacy/docs/Fs123Protocol`. For 7.2, see `legacy/docs/Fs123Protocol-7.2`.

## Overview

Protocol 8.0 extends the fs123 protocol with three major changes:

1. **Descriptive function names** — Single-letter function codes (e.g., `/f`)
   are replaced with descriptive names matching FUSE/POSIX operations (e.g.,
   `/read`).

2. **JSON and binary response formats** — The netstring key-value encoding
   used in v7.3 is replaced. Structured metadata endpoints return JSON.
   Raw data endpoints return binary bodies with metadata in HTTP headers.

3. **Write operations** — The protocol is no longer read-only. Servers may
   optionally support metadata mutation operations and a multi-part file upload
   mechanism.

The URL structure, HTTP cache-control semantics, ESTALE cookies, and validators
all work as in v7.3. Read operations are semantically identical to their v7.3
counterparts.

## Changes from Protocol 7.3

### Function Name Mapping

| v7.3 | v8.0 | Operation |
|------|------|-----------|
| `/a` | `/stat` | File/directory attributes |
| `/d` | `/readdir` | Directory listing |
| `/f` | `/read` | File content read |
| `/l` | `/readlink` | Symbolic link target |
| `/s` | `/statvfs` | Filesystem statistics |
| `/x` | `/getxattr`, `/listxattr` | Extended attributes (split) |
| `/n` | `/n` | Server statistics (unchanged) |
| `/p` | `/p` | Passthrough (unchanged) |

The single combined `/x` endpoint is split into two distinct endpoints in v8:
- `/getxattr` — retrieve a single extended attribute value
- `/listxattr` — list all extended attribute names

### Response Format

v7.3 uses a single netstring key-value encoding for all responses. v8.0
replaces this with format-appropriate encodings:

**JSON responses** (`Content-Type: application/json`) are used for endpoints
that return structured metadata: `/stat`, `/readdir`, `/readlink`, `/statvfs`,
`/n`, and all write operations. The JSON object always contains an `errno`
field. On success (errno 0), additional fields carry the response data:

```json
{
  "errno": 0,
  "content": { ... },
  "validator": 12345,
  "estalecookie": 67890
}
```

On error, only `errno` is present:

```json
{"errno": 2}
```

**Binary responses** (`Content-Type: application/octet-stream`) are used for
endpoints that return raw byte data: `/read`, `/getxattr`, `/listxattr`. The
HTTP body contains the raw data with no framing. Metadata is conveyed in
custom HTTP headers:

| Header | Description |
|--------|-------------|
| `X-Fs123-Errno` | Error code (always present, "0" on success) |
| `X-Fs123-Validator` | 64-bit validator (present on success, when applicable) |
| `X-Fs123-Estalecookie` | 64-bit ESTALE cookie (present on success, when applicable) |

On error (`X-Fs123-Errno` non-zero), the body is empty.

### Cache-Control for Writes

All write operation responses are returned with `max-age=0` and no
`stale-while-revalidate` directive. Write results must never be cached.

## URL Structure

Unchanged from v7.3:

```
http://host[:port]/SEL/EC/TOR/fs123/8/0/FUNCTION/PA/TH?QUERY
```

Examples:

```
http://server:8080/fs123/8/0/stat/path/to/file
http://server:8080/fs123/8/0/read/path/to/file?128;0
http://server:8080/fs123/8/0/mkdir/new/directory?493
http://server:8080/fs123/8/0/create_upload/path/to/file?420
```

## Read Operations

These are semantically identical to their v7.3 counterparts, differing in
function name and response encoding.

### `/stat` (was `/a`)

```
QUERY = <empty>
Response format: JSON
```

Returns file attributes as a JSON object. The `content` field contains
`struct stat` fields with their standard names:

```json
{
  "errno": 0,
  "content": {
    "st_mode": 33188,
    "st_nlink": 1,
    "st_uid": 1000,
    "st_gid": 1000,
    "st_size": 1024,
    "st_mtim": 1700000000,
    "st_ctim": 1700000001,
    "st_atim": 1700000002,
    "st_ino": 12345,
    "st_mtim_nsec": 0,
    "st_ctim_nsec": 0,
    "st_atim_nsec": 0,
    "st_dev": 0,
    "st_blocks": 8,
    "st_blksize": 4096,
    "st_rdev": 0
  },
  "validator": 1700000000000000000,
  "estalecookie": 12345
}
```

Note: v7.3 used `st_mtime`/`st_ctime`/`st_atime` for the time fields; v8.0
uses `st_mtim`/`st_ctim`/`st_atim` (without the trailing 'e') to align with
the POSIX `struct stat` field names.

An `estalecookie` is required for a `/stat` reply that describes a regular
file or a directory (but not for a symbolic link).

### `/readdir` (was `/d`)

```
QUERY = Len;Start
Response format: JSON
```

Returns directory entries as a JSON object. Len is in kibibytes. Start is a
URL-encoded opaque cursor from a previous `nextstart` value, or empty for the
first request.

```json
{
  "errno": 0,
  "content": {
    "entries": [
      {"d_name": "file.txt", "d_type": 8, "estalecookie": 12345},
      {"d_name": "subdir", "d_type": 4, "estalecookie": 67890}
    ]
  },
  "estalecookie": 11111,
  "nextstart": ""
}
```

An empty `nextstart` indicates no more entries remain. A non-empty value is
an opaque cursor for the next request.

### `/read` (was `/f`)

```
QUERY = Len;Offset
Response format: Binary
```

Returns file content as raw bytes. Both Len and Offset are in kibibytes.

Response headers:
```
Content-Type: application/octet-stream
X-Fs123-Errno: 0
X-Fs123-Validator: 1700000000000000000
X-Fs123-Estalecookie: 12345
```

The body contains the raw file bytes. EOF is indicated by a short read
(content length less than requested Len).

### `/readlink` (was `/l`)

```
QUERY = <empty>
Response format: JSON
```

Returns the symlink target:

```json
{
  "errno": 0,
  "content": {"target": "/path/to/target"}
}
```

If the path is not a symbolic link, errno is set to EINVAL.

### `/statvfs` (was `/s`)

```
QUERY = <empty>
Response format: JSON
```

Returns filesystem statistics as a JSON object:

```json
{
  "errno": 0,
  "content": {
    "f_bsize": 4096,
    "f_frsize": 4096,
    "f_blocks": 1000000,
    "f_bfree": 500000,
    "f_bavail": 400000,
    "f_files": 100000,
    "f_ffree": 50000,
    "f_favail": 40000,
    "f_fsid": 12345,
    "f_flag": 1,
    "f_namemax": 255
  }
}
```

### `/getxattr` (was `/x` with non-empty Name)

```
QUERY = Len;Name;
Response format: Binary
```

Returns the value of extended attribute `Name` on `/PA/TH` as raw bytes in
the body. Len is in kibibytes. Name is URL-encoded.

Response headers:
```
Content-Type: application/octet-stream
X-Fs123-Errno: 0
```

### `/listxattr` (was `/x` with empty Name)

```
QUERY = Len;
Response format: Binary
```

Returns all NUL-terminated attribute names as raw bytes. Returns `ERANGE` if
the list exceeds Len kibibytes.

### `/n` — Server statistics

```
QUERY = <empty>
Response format: JSON
```

Returns server information:

```json
{
  "errno": 0,
  "content": {
    "version": "0.1.0",
    "backend": "FileBackend(/srv/data)",
    "max_age": 300,
    "stale_while_revalidate": 60
  }
}
```

### `/p` — Passthrough

Unchanged from v7.3. Semantics are server-defined. Servers that do not
support passthrough return 501.

## Write Operations (New in v8.0)

Write support is optional. A server that does not support writes returns
`errno=EROFS` (Read-only file system) for all write operations.

Servers advertise write support via the `--writable` configuration flag.

**HTTP Method**: All requests, including writes, use HTTP GET. The request
body carries data for operations that require it (e.g., `/upload_part`,
`/setxattr`).

### Metadata Operations

These operations modify filesystem metadata without writing file content.
All metadata operations return JSON responses with `max-age=0`.

#### `/mkdir` — Create directory

```
QUERY = mode
```

`mode` is the Unix permission bits as a decimal integer (e.g., 493 for 0755).

#### `/rmdir` — Remove directory

```
QUERY = <empty>
```

The directory must be empty.

#### `/chmod` — Change permissions

```
QUERY = mode
```

#### `/chown` — Change ownership

```
QUERY = uid;gid
```

Both `uid` and `gid` are decimal integers.

#### `/utimens` — Change timestamps

```
QUERY = atime_sec;atime_nsec;mtime_sec;mtime_nsec
```

All values are decimal integers. Sets both access time and modification time
with nanosecond precision.

#### `/symlink` — Create symbolic link

```
URL path = the link path to create
QUERY = target
```

`target` is URL-encoded. The `/PA/TH` in the URL is the path where the
symlink is created; the query parameter is the link target.

#### `/link` — Create hard link

```
URL path = the existing file (oldpath)
QUERY = newpath
```

`newpath` is URL-encoded.

#### `/unlink` — Remove file or link

```
QUERY = <empty>
```

#### `/rename` — Rename/move

```
URL path = the existing path (oldpath)
QUERY = newpath
```

`newpath` is URL-encoded.

#### `/access` — Check access permissions

```
QUERY = mask
```

`mask` is the access mode flags as a decimal integer (combination of R_OK,
W_OK, X_OK, F_OK).

#### `/setxattr` — Set extended attribute

```
QUERY = name;flags
Body: attribute value (binary)
```

`name` is the attribute name. `flags` is a decimal integer (XATTR_CREATE,
XATTR_REPLACE, or 0).

#### `/removexattr` — Remove extended attribute

```
QUERY = name
```

### Write-Once File Upload

v8.0 provides file creation through a multi-part upload mechanism inspired by
object-store upload APIs (e.g., S3 multipart upload). This design avoids
POSIX random-access write semantics, making it compatible with backends that
do not support `seek`/`write`.

Files are created atomically: they do not appear at their final path until the
upload is completed. If a file already exists at the target path, the upload
is rejected with `EEXIST`.

Upload operations return JSON responses.

#### Upload Flow

```
1. create_upload   →  receive upload_id (JSON)
2. upload_part (part 0, 1, 2, ...)  →  append data (JSON)
3. complete_upload  →  atomically publish file (JSON)
   (or abort_upload to cancel)
```

#### `/create_upload` — Initialize upload

```
QUERY = mode
Reply: {"errno": 0, "upload_id": "<uuid>"}
```

`mode` is the file permission bits as a decimal integer.

Creates a temporary file in a server-side staging area (`.fs123_uploads/`).
Returns a UUID identifying the upload session. Fails with `EEXIST` if a file
already exists at `/PA/TH`.

#### `/upload_part` — Upload a chunk

```
QUERY = upload_id;part_number
Body: binary data
Reply: {"errno": 0}
```

`upload_id` is the UUID from `create_upload`. `part_number` is a zero-based
sequential integer. Parts must be uploaded in order (0, 1, 2, ...). Each
part's data is appended to the temporary file.

#### `/complete_upload` — Finalize upload

```
QUERY = upload_id
Reply: {"errno": 0}
```

Sets the file mode to the value specified in `create_upload` and atomically
renames the temporary file to the final `/PA/TH`. The upload session is
removed.

#### `/abort_upload` — Cancel upload

```
QUERY = upload_id
Reply: {"errno": 0}
```

Deletes the temporary file and removes the upload session.

#### Upload Session Management

- Upload sessions are identified by UUID and tracked server-side.
- Stale sessions (those not completed or aborted within a timeout period) may
  be reaped by the server.
- Temporary files are stored in a `.fs123_uploads/` directory managed by the
  backend.

## Error Handling

Error handling follows v7.3 conventions:

- **5xx**: Internal server errors (client sees EIO)
- **4xx**: Protocol errors (client sees EIO)
- **200 with non-zero errno**: Filesystem-level errors

For JSON responses, errno is in the JSON body. For binary responses, errno
is in the `X-Fs123-Errno` header.

Common errno values for write operations:

| errno | Meaning |
|-------|---------|
| EROFS | Server does not support writes |
| EEXIST | File already exists (create_upload, mkdir) |
| ENOENT | Path does not exist |
| EPERM | Permission denied |
| ENOSYS | Operation not implemented by backend |
| ENOTEMPTY | Directory not empty (rmdir) |
| EINVAL | Invalid argument (bad upload_id, wrong part_number) |

## Comparison Summary

| Aspect | v7.3 | v8.0 |
|--------|------|------|
| Function names | Single letter (`/a`, `/d`, `/f`, `/l`, `/s`, `/x`) | Descriptive (`/stat`, `/readdir`, `/read`, `/readlink`, `/statvfs`, `/getxattr`, `/listxattr`) |
| xattr endpoints | Combined `/x` | Split `/getxattr` + `/listxattr` |
| Structured responses | Netstring key-value pairs | JSON (`application/json`) |
| Raw data responses | Netstring-wrapped content | Binary body + `X-Fs123-*` headers |
| Stat field names | Positional space-separated integers | Named JSON fields (`st_mode`, `st_nlink`, ...) |
| Time field names | `st_mtime`, `st_ctime`, `st_atime` | `st_mtim`, `st_ctim`, `st_atim` |
| Directory entries | Netstring name + space-separated fields per line | JSON array of objects |
| Readlink content | Raw target string in netstring | JSON `{"target": "..."}` |
| Statfs content | Space-separated integers | Named JSON fields (`f_bsize`, `f_blocks`, ...) |
| Server stats (`/n`) | Free-form text (YAML recommended) | JSON object |
| Write support | None (read-only) | Metadata operations + multi-part upload |
| Write caching | N/A | `max-age=0`, never cached |
| Backend trait | `Backend` (read-only) | `Backend` + optional `WritableBackend` |
| Server flag | — | `--writable` enables write operations |
