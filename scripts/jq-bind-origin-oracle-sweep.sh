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
# The `destructure-*` rows are #2649: `SRC as PATTERN | BODY` moves the path
# register through every pattern step, so a pattern variable is a tracked
# node exactly where jq's own step check accepts it. Rows where jq itself
# refuses (a second object entry, an array element in reverse order, a
# navigated source) are carried as `agree` rows -- both binaries must exit 5
# with nothing on stdout, which is what pins the *refusing* half of the arm.
# The two `destructure-alt-artefact-guard*` rows are the load-bearing
# not-a-fabrication assertion: jq answers `["a",0]` / deletes `.a[0]` there,
# and the resolver must keep refusing rather than retry onto the `?// $z`
# alternative -- if it ever answered `[]` (or deleted the whole document)
# the row would classify `mismatch`, not `agree`, and fail the sweep.
#
# Usage:
#   cargo build --release --features cli
#   ./scripts/jq-bind-origin-oracle-sweep.sh                 # TSV + summary; exit 1 on fabricate/mismatch/new refuse-only
#   ./scripts/jq-bind-origin-oracle-sweep.sh --list-cases    # rows only, no binaries needed
#
# Env: SUCCINCTLY (default target/release/succinctly), JQ (default /usr/bin/jq;
# whichever is used must match the pin in tests/data/jq-golden/JQ_VERSION).

set -euo pipefail
cd "$(dirname "$0")/.."
SUCCINCTLY="${SUCCINCTLY:-target/release/succinctly}"
PIN="$(cat tests/data/jq-golden/JQ_VERSION)"
if [[ "${1:-}" != "--list-cases" ]]; then
  if [[ ! -x "$SUCCINCTLY" ]]; then
    echo "error: succinctly binary not found at $SUCCINCTLY -- run: cargo build --release --features cli" >&2
    exit 1
  fi
  # The pinned oracle, never whichever jq is first on PATH (Homebrew's is a
  # newer version) -- the same rule the sibling sweeps apply.
  JQ="${JQ:-/usr/bin/jq}"
  if ! "$JQ" --version 2>/dev/null | grep -q "^$PIN"; then
    echo "error: $JQ is not the pinned oracle ($PIN): $("$JQ" --version 2>&1)" >&2
    exit 1
  fi
fi

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
try-catches-artefact	{"a":{"b":1},"c":1,"d":2}	path((try (.a | tostring | .[0:1]) catch "x") as $y | if $y == "{" then .c else .d end)
optional-swallows-artefact	{"a":{"b":1},"c":1,"d":2}	path(((.a | tostring | .[0:1])?) as $y | if $y == "{" then .c else .d end)
try-on-constructed	{"a":{"b":1}}	path((try ([1]|.[0]) catch "c") as $y | if $y == 1 then . else error("wrong") end)
handlerless-try-write	{"1":true,"c":true}	del(([1] | try .[0]) as $y | .[$y|tostring])
recurse-on-literal	{"a":1}	path((1 | ..) as $y | .a)
recurse-on-constructed	{"a":{"b":1}}	path(.a as $y | ([1,[2]] | ..) as $z | .)
getpath-on-constructed	{"a":1}	path(([1] | getpath([])) as $y | .)
side-effect-input	{"a":{"b":1}} 2 3	path(([input] | .[0]) as $y | .), input
slice-shared-buffer	{"a":[1,2,3]}	path(.a[0:2] as $y | .a[0:2] | $y)
slice-open-end	{"a":[1,2,3]}	path(.a[1:] as $y | .a[1:] | $y)
slice-null	{"a":null}	path(.a[1:2] as $y | .a[1:2] | $y)
slice-empty	{"a":[1,2,3]}	path(.a[1:1] as $y | .a[1:1] | $y)
slice-out-of-range	{"a":[1,2,3]}	path(.a[5:9] as $y | .a[5:9] | $y)
slice-string	{"a":"hello"}	path(.a[1:2] as $y | .a[1:2] | $y)
slice-empty-write	{"a":[1,2,3]}	(.a[1:1] as $y | .a[1:1] | $y) = [9]
nested-head-field	{"a":{"b":{"c":1}}}	path(.a as $y | ($y.b | .c) as $w | .a.b.c | $w)
nested-head-pipe	{"a":{"b":{"c":1}}}	path(.a as $y | (($y | .b) | .c) as $w | .a.b.c | $w)
recurse-body-marker	{"a":{"b":1}}	path(.a as $y | .a | recurse(if . == $y then $y.b else empty end))
recurse-body-marker-del	{"a":{"b":1}}	del(.a as $y | .a | recurse(if . == $y then $y.b else empty end))
select-wrapped-source	{"a":{"b":1}}	path((.a | select(.b)) as $y | .a | $y)
alternative-source	{"a":{"b":1}}	path((.a // 1) as $y | .a | $y)
if-source	{"a":{"b":1}}	path((if .a then .a else .b end) as $y | .a | $y)
optional-source-spelling	{"a":{"b":1}}	path(.a? as $y | .a | $y)
negative-index-spelling	[{"b":1},{"b":1}]	path(.[0] as $y | .[-2] | $y)
full-slice-is-the-array	{"a":[1,2,3]}	path(.a[0:3] as $y | .a | $y)
marker-not-at-head	{"a":{"b":1}}	path(.a as $y | (.c | $y | .b) as $w | .a.b | $w)
slice-spelling	{"a":[1,2,3]}	path(.a[1:] as $y | .a[1:3] | $y)
catch-handler-var	{"a":{"b":1}}	path(.a as $y | .a | try error("x") catch $y)
destructure-stage	{"a":{"b":1},"c":{"b":1}}	path(.a as $y | .a | . as [$q] ?// $q | $y)
literal-then-fold-untracked-init	{"a":{"b":1},"c":{"b":1}}	path(.a as $y | .a | 5 | reduce (1) as $i (0; $y))
destructure-object-var	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q} | $q)
destructure-var-index	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q} | $q[0])
destructure-nested-object	{"a":[1,2,3],"b":{"c":5}}	path(. as {b:{c:$r}} | $r)
destructure-var-field	{"a":[1,2,3],"b":{"c":5}}	path(. as {b:$b} | $b.c)
destructure-shorthand	{"a":[1,2,3],"b":{"c":5}}	path(. as {$a} | $a)
destructure-shorthand-nested	{"a":[1,2,3],"b":{"c":5}}	path(. as {$b:{c:$x}} | $x)
destructure-object-array	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:[$h]} | $h)
destructure-nested-array	[[1,2],[3,4]]	path(. as [[$a]] | $a)
destructure-var-iterate	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q} | $q | .[])
destructure-var-recurse	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q} | $q | recurse)
destructure-alt-first	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q} ?// [$q] | $q)
destructure-alt-second	{"a":[1,2,3],"b":{"c":5}}	path(. as [$q] ?// {a:$q} | $q)
destructure-alt-partial-bind	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q, x:{y:$z}} ?// {b:$r} | $r)
destructure-null-body	{"a":null}	path(. as {a:$q} | null)
destructure-null-doc	null	path(. as {a:$q,b:$r} | $r)
destructure-missing-key	{}	path(. as {a:{b:$q}} | $q)
destructure-del	{"a":[1,2,3],"b":{"c":5}}	del(. as {a:$q} | $q)
destructure-update-add	{"a":[1,2,3],"b":{"c":5}}	(. as {a:$q} | $q[1]) += 10
destructure-body-navigation	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q} | .a)
destructure-second-entry	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q, b:$r} | $r)
destructure-array-reverse	{"a":[1,2,3],"b":{"c":5}}	path(.a as [$x,$y] | $y)
destructure-duplicate-key	{"a":[1,2,3],"b":{"c":5}}	path(. as {b:$m, b:$n} | $n)
destructure-navigated-source	{"a":[1,2,3],"b":{"c":5}}	[path(.b as {c:$v} | $v)]
destructure-nested-pattern-on-copy	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q} | ($q|.[0]) as [$z] | $z)
destructure-bind-after-pattern	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q} | .a as $z | $z)
destructure-pattern-on-bound-copy	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q} | $q as [$x] | $x)
destructure-marker-source-literal	{"a":[1,2,3],"b":{"c":5}}	path(. as $x | 5 | $x as {a:$q} | $q)
destructure-marker-source-navigated	{"a":[1,2,3],"b":{"c":5}}	path(.b as $y | .b | 5 | $y as {c:$w} | $w)
destructure-alt-terminal-refusal	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q} ?// {b:$r} | $r)
destructure-alt-bare-var	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q} ?// $r | $r)
destructure-alt-navigation	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q} ?// $z | .a)
destructure-alt-artefact-guard	{"a":[1,2,3]}	path(. as {a:$q} ?// $z | if $q then $q[0] else $z end)
destructure-alt-artefact-guard-del	{"a":[1,2,3]}	del(. as {a:$q} ?// $z | if $q then $q[0] else $z end)
destructure-comma-marker-nav	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q} | $q[0], $q)
carried-register-passthrough	{"a":{"b":1}}	path(.a as $y | .a | 5 | select(true) | $y)
destructure-passthrough-stage	{"a":[1,2,3],"b":{"c":5}}	path(. as {a:$q} | select(true) | $q)
CASES_EOF
)

# Known refuse-only rows (jq answers, succinctly refuses), each with the
# reason it is deliberately left refusing. A new one is a sweep failure.
# Quoted heredoc: a reason may quote a filter verbatim ($q, "a") without
# the shell expanding it.
REFUSE_ONLY=$(cat <<'REFUSE_EOF'
value-mode-binding-same-node:eval_as (value mode) binds with no path; the value-mode half of #2042 is the accepting-direction twin of #2642
tojson-between:tojson/fromjson are not on cannot_move_register's proven allowlist (#2041)
def-body-in-path:a def inside path() resolves as an opaque leaf, before #2042 too
source-rebuilt-container:the source navigates inside a construction, which jq's suspended tracking allows but the resolver refuses; falls back to a plain value
select-wrapped-source:the witness grammar is pure navigation (is_pure_navigation); a select-wrapped source binds by value
alternative-source:the witness grammar is pure navigation; a // source binds by value
if-source:the witness grammar is pure navigation; an if source binds by value
optional-source-spelling:a ? component never matches a plain one on either side (path spelling, not node identity)
negative-index-spelling:a negative index is stored as written, so .[-2] never matches .[0]'s path
full-slice-is-the-array:jq's full slice is the array itself; the bind path ends in a slice component, .a does not
marker-not-at-head:a marker is re-rooted only at the head of a source; elsewhere it is certified against the ambient position
slice-spelling:jq's .a[1:] and .a[1:3] of a 3-array are the same jv; the slice components differ, so the spelling never matches (open-ended twin of full-slice-is-the-array)
catch-handler-var:the handler resolves under an unknown frame and a raising try stage does not carry the register; pre-existing, the root marker refuses too
literal-then-fold-untracked-init:after a literal the register is only carried, and a fold with an untracked INIT seeds its register from the ambient literal; pre-existing, the root marker refuses too
destructure-bind-after-pattern:#2649 residue 1 -- a plain bind on the ambient input after a pattern moved the register: resolve_bind_source needs a trackable stage, and the pattern's body stage is not
destructure-pattern-on-bound-copy:#2649 residue 1 -- a nested pattern whose source is the bound copy ($q as [$x]); the arm's source rule only trusts Identity/TrackedVar at the head of a trackable stage
destructure-marker-source-literal:#2649 residue 2 -- a marker-headed pattern source on an untracked stage; the arm cannot see the register the stage carries, and reading stage_frame alone would fabricate
destructure-marker-source-navigated:#2649 residue 2 -- same as destructure-marker-source-literal with a navigated bind (.b as $y | .b | 5 | $y as {c:$w})
destructure-alt-navigation:#2649 residue 3 -- the body navigates the ambient input, which raises a near-access refusal the artefact guard cannot tell from an artefact, so the ?// does not retry
destructure-alt-artefact-guard:#2649 artefact guard -- MUST stay a refusal: retrying here would answer [] where jq answers ["a",0]; answering anything makes this row mismatch (fabrication assertion, not a bare allowlist entry)
destructure-alt-artefact-guard-del:#2649 artefact guard -- the write twin: a retry would delete the whole document where jq deletes .a[0]
destructure-comma-marker-nav:#2649 residue 4 -- pre-existing comma shape: a nested Pipe gets no register, so $q[0] inside a comma raises near-access (limitations.md, #2042)
carried-register-passthrough:pre-existing (#2042): once the register is only *carried* (an untracked stage), a select/label/first/getpath passthrough re-seeds it from the ambient value and the marker no longer re-establishes; if/try/`. as $q | .`/literals keep it. Twin of literal-then-fold-untracked-init, found by the #2649 fuzz
destructure-passthrough-stage:the destructuring door onto carried-register-passthrough -- a pattern body starts on an untracked stage, so the same select/label/first/getpath passthroughs drop the register; the baseline binary refuses the plain-bind twin identically, so this is not #2649's
REFUSE_EOF
)

if [[ "${1:-}" == "--list-cases" ]]; then
  printf '%s\n' "$CASES"
  exit 0
fi

fab=0; mis=0; new_refuse=0; agree=0; refuse=0
printf 'id\tclass\tfilter\tsucc_exit\tsucc_out\toracle_exit\toracle_out\n'
while IFS=$'\t' read -r id input filter; do
  [[ -z "$id" ]] && continue
  # One spawn per binary: stdout and the exit code from the same run.
  jout=$(printf '%s' "$input" | "$JQ" -c "$filter" 2>/dev/null) && jex=0 || jex=$?
  sout=$(printf '%s' "$input" | "$SUCCINCTLY" jq -c "$filter" 2>/dev/null) && sex=0 || sex=$?
  jout=${jout//$'\n'/ }; sout=${sout//$'\n'/ }
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
