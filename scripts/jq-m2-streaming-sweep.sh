#!/usr/bin/env bash
#
# jq M2 streaming sweep: the shipped route vs the pinned jq oracle (#1653,
# #2103).
#
# ## What it found
#
# #1653 wanted the M2 route -- a JSON document read from a file or stdin, the
# CLI's default -- to write each output as the evaluator produces it, so
# stdout and stderr interleave the way real jq's lazy generator does. That is
# not only an ordering change: the eager evaluator's `to_owned_with_cursor`
# doubled as a validity gate, so a filter with a native streaming arm skipped
# #1194/#1642 checks its eager twin performed as a side effect of building an
# `OwnedValue`. This sweep is what measured that. It ran each (document,
# filter) case on both routes plus the oracle, and the set it reported -- 19
# rows where the eager route matched jq's exit 5 and the streaming route
# answered at exit 0, all of them a filter that reads nothing it validates --
# is the set #2103 decided.
#
# #2103 took the streaming answer: **a filter validates only what it reads**,
# recorded on its merits (against ADR-0018's own decision order, which favours
# the behaviour given up) in `docs/compliance/jq/limitations.md` under "Every
# M2 filter streams". There is now one route, so there is nothing left to diff
# route-against-route.
#
# ## What it guards now
#
# One leg -- the shipped binary, no environment overrides -- against pinned jq
# 1.7.1, over the same DOCS x FILTERS the flip was measured on (kept verbatim:
# they are the measured shapes, not an illustrative sample). Each case
# contributes one line to a checked-in golden table:
#
#     <doc>\t<filter>\t<jq exit>\t<succinctly exit>\t<succinctly stdout, newlines as |>
#
# Without `--update` the freshly-generated table is diffed against the golden
# and any difference fails. So what this catches is a cell moving from jq's
# exit code to a different one, or output changing, on the shapes where
# succinctly's lazy detection matches jq today.
#
# Most `jq=5 succ=0` cells in the golden are *not* regressions and never were:
# succinctly is a semi-index and detects a malformed document only
# opportunistically, where jq is a full validating parser and rejects every
# malformed document below at parse time whatever the filter. The golden
# records where each cell stands so a *move* is visible; it does not claim the
# cells agree.
#
# This is a verification tool, not a CI gate -- same standing as its sibling
# scripts/jq-fanout-oracle-sweep.sh. The pinned rows in tests/jq_cli_tests.rs
# are what CI enforces.
#
# Usage:
#   cargo build --release --features cli
#   ./scripts/jq-m2-streaming-sweep.sh [--update] [path-to-succinctly-binary]
#
#   --update   regenerate scripts/jq-m2-streaming-sweep.expected in place
#              instead of comparing against it. Inspect the diff before
#              committing it: every changed row is a behaviour change.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PIN="$(cat "$REPO_ROOT/tests/data/jq-golden/JQ_VERSION")"
GOLDEN="$REPO_ROOT/scripts/jq-m2-streaming-sweep.expected"

UPDATE=0
ARGS=()
for arg in "$@"; do
  case "$arg" in
    --update) UPDATE=1 ;;
    *) ARGS+=("$arg") ;;
  esac
done
SUCC="${ARGS[0]:-$REPO_ROOT/target/release/succinctly}"

if [[ ! -x "$SUCC" ]]; then
  echo "error: succinctly binary not found at $SUCC — run: cargo build --release --features cli" >&2
  exit 1
fi

# Prefer the pinned oracle binary directly (macOS ships jq-1.7.1 at
# /usr/bin/jq; a PATH jq, e.g. Homebrew's, is often newer) — same reasoning
# as tests/jq_cli_tests.rs's own pinned-oracle comments.
if [[ -x /usr/bin/jq ]] && /usr/bin/jq --version | grep -q "^$PIN"; then
  JQ=/usr/bin/jq
elif command -v jq >/dev/null 2>&1 && jq --version | grep -q "^$PIN"; then
  JQ="$(command -v jq)"
else
  echo "error: no jq matching pin $PIN found at /usr/bin/jq or on PATH" >&2
  exit 1
fi
echo "oracle: $JQ ($("$JQ" --version)), succinctly: $SUCC" >&2

WORK="$(mktemp -d -t jq-m2-sweep)"
trap 'rm -rf "$WORK"' EXIT
DOC_FILE="$WORK/doc.json"

# Documents. The malformed ones are the three fault kinds the codebase keeps
# deliberately distinct (src/jq/document.rs's normal / decode-failure /
# structurally-malformed split, #1642), plus the delimiter fault #1677 added
# and the duplicate-key case that is legitimately exit 0 in jq too.
DOCS=(
  '{"a":1,"b":2}'                      # well formed
  '[1,2,3]'                            # well formed, array
  '{"a":1,"a":2}'                      # duplicate key: jq collapses, exit 0
  '{123:1,"b":2}'                      # #1194 structurally malformed key
  '{"a":1,"b"}'                        # #1194 unpaired tail
  '{"a":1, invalid}'                   # #1194 malformed tail member
  '{"a" 1, "b":2}'                     # #1677 missing `:` delimiter
  '[1,,3]'                             # #1597 malformed comma, mid-array
  '{"\ud800":1,"\ud800":2}'            # #1642 colliding undecodable keys
  '{"\ud800":1,"b":2}'                 # single undecodable key (preserved)
  '{"a\q":1,"b":2}'                    # invalid escape in key
  '[{"x":"\ud800"}]'                # #2103 lone-surrogate value in an element
)

# Filters. The two groups below were the point of the sweep while there were
# two routes to compare, and are kept because they are the shapes the flip was
# measured on:
#
#   * shapes whose eager twin also forwarded a cursor without materializing,
#     so they answered identically on both routes — the hot default route,
#     which may not change;
#   * shapes whose eager twin fell to `eval_single`'s wildcard, materializing
#     the ambient value via `to_owned_with_cursor` and validating the document
#     as a side effect, where `eval_each_generic` has a native streaming arm
#     that does not. That group is where #2103's 19 rows live.
FILTERS=(
  # forwarded a cursor on both routes
  '.'
  '.a'
  '.b'
  '.[]'
  '.a, .b'
  'keys_unsorted'
  'keys_unsorted[]'
  'keys'
  'length'
  'to_entries'
  'first(keys_unsorted[])'
  'limit(1; keys_unsorted[])'
  'limit(2; keys_unsorted[])'
  'first(.[])'
  'map(.x)'
  'map(.x) | .[]'
  'first(map(.x) | .[])'
  'limit(1; map(.x) | .[])'
  # materializing eager twin / native streaming arm
  '.,.'
  'if . then . else . end'
  'try .'
  'try (1+1) catch "x"'
  'try (1+1)'
  '.?'
  'label $x | .'
  '. as $x | $x'
  'def f: .; f'
  'first(.)'
  'limit(1;.)'
  '[.]'
  '{k: .}'
  '. | .'
  '1+1'
  'keys_unsorted, length'
  'debug'
  '(., debug)'
  # descended shapes: after a first stage that descends, the ambient value is
  # a proper descendant, so a branch forwarding it is not forwarding the root.
  # These measured how far past the root the streaming route could reach.
  '.[]|debug'
  '.[] | (., debug)'
  '.[] | .,.'
  '.[] | if . then . else . end'
  '.[] | try (1+1) catch "x"'
  '.[] | 1+1'
  '.[] | label $x | .'
  '.[] | def f: .; f'
  '.[] | . as $x | $x'
  '.a | (., debug)'
  '.a | if . then . else . end'
  '.a | 1+1'
  '.[] | select(. != null)'
  '.[] | tostring'
)

ACTUAL="$WORK/actual.tsv"
: > "$ACTUAL"

run_case() {
  local doc="$1" filter="$2"
  printf '%s' "$doc" > "$DOC_FILE"

  local jq_code succ_out succ_code

  "$JQ" -c "$filter" "$DOC_FILE" >/dev/null 2>&1 && jq_code=0 || jq_code=$?

  succ_out="$("$SUCC" jq -c "$filter" "$DOC_FILE" 2>/dev/null)" && succ_code=0 || succ_code=$?

  printf '%s\t%s\t%s\t%s\t%s\n' \
    "$doc" "$filter" "$jq_code" "$succ_code" \
    "$(printf '%s' "$succ_out" | tr '\n' '|')" >> "$ACTUAL"
}

for doc in "${DOCS[@]}"; do
  for filter in "${FILTERS[@]}"; do
    run_case "$doc" "$filter"
  done
done

total="$(wc -l < "$ACTUAL" | tr -d ' ')"

if ((UPDATE)); then
  cp "$ACTUAL" "$GOLDEN"
  echo "== $total cases written to $GOLDEN =="
  exit 0
fi

if [[ ! -f "$GOLDEN" ]]; then
  echo "error: golden table $GOLDEN not found — run with --update to create it" >&2
  exit 1
fi

if diff -u "$GOLDEN" "$ACTUAL" > "$WORK/diff.txt"; then
  echo "== $total cases: table matches $(basename "$GOLDEN") =="
  exit 0
fi

sed -e "s#$GOLDEN#expected#" -e "s#$ACTUAL#actual#" "$WORK/diff.txt"
changed="$(grep -c '^[+-][^+-]' "$WORK/diff.txt" || true)"
echo "FAIL: $total cases, $changed changed line(s) vs $(basename "$GOLDEN") — see the diff above." >&2
echo "      Each one is a behaviour change: re-derive it, then re-run with --update." >&2
exit 1
