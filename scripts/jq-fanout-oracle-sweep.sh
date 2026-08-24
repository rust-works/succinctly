#!/usr/bin/env bash
#
# Oracle sweep for the binary-fanout interleaving fix (#1481).
#
# Cross-products a set of "consumer" contexts against a set of operand shapes
# and terminating side effects, for both `Expr::Compare` (`==`) and
# `Expr::Arithmetic` (`+`, as the shared-loop representative), running each
# generated filter through both the pinned jq oracle and the built succinctly
# binary and diffing stdout/stderr/exit code. This is the sweep methodology
# from docs/plan/jq-lazy-generator-consumers.md's "Verification approach for
# the follow-up implementation PRs" section, scoped down from its full
# C x G x X cross product to what #1481 specifically needs.
#
# This is a verification tool, not a CI gate: the pinned regression rows in
# tests/jq_cli_tests.rs (test_short_circuit_side_effect_shapes_already_match_jq_820's
# "closed by #1481" section) are what CI actually enforces. Run this manually
# after touching binary-fanout evaluation order to confirm the divergence
# count, and keep it around for the next lazy-evaluation issue instead of
# rebuilding a sweep from scratch again.
#
# Usage:
#   cargo build --release --features cli
#   ./scripts/jq-fanout-oracle-sweep.sh [path-to-succinctly-binary]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PIN="$(cat "$REPO_ROOT/tests/data/jq-golden/JQ_VERSION")"
SUCC="${1:-$REPO_ROOT/target/release/succinctly}"

if [[ ! -x "$SUCC" ]]; then
  echo "error: succinctly binary not found at $SUCC — run: cargo build --release --features cli" >&2
  exit 1
fi

# Prefer the pinned oracle binary directly (macOS ships jq-1.7.1 at
# /usr/bin/jq; a PATH jq, e.g. Homebrew's, is often a newer version) — same
# reasoning as tests/jq_cli_tests.rs's own pinned-oracle comments.
if [[ -x /usr/bin/jq ]] && /usr/bin/jq --version | grep -q "^$PIN"; then
  JQ=/usr/bin/jq
elif command -v jq >/dev/null 2>&1 && jq --version | grep -q "^$PIN"; then
  JQ="$(command -v jq)"
else
  echo "error: no jq matching pin $PIN found at /usr/bin/jq or on PATH" >&2
  exit 1
fi
echo "oracle: $JQ ($("$JQ" --version)), succinctly: $SUCC" >&2

# 14 operand shapes (docs/plan/jq-lazy-generator-consumers.md's own sweep
# methodology), __X__ substituted with each terminator below.
G_SHAPES=(
  '(1, __X__)'
  '(__X__, 1)'
  '(empty, __X__)'
  '(1, 2, __X__)'
  '(1 | __X__)'
  '((1,2) | __X__)'
  '((1, __X__))'
  'if true then (1, __X__) else 9 end'
  'try (1, __X__) catch 9'
  'label $o | (1, __X__)'
  '1 as $v | (1, __X__)'
  'first(1, __X__)'
  'limit(3; 1, __X__)'
  '[1, __X__]'
)

# 7 terminating side effects. Every generated filter is wrapped in an outer
# `label $o | (...)` so a bare `break $o` always has a valid target, even
# though shape 10 above nests its own `label $o` (a deliberate shadowing
# case, not a bug in the sweep).
X_TERMS=(
  '("B"|stderr)'
  '("m"|halt_error(3))'
  'error("e")'
  'break $o'
  'input'
  'debug'
  '7'
)

# Plenty of stdin documents so an `input` terminator never runs the queue dry
# regardless of how many times a shape evaluates it — every case gets the
# same stdin; jq/succinctly alike leave it unread when a filter never calls
# `input`/`inputs`.
STDIN_DOCS='1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20'

total=0
diverged=0
divergence_log=""

run_case() {
  local label="$1" filter="$2"
  total=$((total + 1))

  local jq_out jq_err jq_code succ_out succ_err succ_code
  jq_out="$(printf '%s' "$STDIN_DOCS" | "$JQ" -cn "$filter" 2>/tmp/jq-fanout-sweep.err)" && jq_code=0 || jq_code=$?
  jq_err="$(cat /tmp/jq-fanout-sweep.err)"
  succ_out="$(printf '%s' "$STDIN_DOCS" | "$SUCC" jq -cn "$filter" 2>/tmp/succ-fanout-sweep.err)" && succ_code=0 || succ_code=$?
  succ_err="$(cat /tmp/succ-fanout-sweep.err)"

  if [[ "$jq_out" != "$succ_out" || "$jq_err" != "$succ_err" || "$jq_code" != "$succ_code" ]]; then
    diverged=$((diverged + 1))
    divergence_log+="[$label] $filter
  jq:          out=$jq_out err=$jq_err exit=$jq_code
  succinctly:  out=$succ_out err=$succ_err exit=$succ_code
"
  fi
}

for g in "${G_SHAPES[@]}"; do
  for x in "${X_TERMS[@]}"; do
    operand="${g//__X__/$x}"

    compare_core="10 == ($operand)"
    run_case "compare/bare"  "label \$o | [$compare_core]"
    run_case "compare/first" "label \$o | [first($compare_core)]"
    run_case "compare/IN"    "label \$o | [IN(10; $operand)]"

    arith_core="10 + ($operand)"
    run_case "arith/bare"  "label \$o | [$arith_core]"
    run_case "arith/first" "label \$o | [first($arith_core)]"
  done
done

rm -f /tmp/jq-fanout-sweep.err /tmp/succ-fanout-sweep.err

echo "$divergence_log"
echo "== $diverged / $total cases diverged from $JQ =="
