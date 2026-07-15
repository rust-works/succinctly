#!/usr/bin/env bash
#
# Regenerate the vendored YAML Test Suite corpus from a pinned upstream tag.
#
# The corpus drives tests/yaml_test_suite.rs. It is vendored (rather than fetched
# at test time) so that `cargo test` needs no network and the exact conformance
# input is reviewable in-tree.
#
# Usage:
#   ./scripts/sync-yaml-test-suite.sh              # regenerate at the pinned tag
#   ./scripts/sync-yaml-test-suite.sh --check      # verify in-tree corpus is current
#
# To move to a newer upstream release, bump SUITE_TAG, run this, and review the
# resulting diff — new failures will surface as known-failures manifest churn.

set -euo pipefail

SUITE_REPO="https://github.com/yaml/yaml-test-suite"
SUITE_TAG="data-2022-01-17"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_FILE="$REPO_ROOT/tests/data/yaml-test-suite-${SUITE_TAG#data-}.json"

check_only=false
[[ "${1:-}" == "--check" ]] && check_only=true

for cmd in git python3; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "error: $cmd is required" >&2; exit 1; }
done

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

echo "Cloning $SUITE_REPO at $SUITE_TAG ..." >&2
git clone --quiet --depth 1 --branch "$SUITE_TAG" "$SUITE_REPO" "$work_dir/suite"

# The upstream `data` branch stores one directory per test case, with subtest
# directories (00, 01, ...) when a source file defines multiple cases. Only four
# files matter to us; out.yaml / test.event / emit.yaml describe dumper and event
# behavior that a semi-index does not model.
#
#   ===       test name
#   in.yaml   input document
#   in.json   expected JSON value(s), absent when not JSON-representable
#   error     marker file: this input MUST be rejected
python3 - "$work_dir/suite" "$work_dir/out.json" <<'PY'
import json, pathlib, sys

root, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
cases = []

for in_yaml in sorted(root.rglob("in.yaml")):
    d = in_yaml.parent
    if ".git" in d.parts:
        continue
    case = {
        "id": str(d.relative_to(root)),
        "name": (d / "===").read_text().strip() if (d / "===").exists() else "",
        "yaml": in_yaml.read_text(),
        "fail": (d / "error").exists(),
    }
    # in.json is absent for inputs with no JSON representation (e.g. non-string
    # keys) and for must-fail inputs. Such cases are still scored for parse
    # success/failure, just not for output equality.
    if (d / "in.json").exists():
        case["json"] = (d / "in.json").read_text()
    cases.append(case)

if not cases:
    sys.exit("error: no test cases found — upstream layout may have changed")

out.write_text(json.dumps(cases, indent=1, sort_keys=True, ensure_ascii=False) + "\n")

n_fail = sum(c["fail"] for c in cases)
n_json = sum("json" in c for c in cases)
print(
    f"{len(cases)} cases: {n_json} with expected JSON, {n_fail} must-fail, "
    f"{len(cases) - n_json - n_fail} valid without JSON representation",
    file=sys.stderr,
)
PY

if $check_only; then
  if ! diff -q "$OUT_FILE" "$work_dir/out.json" >/dev/null 2>&1; then
    echo "error: $OUT_FILE is out of date — run ./scripts/sync-yaml-test-suite.sh" >&2
    exit 1
  fi
  echo "corpus is up to date" >&2
else
  mkdir -p "$(dirname "$OUT_FILE")"
  cp "$work_dir/out.json" "$OUT_FILE"
  echo "wrote $OUT_FILE" >&2
fi
