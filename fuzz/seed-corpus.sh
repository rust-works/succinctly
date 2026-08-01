#!/usr/bin/env bash
#
# Materialise the fuzzing seed corpus from inputs already in the repo.
#
# Generated rather than committed: every seed here duplicates something already
# tracked (the vendored JSONTestSuite corpus, the real-workload bench seed, the
# proptest regression seeds), so committing them again would bloat the tree and
# leave two copies to keep in sync.
#
# Usage:
#   ./fuzz/seed-corpus.sh
#   cargo +nightly fuzz run validate_never_panics \
#       fuzz/corpus/validate_never_panics fuzz/seed-corpus
#
# libFuzzer accepts several corpus directories; the first is where it writes new
# inputs, the rest are read-only seeds. That is why one shared seed set serves
# all three targets.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="$REPO_ROOT/fuzz/seed-corpus"

command -v python3 >/dev/null 2>&1 || { echo "error: python3 is required" >&2; exit 1; }

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

# libFuzzer requires every corpus directory named on the command line to exist
# already, and cargo-fuzz only auto-creates one when it is passing the corpus
# path itself. The two-directory invocation in README.md ("write here, read
# seeds from there") therefore fails on a fresh checkout with
#   ERROR: The required directory "fuzz/corpus/<target>" does not exist
# for every target that has never been run the default way. Create them here,
# since this is the script you are told to run first.
for target in validate_never_panics validate_vs_serde_json validate_position_invariant; do
  mkdir -p "$REPO_ROOT/fuzz/corpus/$target"
done

# 1. Every JSONTestSuite case — 318 inputs that between them cover the whole
#    grammar plus the malformed shapes a conformance suite thinks to try.
python3 - "$REPO_ROOT" "$OUT_DIR" <<'PY'
import base64, glob, json, pathlib, sys

repo, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
corpora = sorted(glob.glob(str(repo / "tests/data/json-test-suite-*.json")))
if not corpora:
    sys.exit("error: no vendored JSONTestSuite corpus — run ./scripts/sync-json-test-suite.sh")

n = 0
for case in json.loads(pathlib.Path(corpora[-1]).read_text()):
    (out / case["id"]).write_bytes(base64.b64decode(case["bytes_b64"]))
    n += 1
print(f"{n} seeds from {pathlib.Path(corpora[-1]).name}", file=sys.stderr)
PY

# 2. Real-workload JSON: shapes a synthetic suite never produces.
find "$REPO_ROOT/tests/data/bench-corpus/seed/json" -type f \
  \( -name '*.json' -o -name '*.geojson' \) -exec cp {} "$OUT_DIR/" \; 2>/dev/null || true

# 3. Anything proptest has already shrunk to a minimal failing case.
if [[ -f "$REPO_ROOT/tests/json_validate_properties.proptest-regressions" ]]; then
  cp "$REPO_ROOT/tests/json_validate_properties.proptest-regressions" \
     "$OUT_DIR/proptest-regressions.txt"
fi

echo "seed corpus ready at $OUT_DIR ($(find "$OUT_DIR" -type f | wc -l | tr -d ' ') files)" >&2
