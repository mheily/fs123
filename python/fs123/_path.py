import ctypes
import fnmatch
import os
import pathlib

from ._ffi import (
    Fs123Dirent,
    Fs123Stat,
    _check,
    _lib,
)

_S_IFLNK = 0o120000
_S_IFMT = 0o170000
_BUF_SIZE = 64 * 1024


class Path(pathlib.PurePosixPath):
    def __new__(cls, *parts):
        normalized_parts = []
        for p in parts:
            s = os.fspath(p)
            normalized_parts.append(s)
        return super().__new__(cls, *normalized_parts)

    def stat(self):
        buf = Fs123Stat()
        _check(_lib.fs123_stat(str(self).encode("utf-8"), ctypes.byref(buf)), "stat")
        return buf

    def exists(self):
        try:
            self.stat()
            return True
        except OSError:
            return False

    def is_file(self):
        try:
            st = self.stat()
            return (st.st_mode & _S_IFMT) == 0o100000
        except OSError:
            return False

    def is_dir(self):
        try:
            st = self.stat()
            return (st.st_mode & _S_IFMT) == 0o040000
        except OSError:
            return False

    def is_symlink(self):
        try:
            st = self.stat()
            return (st.st_mode & _S_IFMT) == _S_IFLNK
        except OSError:
            return False

    def readlink(self):
        buf = ctypes.create_string_buffer(4096)
        rc = _lib.fs123_readlink(str(self).encode("utf-8"), buf, ctypes.sizeof(buf))
        _check(rc, "readlink")
        return Path(buf.value.decode("utf-8"))

    def iterdir(self):
        dp = _lib.fs123_opendir(str(self).encode("utf-8"))
        _check(0 if dp else -1, "opendir")
        try:
            entries = []
            ent = Fs123Dirent()
            while True:
                rc = _lib.fs123_readdir(dp, ctypes.byref(ent))
                if rc == 0:
                    break
                if rc == -1:
                    _check(rc, "readdir")
                name = ent.name.decode("utf-8")
                if name not in (".", ".."):
                    entries.append(name)
        finally:
            _lib.fs123_closedir(dp)
        for name in entries:
            yield self / name

    def glob(self, pattern):
        for child in self.iterdir():
            if fnmatch.fnmatch(child.name, pattern):
                yield child

    def read_bytes(self):
        chunks = []
        with self.open("rb") as f:
            while True:
                data = f.read(_BUF_SIZE)
                if not data:
                    break
                chunks.append(data)
        return b"".join(chunks)

    def read_text(self, encoding="utf-8"):
        return self.read_bytes().decode(encoding)

    def open(self, mode="r"):
        return Fs123File(str(self), mode)

    def fsync(self):
        _check(_lib.fs123_fsync(str(self).encode("utf-8")), "fsync")

    def __repr__(self):
        return f"fs123.Path('{self}')"


class Fs123File:
    def __init__(self, path, mode="r"):
        self._path = path
        self._mode = mode
        self._binary = "b" in mode
        self._handle = _lib.fs123_open(path.encode("utf-8"), b"rb")
        _check(0 if self._handle else -1, "open")
        self._closed = False
        self._line_buf = b""

    @property
    def closed(self):
        return self._closed

    @property
    def mode(self):
        return self._mode

    @property
    def name(self):
        return self._path

    def readable(self):
        return True

    def writable(self):
        return False

    def seekable(self):
        return True

    def read(self, size=-1):
        if self._closed:
            raise ValueError("I/O operation on closed file")
        if size < 0:
            chunks = []
            if self._line_buf:
                chunks.append(self._line_buf)
                self._line_buf = b""
            while True:
                buf = ctypes.create_string_buffer(_BUF_SIZE)
                n = _lib.fs123_read(self._handle, buf, _BUF_SIZE)
                if n < 0:
                    _check(-1, "read")
                if n == 0:
                    break
                chunks.append(buf.raw[:n])
            data = b"".join(chunks)
        else:
            parts = []
            remaining = size
            if self._line_buf:
                take = min(len(self._line_buf), remaining)
                parts.append(self._line_buf[:take])
                self._line_buf = self._line_buf[take:]
                remaining -= take
            if remaining > 0:
                buf = ctypes.create_string_buffer(remaining)
                n = _lib.fs123_read(self._handle, buf, remaining)
                if n < 0:
                    _check(-1, "read")
                parts.append(buf.raw[:n])
            data = b"".join(parts)
        if self._binary:
            return data
        return data.decode("utf-8")

    def readline(self):
        if self._binary:
            raise OSError("readline not available in binary mode")
        while True:
            nl = self._line_buf.find(b"\n")
            if nl >= 0:
                line = self._line_buf[:nl + 1]
                self._line_buf = self._line_buf[nl + 1:]
                return line.decode("utf-8")
            buf = ctypes.create_string_buffer(_BUF_SIZE)
            n = _lib.fs123_read(self._handle, buf, _BUF_SIZE)
            if n < 0:
                _check(-1, "read")
            if n == 0:
                if self._line_buf:
                    line = self._line_buf
                    self._line_buf = b""
                    return line.decode("utf-8")
                return ""
            self._line_buf += buf.raw[:n]

    def readlines(self):
        return list(self)

    def seek(self, offset, whence=0):
        if self._closed:
            raise ValueError("I/O operation on closed file")
        self._line_buf = b""
        rc = _lib.fs123_seek(self._handle, ctypes.c_int64(offset), ctypes.c_int(whence))
        _check(rc, "seek")
        return rc

    def tell(self):
        return self.seek(0, 1)

    def close(self):
        if not self._closed:
            _lib.fs123_close(self._handle)
            self._closed = True

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.close()
        return False

    def __iter__(self):
        return self

    def __next__(self):
        line = self.readline()
        if not line:
            raise StopIteration
        return line

    def __del__(self):
        handle = getattr(self, "_handle", None)
        closed = getattr(self, "_closed", True)
        if not closed and handle:
            try:
                _lib.fs123_close(handle)
            except Exception:
                pass
            self._closed = True
