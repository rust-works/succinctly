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
# A case where yq rejects the input additionally gets expected.status (its
# exit code) and expected.err (its stderr) instead of a passing expected.out,
# so a case that exercises input yq refuses to process has an oracle for that
# too.
#
# A case where yq exits 0 having printed nothing gets a marker file
# expected.empty beside its empty expected.out, so the Rust loader can tell a
# silent success from a fixture that was never captured. Real yq does this:
# `key` and `parent` at the document root emit no output at all (#2421).
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
  #
  # `|| [[ -n "$arg" ]]` keeps a last line with no trailing newline: `read`
  # returns non-zero on it, so a plain `while read` loop would silently drop
  # it, while the Rust loader's `.lines()` keeps it — the two disagreeing
  # would capture a golden with the args omitted, then compare it against a
  # run that applied them.
  args=()
  while IFS= read -r arg || [[ -n "$arg" ]]; do
    [[ -n "$arg" ]] && args+=("$arg")
  done < "$dir/args"
  filter="$(cat "$dir/filter")"

  # `${args[@]+...}` guards the empty-array case under `set -u` on bash 3.2
  # (macOS), where a case with no CLI args would otherwise trip "unbound
  # variable".
  # A non-zero exit is a legitimate fixture, not a script failure: a case
  # that exercises input yq rejects captures yq's exit code and stderr too,
  # and those are the only oracle for a failure — neither reaches stdout.
  set +e
  yq ${args[@]+"${args[@]}"} "$filter" < "$dir/input.yaml" \
    > "$work_dir/out" 2> "$work_dir/err"
  status=$?
  set -e

  if $check_only; then
    if ! diff -u "$dir/expected.out" "$work_dir/out"; then
      echo "error: case $name expected.out does not match yq $PIN" >&2
      stale=$((stale + 1))
    fi
    if [[ $status -eq 0 ]]; then
      if [[ -f "$dir/expected.status" || -f "$dir/expected.err" ]]; then
        echo "error: case $name now passes under yq $PIN but has expected.status/err" >&2
        stale=$((stale + 1))
      fi
      # The silent-success marker is a golden too: it says yq printed nothing
      # on success (#2421), and it going stale in either direction is drift.
      if [[ -s "$work_dir/out" && -f "$dir/expected.empty" ]]; then
        echo "error: case $name now prints output under yq $PIN but has expected.empty" >&2
        stale=$((stale + 1))
      fi
      if [[ ! -s "$work_dir/out" && ! -f "$dir/expected.empty" ]]; then
        echo "error: case $name prints nothing under yq $PIN but has no expected.empty" >&2
        stale=$((stale + 1))
      fi
    else
      if [[ "$(cat "$dir/expected.status" 2>/dev/null)" != "$status" ]]; then
        echo "error: case $name exits $status under yq $PIN, expected.status says" \
             "$(cat "$dir/expected.status" 2>/dev/null || echo '<missing>')" >&2
        stale=$((stale + 1))
      fi
      if ! diff -u "$dir/expected.err" "$work_dir/err" 2>/dev/null; then
        echo "error: case $name expected.err does not match yq $PIN" >&2
        stale=$((stale + 1))
      fi
    fi
  else
    cp "$work_dir/out" "$dir/expected.out"
    if [[ $status -eq 0 ]]; then
      # A case that used to fail and now passes sheds its failure fixtures.
      rm -f "$dir/expected.status" "$dir/expected.err"
      # `expected.empty` declares "yq exited 0 and printed nothing", which real
      # yq genuinely does (`key`/`parent` at the document root, #2421). The
      # Rust loader would otherwise reject an empty expected.out as a fixture
      # that was never captured, and the corpus could not hold those cases.
      if [[ -s "$dir/expected.out" ]]; then
        rm -f "$dir/expected.empty"
      else
        : > "$dir/expected.empty"
      fi
      echo "wrote cases/$name/expected.out" >&2
    else
      rm -f "$dir/expected.empty"
      echo "$status" > "$dir/expected.status"
      cp "$work_dir/err" "$dir/expected.err"
      echo "wrote cases/$name/expected.{out,err,status} (yq exit $status)" >&2
    fi
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
