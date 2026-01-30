# fs123-server

A Rust implementation of the fs123 protocol server.

## Overview

fs123-server implements the server side of the fs123 protocol, which provides filesystem access over HTTP. This is a read-only distributed filesystem where filesystem operations are translated into HTTP requests.

## Building

```bash
cargo build
```

## Testing

```bash
cargo test
```

## Running

```bash
# Create a directory to export
mkdir -p /tmp/fs123-export
echo "Hello, World!" > /tmp/fs123-export/test.txt

# Run the server (exports /tmp/fs123-export by default)
cargo run
```

The server will start on `http://127.0.0.1:8080`.

## Protocol Endpoints

The server implements the following fs123 protocol endpoints:

- `/a` - File/directory attributes (stat)
- `/d` - Directory listings
- `/f` - File content reads
- `/l` - Symbolic link targets
- `/s` - Filesystem statistics (statfs)
- `/x` - Extended attributes (currently returns ENOTSUP)
- `/n` - Server statistics
- `/p` - Passthrough (not implemented)

## Example Requests

```bash
# Get attributes for /test.txt
curl 'http://127.0.0.1:8080/fs123/7/3/a/test.txt'

# Read first 1 KiB of /test.txt
curl 'http://127.0.0.1:8080/fs123/7/3/f/test.txt?1;0'

# List directory /
curl 'http://127.0.0.1:8080/fs123/7/3/d/?64;'

# Get server statistics
curl 'http://127.0.0.1:8080/fs123/7/3/n/'
```

## Protocol Documentation

See the [legacy docs/](../../legacy/docs/) directory for complete protocol specifications:

- `Fs123Protocol` - Protocol specification (v7.3)
- `Fs123Consistency` - Consistency model
- `Fs123CacheControl` - Caching strategy

Also see [CLAUDE.md](CLAUDE.md) for development guidance.

## License

See [LICENSE.txt](../../LICENSE.txt).
