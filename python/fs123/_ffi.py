import ctypes
import os
import sys
from ctypes import (
    c_char,
    c_char_p,
    c_int,
    c_int64,
    c_size_t,
    c_ssize_t,
    c_uint8,
    c_uint32,
    c_uint64,
    c_void_p,
    POINTER,
)


class Fs123Stat(ctypes.Structure):
    _fields_ = [
        ("st_mode", c_uint32),
        ("st_nlink", c_uint64),
        ("st_uid", c_uint32),
        ("st_gid", c_uint32),
        ("st_size", c_int64),
        ("st_mtime", c_int64),
        ("st_ctime", c_int64),
        ("st_atime", c_int64),
        ("st_ino", c_uint64),
        ("st_mtime_nsec", c_int64),
        ("st_ctime_nsec", c_int64),
        ("st_atime_nsec", c_int64),
        ("st_dev", c_uint64),
        ("st_blocks", c_int64),
        ("st_blksize", c_int64),
        ("st_rdev", c_uint64),
    ]

    def __repr__(self):
        fields = ", ".join(f"{name}={getattr(self, name)}" for name, _ in self._fields_)
        return f"Fs123Stat({fields})"


class Fs123Dirent(ctypes.Structure):
    _fields_ = [
        ("name", c_char * 256),
        ("d_type", c_uint8),
    ]


def _load_library():
    if sys.platform == "darwin":
        libname = "libfs123.dylib"
    else:
        libname = "libfs123.so"

    package_dir = os.path.dirname(os.path.abspath(__file__))

    search_paths = [
        os.path.join(package_dir, libname),
        os.path.join(package_dir, "..", libname),
        os.path.join(package_dir, "..", "lib", libname),
    ]

    ld_path = os.environ.get("LD_LIBRARY_PATH", "")
    for p in ld_path.split(os.pathsep):
        if p:
            search_paths.append(os.path.join(p, libname))

    for path in search_paths:
        if os.path.exists(path):
            return ctypes.CDLL(path)

    from ctypes.util import find_library

    found = find_library("fs123")
    if found:
        return ctypes.CDLL(found)

    raise OSError(f"Cannot find {libname}. Ensure libfs123 is installed or set LD_LIBRARY_PATH.")


_lib = _load_library()

_lib.fs123_errno.argtypes = []
_lib.fs123_errno.restype = c_int

_lib.fs123_strerror.argtypes = []
_lib.fs123_strerror.restype = c_char_p

_lib.fs123_set_proto.argtypes = [c_char_p]
_lib.fs123_set_proto.restype = None

_lib.fs123_mount.argtypes = [c_char_p, c_char_p, c_char_p]
_lib.fs123_mount.restype = c_int

_lib.fs123_umount.argtypes = [c_char_p]
_lib.fs123_umount.restype = c_int

_lib.fs123_mountall.argtypes = []
_lib.fs123_mountall.restype = c_int

_lib.fs123_fsync.argtypes = [c_char_p]
_lib.fs123_fsync.restype = c_int

_lib.fs123_stat.argtypes = [c_char_p, POINTER(Fs123Stat)]
_lib.fs123_stat.restype = c_int

_lib.fs123_opendir.argtypes = [c_char_p]
_lib.fs123_opendir.restype = c_void_p

_lib.fs123_readdir.argtypes = [c_void_p, POINTER(Fs123Dirent)]
_lib.fs123_readdir.restype = c_int

_lib.fs123_closedir.argtypes = [c_void_p]
_lib.fs123_closedir.restype = None

_lib.fs123_open.argtypes = [c_char_p, c_char_p]
_lib.fs123_open.restype = c_void_p

_lib.fs123_read.argtypes = [c_void_p, c_void_p, c_size_t]
_lib.fs123_read.restype = c_ssize_t

_lib.fs123_seek.argtypes = [c_void_p, c_int64, c_int]
_lib.fs123_seek.restype = c_int64

_lib.fs123_close.argtypes = [c_void_p]
_lib.fs123_close.restype = c_int

_lib.fs123_readlink.argtypes = [c_char_p, c_char_p, c_size_t]
_lib.fs123_readlink.restype = c_ssize_t


def _check(rc, func_name=""):
    if rc == -1:
        errno_val = _lib.fs123_errno()
        msg = _lib.fs123_strerror()
        msg_str = msg.decode("utf-8", errors="replace") if msg else f"fs123 error {errno_val}"
        if func_name:
            msg_str = f"{func_name}: {msg_str}"
        raise OSError(errno_val, msg_str)
