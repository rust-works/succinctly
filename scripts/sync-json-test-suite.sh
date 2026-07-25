#!/usr/bin/env bash
#
# Regenerate the vendored JSONTestSuite corpus from a pinned upstream commit.
#
# The corpus drives tests/json_test_suite.rs. It is vendored (rather than fetched
# at test time) so that `cargo test` needs no network and the exact conformance
# input is reviewable in-tree.
#
# Usage:
#   ./scripts/sync-json-test-suite.sh              # regenerate at the pinned commit
#   ./scripts/sync-json-test-suite.sh --check      # verify in-tree corpus is current
#
# To move to a newer upstream revision, bump SUITE_SHA, run this, and review the
# resulting diff — new failures will surface as known-failures manifest churn.
#
# Unlike the YAML suite (which is tagged), JSONTestSuite publishes no releases, so
# we pin a commit SHA and encode its short form in the corpus filename.

set -euo pipefail

SUITE_REPO="https://github.com/nst/JSONTestSuite"
SUITE_SHA="1ef36fa01286573e846ac449e8683f8833c5b26a"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_FILE="$REPO_ROOT/tests/data/json-test-suite-${SUITE_SHA:0:7}.json"

check_only=false
[[ "${1:-}" == "--check" ]] && check_only=true

for cmd in git python3; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "error: $cmd is required" >&2; exit 1; }
done

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

echo "Cloning $SUITE_REPO at ${SUITE_SHA:0:7} ..." >&2
git init --quiet "$work_dir/suite"
git -C "$work_dir/suite" remote add origin "$SUITE_REPO"
git -C "$work_dir/suite" fetch --quiet --depth 1 origin "$SUITE_SHA"
git -C "$work_dir/suite" checkout --quiet FETCH_HEAD

# Upstream stores one file per case in test_parsing/, with the verdict encoded in
# the filename prefix:
#
#   y_*   MUST be accepted
#   n_*   MUST be rejected
#   i_*   implementation-defined — either verdict conforms
#
# test_transform/ is deliberately excluded: it tests *value* semantics (number
# precision, duplicate-key resolution) that a validator does not model, the same
# reason the YAML sync drops out.yaml / test.event / emit.yaml.
python3 - "$work_dir/suite/test_parsing" "$work_dir/out.json" <<'PY'
import base64, json, pathlib, sys

root, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
cases = []

for path in sorted(root.glob("*.json")):
    prefix = path.name[0]
    if prefix not in ("y", "n", "i"):
        sys.exit(f"error: unexpected case prefix in {path.name} — upstream layout may have changed")
    # Payloads are base64-encoded, not embedded as text: many cases are
    # deliberately invalid UTF-8 (n_string_invalid_utf8_after_escape.json) or
    # non-UTF-8 encodings (i_string_UTF-16LE_with_BOM.json), so `read_text()`
    # would throw. This also keeps the vendored corpus itself valid JSON.
    cases.append({
        "id": path.name,
        "expect": prefix,
        "bytes_b64": base64.b64encode(path.read_bytes()).decode("ascii"),
    })

if not cases:
    sys.exit("error: no test cases found — upstream layout may have changed")

out.write_text(json.dumps(cases, indent=1, sort_keys=True) + "\n")

counts = {p: sum(c["expect"] == p for c in cases) for p in ("y", "n", "i")}
print(
    f"{len(cases)} cases: {counts['y']} must-accept, {counts['n']} must-reject, "
    f"{counts['i']} implementation-defined",
    file=sys.stderr,
)
PY

if $check_only; then
  if ! diff -q "$OUT_FILE" "$work_dir/out.json" >/dev/null 2>&1; then
    echo "error: $OUT_FILE is out of date — run ./scripts/sync-json-test-suite.sh" >&2
    exit 1
  fi
  echo "corpus is up to date" >&2
else
  mkdir -p "$(dirname "$OUT_FILE")"
  cp "$work_dir/out.json" "$OUT_FILE"
  echo "wrote $OUT_FILE" >&2
fi
