#!/usr/bin/env bash
#
# Eager-vs-streaming parity sweep for the jq M2 lazy path (#1653).
#
# #1653 wants the M2 route -- a JSON document read from a file or stdin, the
# CLI's default -- to write each output as the evaluator produces it, so
# stdout and stderr interleave the way real jq's lazy generator does. PR
# #1892 converted every other route; the M2 one still calls the *eager*
# `eval_with_cursor`. Switching it to the demand-driven `eval_each_with_cursor`
# is not just an ordering change: the eager evaluator's materializing arms
# double as a validity gate, so arms that stream natively skip #1194/#1642
# checks their eager twins performed as a side effect of building an
# `OwnedValue`. An earlier attempt at the switch was reverted for exactly
# that reason.
#
# This sweep separates the two variables. It runs each (document, filter)
# case three ways -- the eager route, the streaming route, and the pinned jq
# oracle -- and reports:
#
#   * PARITY divergences: eager vs streaming disagree on stdout/stderr/exit.
#     **These are the gate.** Every one is a check the streaming route lost
#     (or gained). Expected result: `0 unexpected`.
#   * ORACLE rows: informational. Succinctly is a semi-index and detects a
#     malformed document only opportunistically, where jq -- a full
#     validating parser -- rejects every document below at parse time, exit
#     5, whatever the filter. So most oracle columns read `jq=5 succ=0`
#     already, on both routes, and closing those is not this issue's job.
#     The rule that matters: **a cell must not move from 5 to 0.**
#
# Both routes are driven out of one binary via `SUCCINCTLY_JQ_M2_EVAL`, so a
# single build measures both and the comparison cannot drift on a stale half.
#
# This is a verification tool, not a CI gate -- same standing as its sibling
# scripts/jq-fanout-oracle-sweep.sh. The pinned rows in tests/jq_cli_tests.rs
# are what CI enforces.
#
# Usage:
#   cargo build --release --features cli
#   ./scripts/jq-m2-streaming-sweep.sh [path-to-succinctly-binary]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PIN="$(cat "$REPO_ROOT/tests/data/jq-golden/JQ_VERSION")"
SUCC="${1:-$REPO_ROOT/target/release/succinctly}"

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
)

# Filters. Two groups, and the split is the point of the sweep.
#
#   * "transparent": shapes whose *eager* twin also forwards a cursor without
#     materializing, so they answer identically on both routes today and must
#     keep doing so — these are the hot default route and may not slow down.
#   * "materializing": shapes whose eager twin falls to `eval_single`'s
#     wildcard, which materializes the ambient value via `to_owned_with_cursor`
#     and so validates the document as a side effect, while `eval_each_generic`
#     has a native streaming arm that does not. This is where the lost checks
#     live.
FILTERS=(
  # transparent
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
  # materializing / native-streaming-arm shapes
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
)

# Attribute a parity divergence to an already-tracked, deliberately-accepted
# gap, or return 1 for "this is new, look at it". Keep every entry tied to an
# issue number.
classify_parity() {
  local doc="$1" filter="$2" eager_out="$3" stream_out="$4" eager_code="$5" stream_code="$6"

  # #1770, generalised by #1653: the streaming route finds a document fault
  # only when the walk reaches it, so outputs produced *before* the fault are
  # already on stdout when it raises, where the eager route returned a single
  # `Error` and printed nothing. The exit code still agrees. Real jq prints
  # nothing either -- it rejects the document at parse time -- so this is a
  # divergence from jq on stdout content, accepted for the same reason #1770
  # accepted `limit(2; keys_unsorted[])` emitting `"a"` alongside exit 5.
  if [[ "$eager_code" == "$stream_code" && -z "$eager_out" && -n "$stream_out" ]]; then
    echo '#1770/#1653 (prefix produced before a lazily-detected document fault)'
    return 0
  fi

  return 1
}

total=0
parity_diverged=0
parity_unexpected=0
oracle_regressed=0
parity_log=""
oracle_log=""
declare -a known_labels=()

run_case() {
  local doc="$1" filter="$2"
  total=$((total + 1))
  printf '%s' "$doc" > "$DOC_FILE"

  local jq_out jq_code eager_out eager_err eager_code stream_out stream_err stream_code

  jq_out="$("$JQ" -c "$filter" "$DOC_FILE" 2>/dev/null)" && jq_code=0 || jq_code=$?

  eager_out="$(SUCCINCTLY_JQ_M2_EVAL=eager "$SUCC" jq -c "$filter" "$DOC_FILE" 2>"$WORK/e.err")" \
    && eager_code=0 || eager_code=$?
  eager_err="$(cat "$WORK/e.err")"

  stream_out="$(SUCCINCTLY_JQ_M2_EVAL=stream "$SUCC" jq -c "$filter" "$DOC_FILE" 2>"$WORK/s.err")" \
    && stream_code=0 || stream_code=$?
  stream_err="$(cat "$WORK/s.err")"

  # Paths appear in diagnostics; strip them so the two legs are comparable.
  eager_err="${eager_err//$DOC_FILE/DOC}"
  stream_err="${stream_err//$DOC_FILE/DOC}"

  if [[ "$eager_out" != "$stream_out" || "$eager_err" != "$stream_err" || "$eager_code" != "$stream_code" ]]; then
    parity_diverged=$((parity_diverged + 1))
    local known
    if known="$(classify_parity "$doc" "$filter" "$eager_out" "$stream_out" "$eager_code" "$stream_code")"; then
      known_labels+=("$known")
    else
      parity_unexpected=$((parity_unexpected + 1))
      parity_log+="[parity] doc=$doc filter=$filter
  eager:   out=$(printf '%s' "$eager_out" | tr '\n' '|') err=$eager_err exit=$eager_code
  stream:  out=$(printf '%s' "$stream_out" | tr '\n' '|') err=$stream_err exit=$stream_code
"
    fi
  fi

  # Directional oracle check: a cell may move toward jq, never away.
  if [[ "$eager_code" == "$jq_code" && "$stream_code" != "$jq_code" ]]; then
    oracle_regressed=$((oracle_regressed + 1))
    oracle_log+="[oracle] doc=$doc filter=$filter — eager matched jq ($jq_code), streaming did not ($stream_code)
"
  fi
}

for doc in "${DOCS[@]}"; do
  for filter in "${FILTERS[@]}"; do
    run_case "$doc" "$filter"
  done
done

printf '%s' "$parity_log"
printf '%s' "$oracle_log"
echo "== $total cases: $parity_unexpected unexpected parity divergence(s), $((parity_diverged - parity_unexpected)) known, $oracle_regressed oracle regression(s) =="
if ((parity_diverged > parity_unexpected)); then
  printf '%s\n' "${known_labels[@]}" | sort | uniq -c | sed 's/^/   known: /'
fi
if ((parity_unexpected > 0 || oracle_regressed > 0)); then
  echo "FAIL: $parity_unexpected unexpected parity divergence(s), $oracle_regressed oracle regression(s) — see above" >&2
  exit 1
fi
