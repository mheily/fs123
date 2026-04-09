/*
 * libfs123 — POSIX-like filesystem access over the fs123 HTTP protocol.
 *
 * Link with -lfs123 (shared: libfs123.so / libfs123.dylib,
 *                     static: libfs123.a).
 *
 * Usage:
 *
 *   // Mount one or more fs123 servers into a virtual namespace.
 *   fs123_mount("http://server1:8080/exports/data", "/mnt/data", NULL);
 *   fs123_mount("http://server2:8080", "/mnt/logs", "cache_ttl_secs=60");
 *
 *   // Use local-looking paths with every other call.
 *   fs123_stat_t sb;
 *   fs123_stat("/mnt/data/file.txt", &sb);
 *
 *   void *fh = fs123_open("/mnt/logs/app.log", "r");
 *   char buf[4096];
 *   ssize_t n = fs123_read(fh, buf, sizeof(buf));
 *   fs123_close(fh);
 *
 *   // Unmount when done.
 *   fs123_umount("/mnt/data");
 *   fs123_umount("/mnt/logs");
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
 * Affects subsequent fs123_mount() calls.  Thread-local.
 * Defaults to "7.3" if never called.
 */
void fs123_set_proto(const char *proto);

/* ── Mount / Unmount ───────────────────────────────────────────── */

/*
 * fs123_mount — mount an fs123 server at a local path prefix.
 *
 * url:        server address, e.g. "http://server:8080/exports/data".
 *             The path component (if any) is used as a selector prefix.
 * mountpoint: absolute path prefix, e.g. "/mnt/data".
 * options:    comma-separated key=value pairs, or NULL for defaults.
 *             Recognized keys:
 *               cache_ttl_secs=N      entry lifetime in seconds (default 30)
 *               cache_max_entries=N   max entries per cache map (default 10000)
 *               mirror=true|false     download files to local disk (default false)
 *
 * Returns 0 on success, -1 on error.
 * Replaces any existing mount at the same mountpoint.
 */
int fs123_mount(const char *url, const char *mountpoint, const char *options);

/*
 * fs123_umount — unmount a previously mounted path.
 * Returns 0 on success, -1 if the mountpoint is not found.
 */
int fs123_umount(const char *mountpoint);

/*
 * fs123_mountall — read (or re-read) the fs123 fstab files and mount
 * every entry found.
 *
 * Two files are consulted, in order:
 *   1. /etc/fs123/fstab                    — system-wide defaults
 *   2. $XDG_CONFIG_HOME/fs123/fstab        — per-user overrides
 *      (falls back to ~/.config/fs123/fstab when XDG_CONFIG_HOME is unset)
 *
 * The per-user file is processed after the system file, so entries
 * at the same mountpoint silently replace the system-wide entry.
 *
 * The file format has three whitespace-separated columns per line:
 *
 *   # url                              mountpoint   options
 *   http://server1:8080/exports/data   /mnt/data    cache_ttl_secs=60
 *   http://server2:8080                /mnt/logs    -
 *
 * Blank lines and lines starting with '#' are ignored.
 * The options column may be omitted or set to "-" or "none".
 *
 * May be called multiple times to pick up configuration changes.
 * Explicit fs123_mount() calls override fstab entries at the same
 * mountpoint.
 *
 * Returns 0.
 */
int fs123_mountall(void);

/* ── Per-file sync ────────────────────────────────────────────── */

/*
 * fs123_fsync — download a single remote file to local disk.
 *
 * Works regardless of whether mirror=true is set on the mount.
 * The file is stored at the local path under the mount point
 * (e.g. /mnt/data/file.txt).  Parent directories are created as
 * needed using the current umask.
 *
 * If the file already exists locally it is not re-downloaded.
 *
 * Returns 0 on success, -1 on error.
 */
int fs123_fsync(const char *path);

/* ── Stat ──────────────────────────────────────────────────────── */

/*
 * fs123_stat — get file attributes for a path in the mount namespace.
 * Returns 0 on success, -1 on error.
 */
int fs123_stat(const char *path, fs123_stat_t *buf);

/* ── Directory operations ──────────────────────────────────────── */

/*
 * fs123_opendir — open a directory for reading.
 * Returns an opaque handle, or NULL on error.
 * The full listing is fetched eagerly.
 */
void *fs123_opendir(const char *path);

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
void *fs123_open(const char *path, const char *mode);

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
ssize_t fs123_readlink(const char *path, char *buf, size_t bufsiz);

#ifdef __cplusplus
}
#endif

#endif /* FS123_H */
