#!/usr/bin/env bash
#
# Regenerate the jq golden fixtures from the pinned jq version.
#
# The goldens drive tests/jq_golden_tests.rs. Each case's expected.out is
# captured from jqlang/jq (the oracle) at the version pinned in
# tests/data/jq-golden/JQ_VERSION — never from succinctly's own output, which
# would enshrine succinctly's bugs as "correct" and reduce the suite to a
# regression test with no oracle value.
#
# Succinctly-only extensions (at_offset, at_position, @dsv, @urid, @props,
# @yaml) have no jq oracle — real jq errors on them — so they never appear in
# this corpus; they stay in the self-snapshot tests/cli_golden_tests.rs.
#
# Usage:
#   ./scripts/sync-jq-golden.sh              # regenerate expected.out files
#   ./scripts/sync-jq-golden.sh --check      # verify goldens match pinned jq
#
# To move to a newer jq, bump JQ_VERSION, install that version, run this, and
# review the diff — new divergences will surface as known-failures manifest
# churn in tests/jq_golden_tests.rs.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GOLDEN_DIR="$REPO_ROOT/tests/data/jq-golden"
PIN="$(cat "$GOLDEN_DIR/JQ_VERSION")"

check_only=false
[[ "${1:-}" == "--check" ]] && check_only=true

command -v jq >/dev/null 2>&1 || {
  echo "error: jq not found on PATH — install jqlang/jq $PIN" >&2
  echo "  https://github.com/jqlang/jq/releases/tag/$PIN" >&2
  exit 1
}

# `jq --version` prints a bare version token, e.g. "jq-1.7.1" on the Linux
# release binary or "jq-1.7.1-apple" on the macOS system build. Accept the
# pinned token exactly, or with a platform suffix ("$PIN-apple"), so the same
# goldens verify on both the CI Linux runner and a developer's macOS.
version_line="$(jq --version)"
if [[ "$version_line" != "$PIN" && "$version_line" != "$PIN-"* ]]; then
  echo "error: jq on PATH is '$version_line' but goldens are pinned to $PIN" >&2
  echo "  install $PIN from https://github.com/jqlang/jq/releases/tag/$PIN" >&2
  exit 1
fi

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

found=0
stale=0
for dir in "$GOLDEN_DIR"/cases/*/; do
  name="$(basename "$dir")"
  found=$((found + 1))

  for f in input.json filter args; do
    [[ -f "$dir$f" ]] || { echo "error: case $name is missing $f" >&2; exit 1; }
  done

  # One CLI arg per line; blank lines ignored. (bash 3.2 compatible — no mapfile.)
  args=()
  while IFS= read -r arg; do
    [[ -n "$arg" ]] && args+=("$arg")
  done < "$dir/args"
  filter="$(cat "$dir/filter")"

  # `${args[@]+...}` guards the empty-array case under `set -u` on bash 3.2
  # (macOS), where a case with no CLI args would otherwise trip "unbound
  # variable".
  jq ${args[@]+"${args[@]}"} "$filter" < "$dir/input.json" > "$work_dir/out" || {
    echo "error: jq failed on case $name" >&2
    exit 1
  }

  if $check_only; then
    if ! diff -u "$dir/expected.out" "$work_dir/out"; then
      echo "error: case $name expected.out does not match jq $PIN" >&2
      stale=$((stale + 1))
    fi
  else
    cp "$work_dir/out" "$dir/expected.out"
    echo "wrote cases/$name/expected.out" >&2
  fi
done

[[ $found -gt 0 ]] || { echo "error: no cases found under $GOLDEN_DIR/cases" >&2; exit 1; }

if $check_only; then
  if [[ $stale -gt 0 ]]; then
    echo "error: $stale golden(s) out of date — run ./scripts/sync-jq-golden.sh" >&2
    exit 1
  fi
  echo "$found goldens up to date with jq $PIN" >&2
else
  echo "captured $found goldens from jq $PIN" >&2
fi
