#!/usr/bin/env bash
#
# Regenerate the yq golden fixtures from the pinned yq version.
#
# The goldens drive tests/yq_golden_tests.rs. Each case's expected.out is
# captured from mikefarah/yq (the oracle) at the version pinned in
# tests/data/yq-golden/YQ_VERSION — never from succinctly's own output, which
# would enshrine succinctly's bugs as "correct" and reduce the suite to a
# regression test with no oracle value.
#
# Usage:
#   ./scripts/sync-yq-golden.sh              # regenerate expected.out files
#   ./scripts/sync-yq-golden.sh --check      # verify goldens match pinned yq
#
# To move to a newer yq, bump YQ_VERSION, install that version, run this, and
# review the diff — new divergences will surface as known-failures manifest
# churn in tests/yq_golden_tests.rs.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GOLDEN_DIR="$REPO_ROOT/tests/data/yq-golden"
PIN="$(cat "$GOLDEN_DIR/YQ_VERSION")"

check_only=false
[[ "${1:-}" == "--check" ]] && check_only=true

command -v yq >/dev/null 2>&1 || {
  echo "error: yq not found on PATH — install mikefarah/yq $PIN" >&2
  echo "  https://github.com/mikefarah/yq/releases/tag/$PIN" >&2
  exit 1
}

version_line="$(yq --version)"
if [[ "$version_line" != *"version $PIN"* ]]; then
  echo "error: yq on PATH is '$version_line' but goldens are pinned to $PIN" >&2
  echo "  install $PIN from https://github.com/mikefarah/yq/releases/tag/$PIN" >&2
  exit 1
fi

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

found=0
stale=0
for dir in "$GOLDEN_DIR"/cases/*/; do
  name="$(basename "$dir")"
  found=$((found + 1))

  for f in input.yaml filter args; do
    [[ -f "$dir$f" ]] || { echo "error: case $name is missing $f" >&2; exit 1; }
  done

  # One CLI arg per line; blank lines ignored. (bash 3.2 compatible — no mapfile.)
  args=()
  while IFS= read -r arg; do
    [[ -n "$arg" ]] && args+=("$arg")
  done < "$dir/args"
  filter="$(cat "$dir/filter")"

  yq "${args[@]}" "$filter" < "$dir/input.yaml" > "$work_dir/out" || {
    echo "error: yq failed on case $name" >&2
    exit 1
  }

  if $check_only; then
    if ! diff -u "$dir/expected.out" "$work_dir/out"; then
      echo "error: case $name expected.out does not match yq $PIN" >&2
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
    echo "error: $stale golden(s) out of date — run ./scripts/sync-yq-golden.sh" >&2
    exit 1
  fi
  echo "$found goldens up to date with yq $PIN" >&2
else
  echo "captured $found goldens from yq $PIN" >&2
fi
