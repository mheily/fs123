import ctypes
import os
import sys
from unittest import mock

import pytest

try:
    from fs123._ffi import Fs123Dirent, Fs123Stat, _check, _lib
    from fs123._path import Fs123File, Path
except OSError:
    pytestmark = pytest.mark.skip(reason="libfs123 not available")
    raise


class TestPurePathOperations:
    def test_construction_single(self):
        p = Path("/mnt/data/file.txt")
        assert str(p) == "/mnt/data/file.txt"

    def test_construction_multi(self):
        p = Path("/mnt/data", "subdir", "file.txt")
        assert str(p) == "/mnt/data/subdir/file.txt"

    def test_truediv(self):
        p = Path("/mnt/data") / "child"
        assert str(p) == "/mnt/data/child"

    def test_truediv_chained(self):
        p = Path("/mnt/data") / "sub" / "file.txt"
        assert str(p) == "/mnt/data/sub/file.txt"

    def test_name(self):
        assert Path("/mnt/data/file.txt").name == "file.txt"

    def test_stem(self):
        assert Path("/mnt/data/file.txt").stem == "file"

    def test_suffix(self):
        assert Path("/mnt/data/file.txt").suffix == ".txt"

    def test_suffixes_multi(self):
        assert Path("/mnt/data/archive.tar.gz").suffixes == [".tar", ".gz"]

    def test_parent(self):
        assert Path("/mnt/data/file.txt").parent == Path("/mnt/data")

    def test_parents(self):
        p = Path("/mnt/data/sub/file.txt")
        assert list(p.parents) == [
            Path("/mnt/data/sub"),
            Path("/mnt/data"),
            Path("/mnt"),
            Path("/"),
        ]

    def test_parts(self):
        assert Path("/mnt/data/file.txt").parts == ("/", "mnt", "data", "file.txt")

    def test_anchor(self):
        assert Path("/mnt/data/file.txt").anchor == "/"

    def test_joinpath(self):
        p = Path("/mnt/data").joinpath("sub", "file.txt")
        assert str(p) == "/mnt/data/sub/file.txt"

    def test_with_name(self):
        p = Path("/mnt/data/file.txt").with_name("other.log")
        assert str(p) == "/mnt/data/other.log"

    def test_with_stem(self):
        p = Path("/mnt/data/file.txt").with_stem("other")
        assert str(p) == "/mnt/data/other.txt"

    def test_with_suffix(self):
        p = Path("/mnt/data/file.txt").with_suffix(".log")
        assert str(p) == "/mnt/data/file.log"

    def test_is_absolute(self):
        assert Path("/mnt/data").is_absolute()
        assert not Path("relative").is_absolute()

    def test_match(self):
        assert Path("/mnt/data/file.txt").match("*.txt")
        assert not Path("/mnt/data/file.txt").match("*.log")

    def test_fspath(self):
        p = Path("/mnt/data/file.txt")
        assert os.fspath(p) == "/mnt/data/file.txt"

    def test_eq(self):
        assert Path("/mnt/data") == Path("/mnt/data")
        assert Path("/mnt/data") != Path("/mnt/other")

    def test_hash(self):
        s = {Path("/mnt/data"), Path("/mnt/data")}
        assert len(s) == 1

    def test_repr(self):
        p = Path("/mnt/data/file.txt")
        assert repr(p) == "fs123.Path('/mnt/data/file.txt')"

    def test_str(self):
        assert str(Path("/mnt/data")) == "/mnt/data"


class TestFFIStructLayout:
    def test_fs123_stat_fields(self):
        stat = Fs123Stat()
        expected = [
            "st_mode", "st_nlink", "st_uid", "st_gid", "st_size",
            "st_mtime", "st_ctime", "st_atime", "st_ino",
            "st_mtime_nsec", "st_ctime_nsec", "st_atime_nsec",
            "st_dev", "st_blocks", "st_blksize", "st_rdev",
        ]
        actual = [name for name, _ in Fs123Stat._fields_]
        assert actual == expected

    def test_fs123_stat_repr(self):
        stat = Fs123Stat()
        r = repr(stat)
        assert "Fs123Stat(" in r
        assert "st_mode=" in r

    def test_fs123_dirent_fields(self):
        expected = ["name", "d_type"]
        actual = [name for name, _ in Fs123Dirent._fields_]
        assert actual == expected

    def test_fs123_dirent_name_size(self):
        ent = Fs123Dirent()
        assert ctypes.sizeof(ent) == 256 + 1  # name[256] + d_type(1), no padding


class TestModuleFunctions:
    def test_set_proto(self):
        _lib.fs123_set_proto(b"7.3")

    def test_errno_and_strerror(self):
        _lib.fs123_errno()
        _lib.fs123_strerror()

    def test_mount_failure_raises_oserror(self):
        with pytest.raises(OSError):
            from fs123 import umount
            umount("/tmp/nope_fs123_test_not_mounted_either")

    def test_umount_failure_raises_oserror(self):
        with pytest.raises(OSError):
            from fs123 import umount
            umount("/tmp/nope_fs123_test_not_mounted")


class TestPathIO:
    def test_stat_nonexistent_raises(self):
        p = Path("/tmp/nope_fs123_nonexistent_path")
        with pytest.raises(OSError):
            p.stat()

    def test_exists_nonexistent(self):
        assert not Path("/tmp/nope_fs123_nonexistent_path").exists()

    def test_is_file_nonexistent(self):
        assert not Path("/tmp/nope_fs123_nonexistent_path").is_file()

    def test_is_dir_nonexistent(self):
        assert not Path("/tmp/nope_fs123_nonexistent_path").is_dir()

    def test_is_symlink_nonexistent(self):
        assert not Path("/tmp/nope_fs123_nonexistent_path").is_symlink()


class TestFs123FileInterface:
    def test_readable(self):
        f = object.__new__(Fs123File)
        f._closed = False
        f._mode = "r"
        f._path = "/tmp/test"
        assert f.readable() is True

    def test_writable(self):
        f = object.__new__(Fs123File)
        f._closed = False
        f._mode = "r"
        assert f.writable() is False

    def test_seekable(self):
        f = object.__new__(Fs123File)
        f._closed = False
        f._mode = "r"
        assert f.seekable() is True

    def test_closed_property(self):
        f = object.__new__(Fs123File)
        f._closed = True
        assert f.closed is True

    def test_mode_property(self):
        f = object.__new__(Fs123File)
        f._mode = "rb"
        assert f.mode == "rb"

    def test_name_property(self):
        f = object.__new__(Fs123File)
        f._path = "/mnt/data/file.txt"
        assert f.name == "/mnt/data/file.txt"

    def test_read_on_closed_raises(self):
        f = object.__new__(Fs123File)
        f._closed = True
        f._binary = True
        with pytest.raises(ValueError, match="closed"):
            f.read()

    def test_seek_on_closed_raises(self):
        f = object.__new__(Fs123File)
        f._closed = True
        with pytest.raises(ValueError, match="closed"):
            f.seek(0)
