#!/usr/bin/env bash
#
# Run the JSON validator fuzzing campaign fuzz/README.md's "When to run it"
# section asks for: >= 4 CPU-hours per target, before any change to the
# validator's control flow. Not a CI script -- CI only compiles the targets
# (json-suite-drift's neighbour, the fuzz-build job); a useful fuzzing run is
# minutes to hours, and a 60-second CI run finds nothing while looking green.
#
# All three targets run concurrently, each with `-j` (cargo-fuzz's fork-mode
# worker count), so wall-clock time is CPU-hours / workers-per-target rather
# than CPU-hours outright.
#
# Usage:
#   ./fuzz/run-campaign.sh                  # 4 CPU-hours/target, auto worker count
#   ./fuzz/run-campaign.sh --cpu-hours 8
#   ./fuzz/run-campaign.sh --workers 4
#
# Requires: nightly toolchain, cargo-fuzz (`cargo install cargo-fuzz`).
#
# Exit code is nonzero if any target crashed, hung, or left an artifact behind
# (fuzz/artifacts/<target>/) -- the three cases fuzz/README.md's "When it finds
# something" section covers. A clean run leaves fuzz/corpus/<target>/ containing
# whatever new inputs libFuzzer discovered; nothing here commits or discards it.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGETS=(validate_never_panics validate_vs_serde_json validate_position_invariant)

cpu_hours=4
workers=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --cpu-hours) cpu_hours="$2"; shift 2 ;;
    --workers) workers="$2"; shift 2 ;;
    *) echo "error: unknown argument: $1" >&2; exit 1 ;;
  esac
done

command -v cargo-fuzz >/dev/null 2>&1 || {
  echo "error: cargo-fuzz is required (cargo install cargo-fuzz)" >&2
  exit 1
}

# Default: enough workers per target that all three run concurrently without
# oversubscribing the machine, at least 1.
if [[ -z "$workers" ]]; then
  ncpu="$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)"
  workers=$(( ncpu / ${#TARGETS[@]} ))
  (( workers < 1 )) && workers=1
fi

# awk, not bash `(( ))`, because --cpu-hours may be fractional (smoke-testing
# this script itself wants e.g. 0.02).
secs="$(awk -v h="$cpu_hours" -v w="$workers" 'BEGIN { s = h * 3600 / w; print (s < 1) ? 1 : int(s) }')"

echo "materialising seed corpus ..." >&2
"$REPO_ROOT/fuzz/seed-corpus.sh" >&2

echo "building fuzz targets (nightly) ..." >&2
( cd "$REPO_ROOT" && cargo +nightly fuzz build ) >&2

run_dir="$(mktemp -d)"
trap 'rm -rf "$run_dir"' EXIT

echo "campaign start: $cpu_hours CPU-hour(s)/target, $workers worker(s)/target, ~$((secs/60)) min wall-clock" >&2

for t in "${TARGETS[@]}"; do
  mkdir -p "$run_dir/$t" "$REPO_ROOT/fuzz/corpus/$t"
  (
    cd "$REPO_ROOT" || exit 1
    # `-j` is cargo-fuzz's own flag (maps to libFuzzer's `-fork=N`, a single
    # controlling process), not libFuzzer's raw `-workers`/`-jobs`: those write
    # one fuzz-N.log per worker into the *current directory*, which litters
    # the repo root since cargo-fuzz always builds/runs from there.
    #
    # Corpus args before `--`, matching fuzz/README.md's own invocation: the
    # first is where new finds are written, the rest are read-only seeds.
    cargo +nightly fuzz run "$t" -j "$workers" \
      "$REPO_ROOT/fuzz/corpus/$t" "$REPO_ROOT/fuzz/seed-corpus" \
      -- -max_total_time="$secs" \
      >"$run_dir/$t/driver.log" 2>&1
    echo "$?" >"$run_dir/$t/exit-code"
  ) &
done
wait

echo >&2
echo "================ campaign results ================" >&2
status=0
for t in "${TARGETS[@]}"; do
  code="$(cat "$run_dir/$t/exit-code" 2>/dev/null || echo '?')"
  # -fork mode logs "#<n>: cov: ... job: <k> ..." progress lines; take the last.
  runs="$(grep -hoE '^#[0-9]+' "$run_dir/$t/driver.log" 2>/dev/null | tail -1 | tr -d '#')"
  arts="$(find "$REPO_ROOT/fuzz/artifacts/$t" -type f 2>/dev/null | wc -l | tr -d ' ')"
  printf '%-28s exit=%-4s execs=%-14s artifacts=%s\n' "$t" "$code" "${runs:-?}" "$arts" >&2
  if [[ "$code" != "0" || "$arts" != "0" ]]; then
    status=1
    find "$REPO_ROOT/fuzz/artifacts/$t" -type f 2>/dev/null | sed 's/^/    /' >&2
    grep -E 'ERROR|panicked|SUMMARY|Assertion' "$run_dir/$t/driver.log" 2>/dev/null | tail -20 | sed 's/^/    /' >&2
  fi
done
echo >&2

if [[ "$status" == 0 ]]; then
  echo "clean: no crashes, no artifacts" >&2
else
  echo "FAILURES FOUND -- see fuzz/artifacts/<target>/ and fuzz/README.md#when-it-finds-something" >&2
fi
exit "$status"
