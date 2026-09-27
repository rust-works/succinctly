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

# `--all-features` turns on `scalar-yaml` (compiles out every YAML SIMD
# backend) and `portable-popcount` (compiles out the AVX-512 popcount path),
# so the step above never lints `yaml/simd/{x86,neon,broadword}.rs` or
# `bits/popcount.rs`'s AVX-512 arm -- the same blind spot ci.yml's own
# `clippy`/`clippy-all-variants` jobs run a second (and, on ARM, third)
# invocation to close (#185, #388, #3006, #3254). Matches ci.yml's `clippy` job.
echo ""
echo "=== Linting succinctly (clippy, YAML SIMD + AVX-512 popcount backend selected) ==="
cargo clippy --all-targets --features std,simd,serde,cli,regex,bench-runner,large-tests,mmap-tests -- -D warnings

# The YAML backend selectors are cfg-exclusive to one target_arch, so the
# invocation above never selects `neon`/`broadword` on an ARM machine either
# -- only `--features broadword-yaml` does, and only on aarch64/arm64 is
# there anything under `target_arch = "aarch64"` for it to actually lint.
# Matches ci.yml's ARM64-only `clippy-all-variants` third invocation.
case "$(uname -m)" in
  aarch64|arm64)
    echo ""
    echo "=== Linting succinctly (clippy, ARM broadword-yaml backend selected) ==="
    cargo clippy --all-targets --features std,simd,broadword-yaml,serde,cli,regex,bench-runner,large-tests,mmap-tests -- -D warnings
    ;;
esac

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
