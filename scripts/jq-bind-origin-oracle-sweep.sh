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
# The `scalar-*` rows are #3182's oracle matrix for scalar node identity: a
# string or number literal root/element bound by `as` and re-materialized by a
# placement (`{k:.} | .k`, `[.] | .[0]`, `[.] | add`), and, captured live, every
# builtin real jq hands its input `jv` back from (`tostring`/`@text` on a
# string, a no-match `ltrimstr`/`rtrimstr`/`sub`, `tonumber` on a number, `abs`,
# `getpath([])`, `setpath([]; .)`) versus every builtin that allocates its
# result (`ascii_downcase`, `. + ""`, `tojson | fromjson`, a slice, `floor`,
# `. + 0`), plus jq's constant pool (`def f: 5; f as $x | f` is one `jv`, two
# literals are two, two equal document scalars are two nodes). jq's rule is
# pointer identity for strings and number literals and value identity for
# `null`/`bool`/a computed number. succinctly's scalars have no storage
# identity: ADR-0024's option D (a refcounted `String` and spelling) was built
# and measured on 2026-09-20 and rejected -- +8% to +50% peak RSS on
# scalar-heavy rows for the rows it recovers -- so the rows jq answers stay
# refuse-only here (the safe direction) and are pinned below with that reason,
# while the rows jq refuses agree.
#
# The `owned-embed-*` rows are #2889 Phase 2: an *identity* bind (`. as $x`)
# whose value is later re-materialized by an embedding stage jq keeps the
# same `jv` through -- `{k:.} | .k`, `. + {}`/`. * {}`/`. + null`/`null + .`
# on an empty/null operand, a fold whose UPDATE returns its input unchanged,
# `[.] | add`/`min`/`max` of one element, `[.,.] | .[1]`, `[[.]] | .[0][0]`,
# `{k:.} | getpath(["k"])` -- so `path($x)`/`($x...) = `/`del($x...)` later
# should answer exactly where jq's `jv_identical($x, .)` holds. This is the
# oracle-before.tsv snapshot named in docs/plan/jq-bind-origin-frame.md: every
# row of that table gets a case here, `DIFF` targets included, so the sweep
# stays runnable (and non-fabricating) against the pre-#2889-Phase-2 binary
# while the fix landed in two passes (Stage A: `eval_generic`'s embed table
# and the owned-value funnels in `eval.rs`; Stage B: the `eval.rs` bind sites
# themselves, `eval_as`/`each_as`, which minted no node before it, so every
# `-n 'input | ...'` row above was refuse-only until Stage B gave them one).
# Most `owned-embed-*` rows now `agree` (#3178 added `sort`/`unique`/`reverse`/
# `to_entries`/literal `getpath`). The residual `owned-embed-refuse-*` ids, and
# `owned-embed-fold-if-identity`, are pinned in REFUSE_ONLY below with the
# mechanism each is missing: a scalar root (never `Rc`-backed, so it can't
# enter the embed table), or a shape `eval::embed_peel_step` does not
# recognize (a slice, a no-op `|=`, `with_entries`) and so runs through the
# owned-value re-index bridge before the read reaches it, same as a fold's
# UPDATE that is not one of the owned fast paths -- see
# docs/compliance/jq/limitations.md's #2889 section.
#
# The `owned-embed-array-multi-*`, `owned-embed-object-add` and
# `owned-embed-array-tie-*` rows are the #2889 review: the relocating fold
# was reached only from the generic evaluator, so it fired for the
# single-element `[.]` alone -- every wider container collapses to
# `GenericItem::Owned` and re-enters through `eval.rs`, which now runs the
# fold too (as a pipe *stage*, which is how it arrives there). The tie rows
# pin which of two equal elements each builtin keeps: `max` the last, `min`
# the first, so the `-loser` spellings are rows where jq itself refuses and
# both binaries must agree on exit 5.
# The `-n 'input | ...'` rows are expressed the same way the pre-existing
# `in-evaluator-input-*` rows are: the document twice on stdin with no `-n`,
# so a leading bare `input` call consumes the first copy and binds the
# second -- this script has no `-n` support of its own.
#
# The `owned-root-*` rows are #3135, the owned-rooted twin of #3037: the same
# navigated bind made on a document with no live cursor behind it -- the
# input queue (`input | .a as $y`), a rebuilt root (`(tojson|fromjson) | .a
# as $y`; `jq -n`'s constructed root takes the same route but has no input
# column here, so it is pinned in `tests/jq_cli_tests.rs`). Such a pipe runs
# on the owned identity pipe, whose position tokens name the bind's node, and
# `marker_is_root` reads them against the funnel's `OwnedRoot` witness. The
# sibling/rebuilt/constructed rows are carried as `agree` rows (both exit 5);
# `no-resolver-use` pins that a bind the resolver never reads keeps its
# route; `wrapped-*` pin the `first(...)`/`[...]` spellings, which reach the
# same door through the eager re-entries.
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
in-evaluator-input-array-collect-embed	{"a":1} {"a":1}	[input | . as $x | {k:.} | .k | path($x)]
in-evaluator-input-array-collect-inputs	{"a":1} {"a":1}	[inputs | . as $x | {k:.} | .k | path($x)]
in-evaluator-input-array-collect-sort	{"a":1} {"a":1}	[input | . as $x | [.] | sort | .[0] | path($x)]
in-evaluator-input-array-collect-write	{"a":1} {"a":1}	[input | . as $x | {k:.} | .k | ($x.a) = 5]
in-evaluator-input-array-collect-rebuilt-copy	{"a":1} {"a":1}	[input | . as $x | [{"a":1}] | .[0] | path($x)]
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
owned-root-input-assign	{"a":{"b":1}} {"a":{"b":1}}	input | .a as $y | .a | ($y.b) = 9
owned-root-input-del	{"a":{"b":1}} {"a":{"b":1}}	input | .a as $y | .a | del($y.b)
owned-root-input-update	{"a":{"b":1}} {"a":{"b":1}}	input | .a as $y | .a | $y |= 5
owned-root-input-compound	{"a":{"b":1}} {"a":{"b":1}}	input | .a as $y | .a | ($y.b) += 1
owned-root-input-alt-assign	{"a":{"b":1}} {"a":{"b":1}}	input | .a as $y | .a | ($y.b) //= 7
owned-root-input-scalar	{"a":{"b":1}} {"a":{"b":1}}	input | .a.b as $y | .a.b | path($y)
owned-root-input-paths	{"a":{"b":1}} {"a":{"b":1}}	input | .a as $y | .a | [paths($y)]
owned-root-input-first-passthrough	{"a":{"b":1}} {"a":{"b":1}}	input | .a as $y | .a | first(.) | path($y)
owned-root-input-def	{"a":{"b":1}} {"a":{"b":1}}	input | def f: .a as $y | .a | path($y); f
owned-root-input-wrapped-first	{"a":{"b":1}} {"a":{"b":1}}	first(input | .a as $y | .a | path($y))
owned-root-input-wrapped-array	{"a":{"b":1}} {"a":{"b":1}}	[input | .a as $y | .a | path($y)]
owned-root-input-update-path-base	{"a":{"b":1}} {"a":{"b":1}}	input | .a as $y | .a | $y |= (. + {p: [path(.)]})
owned-root-input-sibling-path	{"a":{"b":1},"c":{"b":1}} {"a":{"b":1},"c":{"b":1}}	input | .a as $y | .c | path($y)
owned-root-input-sibling-assign	{"a":{"b":1},"c":{"b":1}} {"a":{"b":1},"c":{"b":1}}	input | .a as $y | .c | ($y.b) = 9
owned-root-input-rebuilt-between	{"a":{"b":1}} {"a":{"b":1}}	input | .a as $y | .a | (tojson|fromjson) | ($y.b) = 9
owned-root-input-constructed-between	{"a":{"b":1}} {"a":{"b":1}}	input | .a as $y | .a | {b:1} | ($y.b) = 9
owned-root-input-iterate-source	{"a":{"b":1},"c":{"b":1}} {"a":{"b":1},"c":{"b":1}}	input | .[] as $y | .a | path($y)
owned-root-input-root-snapshot-beside	{"a":{"b":1}} {"a":{"b":1}}	input | . as $x | .a as $y | .a | path($y), path($x)
owned-root-input-no-resolver-use	{"a":{"b":1}} {"a":{"b":1}}	input | .a as $y | ($y|length)
owned-root-rebuilt-assign	{"a":{"b":1}}	(tojson|fromjson) | .a as $y | .a | ($y.b) = 9
owned-root-rebuilt-below-root	{"a":{"b":{"c":1}}}	.a | (tojson|fromjson) | .b as $y | .b | path($y)
owned-root-rebuilt-array-del	{"a":[1,2]}	(tojson|fromjson) | .a as $y | .a | del($y[0])
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
fold-body-nested-try	{"a":{"b":1}}	del(foreach .a as $v (.; try ($v | .b); .))
fold-body-nested-if	{"a":{"b":1}}	path(foreach .a as $v (.; if true then ($v | .b) else . end; .))
fold-extract-nested-try	{"a":{"b":1}}	path(.a as $y | .a | foreach range(1) as $i (0; .; try ($y | .b)))
fold-body-nested-try-sibling-control	{"a":{"b":1},"c":{"b":1}}	del(.a as $y | foreach .c as $v (.; try ($y | .b); .))
fold-body-fanout-declines	{"a":{"b":1}}	(foreach .a as {a:$v} ?// {c:$v} (0; ($v[0]?, $v))) = 9
fold-body-fanout-declines-del	{"a":{"b":1}}	del(foreach .a as $v (.; (($v | .b?), 1); try ($v | .b?)))
owned-embed-agree-array-element	{"a":1}	. as $x | [.] | .[0] | path($x)
owned-embed-refuse-path-nested-embed	{"a":1}	. as $x | [.] | path(.[0] | $x)
owned-embed-path-nested-multi-first	{"a":1}	. as $x | [.,.] | path(.[0] | $x)
owned-embed-path-nested-multi-second	{"a":1}	. as $x | [.,.] | path(.[1] | $x)
owned-embed-path-nested-object	{"a":1}	. as $x | {k:.} | path(.k | $x)
owned-embed-path-nested-deep	{"a":1}	. as $x | [[.]] | path(.[0][0] | $x)
owned-embed-path-nested-iterate	{"a":1}	. as $x | [.] | path(.[] | $x)
owned-embed-path-nested-continue	{"a":1}	. as $x | [.] | path(.[0] | $x | .a)
owned-embed-path-nested-identity-stage	{"a":1}	. as $x | [.] | . | path(.[0] | $x)
owned-embed-path-nested-tail	{"a":1}	. as $x | [.,.] | path(.[] | $x) | .[0]
owned-embed-path-nested-tail-stop	{"a":1}	. as $x | first([.,.] | path(.[] | $x) | .[0])
owned-embed-path-nested-tail-raise	{"a":1}	. as $x | [.,.] | path(.[] | $x) | error
owned-embed-path-nested-array-root	[1]	. as $x | [.] | path(.[0] | $x)
owned-embed-path-nested-in-fold	{"a":1}	reduce (1) as $i (.; . as $x | [.] | path(.[0] | $x))
owned-embed-path-nested-rebuilt-sibling	{"a":1}	. as $x | [.,{a:1}] | path(.[1] | $x)
owned-embed-path-nested-rebuilt-copy	{"a":1}	. as $x | [{"a":1}] | path(.[0] | $x)
owned-embed-path-nested-past-embed	{"a":1}	. as $x | [.] | path(.[0] | .a | $x)
owned-embed-refuse-path-nested-ancestor-bind	{"a":{"b":1}}	. as $x | .a as $y | [.] | path(.[0].a | $y)
owned-embed-refuse-path-nested-scalar	1	. as $x | [.] | path(.[0] | $x)
owned-embed-refuse-path-nested-comma	{"a":1}	. as $x | [.] | (path(.[0] | $x), path(.[0] | $x | .a))
owned-embed-refuse-path-nested-wrapper	{"a":1}	. as $x | [.] | [limit(1; path(.[] | $x))]
owned-embed-path-nested-del	{"a":1}	. as $x | [.] | del(.[0] | $x)
owned-embed-path-nested-assign	{"a":1}	. as $x | [.] | (.[0] | $x) = 5
owned-embed-path-nested-update	{"a":1}	. as $x | [.] | (.[0] | $x) |= 5
owned-embed-path-nested-update-empty	{"a":1}	. as $x | [.] | (.[0] | $x) |= empty
owned-embed-path-nested-compound	{"a":1}	. as $x | [[.]] | (.[0][0] | $x).a += 1
owned-embed-path-nested-alternative	{"a":1}	. as $x | [.] | (.[0] | $x | .a) //= 3
owned-embed-path-nested-del-object	{"a":1}	. as $x | {k:.} | del(.k | $x)
owned-embed-path-nested-del-fanout	{"a":1}	. as $x | [.,.] | del(.[] | $x)
owned-embed-path-nested-del-computed-key	{"a":1}	. as $x | [.,.,.] | del(.[0,1] | $x)
owned-embed-path-nested-del-optional	{"a":1}	. as $x | [.] | del(.[0]? | $x)
owned-embed-path-nested-assign-rhs-fork	{"a":1}	. as $x | [.] | (.[0] | $x) = (1,2)
owned-embed-path-nested-assign-tail	{"a":1}	. as $x | [.] | (.[0] | $x | .a) = 5 | .[0]
owned-embed-path-nested-del-rebuilt-copy	{"a":1}	. as $x | [{"a":1}] | del(.[0] | $x)
owned-embed-path-nested-assign-rebuilt-copy	{"a":1}	. as $x | [{"a":1}] | (.[0] | $x) = 5
owned-embed-path-nested-del-fanout-rebuilt-sibling	{"a":1}	. as $x | [.,{"a":1}] | del(.[] | $x)
owned-embed-path-nested-del-optional-rebuilt-copy	{"a":1}	. as $x | [{"a":1}] | del((.[0] | $x)?)
owned-embed-path-nested-del-zero-paths	{"a":1}	. as $x | [.] | del(.[0] | $x | .[0]?)
owned-embed-path-nested-update-zero-paths	{"a":1}	. as $x | [.] | (.[0] | $x | .[0]?) |= 5
owned-embed-path-nested-del-arithmetic-key	{"a":1}	. as $x | [.,1] | del(.[.[1] - 1] | $x)
owned-embed-path-nested-del-integral-float	{"a":1}	. as $x | [.,1] | del(.[0.0] | $x)
owned-embed-refuse-path-nested-del-fractional	{"a":1}	. as $x | [.,1] | del(.[-0.5] | $x)
owned-embed-refuse-path-nested-del-slice	[1,2]	. as $x | [.] | del(.[0] | $x | .[0:1])
owned-embed-refuse-path-nested-as-source	{"a":1}	. as $x | [.] | path(.[0] | $x) as $p | $p
owned-embed-refuse-path-nested-reduce-source	{"a":1}	. as $x | [.] | reduce path(.[0] | $x) as $p (0; 1)
owned-embed-refuse-path-nested-binary-operand	{"a":1}	. as $x | [.] | select(path(.[0] | $x) == [0])
owned-embed-refuse-path-nested-first-body	{"a":1}	. as $x | [.] | first(path(.[] | $x) | .[0])
owned-embed-refuse-path-nested-label-body	{"a":1}	. as $x | [.] | label $out | path(.[0] | $x) | ., break $out
owned-embed-path-nested-after-sort	{"a":1}	. as $x | [.] | sort | path(.[0] | $x)
owned-embed-path-nested-after-reverse	{"a":1}	. as $x | [.] | reverse | path(.[0] | $x)
owned-embed-path-nested-after-unique	{"a":1}	. as $x | [.] | unique | path(.[0] | $x)
owned-embed-path-nested-after-to-entries	{"a":1}	. as $x | {k:.} | to_entries | path(.[0].value | $x)
owned-embed-refuse-path-nested-after-with-entries	{"a":1}	. as $x | {k:.} | with_entries(.) | path(.k | $x)
owned-embed-refuse-path-nested-after-add	{"a":1}	. as $x | [[.]] | add | path(.[0] | $x)
owned-embed-refuse-path-nested-after-update	{"a":1}	. as $x | [.] | .[0] |= . | path(.[0] | $x)
owned-embed-path-nested-after-map	{"a":1}	. as $x | [.] | map(.) | path(.[0] | $x)
owned-embed-path-nested-after-slice	{"a":1}	. as $x | [.] | .[0:1] | path(.[0] | $x)
owned-embed-path-nested-getpath	{"a":1}	. as $x | [.] | path(getpath([0]) | $x)
owned-embed-object-member	{"a":1}	. as $x | {k:.} | .k | path($x)
owned-embed-add-empty-right	{"a":1}	. as $x | . + {} | path($x)
owned-embed-fold-empty	{"a":1}	. as $x | reduce empty as $i (.; .) | path($x)
owned-embed-fold-single-identity	{"a":1}	. as $x | reduce (1) as $i (.; .) | path($x)
owned-embed-fold-multi-identity	{"a":1}	. as $x | reduce (1,2) as $i (.; .) | path($x)
owned-embed-fold-if-identity	{"a":1}	. as $x | reduce (1) as $i (.; if true then . else 1 end) | path($x)
owned-embed-foreach-identity	{"a":1}	. as $x | foreach (1) as $i (.; .) | path($x)
owned-embed-foreach-extract-identity	{"a":1}	. as $x | foreach (1) as $i (.; .; .) | path($x)
owned-embed-object-iterate	{"a":1}	. as $x | {k:.} | .[] | path($x)
owned-embed-nested-object-member	{"a":1}	. as $x | {k:{j:.}} | .k.j | path($x)
owned-embed-nested-array-element	{"a":1}	. as $x | [[.]] | .[0][0] | path($x)
owned-embed-array-second-copy	{"a":1}	. as $x | [.,.] | .[1] | path($x)
owned-embed-array-add	{"a":1}	. as $x | [.] | add | path($x)
owned-embed-array-min	{"a":1}	. as $x | [.] | min | path($x)
owned-embed-array-max	{"a":1}	. as $x | [.] | max | path($x)
owned-embed-array-multi-max	{"a":1}	. as $x | [.,.] | max | path($x)
owned-embed-array-multi-min	{"a":1}	. as $x | [.,.] | min | path($x)
owned-embed-array-multi-add-null-head	{"a":1}	. as $x | [null,.] | add | path($x)
owned-embed-array-multi-add-null-tail	{"a":1}	. as $x | [.,null] | add | path($x)
owned-embed-object-add	{"a":1}	. as $x | {a:.} | add | path($x)
owned-embed-array-tie-max-last-wins	{"a":1}	. as $x | [{a:1},.] | max | path($x)
owned-embed-array-tie-min-first-wins	{"a":1}	. as $x | [.,{a:1}] | min | path($x)
owned-embed-array-tie-max-loser	{"a":1}	. as $x | [.,{a:1}] | max | path($x)
owned-embed-array-tie-min-loser	{"a":1}	. as $x | [{a:1},.] | min | path($x)
owned-embed-refuse-identity-stage-max	{"a":1}	. as $x | [.] | . | max | path($x)
owned-embed-identity-stage-twice-max	{"a":1}	. as $x | [.] | . | . | max | path($x)
owned-embed-identity-stage-paren-max	{"a":1}	. as $x | [.] | (. | max) | path($x)
owned-embed-identity-stage-first	{"a":1}	. as $x | [.] | . | .[0] | path($x)
owned-embed-identity-stage-paren-first	{"a":1}	. as $x | [.] | (. | .[0]) | path($x)
owned-embed-identity-stage-map-barrier	[1,2]	[.[]] | map(if . == 2 then error else . end) | . | first
owned-embed-identity-stage-map-barrier-after	[1,2]	[.[]] | . | map(if . == 2 then error else . end) | first
owned-embed-identity-stage-map-barrier-after-index	[1,2]	[.[]] | . | map(if . == 2 then error else . end) | .[0]
owned-embed-identity-stage-map-barrier-paren	[1,2]	[.[]] | (. | map(if . == 2 then error else . end)) | first
owned-embed-paren-stage-index	{"a":1}	. as $x | [.,.] | (.[0]) | path($x)
owned-embed-paren-stage-identity-index	{"a":1}	. as $x | [.,.] | (. | .[0]) | path($x)
owned-embed-paren-stage-index-identity	{"a":1}	. as $x | [.,.] | (.[0] | .) | path($x)
owned-embed-paren-stage-identity-max	{"a":1}	. as $x | [.,.] | (. | max) | path($x)
owned-embed-paren-stage-max	{"a":1}	. as $x | [.,.] | (max) | path($x)
owned-embed-paren-stage-bare-identity	{"a":1}	. as $x | [.,.] | (.) | max | path($x)
owned-embed-paren-stage-object-member	{"a":1}	. as $x | {k:.} | (.k) | path($x)
owned-embed-paren-stage-identity-object-member	{"a":1}	. as $x | {k:.} | (. | .k) | path($x)
owned-embed-paren-stage-iterate	{"a":1}	. as $x | [.,.] | (.[]) | path($x)
owned-embed-sort-element	{"a":1}	. as $x | [.] | sort | .[0] | path($x)
owned-embed-unique-element	{"a":1}	. as $x | [.] | unique | .[0] | path($x)
owned-embed-to-entries-value	{"a":1}	. as $x | {k:.} | to_entries | .[0].value | path($x)
owned-embed-object-getpath	{"a":1}	. as $x | {k:.} | getpath(["k"]) | path($x)
owned-embed-add-null-right	{"a":1}	. as $x | . + null | path($x)
owned-embed-add-null-left	{"a":1}	. as $x | null + . | path($x)
owned-embed-mul-empty-right	{"a":1}	. as $x | . * {} | path($x)
owned-embed-concat-empty-right	{"a":1}	. as $x | [.] + [] | .[0] | path($x)
owned-embed-object-member-write	{"a":1}	. as $x | {k:.} | .k | ($x.a) = 9
owned-embed-add-empty-update	{"a":1}	. as $x | . + {} | ($x.a) |= 9
owned-embed-fold-empty-write	{"a":1}	. as $x | reduce empty as $i (.; .) | ($x.a) = 9
owned-embed-object-member-del	{"a":1}	. as $x | {k:.} | .k | del($x.a)
owned-embed-array-add-write	{"a":1}	. as $x | [.] | add | ($x.a) = 9
owned-embed-add-null-write	{"a":1}	. as $x | . + null | ($x.b) = 2
owned-embed-array-input-concat-empty	[1,2]	. as $x | . + [] | path($x)
owned-embed-array-input-object-member	[1,2]	. as $x | {k:.} | .k | path($x)
owned-embed-agree-array-input-element	[1,2]	. as $x | [.] | .[0] | path($x)
owned-embed-array-input-add	[1,2]	. as $x | [.] | add | path($x)
owned-embed-array-input-concat-write	[1,2]	. as $x | . + [] | ($x[0]) = 9
owned-embed-refuse-scalar-string-root	"s"	. as $x | {k:.} | .k | path($x)
owned-embed-refuse-scalar-number-root	5	. as $x | {k:.} | .k | path($x)
owned-embed-agree-scalar-string-element	"s"	. as $x | [.] | .[0] | path($x)
owned-embed-agree-scalar-number-element	5	. as $x | [.] | .[0] | path($x)
owned-embed-agree-array-negative-index	{"a":1}	. as $x | [.] | .[-1] | path($x)
owned-embed-agree-array-first	{"a":1}	. as $x | [.] | first | path($x)
owned-embed-agree-array-last	{"a":1}	. as $x | [.] | last | path($x)
owned-embed-agree-array-iterate	{"a":1}	. as $x | [.] | .[] | path($x)
owned-embed-agree-limit-element	{"a":1}	. as $x | [limit(1; .)] | .[0] | path($x)
owned-embed-agree-def-identity	{"a":1}	def f: .; . as $x | f | path($x)
owned-embed-agree-rebind-array-element	{"a":1}	. as $x | ([.] | .[0]) as $y | path($y)
owned-embed-agree-direct-write	{"a":1}	. as $x | ($x.a) = 9
owned-embed-agree-first-call	{"a":1}	. as $x | first(.) | path($x)
owned-embed-agree-select-passthrough	{"a":1}	. as $x | select(true) | path($x)
owned-embed-agree-try-passthrough	{"a":1}	. as $x | try . catch 1 | path($x)
owned-embed-refuse-add-empty-left	{"a":1}	. as $x | {} + . | path($x)
owned-embed-refuse-add-nonempty-right	{"a":1}	. as $x | . + {a:1} | path($x)
owned-embed-refuse-array-rebuild	{"a":1}	. as $x | [.[]] | path($x)
owned-embed-refuse-tojson-roundtrip	{"a":1}	. as $x | tojson | fromjson | path($x)
owned-embed-refuse-with-entries	{"a":1}	. as $x | with_entries(.) | path($x)
owned-embed-refuse-update-self	{"a":1}	. as $x | .a |= . | path($x)
owned-embed-refuse-object-literal-member	{"a":1}	. as $x | {k:{a:1}} | .k | path($x)
owned-embed-refuse-object-member-then-literal	{"a":1}	. as $x | {k:.} | .k | {a:1} | path($x)
owned-embed-refuse-fold-literal-body	{"a":1}	. as $x | reduce (1) as $i (.; {a:1}) | path($x)
owned-embed-refuse-literal-write	{"a":1}	. as $x | {a:1} | ($x.a) = 9
owned-embed-refuse-object-member-then-write	{"a":1}	. as $x | {k:.} | .k | .b = 2 | path($x)
owned-embed-refuse-object-member-write-then-write	{"a":1}	. as $x | {k:.} | .k | .b = 2 | ($x.a) = 9
owned-embed-refuse-array-element-then-write	{"a":1}	. as $x | [.] | .[0] | .b = 2 | path($x)
owned-embed-refuse-fold-write-body	{"a":1}	. as $x | reduce (1) as $i (.; .b = 2) | path($x)
owned-embed-refuse-object-member-add-empty-write	{"a":1}	. as $x | {k:.} | .k | . + {} | .b = 2 | path($x)
owned-embed-concat-empty-left	{"a":1}	. as $x | [] + [.] | .[0] | path($x)
owned-embed-refuse-object-member-del-self	{"a":1}	. as $x | {k:.} | .k | del(.a) | path($x)
owned-embed-agree-map-identity	{"a":1}	. as $x | [.] | map(.) | .[0] | path($x)
owned-embed-agree-slice-singleton	{"a":1}	. as $x | [.] | .[0:1] | .[0] | path($x)
owned-embed-reverse-element	{"a":1}	. as $x | [.] | reverse | .[0] | path($x)
owned-embed-refuse-update-noop-element	{"a":1}	. as $x | [.] | .[0] |= . | .[0] | path($x)
owned-embed-refuse-string-add-empty	"s"	. as $x | . + "" | path($x)
owned-embed-refuse-number-add-zero	5	. as $x | . + 0 | path($x)
owned-embed-refuse-concat-empty-left-nonempty-right	[1,2]	. as $x | [] + . | path($x)
owned-embed-refuse-array-map-root	[1,2]	. as $x | map(.) | path($x)
owned-embed-refuse-array-slice	[1,2]	. as $x | .[0:2] | path($x)
owned-embed-refuse-array-reverse-root	[1,2]	. as $x | reverse | path($x)
owned-embed-refuse-input-exhausted	{"a":1}	input | . as $x | {k:.} | .k | path($x)
owned-embed-refuse-input-route-object-member	{"a":1} {"a":1}	input | . as $x | {k:.} | .k | path($x)
owned-embed-refuse-input-route-add-empty	{"a":1} {"a":1}	input | . as $x | . + {} | path($x)
owned-embed-refuse-input-route-fold-empty	{"a":1} {"a":1}	input | . as $x | reduce empty as $i (.; .) | path($x)
owned-embed-refuse-input-route-array-element	{"a":1} {"a":1}	input | . as $x | [.] | .[0] | path($x)
owned-embed-agree-input-route-direct-write	{"a":1} {"a":1}	input | . as $x | ($x.a) = 9
owned-embed-refuse-input-route-literal-write	{"a":1} {"a":1}	input | . as $x | {a:1} | ($x.a) = 9
owned-embed-refuse-input-route-object-member-write	{"a":1} {"a":1}	input | . as $x | {k:.} | .k | ($x.a) = 9
scalar-string-keeps-tostring	"abc"	. as $x | tostring | path($x)
scalar-string-keeps-text	"abc"	. as $x | @text | path($x)
scalar-string-keeps-tostring-twice	"abc"	. as $x | tostring | tostring | path($x)
scalar-string-keeps-ltrimstr-nomatch	"abc"	. as $x | ltrimstr("z") | path($x)
scalar-string-keeps-ltrimstr-nonstring-arg	"abc"	. as $x | ltrimstr(1) | path($x)
scalar-string-keeps-rtrimstr-nomatch	"abc"	. as $x | rtrimstr("z") | path($x)
scalar-string-keeps-sub-nomatch	"abc"	. as $x | sub("z";"y") | path($x)
scalar-string-keeps-gsub-nomatch	"abc"	. as $x | gsub("z";"y") | path($x)
scalar-string-keeps-add-one	"abc"	. as $x | [.] | add | path($x)
scalar-string-keeps-min-one	"abc"	. as $x | [.] | min | path($x)
scalar-string-keeps-max-one	"abc"	. as $x | [.] | max | path($x)
scalar-string-keeps-sort-one	"abc"	. as $x | [.] | sort | .[0] | path($x)
scalar-string-keeps-unique-one	"abc"	. as $x | [.] | unique | .[0] | path($x)
scalar-string-keeps-first-one	"abc"	. as $x | [.] | first | path($x)
scalar-string-keeps-flatten-one	"abc"	. as $x | [.] | flatten | .[0] | path($x)
scalar-string-keeps-reverse-one	"abc"	. as $x | [.] | reverse | .[0] | path($x)
scalar-string-keeps-if	"abc"	. as $x | if . then . else . end | path($x)
scalar-string-keeps-select	"abc"	. as $x | select(true) | path($x)
scalar-string-keeps-getpath-empty	"abc"	. as $x | getpath([]) | path($x)
scalar-string-keeps-setpath-empty	"abc"	. as $x | setpath([]; .) | path($x)
scalar-string-keeps-limit	"abc"	. as $x | limit(1; .) | path($x)
scalar-string-keeps-first-f	"abc"	. as $x | first(.) | path($x)
scalar-string-keeps-recurse	"abc"	. as $x | recurse | path($x)
scalar-string-keeps-dotdot	"abc"	. as $x | .. | path($x)
scalar-string-keeps-try	"abc"	. as $x | try . catch . | path($x)
scalar-string-keeps-reduce-empty	"abc"	. as $x | reduce empty as $i (.; .) | path($x)
scalar-string-keeps-alt-null	"abc"	. as $x | null // . | path($x)
scalar-string-keeps-alt-self	"abc"	. as $x | (. // 1) | path($x)
scalar-string-keeps-destructure-alt	"abc"	. as $x | . as [$a] ?// $a | $a | path($x)
scalar-string-keeps-length-bind	"abc"	. as $x | length as $l | . | path($x)
scalar-string-keeps-test-bind	"abc"	. as $x | test("a") as $t | . | path($x)
scalar-string-keeps-startswith-bind	"abc"	. as $x | startswith("a") as $b | . | path($x)
scalar-string-fresh-ascii-downcase	"abc"	. as $x | ascii_downcase | path($x)
scalar-string-fresh-ascii-upcase	"abc"	. as $x | ascii_upcase | path($x)
scalar-string-fresh-ltrimstr-match	"abc"	. as $x | ltrimstr("a") | path($x)
scalar-string-fresh-rtrimstr-match	"abc"	. as $x | rtrimstr("c") | path($x)
scalar-string-fresh-concat-empty-right	"abc"	. as $x | . + "" | path($x)
scalar-string-fresh-concat-empty-left	"abc"	. as $x | "" + . | path($x)
scalar-string-fresh-tojson-fromjson	"abc"	. as $x | tojson | fromjson | path($x)
scalar-string-fresh-json-fromjson	"abc"	. as $x | @json | fromjson | path($x)
scalar-string-fresh-splits	"abc"	. as $x | splits("z") | path($x)
scalar-string-fresh-split-first	"abc"	. as $x | split("z") | .[0] | path($x)
scalar-string-fresh-join	"abc"	. as $x | [.,.] | join("") | path($x)
scalar-string-fresh-slice-full	"abc"	. as $x | .[0:] | path($x)
scalar-string-fresh-slice	"abc"	. as $x | .[0:1] | path($x)
scalar-string-fresh-explode-implode	"abc"	. as $x | explode | implode | path($x)
scalar-string-fresh-base64-roundtrip	"abc"	. as $x | @base64 | @base64d | path($x)
scalar-string-fresh-sub-match	"abc"	. as $x | sub("a";"a") | path($x)
scalar-string-fresh-tojson	"abc"	. as $x | tojson | path($x)
scalar-string-fresh-repeat-one	"abc"	. as $x | . * 1 | path($x)
scalar-string-fresh-string-literal-copy	"abc"	. as $x | "abc" | path($x)
scalar-string-fresh-interp	"abc"	. as $x | "\(.)" | path($x)
scalar-number-keeps-tonumber	5	. as $x | tonumber | path($x)
scalar-number-keeps-abs	5	. as $x | abs | path($x)
scalar-number-keeps-add-one	5	. as $x | [.] | add | path($x)
scalar-number-keeps-min-one	5	. as $x | [.] | min | path($x)
scalar-number-keeps-sort-one	5	. as $x | [.] | sort | .[0] | path($x)
scalar-number-keeps-unique-one	5	. as $x | [.] | unique | .[0] | path($x)
scalar-number-keeps-max-by-one	5	. as $x | [.] | max_by(.) | path($x)
scalar-number-keeps-getpath-empty	5	. as $x | getpath([]) | path($x)
scalar-number-keeps-setpath-empty	5	. as $x | setpath([]; .) | path($x)
scalar-number-keeps-if	5	. as $x | if . then . else . end | path($x)
scalar-number-keeps-alt-self	5	. as $x | (. // 1) | path($x)
scalar-number-keeps-alt-null	5	. as $x | null // . | path($x)
scalar-number-keeps-select	5	. as $x | select(. > 0) | path($x)
scalar-number-keeps-limit	5	. as $x | limit(1; .) | path($x)
scalar-number-keeps-first-f	5	. as $x | first(.) | path($x)
scalar-number-keeps-reduce-empty	5	. as $x | reduce empty as $i (.; .) | path($x)
scalar-number-keeps-try	5	. as $x | try . catch . | path($x)
scalar-number-keeps-infinite-bind	5	. as $x | infinite as $i | . | path($x)
scalar-number-keeps-ltrimstr-passthrough	5	. as $x | ltrimstr("z") | path($x)
scalar-number-keeps-tostring-bind	5	. as $x | tostring as $s | . | path($x)
scalar-number-fresh-floor	5	. as $x | floor | path($x)
scalar-number-fresh-fabs	5	. as $x | fabs | path($x)
scalar-number-fresh-round	5	. as $x | round | path($x)
scalar-number-fresh-add-zero	5	. as $x | . + 0 | path($x)
scalar-number-fresh-neg-neg	5	. as $x | -(-.) | path($x)
scalar-number-fresh-mul-one	5	. as $x | . * 1 | path($x)
scalar-number-fresh-tojson-fromjson	5	. as $x | tojson | fromjson | path($x)
scalar-number-fresh-tostring-tonumber	5	. as $x | tostring | tonumber | path($x)
scalar-number-fresh-text-tonumber	5	. as $x | @text | tonumber | path($x)
scalar-number-fresh-number-literal-copy	5	. as $x | 5 | path($x)
scalar-number-fresh-plain-literal-copy	5	. as $x | 5.0 | path($x)
scalar-constant-pool-number-def	null	def f: 5; f as $x | f | path($x)
scalar-constant-pool-string-def	null	def f: "s"; f as $x | f | path($x)
scalar-constant-pool-number-two-literals	null	5 as $x | 5 | path($x)
scalar-constant-pool-string-two-literals	null	"s" as $x | "s" | path($x)
scalar-constant-pool-array-literal-elements	null	[5,5] | .[0] as $x | .[1] | path($x)
scalar-constant-pool-string-array-literal-elements	null	["s","s"] | .[0] as $x | .[1] | path($x)
scalar-document-equal-siblings-number	[5,5]	.[0] as $x | .[1] | path($x)
scalar-document-equal-siblings-string	["s","s"]	.[0] as $x | .[1] | path($x)
scalar-document-element-embed-number	[5,5]	.[0] as $x | {k:.[0]} | .k | path($x)
scalar-document-element-embed-string	["s","t"]	.[0] as $x | [.[0]] | .[0] | path($x)
scalar-root-identity-string	"s"	. as $x | . | path($x)
scalar-root-rebind-string	"s"	. as $x | {k:.} | .k as $y | .k | path($y)
scalar-root-nested-embed-string	"s"	. as $x | {k:[.]} | .k[0] | path($x)
scalar-root-embed-write-string	"s"	. as $x | {k:.} | .k | $x = 1
scalar-root-embed-write-number	5	. as $x | [.] | .[0] | ($x) |= . + 1
scalar-root-rebuilt-refuses-string	"s"	. as $x | {k:"s"} | .k | path($x)
scalar-root-rebuilt-refuses-number	5	. as $x | {k:5} | .k | path($x)
scalar-root-computed-refuses-number	5	. as $x | {k:(.+0)} | .k | path($x)
scalar-root-float-literal	1.0	. as $x | {k:.} | .k | path($x)
scalar-root-bool	true	. as $x | {k:.} | .k | path($x)
scalar-root-null	null	. as $x | {k:.} | .k | path($x)
scalar-root-bool-literal-copy	true	. as $x | true | path($x)
scalar-nested-embed-yq-untouched-number	5	. as $x | [.] | path(.[0] | $x)
CASES_EOF

# Known refuse-only rows (jq answers, succinctly refuses), each with the
# reason it is deliberately left refusing. A new one is a sweep failure.
# Quoted heredoc: a reason may quote a filter verbatim ($q, "a") without
# the shell expanding it. Read with `read -d ''` rather than `$(cat <<EOF)`
# for the reason given at CASES above (#3037 hit the same scanner from the
# other side: a `(` before a `#` in a reason made bash read the `#` as a
# comment inside the `$( )`, so the closing `)` vanished).
read -r -d '' REFUSE_ONLY <<'REFUSE_EOF' || true
def-body-in-path:a def inside path() resolves as an opaque leaf, before #2042 too
source-rebuilt-container:the source navigates inside a construction, which jq's suspended tracking allows but the resolver refuses; falls back to a plain value
select-wrapped-source:the witness grammar is pure navigation (is_pure_navigation); a select-wrapped source binds by value
alternative-source:the witness grammar is pure navigation; a // source binds by value
if-source:the witness grammar is pure navigation; an if source binds by value
optional-source-spelling:a ? component never matches a plain one on either side (path spelling, not node identity)
full-slice-is-the-array:jq's full slice is the array itself; the bind path ends in a slice component, .a does not
marker-not-at-head:a marker is re-rooted only at the head of a source; elsewhere it is certified against the ambient position
slice-spelling:jq's .a[1:] and .a[1:3] of a 3-array are the same jv; the slice components differ, so the spelling never matches (open-ended twin of full-slice-is-the-array)
destructure-bind-after-pattern:#2649 residue 1 -- a plain bind on the ambient input after a pattern moved the register: resolve_bind_source needs a trackable stage, and the pattern's body stage is not
destructure-alt-navigation:#2649 residue 3 -- the body navigates the ambient input, which raises a near-access refusal the artefact guard cannot tell from an artefact, so the ?// does not retry
destructure-comma-marker-nav:#2649 residue 4 -- pre-existing comma shape: a nested Pipe gets no register, so $q[0] inside a comma raises near-access (limitations.md, #2042)
carried-register-passthrough:pre-existing (#2042): once the register is only *carried* (an untracked stage), a select/label/first/getpath passthrough re-seeds it from the ambient value and the marker no longer re-establishes; if/try/`. as $q | .`/literals keep it. Twin of literal-then-fold-untracked-init, found by the #2649 fuzz
destructure-passthrough-stage:the destructuring door onto carried-register-passthrough -- a pattern body starts on an untracked stage, so the same select/label/first/getpath passthroughs drop the register; the baseline binary refuses the plain-bind twin identically, so this is not #2649's
in-evaluator-input-fold-source:#3036 -- the loop variable of a fold is Snapshot with no node, and UPDATE runs against the re-indexed accumulator; the generic evaluator has refused this since #2642
navigated-bind-positional-assign:#3037 residual -- the marker certified at a non-root register position inside an assignment's own resolver; the path() twin answers since #3179 (the resolver's materialized root now holds $y's own value), the assignment resolver does not materialize through to_owned_cursor
owned-embed-refuse-path-nested-ancestor-bind:#3177 -- $x is bound first, so [.] reuses $x's own value, whose .a is $x's materialization, not $y's: sharing here needs an *ancestor* lookup when $y is bound, the mirror of #3179's nested reuse; jq answers [0,"a"]
owned-embed-refuse-path-nested-comma:#3177 -- path() reached through a comma wrapper is not at the head of the owned re-entry's pipe, so it still crosses the bridge; taking it natively would mean re-implementing the wrapper's driver over an owned value (#3189)
owned-embed-refuse-path-nested-wrapper:#3177 -- same as owned-embed-refuse-path-nested-comma, through an array constructor and limit
owned-embed-refuse-path-nested-del-fractional:#3188 -- the write door will not re-spell a fractional index: del(.[-0.5]) deletes element 0 here and nothing in jq (#3302), so the static spelling would be a wrong answer
owned-embed-refuse-path-nested-del-slice:#3188 -- a slice component has no static spelling the evaluator indexes by (.[{"start":0,"end":1}], #3300), so the write door declines
owned-embed-refuse-path-nested-as-source:#3177 review -- path() as a bind's source is not the head of the owned re-entry's pipe; the as driver owns the inner pipe, so it still bridges (#3189)
owned-embed-refuse-path-nested-reduce-source:#3177 review -- same as owned-embed-refuse-path-nested-as-source, as a reduce source
owned-embed-refuse-path-nested-binary-operand:#3177 review -- same as owned-embed-refuse-path-nested-as-source, as a binary operand inside select
owned-embed-refuse-path-nested-first-body:#3177 review -- same as owned-embed-refuse-path-nested-as-source, as the body of first
owned-embed-refuse-path-nested-label-body:#3177 review -- same as owned-embed-refuse-path-nested-as-source, as the body of label
owned-embed-refuse-path-nested-after-with-entries:#3177 review -- a stage ahead of path() still bridges first, for with_entries (jq ["k"])
owned-embed-refuse-path-nested-after-add:#3177 review -- a stage ahead of path() still bridges first, for a one-element add
owned-embed-refuse-path-nested-after-update:#3177 review -- a stage ahead of path() still bridges first, for a no-op |= (jq's setpath places the same jv)
owned-embed-fold-if-identity:#2889 -- an `if` UPDATE returning `.` is not one of eval_owned_navigation's recognized shapes, so embed_peel_step declines and the accumulator goes through the owned re-index bridge
owned-embed-refuse-update-noop-element:#2889 -- a `|=` writes through the assignment resolver first, which re-indexes before the trailing read reaches embed_peel_step
owned-embed-refuse-array-slice:#2889 -- a slice is not one of embed_peel_step's Field/Index/Iterate shapes, so it re-indexes before the read
identity-if-arms-differ:#2978 -- identity_bind_position is static: an if whose arms sit at different positions ($p at [], . at ["a"]) proves neither, so the bind stays a bare Snapshot and getpath has no position to compose from; jq evaluates the condition
identity-try-if-nonraising:#2978 review -- a try body holding an if is not a passthrough (its condition may raise and bind the value of the handler); the gate is static, so an if whose condition happens not to raise pays a refusal. The raising twin (identity-trap-raising-try-*) is the write-side fabrication this prevents
untracked-opaque-stage-lost-register:#3120 review -- an opaque stage (reduce, a def call, first) drops the carried register, so the walk has none and refuses without retrying; jq refuses the step too and retries onto the bare alternative, whose empty body then writes nothing. main echoed the document by the ambient-null coincidence the fix removes
untracked-later-step-refusal-no-retry:#3120 review -- refusal_is_exact is decided per source, and a marker that is the register is value-equal to it, so a later-step refusal after a certified first step is treated as a guess and does not retry; jq retries onto $w. The trackable twin retries and agrees
catch-payload-own-node-refuse-only:#3133 -- error(.) raises the register node itself and jq answers ["a"]; the payload equals the register by value but is not null/bool and carries no marker, so it cannot be told from a rebuilt copy (catch-rebuilt-payload-refuses) and the handler stays untracked
computed-identity-bind-mixed-if:#3133 -- an if source with one arm a computed `.` and the other a marker binds Untracked on an untracked stage (the condition is not evaluated); jq evaluates it and binds the marker
fold-body-fanout-declines:#3145 review -- the register reaches a fold body only when that body cannot fan out: a nested pipe sees it while a sibling branch of the same multi-output body does not, so an UPDATE that refused wholesale could half-succeed and drive a ?// retry jq never performs, writing a key jq never names. Refusing the whole body is the safe side of that asymmetry
fold-body-fanout-declines-del:#3145 review -- the del twin: jq writes {"a":{}}, the half-success wrote nothing at exit 0, and declining refuses loudly instead
owned-embed-refuse-scalar-string-root:#2889/#3182 -- a scalar root is not Rc-backed, so it can never enter or be witnessed by the embed table. ADR-0024's option D (refcounted `String`/spelling) was built and measured on 2026-09-20 and rejected: +8% to +50% peak RSS on scalar-heavy rows for the identity it buys, see the ADR's option D result
owned-embed-refuse-scalar-number-root:same as owned-embed-refuse-scalar-string-root, for a number root
owned-embed-refuse-path-nested-scalar:same as owned-embed-refuse-scalar-string-root, at a nested position (#3177)
scalar-string-keeps-tostring:same as owned-embed-refuse-scalar-string-root
scalar-string-keeps-text:same as owned-embed-refuse-scalar-string-root
scalar-string-keeps-tostring-twice:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-string-keeps-ltrimstr-nomatch:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-string-keeps-ltrimstr-nonstring-arg:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-string-keeps-rtrimstr-nomatch:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-string-keeps-sub-nomatch:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-string-keeps-gsub-nomatch:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-string-keeps-add-one:same as owned-embed-refuse-scalar-string-root
scalar-string-keeps-min-one:same as owned-embed-refuse-scalar-string-root
scalar-string-keeps-max-one:same as owned-embed-refuse-scalar-string-root
scalar-string-keeps-sort-one:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-string-keeps-unique-one:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-string-keeps-flatten-one:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-string-keeps-reverse-one:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-string-keeps-setpath-empty:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-string-keeps-reduce-empty:same as owned-embed-refuse-scalar-string-root
scalar-string-keeps-destructure-alt:same as owned-embed-refuse-scalar-string-root
scalar-number-keeps-tonumber:same as owned-embed-refuse-scalar-string-root
scalar-number-keeps-abs:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-number-keeps-add-one:same as owned-embed-refuse-scalar-string-root
scalar-number-keeps-min-one:same as owned-embed-refuse-scalar-string-root
scalar-number-keeps-sort-one:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-number-keeps-unique-one:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-number-keeps-max-by-one:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-number-keeps-setpath-empty:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-number-keeps-reduce-empty:same as owned-embed-refuse-scalar-string-root
scalar-number-keeps-ltrimstr-passthrough:same as owned-embed-refuse-scalar-string-root; this builtin is also bridged through the owned re-index round trip (the class #3178 closed only for sort/unique/reverse/to_entries/getpath), so it would stay refuse-only even with scalar storage identity
scalar-constant-pool-number-def:same as owned-embed-refuse-scalar-string-root: the literal's value is a fresh materialization on each evaluation here, where jq's constant pool loads one `jv`; the two-literal twin refuses in both
scalar-constant-pool-string-def:same as owned-embed-refuse-scalar-string-root: the literal's value is a fresh materialization on each evaluation here, where jq's constant pool loads one `jv`; the two-literal twin refuses in both
scalar-document-element-embed-number:same as owned-embed-refuse-scalar-string-root
scalar-root-nested-embed-string:same as owned-embed-refuse-scalar-string-root
scalar-root-embed-write-string:same as owned-embed-refuse-scalar-string-root
scalar-root-float-literal:same as owned-embed-refuse-scalar-string-root
scalar-nested-embed-yq-untouched-number:same as owned-embed-refuse-scalar-string-root
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
