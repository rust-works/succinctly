#!/bin/bash
# Build, check, and lint both succinctly and bench-compare crates

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

echo "=== Checking succinctly ==="
cd "$ROOT_DIR"
cargo check --all-targets --all-features

echo ""
echo "=== Linting succinctly (clippy) ==="
cargo clippy --all-targets --all-features -- -D warnings

echo ""
echo "=== Building succinctly ==="
cargo build --release

echo ""
echo "=== Building succinctly (with CLI) ==="
cargo build --release --features cli

echo ""
echo "=== Building succinctly (with bench-runner) ==="
cargo build --release --features bench-runner

echo ""
echo "=== Checking bench-compare ==="
cd "$ROOT_DIR/bench-compare"
cargo check --all-targets

echo ""
echo "=== Linting bench-compare (clippy) ==="
cargo clippy --all-targets -- -D warnings

echo ""
echo "=== Building bench-compare ==="
cargo build --release

echo ""
echo "=== Build complete ==="
echo "Binaries:"
echo "  $ROOT_DIR/target/release/succinctly"
echo ""
echo "Run tests:"
echo "  cd $ROOT_DIR && cargo test"
echo ""
echo "Run benchmarks:"
echo "  cd $ROOT_DIR && cargo bench"
echo "  cd $ROOT_DIR/bench-compare && cargo bench"
