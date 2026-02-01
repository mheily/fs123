#!/bin/bash
# Test script to demonstrate different ESTALE cookie strategies

set -e

echo "=== Testing ESTALE Cookie Strategies ==="
echo

# Create a test directory with some files
TEST_DIR="/tmp/fs123-estale-test"
mkdir -p "$TEST_DIR"
echo "Test content" > "$TEST_DIR/testfile.txt"
mkdir -p "$TEST_DIR/subdir"
echo "Subdir content" > "$TEST_DIR/subdir/file.txt"

echo "Test directory created at: $TEST_DIR"
echo

# Test 1: Default strategy (ioc_getversion)
echo "Test 1: Default strategy (GetVersionIoctl)"
echo "Command: cargo run -p fs123-server -- --export-root '$TEST_DIR' --bind 127.0.0.1:8123"
echo "(Run this in one terminal, then use curl to test)"
echo

# Test 2: Inode strategy
echo "Test 2: Inode strategy"
echo "Command: cargo run -p fs123-server -- --export-root '$TEST_DIR?estalecookie=inode' --bind 127.0.0.1:8123"
echo

# Test 3: None strategy
echo "Test 3: None strategy (disabled)"
echo "Command: cargo run -p fs123-server -- --export-root '$TEST_DIR?estalecookie=none' --bind 127.0.0.1:8123"
echo

# Test requests
echo "=== Test Requests ==="
echo
echo "1. Get attributes for /testfile.txt:"
echo "   curl 'http://127.0.0.1:8123/fs123/7/3/a/testfile.txt'"
echo
echo "2. Read file content:"
echo "   curl 'http://127.0.0.1:8123/fs123/7/3/f/testfile.txt?1;0'"
echo
echo "3. List directory:"
echo "   curl 'http://127.0.0.1:8123/fs123/7/3/d/?64;'"
echo

echo "=== Expected Behavior ==="
echo
echo "With GetVersionIoctl:"
echo "  - On ext3/ext4/xfs/btrfs: estalecookie will be the FS_IOC_GETVERSION value"
echo "  - On other filesystems: Falls back to inode number"
echo
echo "With Inode:"
echo "  - estalecookie will always be the inode number"
echo
echo "With None:"
echo "  - estalecookie will always be 0"
echo

# Cleanup instructions
echo "=== Cleanup ==="
echo "To remove test directory:"
echo "  rm -rf $TEST_DIR"
