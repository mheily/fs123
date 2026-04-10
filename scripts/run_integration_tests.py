#!/usr/bin/env python3
"""Integration test runner for fs123.

Builds the project, starts a temporary fs123-server, runs both the Rust
(libfs123) and Python (pyfs123) integration test suites against it, then
tears everything down.

Usage:
    python scripts/run_integration_tests.py [--release] [--rust-only] [--python-only]
"""

import argparse
import atexit
import os
import platform
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time


def find_project_root():
    """Walk up from this script to find the workspace Cargo.toml."""
    d = os.path.dirname(os.path.abspath(__file__))
    while True:
        cargo = os.path.join(d, "Cargo.toml")
        if os.path.exists(cargo):
            with open(cargo) as f:
                if "[workspace]" in f.read():
                    return d
        parent = os.path.dirname(d)
        if parent == d:
            break
        d = parent
    sys.exit("Could not find workspace Cargo.toml")


def find_free_port():
    """Bind to port 0 to get an OS-assigned free port."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_for_server(host, port, timeout=10):
    """Poll TCP connect until the server is ready."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.1)
    return False


def create_fixtures(path):
    """Create test fixtures matching the Rust create_test_fixtures()."""
    # Regular files
    _write(path, "hello.txt", b"Hello, fs123!")
    _write(path, "binary.dat", bytes(range(256)))
    _write(path, "large.dat", bytes(i % 256 for i in range(10240)))
    _write(path, "empty.txt", b"")

    # Subdirectory with files
    subdir = os.path.join(path, "subdir")
    os.makedirs(subdir)
    _write(subdir, "nested.txt", b"Nested content")
    _write(subdir, "another.txt", b"Another file")

    # Deeply nested directory
    deep = os.path.join(path, "a", "b", "c", "d")
    os.makedirs(deep)
    _write(deep, "deep.txt", b"Deep file")

    # Symlinks
    os.symlink("hello.txt", os.path.join(path, "link_to_hello"))
    os.symlink(os.path.join(path, "hello.txt"), os.path.join(path, "absolute_link"))
    os.symlink("subdir", os.path.join(path, "link_to_subdir"))

    # Special filenames
    _write(path, "file with spaces.txt", b"Has spaces")
    _write(path, "\u65e5\u672c\u8a9e.txt", b"Japanese filename")

    # Restricted permissions
    restricted = os.path.join(path, "restricted.txt")
    with open(restricted, "wb") as f:
        f.write(b"Restricted content")
    os.chmod(restricted, 0o600)


def _write(directory, name, content):
    with open(os.path.join(directory, name), "wb") as f:
        f.write(content)


def lib_name():
    if platform.system() == "Darwin":
        return "libfs123.dylib"
    return "libfs123.so"


def main():
    parser = argparse.ArgumentParser(description="Run fs123 integration tests")
    parser.add_argument("--release", action="store_true", help="Use release builds")
    parser.add_argument("--rust-only", action="store_true", help="Run only Rust tests")
    parser.add_argument("--python-only", action="store_true", help="Run only Python tests")
    args = parser.parse_args()

    root = find_project_root()
    profile = "release" if args.release else "debug"
    target_dir = os.path.join(root, "target", profile)
    server_bin = os.path.join(target_dir, "fs123-server")

    # ── Step 1: Build ────────────────────────────────────────────────
    print("=" * 60)
    print("Building workspace...")
    print("=" * 60)
    build_cmd = ["cargo", "build", "-p", "fs123-server", "-p", "libfs123"]
    if args.release:
        build_cmd.append("--release")
    subprocess.check_call(build_cmd, cwd=root)

    if not os.path.exists(server_bin):
        sys.exit(f"Server binary not found at {server_bin}")

    # ── Step 2: Create fixtures ──────────────────────────────────────
    tmpdir = tempfile.mkdtemp(prefix="fs123_integ_")
    print(f"\nFixture directory: {tmpdir}")
    create_fixtures(tmpdir)

    # ── Step 3: Start server ─────────────────────────────────────────
    port = find_free_port()
    bind_addr = f"127.0.0.1:{port}"
    print(f"\nStarting fs123-server on {bind_addr} (export: {tmpdir})")

    server_proc = subprocess.Popen(
        [server_bin, "--bind", bind_addr, "--export-root", tmpdir],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    # Ensure cleanup on any exit path
    symlink_path = None

    def cleanup():
        nonlocal symlink_path
        print("\nCleaning up...")
        if server_proc.poll() is None:
            server_proc.terminate()
            try:
                server_proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server_proc.kill()
                server_proc.wait()
        if symlink_path and os.path.islink(symlink_path):
            os.unlink(symlink_path)
        if os.path.exists(tmpdir):
            shutil.rmtree(tmpdir, ignore_errors=True)
        print("Done.")

    atexit.register(cleanup)
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(1))

    if not wait_for_server("127.0.0.1", port):
        stderr_out = server_proc.stderr.read().decode(errors="replace") if server_proc.stderr else ""
        sys.exit(f"Server failed to start within timeout.\nStderr: {stderr_out[:2000]}")

    print("Server ready.")

    # ── Step 4: Symlink library for Python ───────────────────────────
    lib_src = os.path.join(target_dir, lib_name())
    lib_dst = os.path.join(root, "python", "fs123", lib_name())
    if os.path.exists(lib_src):
        if os.path.islink(lib_dst) or os.path.exists(lib_dst):
            os.unlink(lib_dst)
        os.symlink(lib_src, lib_dst)
        symlink_path = lib_dst
        print(f"Symlinked {lib_name()} -> {lib_src}")
    else:
        print(f"WARNING: {lib_src} not found, Python tests may fail to load library")

    # ── Step 5: Set environment ──────────────────────────────────────
    env = os.environ.copy()
    env["FS123_TEST_SERVER_URL"] = f"http://127.0.0.1:{port}"
    env["FS123_TEST_MOUNTPOINT"] = "/fs123_test"
    env["FS123_TEST_EXPORT_DIR"] = tmpdir

    # Library search path for both Rust linker and Python ctypes
    if platform.system() == "Darwin":
        env["DYLD_LIBRARY_PATH"] = target_dir + ":" + env.get("DYLD_LIBRARY_PATH", "")
    else:
        env["LD_LIBRARY_PATH"] = target_dir + ":" + env.get("LD_LIBRARY_PATH", "")

    rust_ok = True
    python_ok = True

    # ── Step 6: Run Rust integration tests ───────────────────────────
    if not args.python_only:
        print("\n" + "=" * 60)
        print("Running Rust (libfs123) integration tests...")
        print("=" * 60)
        rc = subprocess.call(
            [
                "cargo", "test", "-p", "libfs123",
                "--test", "integration_tests",
                "--", "--ignored", "--test-threads=1",
            ],
            cwd=root,
            env=env,
        )
        rust_ok = rc == 0
        if rust_ok:
            print("\nRust tests: PASSED")
        else:
            print(f"\nRust tests: FAILED (exit code {rc})")

    # ── Step 7: Run Python integration tests ─────────────────────────
    if not args.rust_only:
        print("\n" + "=" * 60)
        print("Running Python (pyfs123) integration tests...")
        print("=" * 60)
        rc = subprocess.call(
            [sys.executable, "-m", "pytest", "python/tests/test_integration.py", "-v"],
            cwd=root,
            env=env,
        )
        python_ok = rc == 0
        if python_ok:
            print("\nPython tests: PASSED")
        else:
            print(f"\nPython tests: FAILED (exit code {rc})")

    # ── Step 8: Summary ──────────────────────────────────────────────
    print("\n" + "=" * 60)
    if not args.python_only:
        print(f"  Rust:   {'PASSED' if rust_ok else 'FAILED'}")
    if not args.rust_only:
        print(f"  Python: {'PASSED' if python_ok else 'FAILED'}")
    print("=" * 60)

    sys.exit(0 if rust_ok and python_ok else 1)


if __name__ == "__main__":
    main()
