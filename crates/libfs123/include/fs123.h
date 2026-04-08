/*
 * libfs123 — POSIX-like filesystem access over the fs123 HTTP protocol.
 *
 * Link with -lfs123 (shared: libfs123.so / libfs123.dylib,
 *                     static: libfs123.a).
 *
 * All functions that accept a URL expect the form:
 *
 *     http://host[:port]/path
 *
 * where /path is the filesystem path on the fs123 server.
 */

#ifndef FS123_H
#define FS123_H

#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Types ─────────────────────────────────────────────────────── */

typedef struct {
    uint32_t st_mode;
    uint64_t st_nlink;
    uint32_t st_uid;
    uint32_t st_gid;
    int64_t  st_size;
    int64_t  st_mtime;
    int64_t  st_ctime;
    int64_t  st_atime;
    uint64_t st_ino;
    int64_t  st_mtime_nsec;
    int64_t  st_ctime_nsec;
    int64_t  st_atime_nsec;
    uint64_t st_dev;
    int64_t  st_blocks;
    int64_t  st_blksize;
    uint64_t st_rdev;
} fs123_stat_t;

typedef struct {
    char    name[256];   /* null-terminated, max 255 chars + NUL */
    uint8_t d_type;
} fs123_dirent_t;

/* d_type constants (same values as <dirent.h>) */
#define FS123_DT_UNKNOWN   0
#define FS123_DT_FIFO      1
#define FS123_DT_CHR       2
#define FS123_DT_DIR       4
#define FS123_DT_BLK       6
#define FS123_DT_REG       8
#define FS123_DT_LNK      10
#define FS123_DT_SOCK     12

/* ── Error handling ────────────────────────────────────────────── */

/*
 * fs123_errno — return the errno from the last failed call (thread-local).
 * Returns 0 if the last call succeeded.
 */
int fs123_errno(void);

/*
 * fs123_strerror — return a human-readable error message (thread-local).
 * Returns NULL if the last call succeeded.
 * The pointer is valid until the next libfs123 call on the same thread.
 */
const char *fs123_strerror(void);

/* ── Configuration ─────────────────────────────────────────────── */

/*
 * fs123_set_proto — set the default fs123 protocol version.
 * Example: fs123_set_proto("7.3") or fs123_set_proto("8.0").
 * Defaults to "7.3" if never called.  Thread-local.
 */
void fs123_set_proto(const char *proto);

/* ── Stat ──────────────────────────────────────────────────────── */

/*
 * fs123_stat — get file attributes.
 * Returns 0 on success, -1 on error.
 */
int fs123_stat(const char *url, fs123_stat_t *buf);

/* ── Directory operations ──────────────────────────────────────── */

/*
 * fs123_opendir — open a directory for reading.
 * Returns an opaque handle, or NULL on error.
 * The full listing is fetched eagerly.
 */
void *fs123_opendir(const char *url);

/*
 * fs123_readdir — read the next directory entry.
 * Returns 1 if an entry was stored, 0 when done, -1 on error.
 */
int fs123_readdir(void *dir, fs123_dirent_t *entry);

/*
 * fs123_closedir — close a directory handle.
 */
void fs123_closedir(void *dir);

/* ── File operations ───────────────────────────────────────────── */

/*
 * fs123_open — open a file for reading.
 * mode must start with 'r' (e.g. "r", "rb"); write modes are rejected.
 * Returns an opaque handle, or NULL on error.
 */
void *fs123_open(const char *url, const char *mode);

/*
 * fs123_read — read bytes from an open file.
 * Returns bytes read, 0 at EOF, -1 on error.
 */
ssize_t fs123_read(void *file, void *buf, size_t count);

/*
 * fs123_seek — reposition read offset.
 * whence: 0 = SEEK_SET, 1 = SEEK_CUR, 2 = SEEK_END.
 * Returns the new offset, or -1 on error.
 */
int64_t fs123_seek(void *file, int64_t offset, int whence);

/*
 * fs123_close — close a file handle.
 * Returns 0.
 */
int fs123_close(void *file);

/* ── Symlinks ──────────────────────────────────────────────────── */

/*
 * fs123_readlink — read the target of a symbolic link.
 * Writes a null-terminated string to buf (at most bufsiz-1 chars + NUL).
 * Returns bytes written (excl. NUL), or -1 on error.
 */
ssize_t fs123_readlink(const char *url, char *buf, size_t bufsiz);

#ifdef __cplusplus
}
#endif

#endif /* FS123_H */
