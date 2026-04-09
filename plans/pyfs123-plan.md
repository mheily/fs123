# Plan: Python FFI wrapper for libfs123

## Context

Create a Python package `fs123` that wraps the libfs123 C shared library via `ctypes`. The package provides a `pathlib.Path`-like `fs123.Path` class for accessing remote fs123 filesystems, plus module-level `mount()`, `umount()`, `mountall()`, and `fsync()` functions.

## Files to create

```
python/
  fs123/
    __init__.py      # Re-exports: Path, mount, umount, mountall, fsync, set_proto, errno, strerror
    _ffi.py          # ctypes bindings: struct definitions, function prototypes, library loading
    _path.py         # fs123.Path class
  pyproject.toml     # Package metadata
  tests/
    test_fs123.py    # Unit tests
```

## Example usage

```python
import fs123

# Option A: mount from fstab files
fs123.mountall()

# Option B: explicit mount
fs123.mount("http://server:8080/exports", "/mnt/data")
fs123.mount("http://server:8080/logs", "/mnt/logs", "cache_ttl_secs=120")

# pathlib-style file access
p = fs123.Path("/mnt/data/config.json")

if p.exists():
    print(p.stat().st_size)
    text = p.read_text()
    data = p.read_bytes()

# Iteration
for child in fs123.Path("/mnt/data").iterdir():
    print(child.name, "dir" if child.is_dir() else "file")

# Symlinks
link = fs123.Path("/mnt/data/latest")
if link.is_symlink():
    print(link.readlink())

# File-like reading
with p.open("r") as f:
    for line in f:
        print(line, end="")

with p.open("rb") as f:
    header = f.read(64)
    f.seek(0, 2)  # SEEK_END
    size = f.tell()

# Per-file mirroring
fs123.fsync("/mnt/data/large_model.bin")

# Unmount
fs123.umount("/mnt/data")
```

---

## Module: `fs123._ffi`

Low-level ctypes bindings. Not part of the public API.

### Library loading

```python
_lib = ctypes.CDLL("libfs123.so")  # or .dylib on macOS
```

Search order: `LD_LIBRARY_PATH`, alongside the package, then system paths. Use `ctypes.util.find_library("fs123")` as fallback.

### C struct definitions

**`Fs123Stat`** (`ctypes.Structure`) -- mirrors `fs123_stat_t`:
- Fields: `st_mode`, `st_nlink`, `st_uid`, `st_gid`, `st_size`, `st_mtime`, `st_ctime`, `st_atime`, `st_ino`, `st_mtime_nsec`, `st_ctime_nsec`, `st_atime_nsec`, `st_dev`, `st_blocks`, `st_blksize`, `st_rdev`

**`Fs123Dirent`** (`ctypes.Structure`) -- mirrors `fs123_dirent_t`:
- Fields: `name` (c_char * 256), `d_type` (c_uint8)

### Function prototypes

All 16 C functions get `argtypes` and `restype` set:

| C function | argtypes | restype |
|---|---|---|
| `fs123_errno` | `()` | `c_int` |
| `fs123_strerror` | `()` | `c_char_p` |
| `fs123_set_proto` | `(c_char_p,)` | `None` |
| `fs123_mount` | `(c_char_p, c_char_p, c_char_p)` | `c_int` |
| `fs123_umount` | `(c_char_p,)` | `c_int` |
| `fs123_mountall` | `()` | `c_int` |
| `fs123_fsync` | `(c_char_p,)` | `c_int` |
| `fs123_stat` | `(c_char_p, POINTER(Fs123Stat))` | `c_int` |
| `fs123_opendir` | `(c_char_p,)` | `c_void_p` |
| `fs123_readdir` | `(c_void_p, POINTER(Fs123Dirent))` | `c_int` |
| `fs123_closedir` | `(c_void_p,)` | `None` |
| `fs123_open` | `(c_char_p, c_char_p)` | `c_void_p` |
| `fs123_read` | `(c_void_p, c_void_p, c_size_t)` | `c_ssize_t` |
| `fs123_seek` | `(c_void_p, c_int64, c_int)` | `c_int64` |
| `fs123_close` | `(c_void_p,)` | `c_int` |
| `fs123_readlink` | `(c_char_p, c_char_p, c_size_t)` | `c_ssize_t` |

### Helper

```python
def _check(rc, func_name=""):
    """Raise OSError if rc == -1, using fs123_errno/fs123_strerror."""
```

---

## Module: `fs123.__init__`

Public module-level functions. All string arguments are encoded to UTF-8 bytes for the C layer.

### `fs123.mount(url: str, mountpoint: str, options: str | None = None) -> None`

Mount an fs123 server. Raises `OSError` on failure.

### `fs123.umount(mountpoint: str) -> None`

Unmount. Raises `OSError` if not mounted.

### `fs123.mountall() -> None`

Read `/etc/fs123/fstab` and `$XDG_CONFIG_HOME/fs123/fstab`, mount all entries.

### `fs123.fsync(path: str | os.PathLike) -> None`

Download a single remote file to local disk (per-file mirror). Raises `OSError` on failure.

### `fs123.set_proto(proto: str) -> None`

Set the default fs123 protocol version (e.g. `"7.3"`).

### `fs123.errno() -> int`

Return the errno from the last failed call (thread-local).

### `fs123.strerror() -> str | None`

Return the error message from the last failed call, or `None`.

---

## Module: `fs123._path`

### Class: `fs123.Path`

A `pathlib.PurePosixPath`-like object that performs filesystem operations through the libfs123 C library. Immutable; all operations that produce a new path return a new `Path` instance.

#### Constructor

**`Path(*parts: str | os.PathLike)`**

Construct from one or more path segments, joined with `/`. Normalizes the result (collapses `//`, resolves `.`).

```python
p = fs123.Path("/mnt/data", "subdir", "file.txt")
# => Path('/mnt/data/subdir/file.txt')
```

#### Path properties (pure, no I/O)

| Property | Type | Description |
|---|---|---|
| `name` | `str` | Final component (`"file.txt"`) |
| `stem` | `str` | Name without suffix (`"file"`) |
| `suffix` | `str` | File extension (`".txt"`) |
| `suffixes` | `list[str]` | All extensions (`[".tar", ".gz"]`) |
| `parent` | `Path` | Parent directory |
| `parents` | sequence of `Path` | Ancestors up to root |
| `parts` | `tuple[str, ...]` | Path components |
| `anchor` | `str` | Root (`"/"`) |

#### Path manipulation methods (pure, no I/O)

| Method | Returns | Description |
|---|---|---|
| `joinpath(*other)` | `Path` | Join path segments |
| `__truediv__(other)` | `Path` | `/` operator -- `p / "child"` |
| `with_name(name)` | `Path` | Replace final component |
| `with_stem(stem)` | `Path` | Replace stem, keep suffix |
| `with_suffix(suffix)` | `Path` | Replace suffix |
| `is_absolute()` | `bool` | Starts with `/` |
| `match(pattern)` | `bool` | Glob-style match |
| `__str__()` | `str` | String representation |
| `__repr__()` | `str` | `fs123.Path('/mnt/data/file')` |
| `__fspath__()` | `str` | For `os.fspath()` compatibility |
| `__eq__`, `__hash__` | | Path equality and hashing |

#### Filesystem query methods (perform I/O)

| Method | Returns | Description |
|---|---|---|
| `stat()` | `Fs123Stat` | Get file attributes |
| `exists()` | `bool` | True if path exists (stat succeeds) |
| `is_file()` | `bool` | True if regular file |
| `is_dir()` | `bool` | True if directory |
| `is_symlink()` | `bool` | True if symlink (checks `st_mode`) |
| `readlink()` | `Path` | Read symlink target |
| `iterdir()` | `Iterator[Path]` | Yield children of a directory |
| `glob(pattern)` | `Iterator[Path]` | Yield matching children (single level, e.g. `"*.txt"`) |

#### File content methods (perform I/O)

| Method | Returns | Description |
|---|---|---|
| `read_bytes()` | `bytes` | Read entire file as bytes |
| `read_text(encoding="utf-8")` | `str` | Read entire file as string |
| `open(mode="r")` | `Fs123File` | Open file, return file-like object |

#### Mirror method

| Method | Returns | Description |
|---|---|---|
| `fsync()` | `None` | Download this file to local disk (calls `fs123_fsync`) |

---

### Class: `Fs123File`

Returned by `Path.open()`. Implements the context manager protocol and the read side of the file-like interface.

| Method | Description |
|---|---|
| `read(size=-1)` | Read up to `size` bytes (or all if -1). Returns `bytes` in binary mode, `str` in text mode. |
| `readline()` | Read one line (text mode only). |
| `readlines()` | Read all lines (text mode only). |
| `seek(offset, whence=0)` | Reposition. Returns new position. |
| `tell()` | Return current position. |
| `close()` | Close the handle. |
| `__enter__` / `__exit__` | Context manager support. |
| `__iter__` / `__next__` | Iterate over lines (text mode). |
| `readable()` | Returns `True`. |
| `writable()` | Returns `False`. |
| `seekable()` | Returns `True`. |
| `closed` | Property: `True` after `close()`. |
| `mode` | Property: the mode string. |
| `name` | Property: the path string. |

**Text vs binary mode:**
- `"r"` / `"rt"` -- text mode: `read()` returns `str`, line iteration works
- `"rb"` -- binary mode: `read()` returns `bytes`

---

### Class: `Fs123Stat`

The ctypes Structure returned by `Path.stat()`. Has all the `st_*` fields listed in the `_ffi` section above. Additionally provides a human-friendly `__repr__`.

---

## Implementation notes

- **pathlib.PurePosixPath as base**: `fs123.Path` subclasses `pathlib.PurePosixPath` (Python 3.12+ allows this; for older versions, compose over it). This gives us all pure path operations for free.
- **String encoding**: All path strings are encoded to UTF-8 `bytes` before passing to C. Results are decoded from UTF-8.
- **Error handling**: Every C call that can fail is followed by `_check()` which reads `fs123_errno()`/`fs123_strerror()` and raises `OSError(errno, message)`.
- **Resource cleanup**: `Fs123File` calls `fs123_close()` on `__del__` as a safety net, but the primary cleanup path is `close()` / `__exit__`.
- **`iterdir()` implementation**: Calls `fs123_opendir`, loops `fs123_readdir` collecting all entries, calls `fs123_closedir`, then yields `Path` objects. The C layer eagerly fetches the full listing anyway, so there's no benefit to lazy yielding across the C boundary.
- **`glob()`**: Implemented in Python using `fnmatch` over `iterdir()` results (single-level only -- matching pathlib's non-recursive glob behavior).
- **`read_bytes()`**: Opens file, reads in a loop until EOF, closes. Uses 64 KiB buffer.
- **`is_symlink()`**: Uses `stat().st_mode & 0o170000 == 0o120000`.

## Tests

Unit tests in `python/tests/test_fs123.py`:

1. **Pure path operations**: `Path` construction, `/` operator, `name`, `stem`, `suffix`, `parent`, `parts`, `with_name`, `with_suffix`, `is_absolute`, `match`, `__fspath__`, `__eq__`, `__hash__`
2. **FFI struct layout**: Verify `Fs123Stat` and `Fs123Dirent` field offsets match C definitions
3. **Mount/umount round-trip**: mount, verify no error, umount (requires no actual server -- just tests the C call succeeds/fails as expected)
4. **Error handling**: Verify `OSError` raised with correct errno for operations on unmounted paths
5. **Fs123File interface**: `readable()`, `writable()`, `seekable()`, `closed`, context manager protocol

## Verification

```bash
cd python
pip install -e .
python -m pytest tests/
```
