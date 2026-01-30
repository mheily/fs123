#!/bin/bash
set -e

# Setup test files
mkdir -p /tmp/fs123-export/testdir
echo "Hello from fs123!" > /tmp/fs123-export/test.txt
echo "File in subdir" > /tmp/fs123-export/testdir/file.txt
echo "Setup complete: created test files"
ls -la /tmp/fs123-export/

# Start server in background
echo "Starting server..."
cargo run &
SERVER_PID=$!
sleep 2

# Test requests
echo ""
echo "=== Testing directory listing ==="
curl 'http://localhost:8080/fs123/7/2/d/?128;0'
echo ""

echo ""
echo "=== Testing file read ==="
curl 'http://localhost:8080/fs123/7/3/f/test.txt?1;0'
echo ""

echo ""
echo "=== Testing attributes ==="
curl 'http://localhost:8080/fs123/7/3/a/test.txt'
echo ""

echo ""
echo "=== Testing server stats ==="
curl 'http://localhost:8080/fs123/7/3/n/'
echo ""

# Cleanup
kill $SERVER_PID 2>/dev/null || true
echo "Tests complete"
