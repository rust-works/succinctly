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
# after touching binary-fanout evaluation order, and keep it around for the
# next lazy-evaluation issue instead of rebuilding a sweep from scratch again.
#
# **Do not blanket-wrap the generated filters in `label $o | ...`.** An
# earlier draft did, so that a bare `break $o` terminator always had a target.
# `Expr::Label` has no native `eval_single` arm in `src/jq/eval_generic.rs`
# (see the `Expr::Array` arm's own comment there), so a `label`-prefixed
# filter falls to that module's wildcard and is bridged wholesale into
# `src/jq/eval.rs` -- which silently routed every case away from
# `eval_generic.rs`, the very copy of the fanout loop #1481 calls out as
# "what a top-level `==` in the CLI actually reaches". The wrapper is
# therefore applied to the `break $o` terminator only, and those rows alone
# cannot speak for `eval_generic.rs`.
#
# **Expected result: `0 unexpected`.** Divergences attributable to an already
# known, separately tracked gap are counted and printed separately (see
# `classify_divergence` below) so the headline number stays a pass/fail
# signal rather than something a reader has to re-triage by hand. The script
# exits non-zero if any *unexpected* divergence appears.
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

# 7 terminating side effects.
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
#
# **Fed from a file, never a pipe.** `printf ... | jq` looks equivalent and is
# not: most filters here never read stdin, so the consumer exits with the
# writer still holding data, `printf` dies of SIGPIPE, and `set -o pipefail`
# promotes that 141 to the pipeline's status — which this script would then
# record as the *oracle's* exit code. It is load-dependent, so it passes in
# isolation and fails when something else is using the machine: 5 phantom
# "divergences" whose stdout and stderr matched exactly, differing only in an
# exit code neither binary actually returned. Same family as the
# `cargo test | grep | tail` false-green. A redirect has no writer to kill.
STDIN_DOCS='1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20'
STDIN_FILE="$(mktemp -t jq-fanout-sweep-stdin)"
trap 'rm -f "$STDIN_FILE" /tmp/jq-fanout-sweep.err /tmp/succ-fanout-sweep.err' EXIT
printf '%s' "$STDIN_DOCS" > "$STDIN_FILE"

# Attribute a divergence to an already-tracked gap, or return 1 for "this is
# new, look at it". Keep every entry tied to an issue number: an unattributed
# entry here would turn this sweep back into the unreadable count it was.
classify_divergence() {
  local filter="$1"

  # #1594: `debug`/`debug(msg)` never write `["DEBUG:",value]` to stderr at
  # all, so every `debug`-terminated case diverges on stderr regardless of
  # evaluation order. Unrelated to fanout; drop this branch once #1594 lands.
  if [[ "$filter" == *debug* ]]; then
    echo '#1594 (debug writes nothing to stderr)'
    return 0
  fi

  return 1
}

total=0
diverged=0
unexpected=0
divergence_log=""
declare -a known_labels=()

run_case() {
  local label="$1" filter="$2"
  total=$((total + 1))

  local jq_out jq_err jq_code succ_out succ_err succ_code
  jq_out="$("$JQ" -cn "$filter" <"$STDIN_FILE" 2>/tmp/jq-fanout-sweep.err)" && jq_code=0 || jq_code=$?
  jq_err="$(cat /tmp/jq-fanout-sweep.err)"
  succ_out="$("$SUCC" jq -cn "$filter" <"$STDIN_FILE" 2>/tmp/succ-fanout-sweep.err)" && succ_code=0 || succ_code=$?
  succ_err="$(cat /tmp/succ-fanout-sweep.err)"

  if [[ "$jq_out" != "$succ_out" || "$jq_err" != "$succ_err" || "$jq_code" != "$succ_code" ]]; then
    diverged=$((diverged + 1))
    local known
    if known="$(classify_divergence "$filter")"; then
      known_labels+=("$known")
      return
    fi
    unexpected=$((unexpected + 1))
    divergence_log+="[$label] $filter
  jq:          out=$jq_out err=$jq_err exit=$jq_code
  succinctly:  out=$succ_out err=$succ_err exit=$succ_code
"
  fi
}

for g in "${G_SHAPES[@]}"; do
  for x in "${X_TERMS[@]}"; do
    operand="${g//__X__/$x}"

    # `break $o` is the one terminator that cannot stand on its own, so those
    # cases — and only those — get an enclosing label. See the header: a
    # blanket wrapper would divert the whole sweep away from eval_generic.rs.
    # Shape 10 nests its own `label $o`, making that combination a deliberate
    # shadowing case rather than a bug in the sweep.
    if [[ "$x" == 'break $o' ]]; then
      open='label $o | '
    else
      open=''
    fi

    compare_core="10 == ($operand)"
    run_case "compare/bare"  "$open[$compare_core]"
    run_case "compare/first" "$open[first($compare_core)]"
    run_case "compare/IN"    "$open[IN(10; $operand)]"

    arith_core="10 + ($operand)"
    run_case "arith/bare"  "$open[$arith_core]"
    run_case "arith/first" "$open[first($arith_core)]"
  done
done

printf '%s' "$divergence_log"
echo "== $total cases vs $JQ: $unexpected unexpected, $((diverged - unexpected)) known =="
if ((diverged > unexpected)); then
  printf '%s\n' "${known_labels[@]}" | sort | uniq -c | sed 's/^/   known: /'
fi
if ((unexpected > 0)); then
  echo "FAIL: $unexpected unexpected divergence(s) — see above" >&2
  exit 1
fi
