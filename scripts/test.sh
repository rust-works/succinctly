#!/bin/bash
# Build, check, lint, and test both succinctly and bench-compare crates

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
echo "=== Testing succinctly ==="
cargo test

echo ""
echo "=== Testing succinctly (with CLI) ==="
cargo test --features cli

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
echo "=== Testing bench-compare ==="
cargo test

echo ""
echo "=== All tests passed ==="
