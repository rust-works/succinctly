#!/usr/bin/env bash
#
# Oracle sweep for #2180 ("A `?//`-alternatives bind sees a short-circuiting
# consumer's stop only when nothing materializes it first").
#
# Cross-products a set of outer short-circuiting "consumer" contexts C against
# a set of "wrapper" constructs W (one per construct named in #2180's plan,
# https://github.com/rust-works/succinctly/issues/2180) against a set of
# `?//`-alternatives generator variants G, running each generated filter
# `[C[W[G]]]` through both the pinned jq oracle and the built succinctly
# binary and diffing stdout/stderr/exit code. This is the sweep methodology
# from docs/plan/jq-lazy-generator-consumers.md's "Verification approach for
# the follow-up implementation PRs" section, modelled closely on
# scripts/jq-fanout-oracle-sweep.sh (same oracle pinning, same file-fed
# stdin, same classify_divergence + "0 unexpected" contract).
#
# This is a verification tool, not a CI gate: the pinned rows in
# tests/jq_cli_tests.rs's test_nested_short_circuit_consumer_hides_the_stop_2180
# are what CI actually enforces. Run this manually after touching #2180's
# work packages (WP1/WP2a/WP2b/WP3), and keep it around as the sweep that
# decides each of those PRs, per the plan's own recommendation.
#
# **What this is not**: it does not attempt every filter shape from #2180's
# probing. Two kinds of row stay in tests/jq_cli_tests.rs only:
#
#   * The `foreach ... as $x ?// $y (...)` *pattern*-position bind, which
#     can't be a separable __G__ substitution the way every other row's
#     *source*-position bind can. WP3 added it anyway, as a constant
#     template (see the `foreach-pattern` entry below) -- the three G
#     variants then generate three identical cases per consumer, which is
#     harmless and keeps the construct in the same table as its siblings.
#   * `foreach`'s UPDATE and INIT positions, which WP3 deliberately left
#     eager (see docs/compliance/jq/limitations.md). Sweeping them would
#     report permanent "known" divergences and defeat this script's
#     0-unexpected/0-known contract, so they are pinned as CLI rows in
#     tests/jq_cli_tests.rs's
#     test_nested_short_circuit_consumer_hides_the_stop_2180 instead.
#
# **Design note, vs. jq-fanout-oracle-sweep.sh's classify_divergence**: that
# script's classify_divergence greps the generated filter text for a known
# marker substring (e.g. `*debug*`). Here, every wrapper construct W is
# generated from a table that already records which work package (if any)
# is expected to close it, so classify_divergence is handed that WP tag
# directly instead of re-deriving it from the filter text -- more robust
# against three different generator variants sharing overlapping substrings
# (e.g. `1 as $x` is a prefix of two of the three G's). The "attribution
# list" the task brief asks for is therefore the WP_TAG column of
# W_ENTRIES below, not a grep list -- delete/blank a construct's tag there
# once its work package lands, and its rows must then actually match jq or
# the sweep fails with a fresh "unexpected" divergence.
#
# **Expected result: `0 unexpected`.** Divergences attributable to a still-open
# #2180 work package are counted and printed separately so the headline number
# stays a pass/fail signal. The script exits non-zero if any *unexpected*
# divergence appears.
#
# Usage:
#   cargo build --release --features cli   # or debug, for local iteration
#   ./scripts/jq-alt-retry-oracle-sweep.sh [path-to-succinctly-binary]

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

# 7 outer short-circuiting consumers, __W__ substituted with each wrapper
# construct below.
#
# `limit(3; ...)` is the one that asks for more than a single output, added by
# #2180 WP3's review: every other consumer here is satisfied by the first
# value, so none of them can reach a `foreach`'s *second* INIT fork, which is
# exactly where WP3's record-and-replay lost a retry.
C_SHAPES=(
  'first(__W__)'
  'isempty(__W__)'
  'limit(1; __W__)'
  'limit(3; __W__)'
  'nth(0; __W__)'
  'any(__W__; .)'
  'IN(__W__)'
)

# 3 `?//`-alternatives generator variants, every construct's __G__.
G_VARIANTS=(
  '1 as $x ?// $y | 1'
  '1 as $x ?// [$y] ?// $z | 1'
  '[1] as [$a] ?// $a | 1'
)

# Wrapper constructs W, one per construct named in #2180's plan (issue
# comment dated 2026-09-08), each `LABEL::WP_TAG::TEMPLATE`. `::` (not `|`,
# since several templates contain a bare pipe) is the field separator.
# WP_TAG is empty for a construct the plan already confirms matches jq
# ("Confirmed correct" / the `limit` control) -- any divergence there is
# unexpected by definition. Delete a row's WP_TAG (blank it) once that work
# package lands; its rows must then match jq or the sweep will fail loudly.
W_ENTRIES=(
  # -- WP1: CLOSED. each_first/each_nth/each_isempty/
  #    each_any_all_gen_cond/each_upper_in (eval.rs) and
  #    each_first_generic/each_nth_generic + bridge_to_each_owned_flow
  #    (eval_generic.rs) all forward demand now, so these rows must match
  #    jq -- their WP_TAG is blanked and any divergence here is unexpected
  #    by definition. --
  'nested-first::::first(__G__)'
  'nested-nth::::nth(0; __G__)'
  'nested-isempty::::isempty(__G__)'
  'nested-any::::any(__G__; .)'
  'nested-IN::::IN(__G__)'
  'nested-IN-src::::IN(1; __G__)'
  # -- confirmed correct: limit is the one consumer that already
  #    demand-forwarded before WP1 (each_limit, #1462/#1596); the worked
  #    example WP1's five arms copy. --
  'nested-limit::::limit(1; __G__)'
  # -- WP2a: CLOSED. each_alternative and each_boolean (eval.rs, the
  #    latter over the new shared demand-driven boolean_fanout_each), plus
  #    eval_each_generic's own Alternative/And/Or arms -- so these rows must
  #    match jq now and any divergence here is unexpected by definition. --
  'alt-fallback::::(__G__)//9'
  'alt-right::::null // (__G__)'
  'and-left::::(__G__) and true'
  'and-right::::true and (__G__)'
  'or-left::::(__G__) or false'
  'or-right::::false or (__G__)'
  # -- WP2b: CLOSED. each_if/each_as/each_as_pattern (+ generic twins) now
  #    drive cond/the bound source through eval_each; Select, Negate,
  #    IndexExpr (key), StringInterpolation and Object all gained
  #    demand-forwarding arms in both files, and eval_each_generic gained
  #    the Range arm eval.rs's each_range already had (#1556) -- so these
  #    rows must match jq now and any divergence here is unexpected by
  #    definition. --
  'if-cond::::if (__G__) then 5 else 6 end'
  'as-source::::(__G__) as $v | $v'
  'as-pattern-source::::(__G__) as [$a] ?// $a | $a'
  'select-arg::::select((__G__) == 1)'
  'negate::::-(__G__)'
  'index-key::::([1]) as $arr | $arr[(__G__)-1]'
  'string-interp::::"\(__G__)"'
  'object-value::::{a:(__G__)} | .a'
  'range-bound::::range((__G__); 3)'
  # -- WP3: CLOSED. each_foreach (eval.rs) and each_foreach_generic
  #    (eval_generic.rs) drive the source through eval_each/eval_each_generic
  #    and each step's EXTRACT through eval_each_owned, over the one shared
  #    fold loop foreach_forks -- which, since WP3's review, both eager entry
  #    points drive the same way too; the sink's Demand::Stop is treated by
  #    foreach's own ?// exactly as an escaping Control::Break is, with the
  #    same state threading -- so these rows must match jq now and any
  #    divergence here is unexpected by definition. UPDATE and INIT stay
  #    eager and are pinned as CLI rows instead (header note above). --
  'foreach-source::::foreach (__G__) as $v (0; .+$v; .)'
  # #2180 WP3's review: a *later* INIT fork's own consumer stop has to reach
  # the source bind just as the first fork's does, and an element a
  # source-side `?//` re-offers must not be counted twice. WP3 drove the
  # source once and replayed a recording for later forks, which failed both
  # (`[1,3,101]` and `[1,6,7]` respectively); only the `limit(3; ...)`
  # consumer above is unsatisfied early enough to see either.
  'foreach-source-multi-init::::foreach (__G__, 2) as $v ((0,100); .+$v; .)'
  'foreach-pattern-multi-init::::foreach (1, 2) as $x ?// $y ((0,100); .+1; .)'
  'foreach-retry-then-replay::::foreach (__G__) as $v ((0, 5); if . == 0 then error("boom") else .+1 end; .)'
  # The pattern-position bind: __G__ does not appear (foreach's own `?//` is
  # the bind under test), so the three G variants generate three identical
  # cases per consumer. See the header note.
  'foreach-pattern::::foreach (1) as $x ?// $y (0; .+1; .)'
  'foreach-extract::::foreach (1) as $v (0; .; (__G__))'
  # -- confirmed correct, deliberately out of scope (limitations.md) --
  'collector::::[(__G__)] | .[]'
  'reduce::::reduce (__G__) as $v (0; .+$v)'
  'last::::last(__G__)'
  'try-optional::::(__G__)?'
  'pipe::::(__G__) | .'
  'arithmetic::::(__G__) + 0'
  'def-defcall::::def f: __G__; f'
  'if-branch::::if true then (__G__) else 9 end'
  'tostring::::(__G__) | tostring'
  'not::::(__G__) | not'
  # Plain identity wrapper: single-level is correct throughout (it's the
  # nesting that breaks it, not the bind itself).
  'identity::::__G__'
)

# Fed from a file, never a pipe — see jq-fanout-oracle-sweep.sh's own
# documented SIGPIPE lesson (a filter that never reads `input` leaves the
# writer holding data, killing a `printf | jq` pipeline with SIGPIPE under
# load and recording a phantom exit-code divergence).
STDIN_FILE="$(mktemp -t jq-alt-retry-sweep-stdin)"
trap 'rm -f "$STDIN_FILE" /tmp/jq-alt-retry-sweep.err /tmp/succ-alt-retry-sweep.err' EXIT
printf '1' > "$STDIN_FILE"

# Attribute a divergence to an already-known, currently-open #2180 work
# package, or return 1 for "this is new, look at it". `wp_tag` comes
# straight from the W_ENTRIES table above (see the design note in the
# header) rather than being re-derived from filter text.
classify_divergence() {
  local wp_tag="$1"
  if [[ -n "$wp_tag" ]]; then
    echo "#2180 $wp_tag"
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
  local label="$1" filter="$2" wp_tag="$3"
  total=$((total + 1))

  local jq_out jq_err jq_code succ_out succ_err succ_code
  jq_out="$("$JQ" -c "$filter" <"$STDIN_FILE" 2>/tmp/jq-alt-retry-sweep.err)" && jq_code=0 || jq_code=$?
  jq_err="$(cat /tmp/jq-alt-retry-sweep.err)"
  succ_out="$("$SUCC" jq -c "$filter" <"$STDIN_FILE" 2>/tmp/succ-alt-retry-sweep.err)" && succ_code=0 || succ_code=$?
  succ_err="$(cat /tmp/succ-alt-retry-sweep.err)"

  if [[ "$jq_out" != "$succ_out" || "$jq_err" != "$succ_err" || "$jq_code" != "$succ_code" ]]; then
    diverged=$((diverged + 1))
    local known
    if known="$(classify_divergence "$wp_tag")"; then
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

for w_entry in "${W_ENTRIES[@]}"; do
  # Split on the first two `::` occurrences only (a template may itself
  # contain `:` inside object construction, e.g. `{a:(__G__)}`).
  w_rest="$w_entry"
  w_label="${w_rest%%::*}"
  w_rest="${w_rest#*::}"
  wp_tag="${w_rest%%::*}"
  w_template="${w_rest#*::}"
  for g in "${G_VARIANTS[@]}"; do
    w_filled="${w_template//__G__/$g}"
    for c in "${C_SHAPES[@]}"; do
      filter="[${c//__W__/$w_filled}]"
      run_case "$w_label" "$filter" "$wp_tag"
    done
  done
done

printf '%s' "$divergence_log"
echo "== $total cases vs $JQ: $unexpected unexpected, $((diverged - unexpected)) known, $((total - diverged)) matched =="
if ((diverged > unexpected)); then
  printf '%s\n' "${known_labels[@]}" | sort | uniq -c | sed 's/^/   known: /'
fi
if ((unexpected > 0)); then
  echo "FAIL: $unexpected unexpected divergence(s) — see above" >&2
  exit 1
fi
