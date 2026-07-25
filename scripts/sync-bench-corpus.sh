#!/usr/bin/env bash
#
# Populate the real-workload benchmark corpus (#301) from its manifest.
#
# The corpus drives shape-statistics (`succinctly dev bench corpus-stats`) and
# corpus-representative end-to-end benchmarks. Provenance (source URL + license)
# and a sha256 content pin for every file live in
# tests/data/bench-corpus/manifest.json.
#
# Two tiers keep the repo lean while staying CI-verifiable:
#   * vendored seed  — tiny files committed under tests/data/bench-corpus/seed/;
#                      copied into the unified corpus root and sha256-verified.
#   * fetched tier   — larger files downloaded on demand into the git-ignored
#                      data/bench/corpus/ and sha256-verified against the pin.
#
# Usage:
#   ./scripts/sync-bench-corpus.sh              # populate data/bench/corpus/
#   ./scripts/sync-bench-corpus.sh --check      # verify vendored seed (offline)
#
# After a full sync, bench the corpus with the existing runners, e.g.:
#   succinctly dev bench yq  --data-dir data/bench/corpus/yaml
#   succinctly dev bench corpus-stats --data-dir data/bench/corpus \
#       --markdown docs/benchmarks/corpus-shape.md

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CORPUS_DIR="$REPO_ROOT/tests/data/bench-corpus"
MANIFEST="$CORPUS_DIR/manifest.json"
SEED_DIR="$CORPUS_DIR/seed"
OUT_DIR="$REPO_ROOT/data/bench/corpus"

check_only=false
[[ "${1:-}" == "--check" ]] && check_only=true

command -v python3 >/dev/null 2>&1 || { echo "error: python3 is required" >&2; exit 1; }
[[ -f "$MANIFEST" ]] || { echo "error: manifest not found: $MANIFEST" >&2; exit 1; }
if ! $check_only; then
  command -v curl >/dev/null 2>&1 || { echo "error: curl is required to fetch" >&2; exit 1; }
fi

# Portable sha256 (Linux: sha256sum; macOS: shasum -a 256).
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Emit one TSV line per manifest entry: file<TAB>sha256<TAB>bytes<TAB>vendored<TAB>url
manifest_rows() {
  python3 - "$MANIFEST" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
files = data["files"] if isinstance(data, dict) else data
if not files:
    sys.exit("error: manifest lists no files")
for e in files:
    print("\t".join([
        e["file"], e["sha256"], str(e["bytes"]),
        "1" if e.get("vendored") else "0", e.get("source_url", ""),
    ]))
PY
}

fail=0
seed_ok=0
fetched=0

while IFS=$'\t' read -r file sha bytes vendored url; do
  seed_path="$SEED_DIR/$file"
  out_path="$OUT_DIR/$file"

  if $check_only; then
    # Offline: verify the committed seed files only.
    [[ "$vendored" == "1" ]] || continue
    if [[ ! -f "$seed_path" ]]; then
      echo "error: vendored seed missing: $seed_path" >&2
      fail=$((fail + 1)); continue
    fi
    got="$(sha256_of "$seed_path")"
    if [[ "$got" != "$sha" ]]; then
      echo "error: sha256 mismatch for $file" >&2
      echo "  expected $sha" >&2
      echo "  got      $got" >&2
      fail=$((fail + 1)); continue
    fi
    seed_ok=$((seed_ok + 1))
    continue
  fi

  # Populate the unified corpus root.
  mkdir -p "$(dirname "$out_path")"
  if [[ "$vendored" == "1" ]]; then
    if [[ ! -f "$seed_path" ]]; then
      echo "error: vendored seed missing: $seed_path" >&2
      fail=$((fail + 1)); continue
    fi
    cp "$seed_path" "$out_path"
  else
    echo "fetching $file ..." >&2
    if ! curl -fsSL --max-time 120 -o "$out_path" "$url"; then
      echo "error: failed to fetch $file from $url" >&2
      fail=$((fail + 1)); continue
    fi
    fetched=$((fetched + 1))
  fi

  got="$(sha256_of "$out_path")"
  if [[ "$got" != "$sha" ]]; then
    echo "error: sha256 mismatch for $file" >&2
    echo "  expected $sha" >&2
    echo "  got      $got  (source: $url)" >&2
    rm -f "$out_path"
    fail=$((fail + 1)); continue
  fi
done < <(manifest_rows)

if [[ $fail -gt 0 ]]; then
  echo "error: $fail corpus file(s) failed — see above" >&2
  exit 1
fi

if $check_only; then
  echo "$seed_ok vendored seed file(s) verified against the manifest" >&2
else
  echo "corpus ready at $OUT_DIR ($fetched fetched)" >&2
fi
