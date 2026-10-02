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
#   * `reduce`'s own INIT position, which has the identical bug (see
#     docs/compliance/jq/limitations.md's #2668 residual, filed as #2899) but
#     has no native, demand-forwarding dispatch arm at all to drive it
#     through -- fixing it needs a new `each_reduce`-style dispatch, not a
#     template row here.
#     Sweeping it would report a permanent "known" divergence and defeat
#     this script's 0-unexpected/0-known contract, so it is pinned as a CLI
#     row in tests/jq_cli_tests.rs's
#     test_nested_short_circuit_consumer_hides_the_stop_2180 instead.
#     `foreach`'s own UPDATE and INIT positions, WP3's original residual, are
#     now closed (#2668) and swept below like every other construct.
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
  #    divergence here is unexpected by definition. --
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
  # -- #2668: CLOSED. fold_step_each (eval.rs) drives UPDATE through
  #    eval_each_owned, with EXTRACT (or the implicit identity push) running
  #    from inside its own sink callback -- so a consumer's stop reaches
  #    UPDATE's own generator, and any ?// bind inside it, before it produces
  #    anything further. --
  'foreach-update::::foreach (1) as $v (0; . + (__G__))'
  # -- #2668: CLOSED. foreach_forks itself takes a ForeachInitDrive closure
  #    (the same shape as its existing ForeachSourceDrive, one level further
  #    out) instead of a pre-collected Vec, so a consumer's stop reaches
  #    INIT's own generator the same way it already reaches the source. --
  'foreach-init::::foreach (1) as $v ((__G__); .+1; .)'
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

# #3293 ("sink-side failure, then clean retry"): a second family, over its
# own input. Each bound variant's first `?//` alternative makes the slice (or
# index) *around* it fail, so only a retry past that failure matches jq; the
# retry then answers, produces nothing, raises, fails to destructure (the
# last alternative's error must surface), or -- the last variant -- answers
# first and fails on the retry a consumer's stop causes. `("A"|stderr)`
# counts the attempts, and stderr is compared exactly. Constructs are
# `LABEL::TAG::TEMPLATE` like W_ENTRIES, `__B__` standing for the bound; TAG
# names the #3293 slice expected to close a construct, blank once it has.
RETRY_INPUT='[10,20,30]'
B_VARIANTS=(
  '([[1]] as [$a] ?// [[$a]] | ("A"|stderr) | $a)'
  '([[1]] as [$a] ?// $b | ("A"|stderr) | $a // empty)'
  '([[1]] as [$a] ?// $b | ("A"|stderr) | $a | if . == null then error("E2") else . end)'
  '([[1]] as [$a] ?// {$z} | ("A"|stderr) | $a)'
  '([1] as [$a] ?// $b | ("A"|stderr) | if $a == null then "x" else $a end)'
)
R_CONSUMERS=(
  '__W__'
  '[first(__W__), 9]'
  '[limit(1; __W__), 9]'
  '[__W__, 9]'
)
R_ENTRIES=(
  # -- slice 6: CLOSED. resolve_slice_expr_sink/drive_slice_bound (eval.rs),
  #    path_context_step_computed_slice (eval_generic.rs), and the consumer
  #    stop each_path_on_owned records for `path(f)`. --
  'path-slice-start::::path(.[__B__:])'
  'path-slice-end::::path(.[:__B__])'
  'path-slice-both::::path(.[__B__:__B__])'
  'del-slice::::del(.[__B__:])'
  'update-slice::::.[__B__:] |= ["x"]'
  'assign-slice::::.[__B__:] = ["x"]'
  'add-assign-slice::::.[__B__:] += ["x"]'
  'alt-assign-slice::::.[__B__:] //= 1'
  'pick-slice::::pick(.[__B__:])'
  'path-slice-optional::::path(.[__B__:]?)'
  'path-slice-then-index::::path(.[__B__:] | .[0])'
  # -- slice 8a: CLOSED. The path-mode `as` bind's source
  #    (resolve_bind_source_sink); its source is untracked in jq, so the
  #    `("A"|stderr) | ...` marker is safe here. --
  'path-bind-source::::path(__B__ as $y | .[$y:])'
  'del-bind-source::::del(__B__ as $y | .[$y:])'
  'update-bind-source::::(__B__ as $y | .[$y:]) |= ["x"]'
  'pick-bind-source::::pick(__B__ as $y | .[$y:])'
  # -- #3471: CLOSED. The read-mode computed slice (`each_slice_expr` in
  #    eval.rs, `each_slice_expr_generic` in eval_generic.rs) pushes each
  #    `(s, e)` pair's slices to the consumer as they are produced, so a
  #    `first`/`limit`/`isempty` stop reaches the bound's `?//`. --
  'slice-start::::.[__B__:]'
  'slice-end::::.[:__B__]'
  'slice-both::::.[__B__:__B__]'
  'slice-start-optional::::.[__B__:]?'
  'slice-fanout-start::::.[(0,1):__B__]'
  'slice-fanout-end::::.[__B__:(2,3)]'
)
# The path-mode computed index (slice 5) is the IX family below, over an object:
# on this array input an array-valued first alternative is jq's `indices` search
# (#3506), so a `.[__B__]` row here never exercised the failure it was written for.

# Fed from a file, never a pipe — see jq-fanout-oracle-sweep.sh's own
# documented SIGPIPE lesson (a filter that never reads `input` leaves the
# writer holding data, killing a `printf | jq` pipeline with SIGPIPE under
# load and recording a phantom exit-code divergence).
#
# Every temp file goes through `mk_tmp`, so one EXIT trap removes all of them and
# a family added later cannot forget to extend it. The stderr captures are
# per-run `mktemp` files too: fixed `/tmp` names collide when two sweeps (or two
# sessions) run at once, and record a phantom divergence.
SWEEP_TMP=()
mk_tmp() { # mk_tmp VAR PREFIX: create a temp file, name it in VAR
  local created
  created="$(mktemp -t "$2")"
  SWEEP_TMP+=("$created")
  printf -v "$1" '%s' "$created"
}
trap 'rm -f "${SWEEP_TMP[@]}"' EXIT
mk_tmp STDIN_FILE jq-alt-retry-sweep-stdin
mk_tmp RETRY_STDIN_FILE jq-alt-retry-sweep-retry-stdin
mk_tmp JQ_ERR_FILE jq-alt-retry-sweep-jq-err
mk_tmp SUCC_ERR_FILE jq-alt-retry-sweep-succ-err
printf '1' > "$STDIN_FILE"
printf '%s' "$RETRY_INPUT" > "$RETRY_STDIN_FILE"

# Attribute a divergence to an already-known, currently-open work package,
# or return 1 for "this is new, look at it". `wp_tag` comes straight from the
# W_ENTRIES/R_ENTRIES tables above (see the design note in the header) rather
# than being re-derived from filter text; a W_ENTRIES tag is #2180's, and an
# R_ENTRIES tag names its own issue.
classify_divergence() {
  local wp_tag="$1"
  if [[ -n "$wp_tag" ]]; then
    case "$wp_tag" in
      '#'*) echo "$wp_tag" ;;
      *) echo "#2180 $wp_tag" ;;
    esac
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
  local label="$1" filter="$2" wp_tag="$3" stdin_file="${4:-$STDIN_FILE}"
  total=$((total + 1))

  local jq_out jq_err jq_code succ_out succ_err succ_code
  jq_out="$("$JQ" -c "$filter" <"$stdin_file" 2>"$JQ_ERR_FILE")" && jq_code=0 || jq_code=$?
  jq_err="$(cat "$JQ_ERR_FILE")"
  succ_out="$("$SUCC" jq -c "$filter" <"$stdin_file" 2>"$SUCC_ERR_FILE")" && succ_code=0 || succ_code=$?
  succ_err="$(cat "$SUCC_ERR_FILE")"

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

for r_entry in "${R_ENTRIES[@]}"; do
  r_rest="$r_entry"
  r_label="${r_rest%%::*}"
  r_rest="${r_rest#*::}"
  r_tag="${r_rest%%::*}"
  r_template="${r_rest#*::}"
  for b in "${B_VARIANTS[@]}"; do
    r_filled="${r_template//__B__/$b}"
    for c in "${R_CONSUMERS[@]}"; do
      run_case "$r_label" "${c//__W__/$r_filled}" "$r_tag" "$RETRY_STDIN_FILE"
    done
  done
done

# #3293 slice 7 (path-mode conditions): the same "sink-side failure, then
# clean retry" shape, with the `?//` in an `if`/`select` *condition* and the
# failure raised downstream of it, in the branch (or after the `select`). Only
# a retry past that failure matches jq. `__T__` stands for the condition; its
# first alternative is truthy and its retry is falsy (or produces nothing,
# raises, or fails to destructure), so the abandoned alternative's stashed
# verdict must not outrank what the retry did. Own input, since the
# constructs navigate `.a`. Entries are `LABEL::TAG::TEMPLATE`, like R_ENTRIES
# (slice 7 closed every one, so none is tagged now).
COND_INPUT='{"a":{"a":1}}'
mk_tmp COND_STDIN_FILE jq-alt-retry-sweep-cond-stdin
printf '%s' "$COND_INPUT" > "$COND_STDIN_FILE"
T_VARIANTS=(
  '([1] as $q ?// $b | ("A"|stderr) | $q)'
  '([1] as $q ?// $b | ("A"|stderr) | $q // empty)'
  '([1] as $q ?// $b | ("A"|stderr) | $q | if . == null then error("E2") else . end)'
  '(1 as $x ?// [$y] | ("A"|stderr) | $x)'
  '([1] as [$a] ?// $b | ("A"|stderr) | if $a == null then "x" else $a end)'
)
T_ENTRIES=(
  'path-if::::path(if __T__ then error("E") else .a end)'
  'path-if-comma::::path(if __T__ then (.a, error("E")) else .a end)'
  'path-select::::path(.a | select(__T__) | error("E"))'
  'del-if::::del(if __T__ then error("E") else .a end)'
  'del-select::::del(.a | select(__T__) | error("E"))'
  'update-if::::(if __T__ then error("E") else .a end) |= 5'
  'assign-if::::(if __T__ then error("E") else .a end) = 5'
  'pick-if::::pick(if __T__ then error("E") else .a end)'
  # The failure is not downstream of the condition, but a consumer's `first`
  # stop is: jq retries the `?//` on that break too, so a retry that raises
  # must still outrank the stashed stop.
  'path-if-else::::path(if __T__ then .a else error("E") end)'
  'path-select-pass::::path(.a | select(__T__) | .a)'
  # Control: `?` catches the failure before the condition sees it.
  'path-if-optional::::path(if __T__ then error("E") else .a end)?'
)
T_CONSUMERS=(
  '__W__'
  '[first(__W__), 9]'
  '[limit(1; __W__), 9]'
  '[__W__, 9]'
)
for t_entry in "${T_ENTRIES[@]}"; do
  t_rest="$t_entry"
  t_label="${t_rest%%::*}"
  t_rest="${t_rest#*::}"
  t_tag="${t_rest%%::*}"
  t_template="${t_rest#*::}"
  for t in "${T_VARIANTS[@]}"; do
    t_filled="${t_template//__T__/$t}"
    for c in "${T_CONSUMERS[@]}"; do
      run_case "$t_label" "${c//__W__/$t_filled}" "$t_tag" "$COND_STDIN_FILE"
    done
  done
done

# #3293 slice 8a (path-mode folds): a `?//` in a fold's SOURCE or INIT whose
# first alternative sends the fold's UPDATE into an error the retry avoids.
# jq path-tracks a fold's source, so the attempt marker is `("A"|stderr) as $_ |`
# here (a bare `("A"|stderr) | ...` resets jq's path register), and the source
# variants leave out a destructuring alternative (#3489 words that error
# differently). `__S__` is the source, `__I__` INIT; the first alternative of
# each binds `[1]` (or leaves INIT whole), which UPDATE refuses.
S_VARIANTS=(
  '([1] as $q ?// $b | ("A"|stderr) as $_ | $q)'
  '([1] as $q ?// $b | ("A"|stderr) as $_ | $q // empty)'
  '([1] as $q ?// $b | ("A"|stderr) as $_ | $q | if . == null then error("E2") else . end)'
)
I_VARIANTS=(
  '([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then . else .[1:] end)'
  '([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then . else empty end)'
  '([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then . else error("E2") end)'
  '([1] as $q ?// {$z} | ("A"|stderr) as $_ | .)'
)
S_ENTRIES=(
  'path-reduce-source::::path(reduce __S__ as $x (.; .[$x:]))'
  'path-foreach-source::::path(foreach __S__ as $x (.; .[$x:]))'
  'path-foreach-extract-source::::path(foreach __S__ as $x (.; .; .[$x:]))'
  'del-reduce-source::::del(reduce __S__ as $x (.; .[$x:]))'
  'update-foreach-source::::(foreach __S__ as $x (.; .[$x:])) |= 5'
)
# The step *succeeds* on the first alternative and a consumer's stop reaches the
# source's `?//` (#3293 review). Only the retries that produce nothing or raise:
# an answering retry over-delivers in jq (a documented, unreproduced shape).
SOK_VARIANTS=("${S_VARIANTS[1]}" "${S_VARIANTS[2]}")
SOK_ENTRIES=(
  'path-foreach-source-step-ok::::path(foreach __S__ as $x (.; .[0:]))'
  'path-foreach-extract-source-step-ok::::path(foreach __S__ as $x (.; .; .[0:]))'
)
I_ENTRIES=(
  'path-reduce-init::::path(reduce 1 as $x (__I__; if length>2 then error("E") else . end))'
  'path-foreach-init::::path(foreach 1 as $x (__I__; if length>2 then error("E") else . end))'
  'path-foreach-extract-init::::path(foreach 1 as $x (__I__; .; if length>2 then error("E") else . end))'
  'del-reduce-init::::del(reduce 1 as $x (__I__; if length>2 then error("E") else . end))'
)
for fold_family in S SOK I; do
  if [[ "$fold_family" == S ]]; then entries=("${S_ENTRIES[@]}"); variants=("${S_VARIANTS[@]}"); token=__S__
  elif [[ "$fold_family" == SOK ]]; then entries=("${SOK_ENTRIES[@]}"); variants=("${SOK_VARIANTS[@]}"); token=__S__
  else entries=("${I_ENTRIES[@]}"); variants=("${I_VARIANTS[@]}"); token=__I__; fi
  for f_entry in "${entries[@]}"; do
    f_rest="$f_entry"
    f_label="${f_rest%%::*}"
    f_rest="${f_rest#*::}"
    f_tag="${f_rest%%::*}"
    f_template="${f_rest#*::}"
    for v in "${variants[@]}"; do
      f_filled="${f_template//$token/$v}"
      for c in "${R_CONSUMERS[@]}"; do
        run_case "$f_label" "${c//__W__/$f_filled}" "$f_tag" "$RETRY_STDIN_FILE"
      done
    done
  done
done

# #3293 slice 8b (`recurse`): `f`'s first `?//` alternative yields a child whose
# own expansion raises `E`; the retry answers, produces nothing, raises, or
# fails to destructure. The marker is `as $_` (jq path-tracks `path(recurse(f))`).
# `__F__` stands for `f`; input is RETRY_INPUT (`[10,20,30]`).
RC_VARIANTS=(
  'if length==3 then ([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then .[0:1] else .[1:] end) elif length==1 then error("E") else empty end'
  'if length==3 then ([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then .[0:1] else empty end) elif length==1 then error("E") else empty end'
  'if length==3 then ([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then .[0:1] else error("E2") end) elif length==1 then error("E") else empty end'
  'if length==3 then ([1] as $q ?// {$z} | ("A"|stderr) as $_ | .[0:1]) elif length==1 then error("E") else empty end'
)
RC_ENTRIES=(
  'recurse::::[recurse(__F__)]'
  'recurse-cond::::[recurse(__F__; true)]'
  'path-recurse::::[path(recurse(__F__))]'
  'del-recurse::::del(recurse(__F__))'
  'update-recurse::::(recurse(__F__)) |= .'
)
# `?//` in `recurse`'s `cond` (the gate): alternative 1 lets the child through
# and its expansion raises `E`; the retry rejects it, passes another, raises,
# or fails to destructure.
RG_VARIANTS=(
  '([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then true else length>1 end)'
  '([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then true else empty end)'
  '([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then true else error("E2") end)'
  '([1] as $q ?// {$z} | ("A"|stderr) as $_ | true)'
)
RG_ENTRIES=(
  'recurse-gate::::[recurse(if length==3 then .[0:1], .[1:] elif length==1 then error("E") else empty end; __C__)]'
  'path-recurse-gate::::[path(recurse(if length==3 then .[0:1], .[1:] elif length==1 then error("E") else empty end; __C__))]'
)
for rg_entry in "${RG_ENTRIES[@]}"; do
  rg_rest="$rg_entry"
  rg_label="${rg_rest%%::*}"
  rg_rest="${rg_rest#*::}"
  rg_tag="${rg_rest%%::*}"
  rg_template="${rg_rest#*::}"
  for v in "${RG_VARIANTS[@]}"; do
    rg_filled="${rg_template//__C__/$v}"
    for c in "${R_CONSUMERS[@]}"; do
      run_case "$rg_label" "${c//__W__/$rg_filled}" "$rg_tag" "$RETRY_STDIN_FILE"
    done
  done
done

for rc_entry in "${RC_ENTRIES[@]}"; do
  rc_rest="$rc_entry"
  rc_label="${rc_rest%%::*}"
  rc_rest="${rc_rest#*::}"
  rc_tag="${rc_rest%%::*}"
  rc_template="${rc_rest#*::}"
  for v in "${RC_VARIANTS[@]}"; do
    rc_filled="${rc_template//__F__/$v}"
    for c in "${R_CONSUMERS[@]}"; do
      run_case "$rc_label" "${c//__W__/$rc_filled}" "$rc_tag" "$RETRY_STDIN_FILE"
    done
  done
done

# #3503 (`while`/`until`): `update`'s first `?//` alternative yields a state whose own
# next round raises `E`; the retry answers, produces nothing, raises, fails to
# destructure, fans out, or the failure is a `halt_error` (never retried past). The
# marker is `as $_`. `__U__` stands for `update`; input is RETRY_INPUT (`[10,20,30]`).
WU_VARIANTS=(
  'if length==3 then ([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then .[0:1] else .[1:] end) elif length==1 then error("E") else [] end'
  'if length==3 then ([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then .[0:1] else empty end) elif length==1 then error("E") else [] end'
  'if length==3 then ([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then .[0:1] else error("E2") end) elif length==1 then error("E") else [] end'
  'if length==3 then ([1] as $q ?// {$z} | ("A"|stderr) as $_ | .[0:1]) elif length==1 then error("E") else [] end'
  'if length==3 then ([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then (.[0:1], .[1:]) else .[1:] end) elif length==1 then error("E") else [] end'
  'if length==3 then ([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then .[0:1] else .[1:] end) elif length==1 then ("h"|halt_error(3)) else [] end'
)
WU_ENTRIES=(
  'while::::while(length>0; __U__)'
  'until::::until(length<=0; __U__)'
  'while-optional::::(while(length>0; __U__))?'
  'until-optional::::(until(length<=0; __U__))?'
)
for wu_entry in "${WU_ENTRIES[@]}"; do
  wu_rest="$wu_entry"
  wu_label="${wu_rest%%::*}"
  wu_rest="${wu_rest#*::}"
  wu_tag="${wu_rest%%::*}"
  wu_template="${wu_rest#*::}"
  for v in "${WU_VARIANTS[@]}"; do
    wu_filled="${wu_template//__U__/$v}"
    for c in "${R_CONSUMERS[@]}"; do
      run_case "$wu_label" "${c//__W__/$wu_filled}" "$wu_tag" "$RETRY_STDIN_FILE"
    done
  done
done

# #3617 (`while`/`until`'s `cond`): the first `?//` alternative picks a branch whose own
# `update` raises `E` on a later round; the retry answers, produces nothing, raises, fails
# to destructure, fans out, or the failure is a `halt_error`. `__K__` stands for `cond`
# (for `until`, negated so the loop runs); `update` is fixed. Input is RETRY_INPUT.
WC_VARIANTS=(
  '([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then length>0 else length>1 end)'
  '([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then length>0 else empty end)'
  '([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then length>0 else error("E2") end)'
  '([1] as $q ?// {$z} | ("A"|stderr) as $_ | length>0)'
  '([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then (length>0, length>1) else length>1 end)'
  'if true then ([1] as $q ?// $b | ("A"|stderr) as $_ | if $q then length>0 else length>1 end) else false end'
)
WC_UPDATES=(
  'if length==3 then .[0:1], .[1:] elif length==1 then error("E") else [] end'
  'if length==3 then .[0:1], .[1:] elif length==1 then ("h"|halt_error(3)) else [] end'
)
WC_ENTRIES=(
  'while-cond::::while(__K__; __U__)'
  'until-cond::::until((__K__)|not; __U__)'
  'while-cond-optional::::(while(__K__; __U__))?'
  'until-cond-optional::::(until((__K__)|not; __U__))?'
)
for wc_entry in "${WC_ENTRIES[@]}"; do
  wc_rest="$wc_entry"
  wc_label="${wc_rest%%::*}"
  wc_rest="${wc_rest#*::}"
  wc_tag="${wc_rest%%::*}"
  wc_template="${wc_rest#*::}"
  for k in "${WC_VARIANTS[@]}"; do
    for u in "${WC_UPDATES[@]}"; do
      wc_filled="${wc_template//__K__/$k}"
      wc_filled="${wc_filled//__U__/$u}"
      for c in "${R_CONSUMERS[@]}"; do
        run_case "$wc_label" "${c//__W__/$wc_filled}" "$wc_tag" "$RETRY_STDIN_FILE"
      done
    done
  done
done

# #3293 slice 5 (path-mode computed index): a `?//` in `.[K]`'s key whose first
# alternative is an array, so indexing the object fails at the sink, and whose
# retry is the string key. The key generators' endings: the retry answers,
# produces nothing, raises, fails to destructure (the last alternative's error
# must surface), and -- the last two -- answers first and fails on the retry a
# consumer's stop causes (the retry raises, or yields an unusable key). Own
# input, since the constructs index an object. Key generators are not
# path-tracked by jq, so the plain `("A"|stderr) | ...` marker is safe.
# Entries are `LABEL::TAG::TEMPLATE`, `__X__` standing for the key.
IX_INPUT='{"a":{"a":1}}'
mk_tmp IX_STDIN_FILE jq-alt-retry-sweep-ix-stdin
printf '%s' "$IX_INPUT" > "$IX_STDIN_FILE"
IX_VARIANTS=(
  '([["a"]] as [$q] ?// [[$q]] | ("A"|stderr) | $q)'
  '([["a"]] as [$q] ?// $b | ("A"|stderr) | $q // empty)'
  '([["a"]] as [$q] ?// $b | ("A"|stderr) | $q | if . == null then error("E2") else . end)'
  '([["a"]] as [$q] ?// {$z} | ("A"|stderr) | $q)'
)
IX_STOP_VARIANTS=(
  '([["a"]] as [$q] ?// $b | ("A"|stderr) | if $q then "a" else error("E2") end)'
  '([["a"]] as [$q] ?// $b | ("A"|stderr) | if $q then "a" else ["x"] end)'
)
IX_ENTRIES=(
  'ix-path::::path(.[__X__])'
  'ix-path-nested::::path(.a | .[__X__])'
  'ix-path-then-field::::path(.[__X__] | .a)'
  'ix-path-optional::::path(.[__X__]?)'
  'ix-del::::del(.[__X__])'
  'ix-assign::::.[__X__] = 5'
  'ix-update::::.[__X__] |= 5'
  'ix-update-nested::::.a[__X__] |= 5'
  'ix-alt-assign::::.[__X__] //= 1'
  'ix-add-assign::::.[__X__] += 1'
  'ix-pick::::pick(.[__X__])'
  'ix-try::::path(try .[__X__] catch .)'
  'ix-bind-source::::path(__X__ as $k | .[$k])'
  'ix-label::::path(label $out | .[__X__])'
  'ix-comma::::path(.[__X__], .a)'
  'ix-def::::path(def f: .[__X__]; f)'
  'ix-first::::path(first(.[__X__]))'
  'ix-limit::::path(limit(1; .[__X__]))'
  'ix-alt::::path(.[__X__] // .a)'
  'ix-if::::path(if true then .[__X__] else . end)'
  'ix-select::::path(.a | select(.a) | .[__X__])'
  'ix-untracked::::path(1 | .[__X__])'
  'ix-reduce::::path(reduce 1 as $x (.; .[__X__]))'
  'ix-recurse::::[path(recurse(if type == "object" then .[__X__] else empty end))]'
  'ix-walk-paths::::path(.. | select(type == "object") | .[__X__])'
  # `paths(f)` streams its paths, so a stopping consumer reaches a retry inside
  # `f` (#3567, and its root pre-check, #3366); the index is only the failing
  # operation.
  'ix-paths::::paths(.[__X__])'
)
# Path-mode `foreach` evaluates its UPDATE before a consumer can stop it
# (#3507), so a `?//` in the UPDATE is never asked to retry on a stop: only the
# stop variants under a `first`/`limit` consumer diverge, and only for that
# reason. The endings that need no stop are swept untagged.
IX_FOREACH_TEMPLATE='path(foreach 1 as $x (.; .[__X__]))'
run_ix_family() {
  local label="$1" tag="$2" template="$3"; shift 3
  local x filled c
  for x in "$@"; do
    filled="${template//__X__/$x}"
    for c in "${R_CONSUMERS[@]}"; do
      run_case "$label" "${c//__W__/$filled}" "$tag" "$IX_STDIN_FILE"
    done
  done
}
for ix_entry in "${IX_ENTRIES[@]}"; do
  ix_rest="$ix_entry"
  ix_label="${ix_rest%%::*}"
  ix_rest="${ix_rest#*::}"
  ix_tag="${ix_rest%%::*}"
  ix_template="${ix_rest#*::}"
  run_ix_family "$ix_label" "$ix_tag" "$ix_template" "${IX_VARIANTS[@]}" "${IX_STOP_VARIANTS[@]}"
done
run_ix_family ix-foreach '' "$IX_FOREACH_TEMPLATE" "${IX_VARIANTS[@]}"
run_ix_family ix-foreach-stop '#3507' "$IX_FOREACH_TEMPLATE" "${IX_STOP_VARIANTS[@]}"

printf '%s' "$divergence_log"
echo "== $total cases vs $JQ: $unexpected unexpected, $((diverged - unexpected)) known, $((total - diverged)) matched =="
if ((diverged > unexpected)); then
  printf '%s\n' "${known_labels[@]}" | sort | uniq -c | sed 's/^/   known: /'
fi
if ((unexpected > 0)); then
  echo "FAIL: $unexpected unexpected divergence(s) — see above" >&2
  exit 1
fi
