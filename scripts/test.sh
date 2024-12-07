#!/bin/bash
set -e

# Set up environment
export RUST_BACKTRACE=1
export RUST_LOG=debug

# Clean up any leftover shared memory segments
cleanup_shm() {
    if [ "$(uname)" == "Darwin" ]; then
        # macOS cleanup - use temporary directory
        rm -f "${TMPDIR:-/tmp}/test_broker_"*
    else
        # Linux cleanup
        for f in /dev/shm/test_broker_*; do
            rm -f "$f" 2>/dev/null || true
        done
    fi
}

cleanup_shm

# Run tests
echo "Running tests..."
cargo test -- --nocapture

# Clean up after tests
cleanup_shm

echo "Tests completed" 