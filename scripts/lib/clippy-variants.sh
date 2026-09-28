# shellcheck shell=bash
#
# The clippy feature-flag lists that exclude `scalar-yaml` and
# `portable-popcount` so both real code paths they'd otherwise compile out
# --  every YAML SIMD backend, and the AVX-512 popcount path -- get linted
# (#185/#388/#3006). Single source of truth for this string (#3354):
# scripts/build.sh, scripts/test.sh, and .github/workflows/ci.yml's
# `clippy`/`clippy-all-variants` jobs all source this file rather than each
# holding their own copy. Update here only.
#
# Sourced, not executed. Plain variable assignments only -- no `set -e`/`-u`
# requirement of its own, since the sourcing script's own settings (e.g.
# `scripts/build.sh`/`scripts/test.sh`'s `set -e`) already govern this file.

# The core list every platform runs: excludes `scalar-yaml` (which compiles
# out every YAML SIMD backend) and `portable-popcount` (which, together with
# `simd`, also compiles out the AVX-512 popcount path).
# shellcheck disable=SC2034  # read by every script/workflow step that sources this
CLIPPY_CORE_FEATURES="std,simd,serde,cli,regex,bench-runner,large-tests,mmap-tests"

# ARM64-only: the core list above plus `broadword-yaml`, which additionally
# selects the ARM broadword YAML backend (`target_arch = "aarch64"`-gated,
# so this variant only lints anything new on that architecture).
# shellcheck disable=SC2034
CLIPPY_ARM_BROADWORD_FEATURES="std,simd,broadword-yaml,serde,cli,regex,bench-runner,large-tests,mmap-tests"
