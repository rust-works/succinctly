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
# The `fold-alt-*` and `alt-untracked-stage-*` rows are #2979: a `?//` chain
# in a fold's own loop pattern, and one on an untracked stage, used to fall to
# by-value evaluation that could only refuse on an output it emitted. The
# rows pin jq's four backtracking rules through both fold kinds and both
# write operators, plus the two refusals kept on purpose (the reasons are in
# REFUSE_ONLY below and in limitations.md).
#
# The `nested-*`, `catch-*` and `computed-identity-bind-*` rows are #3133: an
# untracked stage's `Frame` carries the register's value into a pipe nested
# under `try`/`if`/`,` and into a `catch` handler (jq restores the register to
# the `try`'s entry when it catches), so a marker there re-establishes and a
# write through it lands -- `del(. as {a:$w} | try ($w | .b))` echoed the
# document where jq writes. The same change makes `. as $x` on an untracked
# stage an Untracked marker (a computed `.` is no node), closing a
# pre-existing write-through of a constructed copy.
#
# The `untracked-*` rows are #3043/#3120: a destructuring bind on an
# *untracked* stage checks its first pattern step against the register the
# pipe carried in (`resolve_seq_stage` hands it to `resolve_as_pattern`),
# never against the stage's own value -- a `null` literal stage used to pass
# the null-identity clause against itself where jq refuses against the
# document root, and `del`/`|=` wrote through it. With the register in hand
# the #2649 residue-2 / #2979 family-B rows answer (their REFUSE_ONLY
# entries are gone), and a `?//` first-step refusal that is provably jq's
# own verdict (the source is not even value-equal to the register) retries.
#
# The `identity-*` rows are #2978: an identity-passthrough bind (`. as $x`,
# and every spelling `is_identity_passthrough` recognises) made inside a
# resolver invocation carries the position `.` was frozen at
# (`Origin::SnapshotAt`), so a later `getpath` on the bound value composes
# an absolute position and the pipe can land back on the register. The
# `identity-trap-*` rows are each a plausible wrong rule's fabrication --
# "an identity bind is at the invocation root" (`below-root-is-not-root`,
# whose `del` twin would corrupt the document), "a rebind takes the frame's
# position" (`rebind-takes-marker-position`), "use the frame off the
# register" (`off-register-frame`) -- carried as `agree` rows: both binaries
# must exit 5 with nothing on stdout.
#
# The `navigated-bind-*` and `root-snapshot-renavigated-*` rows are #3037: a
# variable bound from a navigated position *outside* any resolver (`.a as
# $y`, `Origin::Untracked`) used where the register really is that same node
# -- promoted to a `Snapshot` at a funnel whose live cursor is the marker's
# own node -- and, in the same funnel, the fabrication the assignment
# family's fallback into `eval_full` had: a root `Snapshot` re-navigated onto
# an equal-valued sibling through `$r` wrote through it on `main` (jq
# refuses). The sibling/rebuilt/ambiguous rows are carried as `agree` rows:
# both binaries must exit 5.
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
# `read -d ''`, not `$(cat <<EOF)`: bash scans a command substitution's body for
# quotes and parens even inside a quoted heredoc, so a filter or a reason with an
# apostrophe or an unbalanced paren in it used to break the whole script (#3120).
read -r -d '' CASES <<'CASES_EOF' || true
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
fold-destructure-foreach-object	{"a":[1,2,3],"b":{"c":5}}	path(foreach .b as {c:$x} (.; .; $x))
fold-destructure-foreach-array	{"a":[1,2,3],"b":{"c":5}}	path(foreach .a as [$x] (.; .; $x))
fold-destructure-reduce-identity	{"a":[1,2,3],"b":{"c":5}}	path(reduce .b as {c:$x} (.; .))
fold-destructure-nested	{"p":{"q":{"r":42}}}	path(foreach .p as {q:{r:$v}} (.; .; $v))
fold-destructure-iterate	{"x":[{"c":1},{"c":2}]}	path(foreach .x[] as {c:$v} (.; .; $v))
fold-destructure-update-add	{"a":[1,2,3],"b":{"c":5}}	(foreach .b as {c:$x} (.; .; $x)) |= .+1
fold-destructure-del	{"a":[1,2,3],"b":{"c":5}}	del(foreach .b as {c:$x} (.; .; $x))
fold-destructure-null-bool-coincidence	null	path(foreach (null) as {a:$x} (.; .; $x))
fold-destructure-nonregister-source	{}	path(. as $x | reduce ([1]) as [$i] (0; $x))
fold-destructure-array-reverse	{"a":[1,2,3],"b":{"c":5}}	path(foreach .a as [$x,$y] (.; .; $x))
fold-destructure-accumulator-nav	{"a":[1,2,3],"b":{"c":5}}	path(reduce .b as {c:$x} (.; .b))
fold-destructure-qq-alt	{"a":[1,2,3],"b":{"c":5}}	path(reduce .b as {c:$x} ?// $z (0; $x))
fold-destructure-duplicate-key	{"a":[1,2,3],"b":{"c":5}}	path(foreach .b as {c:$x,c:$x} (.; .; $x))
fold-destructure-nonnull-register	{}	path(foreach (null) as {a:$x} (.; .; $x))
fold-alt-walk-retry	{"a":[1]}	path(foreach .a as {b:$v} ?// [$v] (.; .; $v))
fold-alt-walk-retry-reduce	{"a":[1]}	path(reduce .a as {b:$v} ?// [$v] (.; $v))
fold-alt-update-null-acc	null	path(reduce 1 as $a ?// $b (.; if $b then . else error("x") end))
fold-alt-update-null-acc-off-register	{"a":1}	path(reduce 1 as $a ?// $b (.; if $b then . else error("x") end))
fold-alt-terminal-retry	{"a":[1]}	path(foreach .a as [$v] ?// $w (.; .; $w))
fold-alt-terminal-retry-after-emit	{"a":{"b":1}}	[path(foreach .a as {b:$v} ?// $w (.; .; $v, 1))]
fold-alt-per-element	{"a":[1,2]}	[path(foreach .a[] as [$x] ?// $y (.; .; $y))]
fold-alt-last-propagates	{"a":{"b":1}}	path(reduce .a as {b:$v} ?// $w (.; $v))
fold-alt-outer-var	{"a":1}	path(. as $x | foreach (1) as $y ?// $z (0; $x; .))
fold-alt-empty-body	{"a":2,"c":2}	path(foreach .a as {a:$v} ?// $v (.c; .; empty))
fold-alt-empty-body-reduce	{"a":{"b":1},"c":{"b":1}}	path(reduce .a as {b:$v} ?// $v (.c; .) | empty)
fold-alt-guessed-walk	{"a":2,"c":2}	path(foreach .a as {a:$v} ?// $v (.c; .; $v))
fold-alt-nested-fold	{"a":2,"c":2,"d":true,"x":{"a":2,"c":{"b":1}},"arr":[2,2,2]}	path(foreach ([.a] | .[0]) as {a:$v0} ?// $v0 (.c; foreach (1) as $i (0; $v0; .); $v0.b?))
fold-alt-del	{"a":[1]}	del(foreach .a as {b:$v} ?// [$v] (.; .; $v))
fold-alt-del-refused	{"a":{"b":1},"c":false,"d":{"b":1},"x":{"a":{"b":1},"c":"s"}}	del(foreach . as {a:$v0} ?// [$v0] (.c; ($v0 | .b?); ($v0 | getpath([]) | .b?)))
fold-alt-update-refused	{"a":{"b":1},"c":{"b":1}}	(foreach .a as {b:$v} ?// $w (.c; .; empty)) |= 5
fold-alt-assign	{"a":{"b":{"c":1}}}	(foreach .a as {b:$v} ?// $w (.; .; $v | .c)) = "w"
fold-alt-halt	null	path(foreach (1) as $x ?// $y (null; .; halt_error))
fold-alt-limit-outside	{"a":1}	[limit(1; path(foreach (1,2) as [$a] ?// $b (.; .; .)))]
alt-untracked-stage-refuses	{"a":1}	path(5 | . as {a:$v} ?// $v | .b?)
alt-untracked-stage-zero-output	{"a":1}	path(5 | 5 as {a:$v} ?// $v | empty)
alt-untracked-stage-marker-head	{"a":1}	path(. as $x | 5 | $x as {a:$q} ?// $z | $q)
in-evaluator-input-write	{"a":1} {"a":1}	input | . as $x | {a:1} | ($x.a = 9)
in-evaluator-inputs-write	{"a":1} {"a":1}	[inputs] | .[0] | . as $x | {a:1} | ($x.a = 9)
in-evaluator-line-sibling-write	{"a":1}	. as $x | {a:1} | (input_line_number, ($x.a = 9))
in-evaluator-line-sibling-del	{"a":1}	. as $x | {a:1} | (input_line_number, del($x.a))
in-evaluator-line-sibling-path	{"a":1}	. as $x | {a:1} | (input_line_number, path($x))
in-evaluator-line-bind-write	{"a":1}	. as $x | {a:1} | input_line_number as $n | ($x.a = 9)
in-evaluator-fold-update-write	{"a":1}	reduce (1) as $i (.; . as $x | {a:1} | ($x.a = 9))
in-evaluator-update-rhs-write	{"a":1}	[.] | .[] |= (. as $x | {a:1} | ($x.a = 9))
in-evaluator-with-entries-path	{"a":1}	with_entries(.value |= (. as $x | 1 | path($x)))
in-evaluator-empty-object-write	{} {}	input | . as $x | {} | ($x.a = 9)
in-evaluator-empty-array-write	[] []	input | . as $x | [] | ($x[0] = 9)
in-evaluator-catch-rebuilt-write	{"a":1}	. as $x | try error({a:1}) catch ($x.a = 9)
in-evaluator-catch-rebuilt-write-input	{"a":1} {"a":1}	input | . as $x | try error({a:1}) catch ($x.a = 9)
in-evaluator-isempty-write	{"a":1}	. as $x | isempty({a:1} | ($x.a = 9))
in-evaluator-any-write	{"a":1}	. as $x | any({a:1}; ($x.a = 9))
in-evaluator-control-no-route	{"a":1}	. as $x | {a:1} | ($x.a = 9)
in-evaluator-control-first	{"a":1}	first(. as $x | {a:1} | ($x.a = 9))
in-evaluator-control-map	{"a":1}	map(. as $x | {a:1} | ($x.a = 9))
in-evaluator-input-direct-write	{"a":1} {"a":1}	input | . as $x | ($x.a) = 9
in-evaluator-input-direct-del	{"a":1} {"a":1}	input | . as $x | del($x.a)
in-evaluator-input-first-path	{"a":1} {"a":1}	input | . as $x | first(.) | path($x)
in-evaluator-input-select-path	{"a":1} {"a":1}	input | . as $x | select(true) | path($x)
in-evaluator-input-empty-object-direct	{} {}	input | . as $x | ($x.a) = 9
in-evaluator-input-scalar-direct	"s" "s"	input | . as $x | ($x) = 9
in-evaluator-update-rhs-direct	{"a":1}	[.] | .[] |= (. as $x | ($x.a = 9))
in-evaluator-fold-update-direct	{"a":1}	reduce (1) as $i (.; . as $x | ($x.a = 9))
in-evaluator-catch-own-value	{"a":1}	. as $x | try error($x) catch path($x)
in-evaluator-catch-own-value-write	{"a":1}	. as $x | try ($x | error) catch ($x.a = 9)
in-evaluator-input-embed-array	{"a":1} {"a":1}	input | . as $x | [.] | .[0] | path($x)
in-evaluator-input-embed-object	{"a":1} {"a":1}	input | . as $x | {k:.} | .k | path($x)
in-evaluator-input-reduce-empty	{"a":1} {"a":1}	input | . as $x | reduce empty as $i (.; .) | path($x)
in-evaluator-input-fold-source	{"a":1} {"a":1}	input | reduce (.) as $x (.; ($x.a = 9))
in-evaluator-input-catch-own-value	{"a":1} {"a":1}	input | . as $x | try error($x) catch path($x)
in-evaluator-def-body-write	{"a":1}	. as $x | def f: ($x.a = 9); {a:1} | f
in-evaluator-def-body-path	{"a":1}	. as $x | def f: path($x); {a:1} | f
in-evaluator-def-body-update-rhs	{"a":1}	. as $x | def f: del($x.a); . |= ({a:1} | f)
in-evaluator-def-body-fold	{"a":1}	. as $x | def f: ($x.a = 9); reduce 1 as $i ({a:1}; f)
in-evaluator-def-body-direct	{"a":1}	. as $x | def f: ($x.a = 9); f
in-evaluator-def-body-any	{"a":1}	. as $x | def f: ($x.a = 9); any(f; true)
in-evaluator-catch-error-then-error	{"a":1}	. as $x | try (error($x) | error) catch path($x)
in-evaluator-resolver-select-cond	{"a":1}	. as $x | path(select(($x.a = 9) | true))
in-evaluator-resolver-computed-key	{"a":1}	. as $x | .[($x.a = 9 | "a")] = 5
in-evaluator-update-paren-root	{"a":1}	. as $x | (.) |= ($x.a = 9)
in-evaluator-update-second-path	{"a":1,"b":2}	. as $x | (.b, .) |= (if type == "object" then ($x.a = 9) else . end)
navigated-bind-same-node-path	{"a":{"b":1}}	.a as $y | .a | path($y)
navigated-bind-same-node-assign	{"a":{"b":1}}	.a as $y | .a | ($y.b) = 9
navigated-bind-same-node-del	{"a":{"b":1}}	.a as $y | .a | del($y.b)
navigated-bind-same-node-update	{"a":{"b":1}}	.a as $y | .a | $y |= 5
navigated-bind-same-node-compound	{"a":{"b":1}}	.a as $y | .a | ($y.b) += 1
navigated-bind-same-node-first	{"a":{"b":1}}	.a as $y | .a | first(.) | path($y)
navigated-bind-same-node-iterate-source	{"a":{"b":1}}	.[] as $y | .a | path($y)
navigated-bind-same-node-scalar	{"a":{"b":1}}	.a.b as $y | .a.b | path($y)
navigated-bind-same-node-continues	{"a":{"b":1}}	.a as $y | .a | path($y.b)
navigated-bind-same-node-limit	{"a":{"b":1}}	limit(1; .a as $y | .a | path($y))
navigated-bind-same-node-def	{"a":{"b":1}}	def f: .a as $y | .a | path($y); f
navigated-bind-same-node-array	{"a":[1,2]}	.a as $y | .a | del($y[0])
navigated-bind-sibling-path	{"a":{"b":1},"c":{"b":1}}	.a as $y | .c | path($y)
navigated-bind-sibling-assign	{"a":{"b":1},"c":{"b":1}}	.a as $y | .c | ($y.b) = 9
navigated-bind-sibling-update	{"a":{"b":1},"c":{"b":1}}	.a as $y | .c | $y |= 5
navigated-bind-sibling-del	{"a":{"b":1},"c":{"b":1}}	.a as $y | .c | del($y.b)
navigated-bind-root-register	{"a":{"b":1}}	.a as $y | path($y)
navigated-bind-descendant	{"a":{"b":{"c":1}}}	.a as $y | .a.b as $z | .a | path($z)
navigated-bind-rebuilt	{"a":{"b":1},"c":{"b":1}}	.a as $y | .a | (tojson|fromjson) | ($y.b) = 9
navigated-bind-constructed	{"a":{"b":1},"c":{"b":1}}	.a as $y | .a | {b:1} | ($y.b) = 9
navigated-bind-ambiguous-source	{"a":[{"b":1},{"b":1}]}	.a[1] as $y | .a[] | ($y.b) = 9
root-snapshot-renavigated-sibling-assign	{"a":{"b":1},"c":{"b":1}}	. as $r | .a | . as $x | $r | .c | ($x.b) = 9
root-snapshot-renavigated-sibling-update	{"a":{"b":1},"c":{"b":1}}	. as $r | .a | . as $x | $r | .c | $x |= 5
root-snapshot-renavigated-sibling-limit	{"a":{"b":1},"c":{"b":1}}	limit(1; . as $r | .a | . as $x | $r | .c | ($x.b) = 9)
root-snapshot-renavigated-sibling-def	{"a":{"b":1},"c":{"b":1}}	def f: . as $r | .a | . as $x | $r | .c | ($x.b) = 9; f
root-snapshot-renavigated-own-node	{"a":{"b":1},"c":{"b":1}}	. as $r | .a | . as $x | $r | .a | ($x.b) = 9
navigated-bind-positional-path	{"a":{"b":1}}	.a as $y | path(.a | $y)
navigated-bind-positional-assign	{"a":{"b":1}}	.a as $y | (.a | ($y.b)) = 9
navigated-bind-embed	{"a":{"b":1}}	.a as $y | {k:.a} | .k | path($y)
navigated-bind-reduce-update	{"a":{"b":1}}	reduce (1) as $i (.; .a as $y | .a | ($y.b) = 9)
navigated-bind-catch-handler	{"a":{"b":1}}	try error(.) catch (.a as $y | .a | path($y))
navigated-bind-bool-sibling	{"a":true,"c":true}	.a as $y | .c | $y |= 5
navigated-bind-owned-root	{"a":{"b":1}}	(tojson|fromjson) | .a as $y | .a | path($y)
navigated-bind-input-root	{"a":{"b":1}} {"a":{"b":1}}	input | .a as $y | .a | path($y)
identity-at-root-getpath	{"a":{"b":2}}	path(. as $x | .a | $x | getpath(["a"]) | .b)
identity-at-root-getpath-del	{"a":{"b":2},"c":1}	del(. as $x | .a | $x | getpath(["a"]) | .b)
identity-at-root-getpath-assign	{"a":{"b":2}}	(. as $x | .a | $x | getpath(["a"]) | .b) = 9
identity-below-root-getpath	{"x":{"a":{"b":2}}}	path(.x | . as $v | .a | $v | getpath(["a"]) | .b)
identity-below-root-getpath-del	{"x":{"a":{"b":2},"c":1}}	del(.x | . as $v | .a | $v | getpath(["a"]) | .b)
identity-below-root-sibling-copy	{"a":{"k":{"b":1}},"c":{"k":{"b":1}}}	path(.c | . as $x | .k | $x | getpath(["k"]) | .b)
identity-rebind-chain	{"a":{"b":2}}	path(. as $x | $x as $y | .a | $y | getpath(["a"]) | .b)
identity-rebind-nested	{"a":{"c":{"b":1}},"c":{"b":1}}	path(. as $x | .a | ($x as $y | .c | $y | getpath(["a","c"]) | .b))
identity-alternative-spelling	{"a":{"b":2}}	path((. // 1) as $x | .a | $x | getpath(["a"]) | .b)
identity-try-spelling	{"a":{"b":2}}	path((try . catch 1) as $x | .a | $x | getpath(["a"]) | .b)
identity-if-spelling	{"a":{"b":2}}	path((if true then . else . end) as $x | .a | $x | getpath(["a"]) | .b)
identity-if-arms-differ	{"a":{"b":2}}	path(. as $p | .a | (if true then $p else . end) as $x | $x | getpath(["a"]) | .b)
identity-pattern-var-alt	{"a":{"b":2}}	path(. as [$x] ?// $x | .a | $x | getpath(["a"]) | .b)
identity-two-getpaths	{"a":{"b":{"c":1}}}	path(. as $x | .a.b | $x | getpath(["a"]) | getpath(["b"]) | .c)
identity-reduce-init	{"a":{"b":2}}	path(. as $x | .a | reduce (1) as $i ($x; getpath(["a"])) | .b)
identity-foreach-init	{"a":{"b":2}}	path(. as $x | .a | foreach (1) as $i ($x; getpath(["a"]); .b))
identity-trap-sibling-equal	{"a":{"b":2},"c":{"b":2}}	path(. as $x | .a | $x | getpath(["c"]) | .b)
identity-trap-sibling-unequal	{"a":{"b":2},"c":{"b":3}}	path(. as $x | .a | $x | getpath(["c"]) | .b)
identity-trap-below-root-is-not-root	{"a":{"c":{"b":1},"a":{"c":{"b":1}}}}	path(.a | . as $x | .c | $x | getpath(["a","c"]) | .b)
identity-trap-below-root-is-not-root-del	{"a":{"c":{"b":1},"a":{"c":{"b":1}}}}	del(.a | . as $x | .c | $x | getpath(["a","c"]) | .b)
identity-trap-rebind-takes-marker-position	{"a":{"c":{"b":1}},"c":{"b":1}}	path(. as $x | .a | ($x as $y | .c | $y | getpath(["c"]) | .b))
identity-trap-off-register-frame	{"a":{"b":{"c":1}}}	path(.a | {b:{c:1}} | . as $x | $x | getpath(["b"]) | .c)
identity-trap-rebuilt-copy	{"a":{"b":2}}	path(. as $x | .a | ($x | tojson | fromjson) | getpath(["a"]) | .b)
identity-trap-alternative-below-null	{"a":null}	path(.a | (. // {"c":{"b":1}}) as $x | .c | $x | getpath(["c"]) | .b)
identity-nested-path-invocation	{"a":{"b":2}}	path(. as $x | .a | path($x | getpath(["a"]) | .b))
identity-trap-raising-try-getpath	{"a":{"k":{"b":1}}}	path(.a | (try (if error("e") then . else . end) catch {"k":{"b":1}}) as $x | .k | $x | getpath(["k"]) | .b)
identity-trap-raising-try-getpath-del	{"a":{"k":{"b":1}}}	del(.a | (try (if error("e") then . else . end) catch {"k":{"b":1}}) as $x | .k | $x | getpath(["k"]) | .b)
identity-trap-raising-try-value-rule	{"a":{"k":{"b":1}}}	path(.a | (try (if error("e") then . else . end) catch {"b":1}) as $x | .k | $x | .b)
identity-trap-raising-try-value-rule-del	{"a":{"k":{"b":1}}}	del(.a | (try (if error("e") then . else . end) catch {"b":1}) as $x | .k | $x | .b)
identity-trap-raising-try-alternative	{"a":{"k":{"b":1}}}	path(.a | ((try (if error("e") then . else . end)) // {"b":1}) as $x | .k | $x | .b)
identity-raise-free-try-nested	{"a":{"k":{"b":1}}}	path(.a | (try (try . catch 1) catch 2) as $x | .k | $x | getpath(["k"]) | .b)
identity-try-if-nonraising	{"a":{"k":{"b":1}}}	path(.a | (try (if true then . else . end) catch 1) as $x | .k | $x | getpath(["k"]) | .b)
untracked-null-stage-array	{"a":1}	path(null | . as [$v] | empty)
untracked-null-stage-object	{"a":1}	path(null | . as {a:{b:$v}} | empty)
untracked-null-stage-below-root	{"a":1}	path(.a | null | . as [$v] | empty)
untracked-null-stage-del	{"a":1}	del(null | . as [$v] | empty)
untracked-null-stage-update	{"a":1}	(null | . as [$v] | empty) |= 5
untracked-null-stage-optional-body	{"a":[1]}	path(null | .a as {arr:[$v]} | .arr[]?)
untracked-null-stage-constructed-source	{"a":1}	path(null | ([.a] | .[0]) as [$v1] | ($v1 | .[]?))
untracked-null-stage-null-register	{"a":null}	path(.a | null | . as {b:$v} | $v)
untracked-null-stage-null-document	null	[path(null | . as [$v] | empty)]
untracked-stage-marker-head-single	{"a":1}	path(. as $x | 5 | $x as {a:$q} | $q)
untracked-stage-marker-head-del	{"a":1}	del(. as $x | 5 | $x as {a:$q} ?// $z | $q)
untracked-stage-marker-not-register	{"a":1}	path(. as $x | 5 | $x as {a:$q} | $x)
untracked-stage-equal-value-guess	{"a":{"b":1}}	path(.a | {b:1} | . as {b:$v} ?// $z | $z)
untracked-catch-handler-null-document	null	path(try error(null) catch (. as {a:$q} | $q))
untracked-catch-handler-del	{"a":1}	del(try error(null) catch (. as {a:$q} | $q))
untracked-nested-if-refuses	{"a":1}	path(.a | null | if true then (. as {b:$v} | empty) else . end)
untracked-nested-if-null-register	{"a":null}	path(.a | 5 | if true then (null as {b:$v} | $v) else . end)
untracked-bare-alt-keeps-register	{"a":1}	path(. as $x | 5 | $x as [$v] ?// $z | $z)
untracked-bare-alt-keeps-register-del	{"a":1}	del(. as $x | 5 | $x as [$v] ?// $z | $z.a)
untracked-opaque-stage-lost-register	{"a":{"b":null}}	del(reduce 1 as $i (null; null) | . as [$v] ?// $v | empty)
untracked-later-step-refusal-no-retry	{"a":{"b":[1]}}	path(.a as $x | .a | 5 | $x as {b:[$q,$r]} ?// $w | $w)
untracked-later-step-refusal-trackable-twin	{"a":{"b":[1]}}	path(.a as $x | .a | $x as {b:[$q,$r]} ?// $w | $w)
nested-try-body-keeps-register	{"a":{"b":1}}	del(. as {a:$w} | try ($w | .b))
nested-try-body-keeps-register-untracked	{"a":{"b":1}}	del(. as $x | 5 | $x as {a:$w} | try ($w | .b))
nested-optional-body-keeps-register	{"a":{"b":1}}	del(. as $x | 5 | $x as {a:$w} | ($w | .b)?)
nested-optional-bare-alt-keeps-register	{"a":1}	del(. as $x | 5 | $x as [$w] ?// $z | ($z | .a)?)
nested-try-source-keeps-register	{"a":{"b":1}}	del(.a as $x | .a | 5 | try ($x as {b:$q} | $q))
nested-try-sibling-copy-control	{"a":{"b":1},"c":{"b":1}}	del(.a as $y | .c | 5 | try ($y | .b))
nested-pipe-null-register	{"a":null}	path(.a | 5 | (null | .b))
nested-pipe-non-null-register	{"a":{"b":1}}	path(.a | 5 | (null | .b))
catch-restores-entry-register	null	path(try (.a | error(null)) catch .b)
catch-restores-entry-register-refuses	{"x":{"a":1,"b":2}}	path(.x | try (.a | error(null)) catch .b)
catch-rebuilt-payload-refuses	{"x":{"a":1,"b":2}}	path(.x | try (.a | error({"a":1,"b":2})) catch .b)
catch-marker-reestablishes-untracked	{"a":{"b":1}}	path(.a as $y | .a | 5 | try error(1) catch $y)
catch-marker-reestablishes-del	{"a":{"b":1}}	del(.a as $y | .a | 5 | try error(1) catch $y)
catch-marker-navigates	{"a":{"b":1}}	path(.a as $y | .a | try error(1) catch ($y | .b))
catch-payload-handed-on	{"a":{"b":1}}	[path(.a | (try error(null) catch .) | empty)]
catch-payload-handed-on-refuses	{"a":{"b":1}}	path(.a | (try error(null) catch .) | .b)
catch-payload-own-node-refuse-only	{"a":{"b":1}}	path(.a | try error(.) catch .)
computed-identity-bind-untracked	{"a":{"b":{"c":1}}}	path(.a | {b:{c:1}} | . as $x | $x)
computed-identity-bind-untracked-del	{"a":{"b":{"c":1}}}	del(.a | {b:{c:1}} | . as $x | $x)
computed-identity-bind-navigates	{"a":{"b":{"c":1}}}	path(.a | {b:{c:1}} | . as $x | $x | .b | .c)
computed-identity-bind-null-register	{"a":null}	path(.a | null | . as $x | $x | .b)
computed-identity-bind-marker-source	{"a":{"b":{"c":1}}}	path(. as $x | 5 | $x as $y | $y)
computed-identity-bind-mixed-if	{"a":{"b":{"c":1}}}	path(. as $x | 5 | (if true then $x else . end) as $y | $y)
catch-break-payload-not-register	{"a":null}	(.a | label $out | try (break $out) catch .b) = 1
catch-break-restores-register	{"a":null}	path(.a as $y | .a | label $out | try (break $out) catch $y)
CASES_EOF

# Known refuse-only rows (jq answers, succinctly refuses), each with the
# reason it is deliberately left refusing. A new one is a sweep failure.
# Quoted heredoc: a reason may quote a filter verbatim ($q, "a") without
# the shell expanding it. Read with `read -d ''` rather than `$(cat <<EOF)`
# for the reason given at CASES above (#3037 hit the same scanner from the
# other side: a `(` before a `#` in a reason made bash read the `#` as a
# comment inside the `$( )`, so the closing `)` vanished).
read -r -d '' REFUSE_ONLY <<'REFUSE_EOF' || true
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
destructure-bind-after-pattern:#2649 residue 1 -- a plain bind on the ambient input after a pattern moved the register: resolve_bind_source needs a trackable stage, and the pattern's body stage is not
destructure-alt-navigation:#2649 residue 3 -- the body navigates the ambient input, which raises a near-access refusal the artefact guard cannot tell from an artefact, so the ?// does not retry
destructure-comma-marker-nav:#2649 residue 4 -- pre-existing comma shape: a nested Pipe gets no register, so $q[0] inside a comma raises near-access (limitations.md, #2042)
carried-register-passthrough:pre-existing (#2042): once the register is only *carried* (an untracked stage), a select/label/first/getpath passthrough re-seeds it from the ambient value and the marker no longer re-establishes; if/try/`. as $q | .`/literals keep it. Twin of literal-then-fold-untracked-init, found by the #2649 fuzz
destructure-passthrough-stage:the destructuring door onto carried-register-passthrough -- a pattern body starts on an untracked stage, so the same select/label/first/getpath passthroughs drop the register; the baseline binary refuses the plain-bind twin identically, so this is not #2649's
in-evaluator-input-embed-array:#3036 -- the in-evaluator twin of the #2642 owned-embed residual: `[.] | .[0]` materializes the element as an owned copy, which re-enters eval.rs as a fresh document (limitations.md, #3036)
in-evaluator-input-embed-object:#3036 -- same as in-evaluator-input-embed-array for `{k:.} | .k`
in-evaluator-input-reduce-empty:#3036 -- same as in-evaluator-input-embed-array for a fold that returns its accumulator unchanged
in-evaluator-input-fold-source:#3036 -- the loop variable of a fold is Snapshot with no node, and UPDATE runs against the re-indexed accumulator; the generic evaluator has refused this since #2642
in-evaluator-input-catch-own-value:#3036 -- on the input-queue route the marker carries no node witness, so `try_payload_root` cannot prove the payload is its node; the generic route keeps it accepted
navigated-bind-positional-path:#3037 residual -- the marker certified at a non-root register position inside the invocation needs a document-absolute bind path -- the Origin::At machinery of #2042, reached from a value-mode bind; scoped separately
navigated-bind-positional-assign:#3037 residual -- same as navigated-bind-positional-path, the write twin
navigated-bind-embed:#2889 -- the owned-embed residual ({k:.a} | .k), unchanged by #3037
navigated-bind-reduce-update:#3037 residual -- the UPDATE of reduce re-enters the eager evaluator with an owned accumulator; its eval_as carries no node for a navigated bind (#2072 gave the generic evaluator that, not this one), and there is no cursor at the funnel to promote against
navigated-bind-catch-handler:#3037 residual -- same as navigated-bind-reduce-update, through a catch handler
navigated-bind-bool-sibling:pre-existing -- the jv_identical rule of jq admits a null/bool by value regardless of node, but the TrackedVar arm of the resolver consults the origin first; ($y | .) = 5 already answers since the . stage re-establishes by value
navigated-bind-owned-root:#3037 residual -- a navigated bind on an owned-rooted document; the marker node is an OwnedIdentity position and marker_is_root reads only a document node against a live cursor, no OwnedRoot twin
navigated-bind-input-root:#3037 residual -- same as navigated-bind-owned-root, on the input-queue route
identity-if-arms-differ:#2978 -- identity_bind_position is static: an if whose arms sit at different positions ($p at [], . at ["a"]) proves neither, so the bind stays a bare Snapshot and getpath has no position to compose from; jq evaluates the condition
identity-try-if-nonraising:#2978 review -- a try body holding an if is not a passthrough (its condition may raise and bind the value of the handler); the gate is static, so an if whose condition happens not to raise pays a refusal. The raising twin (identity-trap-raising-try-*) is the write-side fabrication this prevents
untracked-opaque-stage-lost-register:#3120 review -- an opaque stage (reduce, a def call, first) drops the carried register, so the walk has none and refuses without retrying; jq refuses the step too and retries onto the bare alternative, whose empty body then writes nothing. main echoed the document by the ambient-null coincidence the fix removes
untracked-later-step-refusal-no-retry:#3120 review -- refusal_is_exact is decided per source, and a marker that is the register is value-equal to it, so a later-step refusal after a certified first step is treated as a guess and does not retry; jq retries onto $w. The trackable twin retries and agrees
catch-payload-own-node-refuse-only:#3133 -- error(.) raises the register node itself and jq answers ["a"]; the payload equals the register by value but is not null/bool and carries no marker, so it cannot be told from a rebuilt copy (catch-rebuilt-payload-refuses) and the handler stays untracked
computed-identity-bind-mixed-if:#3133 -- an if source with one arm a computed `.` and the other a marker binds Untracked on an untracked stage (the condition is not evaluated); jq evaluates it and binds the marker
REFUSE_EOF

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
