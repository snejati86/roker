#!/bin/bash
set -e

# Run formatting check
cargo fmt -- --check

# Run clippy
cargo clippy -- -D warnings

# Run tests with different configurations
echo "Running tests in debug mode..."
cargo test

echo "Running tests in release mode..."
cargo test --release

echo "Running tests with all features..."
cargo test --all-features

# Run specific test categories
echo "Running doc tests..."
cargo test --doc

echo "Running integration tests..."
cargo test --test '*'

# Run benchmarks if any
if [ -d "benches" ]; then
    echo "Running benchmarks..."
    cargo bench
fi

echo "All tests completed successfully!" 