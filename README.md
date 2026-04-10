# fs123 (Rust rewrite)

*WARNING* THIS IS AN UNFINISHED EXPERIMENTAL WORK IN PROGRESS -- DO NOT USE WITH ANY IMPORTANT DATA.
See [LICENSE.txt](./LICENSE.txt) for the full details, but TL;DR **USE THIS AT YOUR OWN RISK**

## Overview

This is a complete rewrite of the fs123 client and server in Rust, using AI to generate most of the code.

## Development HOWTO

Requirements to build this software:
* MacOS with MacFUSE installed, or Linux (see the Dockerfile for required packages).
* Docker

### Build instructions

* Run the `./configure` script to setup the working copy.

```
./configure
```

Start the containers.

```
docker-compose up -d
```

Login

```
docker-compose exec client bash
```

View the contents of the mounted fs123 filesystem.

```
# ls /mnt
hello.txt  hello2.txt
```

### Running Tests

**Unit tests** (no special requirements):

```
cargo test --workspace
```

**Integration tests** require FUSE support and no sandbox restrictions. They are
marked `#[ignore]` by default. Docker is the recommended environment.

Build the binaries first:

```
cargo build --workspace
```

**HTTP-level integration tests** test the server directly without FUSE:

```
cargo test -p fs123-server --test fuse_integration_tests -- test_http --ignored
```

**FUSE integration tests** test the full stack through a mounted filesystem.
Use `--test-threads=1` to avoid mount conflicts:

```
cargo test -p fs123-server --test fuse_integration_tests -- --ignored --test-threads=1
```

**All tests** (unit + integration):

```
cargo test -p fs123-server --test fuse_integration_tests -- --include-ignored --test-threads=1
```

#### Docker (recommended)

Docker is the easiest way to run integration tests since the container has
FUSE support and no sandbox restrictions:

```
docker-compose up -d
docker-compose exec client cargo test --workspace -- --include-ignored --test-threads=1
```

To run only the HTTP-level integration tests in Docker:

```
docker-compose exec client cargo test -p fs123-server --test fuse_integration_tests -- test_http --ignored
```

#### Python (pyfs123) integration tests

The Python tests exercise the `pyfs123` package (mount, stat, read, iterdir,
symlinks, fsync, etc.) against a running fs123-server. They require `libfs123`
to be built and the Python virtualenv to be set up (done by `./configure`).

The easiest way to run everything is the unified test runner, which starts a
temporary server, sets up the environment, and runs both the Rust and Python
integration suites:

```
python scripts/run_integration_tests.py
```

Options:

```
python scripts/run_integration_tests.py --rust-only     # skip Python tests
python scripts/run_integration_tests.py --python-only   # skip Rust tests
python scripts/run_integration_tests.py --release        # use release builds
```

To run the Python tests manually, first start a server and set the required
environment variables:

```
export FS123_TEST_SERVER_URL=http://127.0.0.1:<port>
export FS123_TEST_MOUNTPOINT=/fs123_test
python -m pytest python/tests/test_integration.py -v
```
