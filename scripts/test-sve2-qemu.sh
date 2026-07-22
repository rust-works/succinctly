#!/usr/bin/env bash
# Run the aarch64 test suite under QEMU user-mode emulation with SVE2 enabled.
#
# Purpose (#194): Apple Silicon (M1-M4) does not implement non-streaming
# SVE/SVE2, so on a Mac the SVE2 suites self-skip (printing `SKIPPED` lines)
# and a green local `cargo test` says nothing about those kernels. This script
# is the local validation path: it compiles the suite for
# aarch64-unknown-linux-gnu inside Docker and executes the test binaries under
# `qemu-aarch64 -cpu max`, which emulates SVE2 (including BITPERM) via TCG.
#
# Routine CI coverage comes from the ubuntu-24.04-arm (Neoverse-N2) runner and
# the SUCCINCTLY_SVE2=1 dispatch step — see CONTRIBUTING.md (SIMD CI coverage).
# Use this script to validate SVE2 changes from any host with Docker; it works
# on both arm64 (native compile, emulated execution) and x86_64 hosts
# (cross-compile, emulated execution).
#
# Usage: scripts/test-sve2-qemu.sh [extra cargo-test args, e.g. a test filter]

set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"

if ! command -v docker >/dev/null 2>&1; then
  echo "error: docker is required — qemu-user is Linux-only, so the emulated run happens in a container." >&2
  echo "       Install Docker Desktop / OrbStack / colima and retry." >&2
  exit 1
fi

# Named volumes keep the emulated build and the crate registry out of the
# host checkout (no root-owned target/ files) while still caching across runs.
docker run --rm \
  -v "$repo":/src \
  -v succinctly-sve2-target:/qemu-target \
  -v succinctly-sve2-registry:/usr/local/cargo/registry \
  -w /src \
  rust:1-bookworm \
  bash -euo pipefail -c '
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends qemu-user >/dev/null

    # On a non-aarch64 container (x86_64 host) the build is a cross-compile
    # and needs the aarch64 cross toolchain; an arm64 container links natively.
    if [ "$(uname -m)" != "aarch64" ]; then
      apt-get install -y -qq --no-install-recommends \
        gcc-aarch64-linux-gnu libc6-dev-arm64-cross >/dev/null
      export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
    fi
    rustup target add aarch64-unknown-linux-gnu

    # -cpu max enables SVE2 + BITPERM under TCG. Pin the SVE vector length to
    # 128 bits (sve-max-vq=1) to match real Neoverse N2/V2 hardware rather
    # than QEMU'\''s wider default.
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER="qemu-aarch64 -cpu max,sve-max-vq=1"
    export CARGO_TARGET_DIR=/qemu-target

    # The expectation test (tests/simd_expectation_tests.rs) makes the run
    # self-verifying: if the emulated CPU stopped exposing sve2/sve2-bitperm
    # this fails the run instead of letting the SVE2 suites silently skip
    # (#193/#194).
    export SUCCINCTLY_EXPECT_SIMD=neon,sve2,sve2-bitperm

    echo "=== SVE2 kernels via runtime detection (DSV, broadword, unit tests) ==="
    cargo test --target aarch64-unknown-linux-gnu --features simd "$@"

    echo "=== JSON SVE2 dispatch (SUCCINCTLY_SVE2=1) ==="
    SUCCINCTLY_SVE2=1 cargo test --target aarch64-unknown-linux-gnu --features simd "$@"
  ' bash "$@"
