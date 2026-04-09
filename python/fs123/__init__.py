import os

from ._ffi import Fs123Stat, _check, _lib
from ._path import Fs123File, Path


def mount(url, mountpoint, options=None):
    url_b = url.encode("utf-8")
    mp_b = mountpoint.encode("utf-8")
    opt_b = options.encode("utf-8") if options else None
    _check(_lib.fs123_mount(url_b, mp_b, opt_b), "mount")


def umount(mountpoint):
    _check(_lib.fs123_umount(mountpoint.encode("utf-8")), "umount")


def mountall():
    _check(_lib.fs123_mountall(), "mountall")


def fsync(path):
    _check(_lib.fs123_fsync(os.fspath(path).encode("utf-8")), "fsync")


def set_proto(proto):
    _lib.fs123_set_proto(proto.encode("utf-8"))


def errno():
    return _lib.fs123_errno()


def strerror():
    msg = _lib.fs123_strerror()
    return msg.decode("utf-8", errors="replace") if msg else None


__all__ = [
    "Path",
    "Fs123File",
    "Fs123Stat",
    "mount",
    "umount",
    "mountall",
    "fsync",
    "set_proto",
    "errno",
    "strerror",
]
