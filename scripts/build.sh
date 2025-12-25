#!/bin/bash

# Build script that runs cargo build, fmt check, and clippy
# Exits on first failure

set -e

echo "🔨 Building project..."
cargo build

echo "✅ Build successful!"

echo "🎨 Checking code formatting..."
cargo fmt --check

echo "✅ Code formatting check passed!"

echo "🔍 Running clippy checks..."
cargo clippy --all-targets --all-features -- -D warnings

echo "✅ All checks passed! 🎉"