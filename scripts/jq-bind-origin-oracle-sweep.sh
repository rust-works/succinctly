#!/usr/bin/env bash
#
# Oracle matrix for `$var` node identity in path position (#2042, #1573).
#
# A variable bound from a *navigated* position (`.a as $y`) is usable in
# `path()`/`del()`/an assignment target exactly where jq's `jv_identical`
# says its value *is* the node the path register currently points at.
# `OwnedValue` has no node identity, so succinctly witnesses it as the pair
# (resolver invocation, absolute bind path) -- `Origin::At` on
# `Expr::TrackedVar` -- and this matrix is the acceptance test for that
# witness: every row is a shape where value equality and node identity
# disagree, or where the frame the bind path was recorded in matters.
#
# Rows are classified by *direction* against the pinned oracle
# (`/usr/bin/jq` 1.7.1), because the two directions are not symmetric:
#
#   agree        same exit and same stdout
#   fabricate    jq refuses, succinctly answers   -- the direction `del()`/`=`
#                then write through; never acceptable
#   mismatch     both answer, differently         -- never acceptable
#   refuse-only  jq answers, succinctly refuses   -- safe; each one must be
#                listed in REFUSE_ONLY below with the reason, so a *new* one
#                fails the sweep too
#
# The matrix is deliberately hand-written (each row was probed live before
# it was added); `scripts/jq-bind-origin-fuzz.py` is the randomised
# companion whose alphabet covers the same constructs.
#
# Usage:
#   cargo build --release --features cli
#   ./scripts/jq-bind-origin-oracle-sweep.sh                 # TSV + summary; exit 1 on fabricate/mismatch/new refuse-only
#   ./scripts/jq-bind-origin-oracle-sweep.sh --list-cases    # rows only, no binaries needed
#
# Env: SUCCINCTLY (default target/release/succinctly), JQ (default /usr/bin/jq).

set -u
cd "$(dirname "$0")/.."
SUCCINCTLY="${SUCCINCTLY:-target/release/succinctly}"
JQ="${JQ:-/usr/bin/jq}"

# id <TAB> input <TAB> filter
CASES=$(cat <<'CASES_EOF'
same-node-sibling-pipe	{"a":{"b":1},"c":{"b":1}}	path(.a as $y | .a | $y)
equal-sibling-pipe	{"a":{"b":1},"c":{"b":1}}	path(.a as $y | .c | $y)
bare-var-at-root	{"a":{"b":1}}	path(.a as $y | $y)
different-value-sibling	{"a":{"b":1},"d":{"b":2}}	path(.a as $y | .d | $y)
navigate-after-var	{"a":{"b":1}}	path(.a as $y | .a | $y.b)
rebind-between	{"a":{"b":1}}	path(.a as $y | .a | . as $w | $y)
first-between	{"a":{"b":1}}	path(.a as $y | first(.a) | $y)
literal-between	{"a":{"b":1}}	path(.a as $y | .a | 5 | $y)
literal-between-navigate	{"a":{"b":1}}	path(.a as $y | .a | 5 | $y.b)
register-moved-by-field	{"a":{"b":1}}	path(.a as $y | .a | .b | $y)
reduce-init-same-node	{"a":{"b":1}}	path(.a as $y | reduce (1) as $i (.a; $y))
reduce-init-root	{"a":{"b":1}}	path(.a as $y | reduce (1) as $i (.; $y))
reduce-init-equal-sibling	{"a":{"b":1},"c":{"b":1}}	path(.a as $y | reduce (1) as $i (.c; $y))
reduce-after-navigate	{"a":{"b":1}}	path(.a as $y | .a | reduce (1) as $i (.; $y))
reduce-literal-then-var	{"a":{"b":1}}	path(.a as $y | reduce (1) as $i (.a; 5 | $y))
reduce-literal-then-var-sibling	{"a":{"b":1},"c":{"b":1}}	path(.a as $y | reduce (1) as $i (.c; 5 | $y))
foreach-init-same-node	{"a":{"b":1}}	path(.a as $y | foreach (1) as $i (.a; $y))
foreach-after-navigate	{"a":{"b":1}}	path(.a as $y | .a | foreach (1) as $i (0; $y; .))
select-passthrough	{"a":{"b":1}}	path(.a as $y | .a | select(true) | $y)
if-passthrough	{"a":{"b":1}}	path(.a as $y | .a | if true then $y else 1 end)
if-passthrough-sibling	{"a":{"b":1},"c":{"b":1}}	path(.a as $y | .c | if true then $y else 1 end)
literal-then-if	{"a":{"b":1}}	path(.a as $y | .a | 5 | if true then $y else 1 end)
literal-then-var-then-select	{"a":{"b":1}}	path(.a as $y | .a | 5 | $y | select(true))
literal-then-var-then-select-sibling	{"a":{"b":1},"c":{"b":1}}	path(.a as $y | .c | 5 | $y | select(true))
literal-then-paren-pipe	{"a":{"b":1}}	path(.a as $y | .a | 5 | ($y | select(true)))
paren-pipe-sibling	{"a":{"b":1},"c":{"b":1}}	path(.a as $y | .c | ($y | select(true)))
paren-navigate	{"a":{"b":1}}	path(.a as $y | .a | ($y | .b))
paren-navigate-sibling	{"a":{"b":1},"c":{"b":1}}	path(.a as $y | .c | ($y | .b))
try-passthrough	{"a":{"b":1}}	path(.a as $y | try .a catch 1 | $y)
label-passthrough	{"a":{"b":1}}	path(.a as $y | label $out | .a | $y)
var-twice	{"a":{"b":1}}	path(.a as $y | .a | $y | $y)
comma-after-var	{"a":{"b":1}}	path(.a as $y | .a | ($y, .b))
comma-stage	{"a":{"b":1}}	path(.a as $y | (.a, .a) | $y)
comma-stage-sibling	{"a":{"b":1},"c":{"b":1}}	[path(.a as $y | (.a, .c) | $y)]
chain-rebind	{"a":{"b":1}}	path(.a as $y | .a | $y as $z | $z)
chain-rebind-sibling	{"a":{"b":1},"c":{"b":1}}	path(.a as $y | .a | $y as $z | .c | $z)
chain-navigate-source	{"a":{"b":{"c":1}}}	path(.a as $y | ($y | .b) as $w | .a.b | $w)
chain-field-source	{"a":{"b":1}}	path(.a as $y | .a | $y.b as $z | .b | $z)
chain-field-source-after-var	{"a":{"b":1}}	path(.a as $y | .a | $y | .b as $z | $z)
inner-binding-outer-var	{"a":{"b":1}}	path(.a as $y | .a | .b as $z | $y)
inner-binding-outer-var-sibling	{"a":{"b":1},"c":{"b":1}}	path(.a as $y | .c | .b as $z | $y)
source-with-navigation	{"a":{"b":1}}	path((.a | .b) as $y | .a.b | $y)
source-getpath	{"a":{"b":1}}	path(getpath(["a"]) as $y | .a | $y)
source-iterate	{"a":[{"b":1}]}	path(.a[] as $y | .a[0] | $y)
source-iterate-equal-elements	{"a":[{"b":1},{"b":1}]}	path(.a[] as $y | .a[] | $y)
source-index	{"a":[[1],[2]]}	path(.a[0] as $y | .a[0] | $y)
source-index-equal-sibling	{"a":[{"b":1},{"b":1}]}	path(.a[0] as $y | .a[1] | $y)
source-comma	{"a":{"b":1},"c":{"b":1}}	path((.a, .c) as $y | .a | $y)
source-error-prefix	{"a":{"b":1}}	path((.a, error("x")) as $y | .a | $y)
source-suspended-tracking	{"a":{"b":1}}	path(([{"a":1}] | .[0] | .a) as $y | .)
source-does-not-move-register	{"a":{"b":1},"b":2}	path(.a as $y | .b)
source-does-not-move-register-identity	{"a":{"b":1},"b":2}	path(.a as $y | .)
root-var-after-navigated-binding	{"a":{"b":1}}	path(. as $x | .a as $y | .a | $x)
nested-as-frame	{"a":{"b":1},"x":{"c":0,"b":1}}	path(.a as $y | .x | (.c as $z | $y))
nested-pipe-frame	{"a":{"b":1},"x":{"a":{"b":1}}}	path(.a as $y | .x | (.a | $y))
nested-pipe-same-node	{"a":{"b":1},"x":{"a":{"b":1}}}	path(.x | (.a as $y | .a | $y))
nested-path-invocation	{"a":{"b":1},"x":{"a":{"b":1}}}	path(.a as $y | .x | path(.a | $y))
nested-as-in-update	{"a":{"b":{"c":1}}}	path(reduce (1) as $i (.a; (.b as $q | .b | $q)))
shadow-inner-sibling	{"a":{"b":1},"c":{"b":1}}	path(.a as $y | .c as $z | .c | $y)
shadow-inner-same	{"a":{"b":1},"c":{"b":1}}	path(.c as $z | .a as $y | .c | $z)
bool-sibling	{"a":true,"c":true}	path(.a as $y | .c | $y)
null-sibling	{"a":null,"c":null}	path(.a as $y | .c | $y)
number-sibling	{"a":1,"c":1}	path(.a as $y | .c | $y)
string-sibling	{"a":"s","c":"s"}	path(.a as $y | .c | $y)
construction-between	{"a":{"b":1}}	path(.a as $y | [.a] | .[0] | $y)
object-construction-between	{"a":{"b":1}}	path(.a as $y | {k: .a} | .k | $y)
tojson-between-sibling	{"a":{"b":1},"c":{"b":1}}	path(.a as $y | .c | tojson | fromjson | $y)
del-same-node	{"a":{"b":1},"c":{"b":1}}	del(.a as $y | .a | $y)
del-equal-sibling	{"a":{"b":1},"c":{"b":1}}	del(.a as $y | .c | $y)
assign-same-node	{"a":{"b":1},"c":{"b":1}}	(.a as $y | .a | $y.b) = 9
assign-equal-sibling	{"a":{"b":1},"c":{"b":1}}	(.a as $y | .c | $y.b) = 9
update-same-node	{"a":{"b":1},"c":{"b":1}}	(.a as $y | .a | $y.b) |= . + 1
multi-binding-partial	[{"b":1},{"b":1}]	path(.[] as $y | .[0] | $y)
value-mode-binding-same-node	{"a":{"b":1}}	.a as $y | path(.a | $y)
value-mode-binding-other-frame	{"a":{"b":1},"x":{"a":{"b":1}}}	.a as $y | .x | path(.a | $y)
tojson-between	{"a":{"b":1}}	path(.a as $y | .a | tojson | fromjson | $y)
def-body-in-path	{"a":{"b":1}}	path(.a as $y | def f: $y; .a | f)
source-rebuilt-container	{"a":{"b":1}}	path(.a as $y | ([$y] | .[0]) as $z | .a | $z)
CASES_EOF
)

# Known refuse-only rows (jq answers, succinctly refuses), each with the
# reason it is deliberately left refusing. A new one is a sweep failure.
REFUSE_ONLY="value-mode-binding-same-node:eval_as (value mode) binds with no path; the value-mode half of #2042 is the accepting-direction twin of #2642
tojson-between:tojson/fromjson are not on cannot_move_register's proven allowlist (#2041)
def-body-in-path:a def inside path() resolves as an opaque leaf, before #2042 too
source-rebuilt-container:the source navigates inside a construction, which jq's suspended tracking allows but the resolver refuses; falls back to a plain value"

if [[ "${1:-}" == "--list-cases" ]]; then
  printf '%s\n' "$CASES"
  exit 0
fi

fab=0; mis=0; new_refuse=0; agree=0; refuse=0
printf 'id\tclass\tfilter\tsucc_exit\tsucc_out\toracle_exit\toracle_out\n'
while IFS=$'\t' read -r id input filter; do
  [[ -z "$id" ]] && continue
  jout=$(printf '%s' "$input" | "$JQ" -c "$filter" 2>/dev/null | tr '\n' ' ')
  jex=$(printf '%s' "$input" | "$JQ" -c "$filter" >/dev/null 2>&1; echo $?)
  sout=$(printf '%s' "$input" | "$SUCCINCTLY" jq -c "$filter" 2>/dev/null | tr '\n' ' ')
  sex=$(printf '%s' "$input" | "$SUCCINCTLY" jq -c "$filter" >/dev/null 2>&1; echo $?)
  if [[ "$jex" == "$sex" && "$jout" == "$sout" ]]; then
    class=agree; agree=$((agree+1))
  elif [[ "$jex" != 0 && "$sex" == 0 ]]; then
    class=fabricate; fab=$((fab+1))
  elif [[ "$jex" == 0 && "$sex" != 0 ]]; then
    if grep -q "^$id:" <<<"$REFUSE_ONLY"; then class=refuse-only; refuse=$((refuse+1)); else class=refuse-only-NEW; new_refuse=$((new_refuse+1)); fi
  elif [[ "$jex" != 0 && "$sex" != 0 ]]; then
    # both refuse; a differing emitted prefix is still a mismatch
    class=mismatch; mis=$((mis+1))
  else
    class=mismatch; mis=$((mis+1))
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$id" "$class" "$filter" "$sex" "$sout" "$jex" "$jout"
done <<<"$CASES"
printf '\n# agree=%d fabricate=%d mismatch=%d refuse-only=%d refuse-only-NEW=%d\n' "$agree" "$fab" "$mis" "$refuse" "$new_refuse" >&2
if (( fab > 0 || mis > 0 || new_refuse > 0 )); then exit 1; fi
