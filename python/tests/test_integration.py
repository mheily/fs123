"""Integration tests for the pyfs123 Python package.

Requires a running fs123-server. Run via scripts/run_integration_tests.py.

Environment variables:
  FS123_TEST_SERVER_URL  - e.g. http://127.0.0.1:18423
  FS123_TEST_MOUNTPOINT  - e.g. /fs123_test
"""

import os

import pytest

import fs123
from fs123 import Path, Fs123File


@pytest.fixture
def mp(mount_server):
    return mount_server


# ── Mount / config ───────────────────────────────────────────────────


class TestMountConfig:
    def test_set_proto(self):
        fs123.set_proto("7.3")

    def test_errno_after_success(self, mp):
        Path(f"{mp}/hello.txt").stat()
        assert fs123.errno() == 0

    def test_strerror_after_success(self, mp):
        Path(f"{mp}/hello.txt").stat()
        assert fs123.strerror() is None

    def test_mount_bad_url_raises(self):
        with pytest.raises(OSError):
            fs123.mount("http://127.0.0.1:1/nonexistent", "/fs123_bad_mount_test")

    def test_umount_not_mounted_raises(self):
        with pytest.raises(OSError):
            fs123.umount("/not_mounted_anywhere")


# ── Path.stat() ──────────────────────────────────────────────────────


class TestStat:
    def test_stat_regular_file(self, mp):
        st = Path(f"{mp}/hello.txt").stat()
        assert st.st_size == 13
        assert (st.st_mode & 0o170000) == 0o100000  # S_IFREG

    def test_stat_directory(self, mp):
        st = Path(f"{mp}/subdir").stat()
        assert (st.st_mode & 0o170000) == 0o040000  # S_IFDIR

    def test_stat_nonexistent_raises(self, mp):
        with pytest.raises(OSError):
            Path(f"{mp}/nonexistent_xyz").stat()


# ── Path predicates ──────────────────────────────────────────────────


class TestPredicates:
    def test_exists_true(self, mp):
        assert Path(f"{mp}/hello.txt").exists()

    def test_exists_false(self, mp):
        assert not Path(f"{mp}/nonexistent_xyz").exists()

    def test_is_file(self, mp):
        assert Path(f"{mp}/hello.txt").is_file()

    def test_is_dir(self, mp):
        assert Path(f"{mp}/subdir").is_dir()

    def test_is_symlink(self, mp):
        assert Path(f"{mp}/link_to_hello").is_symlink()

    def test_is_file_on_dir(self, mp):
        assert not Path(f"{mp}/subdir").is_file()

    def test_is_dir_on_file(self, mp):
        assert not Path(f"{mp}/hello.txt").is_dir()


# ── Path.readlink ────────────────────────────────────────────────────


class TestReadlink:
    def test_readlink(self, mp):
        target = Path(f"{mp}/link_to_hello").readlink()
        assert str(target) == "hello.txt"

    def test_readlink_nonexistent_raises(self, mp):
        with pytest.raises(OSError):
            Path(f"{mp}/nonexistent_xyz").readlink()


# ── Path.iterdir / glob ─────────────────────────────────────────────


class TestIterdir:
    def test_iterdir_root(self, mp):
        names = {p.name for p in Path(mp).iterdir()}
        assert "hello.txt" in names
        assert "subdir" in names
        assert "binary.dat" in names
        assert "empty.txt" in names

    def test_iterdir_subdir(self, mp):
        names = {p.name for p in Path(f"{mp}/subdir").iterdir()}
        assert names == {"nested.txt", "another.txt"}

    def test_glob_txt(self, mp):
        matches = {p.name for p in Path(mp).glob("*.txt")}
        assert "hello.txt" in matches
        assert "empty.txt" in matches
        # binary.dat should not match
        assert "binary.dat" not in matches


# ── Path.read_bytes / read_text ──────────────────────────────────────


class TestReadContent:
    def test_read_text(self, mp):
        assert Path(f"{mp}/hello.txt").read_text() == "Hello, fs123!"

    def test_read_bytes(self, mp):
        data = Path(f"{mp}/binary.dat").read_bytes()
        assert data == bytes(range(256))

    def test_read_text_large(self, mp):
        data = Path(f"{mp}/large.dat").read_bytes()
        assert len(data) == 10240
        expected = bytes(i % 256 for i in range(10240))
        assert data == expected

    def test_read_text_empty(self, mp):
        assert Path(f"{mp}/empty.txt").read_text() == ""

    def test_read_text_unicode_filename(self, mp):
        assert Path(f"{mp}/日本語.txt").read_text() == "Japanese filename"

    def test_read_text_spaces_filename(self, mp):
        assert Path(f"{mp}/file with spaces.txt").read_text() == "Has spaces"

    def test_read_text_deep_nesting(self, mp):
        assert Path(f"{mp}/a/b/c/d/deep.txt").read_text() == "Deep file"


# ── Fs123File (via Path.open) ────────────────────────────────────────


class TestFs123File:
    def test_open_read_text(self, mp):
        with Path(f"{mp}/hello.txt").open("r") as f:
            data = f.read()
        assert data == "Hello, fs123!"
        assert isinstance(data, str)

    def test_open_read_binary(self, mp):
        with Path(f"{mp}/hello.txt").open("rb") as f:
            data = f.read()
        assert data == b"Hello, fs123!"
        assert isinstance(data, bytes)

    def test_open_read_sized(self, mp):
        with Path(f"{mp}/hello.txt").open("rb") as f:
            data = f.read(5)
        assert data == b"Hello"

    def test_open_readline(self, mp):
        # hello.txt has no newline, so readline returns the whole content
        with Path(f"{mp}/hello.txt").open("r") as f:
            line = f.readline()
        assert line == "Hello, fs123!"

    def test_open_readlines(self, mp):
        with Path(f"{mp}/hello.txt").open("r") as f:
            lines = f.readlines()
        assert len(lines) >= 1
        assert "".join(lines) == "Hello, fs123!"

    def test_open_iter(self, mp):
        with Path(f"{mp}/hello.txt").open("r") as f:
            collected = list(f)
        assert "".join(collected) == "Hello, fs123!"

    def test_open_seek_tell(self, mp):
        with Path(f"{mp}/hello.txt").open("rb") as f:
            f.seek(7)
            assert f.tell() == 7
            data = f.read(6)
            assert data == b"fs123!"

    def test_open_seek_end(self, mp):
        with Path(f"{mp}/hello.txt").open("rb") as f:
            pos = f.seek(0, 2)  # SEEK_END
            assert pos == 13

    def test_open_context_manager(self, mp):
        f = Path(f"{mp}/hello.txt").open("r")
        assert not f.closed
        with f:
            f.read()
        assert f.closed

    def test_open_properties(self, mp):
        with Path(f"{mp}/hello.txt").open("rb") as f:
            assert f.name == f"{mp}/hello.txt"
            assert f.mode == "rb"
            assert f.readable() is True
            assert f.writable() is False
            assert f.seekable() is True

    def test_read_after_close_raises(self, mp):
        f = Path(f"{mp}/hello.txt").open("r")
        f.close()
        with pytest.raises(ValueError, match="closed"):
            f.read()


# ── fsync ────────────────────────────────────────────────────────────


class TestFsync:
    def test_path_fsync(self, mp):
        Path(f"{mp}/hello.txt").fsync()

    def test_module_fsync(self, mp):
        fs123.fsync(f"{mp}/hello.txt")
