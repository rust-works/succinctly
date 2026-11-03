#!/usr/bin/env python3
"""Differential grid for a fold source that moves jq's path register inside another fold (#4505).

A `reduce`/`foreach` SOURCE inside another fold's UPDATE runs path-checked `INDEX` steps (a
navigation, or an array/object pattern) on whatever it meets. When it meets the register (`.`
or an outer `$x` bound to it) a `foreach` source does not backtrack the move, so the enclosing
fold's later emission of the accumulator raises in jq; when an enclosing `reduce` source already
moved the register onto an element, neither `.` nor `$x` is the register and the step raises for a
`reduce` source too. `jq-path-register-sweep.py` has no outer-`$x`-destructured-in-a-body operand,
so it cannot gate this: the grid here is outer fold x inner fold x source shape x write context x
document, 28 x 3 x 159 x 7 x 13 = 1,215,396 rows (6 x 3 x 56 x 7 x 13 when it was written).

Rows are classified against jq (`/usr/bin/jq`) per build, as `jq-path-register-sweep.py` does:
MATCH, ACCEPT_WRONG (jq refused, the build answered: the dangerous one, `del`/`=`/`|=` write
through it), REFUSE_WRONG (the safe direction) and DIFF. Exit status 1 when the candidate has an
ACCEPT_WRONG outside the documented residual (`KNOWN_ACCEPT_WRONG`), a row that matched jq on
`--base` and no longer does (a lost match: a refuse-only fallback has leaked under `try` before,
#3186/#3267), or a candidate timeout. Run it against a `main` build as `--base` to see what a change moved.
`--jq-gate` also fails the run on any DIFF of the candidate against jq (a wrong answer where both answered,
which the ACCEPT_WRONG rule does not see). No DIFF is documented today, and it is how `succinctly dev compare jq
--dimension sweeps` runs this script (#5552), over `--sample K`: about one row in K of the 1.2M-row grid, chosen
by a hash of the row's index and `--seed` while the grid is generated.

Usage: scripts/jq-fold-body-register-grid-sweep.py --bin BIN [--base BIN] [--jq JQ] [--show N]
                                                   [--only SUBSTR[,SUBSTR...]] [--only-source SUBSTR[,SUBSTR...]]
                                                   [--jobs N] [--wrapped | --deep] [--dump FILE]
                                                   [--sample K] [--seed S] [--jq-gate]
"""
import argparse
import collections
import concurrent.futures
import itertools
import json
import subprocess
import sys
import time

OUTER = [
    ("fe-lit", "foreach (1,2) as $i (.; {I}; .)"),
    ("rd-lit", "reduce (1,2) as $i (.; {I})"),
    # a navigating source leaves jq's register on the element for the whole step
    ("fe-nav", "foreach .[]? as $i (.; {I}; .)"),
    ("rd-nav", "reduce .[]? as $i (.; {I})"),
    ("fe-nava", "foreach .a? as $i (.; {I}; .)"),
    ("rd-nava", "reduce .a? as $i (.; {I})"),
    # (#4790) a source that moves the register without naming the element's position: `..` and a comma of
    # navigations leave it on the element each output is, so a `null` `$x` is not it unless that element is `null`
    ("rd-rec", "reduce .. as $i (.; {I})"),
    ("rd-comma", "reduce (.a?, .) as $i (.; {I})"),
    # (#4795) a subset of navigation's outputs (`first`/`last`/`limit`/`nth`) and a `select` stage after one
    # leave the register on the node each output is; the computed ones (`tostring`, a literal) and an empty
    # `last` (which yields `null`) are the controls that must not
    ("rd-first-rec", "reduce first(..) as $i (.; {I})"),
    ("rd-last-rec", "reduce last(..) as $i (.; {I})"),
    ("rd-limit1-rec", "reduce limit(1; ..) as $i (.; {I})"),
    ("rd-limit2-rec", "reduce limit(2; ..) as $i (.; {I})"),
    ("rd-nth-rec", "reduce nth(1; ..) as $i (.; {I})"),
    ("rd-sel-rec", "reduce (.. | select(. != 5)) as $i (.; {I})"),
    ("rd-sel-null", "reduce (.. | select(. == null)) as $i (.; {I})"),
    ("rd-sel-nav", "reduce (.[]? | select(. != 5)) as $i (.; {I})"),
    ("rd-first-nav", "reduce first(.[]?) as $i (.; {I})"),
    ("rd-last-nav", "reduce last(.[]?) as $i (.; {I})"),
    ("rd-last-nava", "reduce last(.a?) as $i (.; {I})"),
    ("rd-last-comma", "reduce last(.a?, .) as $i (.; {I})"),
    ("rd-last-empty", "reduce last(empty) as $i (.; {I})"),
    ("rd-first-str", "reduce first(.[]? | tostring) as $i (.; {I})"),
    ("rd-limit-lit", "reduce limit(2; 1, 2) as $i (.; {I})"),
    ("rd-sel-str", "reduce (.. | select(. != 5) | tostring) as $i (.; {I})"),
    ("fe-first-rec", "foreach first(..) as $i (.; {I}; .)"),
    ("fe-sel-rec", "foreach (.. | select(. != 5)) as $i (.; {I}; .)"),
    # (#4627) EXTRACT is the loop element, which a navigating source leaves at jq's register
    ("fe-nav-i", "foreach .[]? as $i (.; {I}; $i)"),
    ("fe-nava-i", "foreach .a? as $i (.; {I}; $i)"),
]
# (#4648) the inner fold three levels down: a fold in the UPDATE of a fold that is the SOURCE of another
# fold in the UPDATE of the outer one, and a fold in the UPDATE of a fold in the UPDATE of the outer one.
# The live register an untracked fold holds reaches the innermost through two bodies. Opt-in (`--deep`):
# `main` is wrong on hundreds of these rows (a `null` literal bound to a variable and destructured, as the
# source of a fold in an UPDATE), so the run is judged against `--base` alone, as `--wrapped` is.
DEEP_OUTER = [
    ("fe3-fe-src", "foreach (1,2) as $i (.; foreach (foreach (1,2) as $k (.; {I}; .)) as $z (.; .; .); .)"),
    ("rd3-fe-src", "reduce (1,2) as $i (.; reduce (foreach (1,2) as $k (.; {I}; .)) as $z (.; .))"),
    ("fe3-fe-upd", "foreach (1,2) as $i (.; foreach (1,2) as $k (.; {I}; .); .)"),
    ("rd3-rd-upd", "reduce (1,2) as $i (.; reduce (1,2) as $k (.; {I}))"),
]
INNER = [
    ("fe", "foreach ({S}) as $y (.; .; .)"),
    ("fe-y", "foreach ({S}) as $y (.; $y; .)"),  # EXTRACT is the element: it is the register
    ("rd", "reduce ({S}) as $y (.; .)"),
]
SOURCES = [
    ("x-pat", "$x as {a:$a} | ."),
    ("x-pat-arr", "$x as [$a] | ."),
    ("sel-x-xpat", "select($x) | $x as {a:$a} | ."),
    ("sel-x-dotpat", "select($x) | . as {a:$a} | ."),
    ("sel-1-xpat", "select(1) | $x as {a:$a} | ."),
    ("sel-x-dotpat-arr", "select($x) | . as [$a] | ."),
    ("try-xpat", "try ($x as {a:$a} | .)"),
    ("try-xpat-catch", "try ($x as {a:$a} | .) catch ."),
    ("opt-xpat", "($x as {a:$a} | .)?"),
    ("try-dotpat", "try (. as {a:$a} | .)"),
    ("try-dotpat-catch", "try (. as {a:$a} | .) catch ."),
    ("x-nav", "$x | .a"),
    ("x-nava", "$x | .a?"),
    ("x-nav-pat", "($x | .a) as {b:$b} | ."),
    ("x-idx", "$x | .[0]"),
    ("x-iter", "$x | .[]?"),
    ("first-xpat", "first($x as {a:$a} | .)"),
    ("if-xpat", "if $x then $x as {a:$a} | . else . end"),
    # (#4604) the marker's value picks the branch: only the taken one can destructure
    ("if-x-else-xpat", "if $x then . else $x as {a:$a} | . end"),
    ("if-notx-xpat", "if $x|not then . else $x as {a:$a} | . end"),
    ("if-notx-then-xpat", "if $x|not then $x as {a:$a} | . else . end"),
    ("x-then-dotpat", "$x | . as {a:$a} | ."),
    ("dotpat", ". as {a:$a} | ."),
    ("sel-x-only", "select($x)"),
    ("x-only", "$x"),
    ("sel-x-nav", "select($x) | .a"),
    ("x-sel-pat", "$x | select(.) | . as {a:$a} | ."),
    ("comma-xpat", "($x as {a:$a} | .), 1"),
    ("lim-xpat", "limit(1; $x as {a:$a} | .)"),
    ("dot-nav", ".a"),
    ("dot-nava", ".a?"),
    ("dot-idx", ".[0]"),
    ("dot-iter", ".[]?"),
    ("dot-sel-nav", "select(.) | .a"),
    ("dot-try-nav", "try .a"),
    ("dot-try-pat", "try (. as [$a] | .)"),
    # a `select` that passes nothing never reaches the destructure behind it; a `?` swallows its raise
    ("sel-false-dotpat", "select(false) | . as {a:$a} | ."),
    ("sel-empty-dotpat", "select(empty) | . as {a:$a} | ."),
    ("sel-not-x-dotpat", "select($x|not) | . as {a:$a} | ."),
    # (#4604) a pure condition over the marker decides by its value: a passing one hands the register on
    ("sel-x-ne-null-dotpat", "select($x != null) | . as {a:$a} | ."),
    ("sel-x-len-dotpat", "select(($x|length) > 0) | . as {a:$a} | ."),
    ("sel-x-has-dotpat", "select($x|has(\"b\")) | . as {a:$a} | ."),
    ("sel-x-ne-null-xpat", "select($x != null) | $x as {a:$a} | ."),
    ("sel-dot-opt-pat", "select(.) | (. as [$a] | .)?"),
    ("dot-opt-pat", "(. as [$a] | .)?"),
    # a `try`/`?` catches the pattern's type error (nothing moved the register) but not a success;
    # a `?//` chain retries the next alternative past the pattern
    ("try-chain-catch", "try ($x as {a:$a} ?// [$a] | .) catch ."),
    # the chain ends on its last alternative (#4585): it type-errors into the handler, or matches and
    # moves the register, or is a bare `$v` that moves nothing
    ("try-chain-catch-rev", "try ($x as [$a] ?// {a:$a} | .) catch ."),
    ("try-chain-catch-miss", "try ($x as {b:$a} ?// {a:$a} | .) catch ."),
    ("try-chain-catch-var", "try ($x as {a:$a} ?// $b | .) catch ."),
    ("try-chain-catch-3", "try ($x as {a:$a} ?// [$a] ?// $b | .) catch ."),
    ("try-chain-catch-3-arr", "try ($x as {a:$a} ?// {b:$a} ?// [$a] | .) catch ."),
    ("try-chain-catch-dot", "try (. as {a:$a} ?// [$a] | .) catch ."),
    # a last alternative of several entries that matches is answered, and a body that navigates is
    # not the register's emission
    ("try-chain-catch-multi", "try ($x as {a:$a} ?// [$a,$b] | .) catch ."),
    ("try-chain-catch-multi-obj", "try ($x as {a:$a,b:$c} ?// {a:$a,b:$c} | .) catch ."),
    ("try-chain-catch-nav", "try ($x as {a:$a} ?// [$a] | .a) catch ."),
    ("try-chain", "try ($x as {a:$a} ?// [$a] | .)"),
    ("opt-chain", "($x as {a:$a} ?// [$a] | .)?"),
    ("try-var-catch", "try ($x as $v | .) catch ."),
    ("try-empty-catch", "try ($x as {} | .) catch ."),
    ("sel-x-opt-pat", "select($x) | (. as {a:$a} | .)?"),
    ("try-arr-catch", "try ($x as [$a] | .) catch ."),
    # (#4616) a fresh value, or a `null`/boolean literal the register is not, bound to a variable and
    # destructured: the pattern's step runs on a node that is not the register
    ("fresh-var", "{a:1} as $n | $n as {b:$b} | ."),
    ("fresh-var-arr", "[1] as $n | $n as [$b] | ."),
    ("lit-null-var", "null as $n | $n as {b:$b} | ."),
    ("lit-true-var", "true as $n | $n as {b:$b} | ."),
    # (#4640) a `try`/`?` around that destructure: the raise is caught, and an `fe-y` EXTRACT reads the caught message
    ("fresh-var-try-catch", "try ({a:1} as $n | $n as {b:$b} | .) catch ."),
    ("fresh-var-try", "try ({a:1} as $n | $n as {b:$b} | .)"),
    ("fresh-var-opt", "({a:1} as $n | $n as {b:$b} | .)?"),
    ("lit-null-var-try-catch", "try (null as $n | $n as {b:$b} | .) catch ."),
    # (#4630) the destructure inside `[ ... ]`: a fresh value raises there too, the register's own does not
    ("fresh-var-wrapped", "{a:1} as $n | [$n as {b:$b} | .]"),
    ("fresh-lit-wrapped", "[{a:1} as {b:$b} | .]"),
    ("dotpat-wrapped", "[. as {b:$b} | .]"),
    # ...but not on a branch that never runs (#4632): jq answers these
    ("wrapped-sel-false", "[select(false) | . as {b:$b} | .]"),
    ("wrapped-limit-0", "[limit(0; . as {b:$b} | .)]"),
    # (#4941) a stage that hands its input on once and cannot raise ahead of the destructure is no control flow
    ("wrapped-sel-true", "[select(true) | . as {b:$b} | .]"),
    ("wrapped-sel-1", "[select(1) | . as {b:$b} | .]"),
    ("wrapped-first-dot", "[first(.) | . as {b:$b} | .]"),
    ("wrapped-limit-dot", "[limit(2; .) | . as {b:$b} | .]"),
    ("wrapped-opt-dot", "[(.)? | . as {b:$b} | .]"),
    ("wrapped-sel-true-alt", "[false // (select(true) | . as {b:$b} | .)]"),
    ("wrapped-sel-false-then", "[select(true) | select(false) | . as {b:$b} | .]"),
    ("wrapped-if-false", "[if false then . as {b:$b} | . else 1 end]"),
    ("wrapped-first", "[first(1, (. as {b:$b} | .))]"),
    # (#4635) ...but a `//` right operand whose left cannot be truthy, and the handler of a `try`
    # that can only raise, do run: jq raises on their destructure of `.`
    ("wrapped-alt-false", "[false // (. as {b:$b} | .)]"),
    ("wrapped-alt-null", "[null // (. as {b:$b} | .)]"),
    ("wrapped-alt-empty", "[empty // (. as {b:$b} | .)]"),
    ("wrapped-alt-notnot", "[(false|not|not) // (. as {b:$b} | .)]"),
    ("wrapped-catch-error", "[try error catch (. as {b:$b} | .)]"),
    ("wrapped-catch-error-msg", "[try error(\"x\") catch (. as {b:$b} | .)]"),
    # ...and not where the other operand decides: the right operand of a truthy left never runs
    ("wrapped-alt-true", "[true // (. as {b:$b} | .)]"),
    ("wrapped-alt-notfalse", "[(false|not) // (. as {b:$b} | .)]"),
    ("wrapped-catch-ok", "[try 1 catch (. as {b:$b} | .)]"),
    # chained and nested: the part that runs can itself be an alternative or a handler
    ("wrapped-alt-chain", "[false // false // (. as {b:$b} | .)]"),
    ("wrapped-alt-nested", "[false // (null // (. as {b:$b} | .))]"),
    ("wrapped-catch-alt", "[try error catch (false // (. as {b:$b} | .))]"),
    ("wrapped-catch-catch", "[try error catch (try error catch (. as {b:$b} | .))]"),
    ("wrapped-catch-id-error", "[try (. | error) catch (. as {b:$b} | .)]"),
    # ...a body that raises nothing (an earlier stage emits nothing) never runs its handler
    ("wrapped-catch-select-false", "[try (select(false) | error) catch (. as {b:$b} | .)]"),
    ("wrapped-catch-empty", "[try (empty | error) catch (. as {b:$b} | .)]"),
    ("wrapped-alt-true-nested", "[false // (true // (. as {b:$b} | .))]"),
    # (#4677) the same run branches with no brackets, where they are the whole source (or the one live
    # stage of a pipe): outside a moved body the `foreach` route reads them too
    ("bare-alt-false", "false // (. as {b:$b} | .)"),
    ("bare-alt-null", "null // (. as {b:$b} | .)"),
    ("bare-alt-empty", "empty // (. as {b:$b} | .)"),
    ("bare-alt-notnot", "(false|not|not) // (. as {b:$b} | .)"),
    ("bare-alt-chain", "false // false // (. as {b:$b} | .)"),
    ("bare-alt-comma", "(false, null) // (. as {b:$b} | .)"),
    ("bare-alt-comma-truthy", "(false, 1) // (. as {b:$b} | .)"),
    ("bare-alt-pipe-id", "(false // (. as {b:$b} | .)) | ."),
    ("bare-alt-arr", "false // (. as [$q] | .)"),
    ("bare-alt-true", "true // (. as {b:$b} | .)"),
    ("bare-alt-notfalse", "(false|not) // (. as {b:$b} | .)"),
    ("bare-alt-left-destructure", "(. as {b:$b} | .) // 3"),
    ("bare-catch-error", "try error catch (. as {b:$b} | .)"),
    ("bare-catch-error-msg", "try error(\"x\") catch (. as {b:$b} | .)"),
    ("bare-catch-error-obj", "try error({a:1}) catch (. as {b:$b} | .)"),
    ("bare-catch-error-arr", "try error([1]) catch (. as [$b] | .)"),
    ("bare-catch-truthy-alt-error", "try (1 // error) catch (. as {b:$b} | .)"),
    ("bare-catch-ok", "try 1 catch (. as {b:$b} | .)"),
    ("bare-catch-select-false", "try (select(false) | error) catch (. as {b:$b} | .)"),
    # (#4682) the destructure in a `try` body, whatever the handler: the pattern either type-errors into
    # the handler (nothing moved) or succeeds and moves the register
    ("try-body-dot-obj-catch", "try (. as {a:$a} | .) catch ."),
    ("try-body-dot-arr-catch", "try (. as [$a] | .) catch ."),
    ("try-body-dot-obj-catch-empty", "try (. as {a:$a} | .) catch empty"),
    ("try-body-dot-obj-catch-lit", "try (. as {a:$a} | .) catch 1"),
    ("try-body-dot-obj-catch-error", "try (. as {a:$a} | .) catch error"),
    ("try-body-dot-obj-nav-catch", "try (. as {a:$a} | .a) catch ."),
    # (#4621) an `empty` body: the destructure's path error is caught and the handler's output makes
    # the fold emit its accumulator, which is not the register; a handler that emits nothing does not
    ("try-dotpat-empty-catch", "try (. as [$a] | empty) catch ."),
    ("try-dotpat-empty-catch-empty", "try (. as [$a] | empty) catch empty"),
    ("try-dotpat-empty", "try (. as [$a] | empty)"),
    # (#4626) the same with an outer `$x` as the destructured value: off the register the step raises
    ("xpat-arr-empty", "$x as [$a] | empty"),
    ("xpat-arr-empty-5", "$x as [$a] | empty | 5"),
    # (#4621) a `?//` chain of destructuring alternatives raises in the last; a bare `$var` alternative runs
    ("xpat-chain-empty", "$x as {a:$a} ?// [$a] | empty"),
    ("try-xpat-chain-empty-catch", "try ($x as {a:$a} ?// [$a] | empty) catch ."),
    ("xpat-chain-var-empty", "$x as [$a] ?// $b | empty"),
    # (#4655) a bind ahead of the destructure: its body runs whenever the bound source emits
    ("bind-comma-xpat-empty", "(1,2) as $q | $x as [$a] | empty"),
    ("bind-obj-xpat-empty", "{a:1} as $q | $x as [$a] | empty"),
    ("bind-arr-chain-empty", "[1] as $q | $x as {a:$a} ?// [$a] | empty"),
    ("bind-gen-xpat-empty", ".[]? as $q | $x as [$a] | empty"),
    ("bind-none-xpat-empty", "empty as $q | $x as [$a] | empty"),
    ("bind-sel-xpat-empty", "(select(false)) as $q | $x as [$a] | empty"),
    ("bind-arrsel-xpat-empty", "[select(false)] as $q | $x as [$a] | empty"),
    ("bind-var-xpat-empty", "$i as $q | $x as [$a] | empty"),
    ("bind-keyexpr-xpat-empty", "{(empty):1} as $q | $x as [$a] | empty"),
    ("bind-limit0-xpat-empty", "limit(0;1) as $q | $x as [$a] | empty"),
    ("try-xpat-arr-empty-catch", "try ($x as [$a] | empty) catch ."),
    ("try-xpat-arr-empty-catch-empty", "try ($x as [$a] | empty) catch empty"),
    ("try-xpat-arr-empty", "try ($x as [$a] | empty)"),
    # the outer fold's own loop variable is another value, not the register
    ("i-idx", "$i | .[0]"),
    ("i-nava", "$i | .a?"),
    # a literal `null`/boolean bound by a destructure (#4603): jq's `jv_identical` makes it the register
    # only when the register holds the same one, otherwise the pattern's step raises
    ("lit-null-pat", "null as {a:$a} | ."),
    ("lit-null-arr", "null as [$a] | ."),
    ("lit-true-pat", "true as {a:$a} | ."),
    ("lit-false-arr", "false as [$a] | ."),
    ("lit-null-var-a", "null as $n | $n as {a:$a} | ."),
    ("try-lit-null", "try (null as {a:$a} | .)"),
    ("try-lit-null-catch", "try (null as {a:$a} | .) catch ."),
    ("opt-lit-null", "(null as {a:$a} | .)?"),
    ("sel-lit-null", "select(1) | null as {a:$a} | ."),
    ("lit-null-chain", "null as {a:$a} ?// [$a] | ."),
    ("lit-null-comma", "(null as {a:$a} | .), 1"),
]
CONTEXTS = [
    ("path", "path(. as $x | {O})"),
    ("del", "del(. as $x | {O})"),
    ("update", "(. as $x | {O}) |= 9"),
    ("assign", "(. as $x | {O}) = 9"),
    # `$x` bound to a node that is not the register (#4588): a navigation, `null` included.
    ("path-nav", "path(.a as $x | {O})"),
    ("del-nav", "del(.a as $x | {O})"),
    ("update-nav", "(.a as $x | {O}) |= 9"),
]
# (#4637) an enclosing `?`/`try` around the whole fold. jq catches a path error raised inside it, but not
# the "with result" one a fold's `null`/value output raises at the end of the path, outside the `try`, so
# the same fold is caught or not by what its source leaves behind. Opt-in (`--wrapped`): these rows were
# never gated, `main` is wrong on hundreds of them, and the run is judged against `--base` alone (a lost
# match or a newly wrong accept fails it).
WRAPPED_CONTEXTS = [
    ("path-opt", "path((. as $x | {O})?)"),
    ("path-try", "path(try (. as $x | {O}))"),
    ("del-opt", "del((. as $x | {O})?)"),
    ("update-try", "(try (. as $x | {O})) |= 9"),
    ("path-nav-opt", "path((.a as $x | {O})?)"),
]
# The ACCEPT_WRONG rows of these contexts still left (#4588): the `bare-catch-error-*` and `try-body-dot-*`
# sources #4677/#4682 added, open on `main` too (180 rows here; 352 more in the contexts outside this set make a run
# exit 1 there as well). They count against a lost match against `--base` like any row, but an ACCEPT_WRONG in them
# does not fail the run. (An `empty` body or a bind ahead of the destructure, a `select($x)` in front of it, and a
# `?//` chain of destructuring patterns over the marker are decided since #4604; a `try`/`?` around the destructure
# of a `$x` bound to another node, as the whole source of a `foreach` that is the whole UPDATE of a `reduce`, since #4709.)
RESIDUAL_CONTEXTS = {"path-nav", "del-nav", "update-nav"}
# ...and their count may not grow: the rows are not enumerated, so a new one is caught by the total.
RESIDUAL_NAV_MAX = 180
DOCS = [
    "null",
    "true",
    "[true]",
    "[null]",
    "[]",
    "[[]]",
    "[[1],[1]]",
    '{"a":true}',
    '{"a":1,"b":2}',
    '{"a":[true]}',
    '{"a":false,"b":null}',
    '{"a":{"b":1}}',
    '[{"a":1}]',
]

# The ACCEPT_WRONG rows left: the `null`-literal variable below. (A `try ... catch` or `?` around a single destructure was listed
# here until #4575, a comma beside the destructure on a `null` document until #4576, and a
# `try ... catch` around a `?//` chain of destructures until #4585.)
KNOWN_ACCEPT_WRONG = {
    # a comma beside a literal `null` destructure, on a `null` document, under `|=`/`=` (#4603)
    ("lit-null-comma", "null"),
    # (#4655) a bind whose source may emit nothing is read as possibly skipping the destructure behind it
    # (`.[]?` does not, on the element jq's loop holds), so the moved `reduce` body is answered where jq raises
    ("bind-gen-xpat-empty", None),
}


def run(prefix, prog, doc, timeout=10):
    cmd = [*prefix, "-c", prog]
    try:
        p = subprocess.run(cmd, input=doc, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout
    except subprocess.TimeoutExpired:
        return "TIMEOUT", ""


def classify(oracle, got):
    (oc, oo), (gc, go) = oracle, got
    if gc == "TIMEOUT":
        return "TIMEOUT"
    if oc == gc and oo == go:
        return "MATCH"
    if oc != 0 and gc == 0:
        return "ACCEPT_WRONG"
    if oc == 0 and gc != 0:
        return "REFUSE_WRONG"
    return "DIFF"


PROGRESS_EVERY = 2000


def _mix64(x):
    """splitmix64's finalizer: a fixed, well-spread 64-bit hash, the same on every machine and Python."""
    x = (x + 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF
    x = ((x ^ (x >> 30)) * 0xBF58476D1CE4E5B9) & 0xFFFFFFFFFFFFFFFF
    x = ((x ^ (x >> 27)) * 0x94D049BB133111EB) & 0xFFFFFFFFFFFFFFFF
    return x ^ (x >> 31)


def keep(index, k, seed):
    """Whether `--sample K` keeps the row at `index` of the generated grid: about one in `k`."""
    return k <= 1 or _mix64((seed << 40) ^ index) % k == 0


def progress_line(done, total, elapsed):
    """`  done/total (elapsed s, rate rows/s, ETA)`: the rate is the run's average so far."""
    rate = done / elapsed if elapsed > 0 else 0.0
    left = int(round((total - done) / rate)) if rate > 0 else 0
    hours, rest = divmod(left, 3600)
    eta = f"{hours}h{rest // 60:02d}m" if hours else f"{rest // 60}m{rest % 60:02d}s"
    return f"  {done}/{total} ({elapsed:.0f}s, {rate:.0f} rows/s, ETA {eta})"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--bin", required=True, help="succinctly build under test")
    ap.add_argument("--base", help="reference succinctly build; a lost match against it fails")
    ap.add_argument("--jq", default="/usr/bin/jq", help="the pinned oracle (jq 1.7.1)")
    ap.add_argument("--show", type=int, default=20, help="max rows printed per section")
    ap.add_argument("--only", default="", help="restrict to rows whose name contains this (comma-separated: any of)")
    ap.add_argument("--only-source", default="", help="restrict to sources whose name contains this (comma-separated: any of)")
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument(
        "--wrapped",
        action="store_true",
        help="run the `?`/`try`-wrapped contexts instead of the gated ones (#4637); judged against --base only",
    )
    ap.add_argument(
        "--deep",
        action="store_true",
        help="run the three-fold-deep outers instead of the gated ones (#4648); judged against --base only",
    )
    ap.add_argument("--dump", help="write every row's classes as JSON lines to this file (name, doc, candidate, base)")
    ap.add_argument("--breakdown", action="store_true", help="count the #4588 residual rows per outer fold and source")
    ap.add_argument("--sample", type=int, default=1, help="keep about one row in K, by a seeded hash of its index")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument(
        "--jq-gate",
        action="store_true",
        help="also fail on any DIFF row of --bin against jq (#5552)",
    )
    args = ap.parse_args()
    if args.wrapped and args.deep:
        ap.error("--wrapped and --deep are alternatives: run them separately")
    if (args.wrapped or args.deep) and not args.base:
        ap.error("--wrapped and --deep are judged against --base: pass a reference build")

    rows = []
    contexts = WRAPPED_CONTEXTS if args.wrapped else CONTEXTS
    product = itertools.product(DEEP_OUTER if args.deep else OUTER, INNER, SOURCES, contexts, DOCS)
    for index, ((on, o), (inn, i), (sn, s), (cn, c), doc) in enumerate(product):
        if not keep(index, args.sample, args.seed):
            continue
        name = f"{on}/{inn}/{sn}/{cn}"
        if args.only and not any(part in name for part in args.only.split(",")):
            continue
        if args.only_source and not any(part in sn for part in args.only_source.split(",")):
            continue
        prog = c.replace("{O}", o.replace("{I}", i.replace("{S}", s)))
        rows.append((name, sn, prog, doc))

    def work(row):
        name, sn, prog, doc = row
        oracle = run([args.jq], prog, doc)
        if oracle[0] == "TIMEOUT":
            return None
        cand = run([args.bin, "jq"], prog, doc)
        base = run([args.base, "jq"], prog, doc) if args.base else None
        return (
            name,
            sn,
            prog,
            doc,
            oracle,
            classify(oracle, cand),
            cand,
            classify(oracle, base) if base else None,
        )

    # Progress goes to stderr, as `jq-path-register-sweep.py` does: the full grid is hours long, and a
    # run that prints nothing until it ends cannot be judged (or killed) early (#5204).
    results = []
    started = time.time()
    print(f"{len(rows)} rows, {args.jobs} jobs", file=sys.stderr)
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as ex:
        for i, r in enumerate(ex.map(work, rows), 1):
            if r:
                results.append(r)
            if i % PROGRESS_EVERY == 0:
                print(progress_line(i, len(rows), time.time() - started), file=sys.stderr)

    if args.dump:
        with open(args.dump, "w") as out:
            for r in results:
                out.write(json.dumps({"name": r[0], "doc": r[3], "prog": r[2], "cand": r[5], "base": r[7]}) + "\n")
    totals = collections.Counter(r[5] for r in results)
    print(f"rows={len(results)} of {len(rows)} (the pinned oracle timed out on the rest)")
    print(f"candidate={dict(sorted(totals.items()))}")
    if args.base:
        print(f"           base={dict(sorted(collections.Counter(r[7] for r in results).items()))}")

    def known(r):
        return (r[1], None) in KNOWN_ACCEPT_WRONG or (r[1], r[3]) in KNOWN_ACCEPT_WRONG

    residual_nav = [
        r
        for r in results
        if r[5] == "ACCEPT_WRONG" and not known(r) and r[0].split("/")[3] in RESIDUAL_CONTEXTS
    ]
    fresh = [
        r
        for r in results
        if r[5] == "ACCEPT_WRONG"
        and not known(r)
        and r[0].split("/")[3] not in RESIDUAL_CONTEXTS
    ]
    residual = [r for r in results if r[5] == "ACCEPT_WRONG" and known(r)]
    lost = [r for r in results if args.base and r[7] == "MATCH" and r[5] != "MATCH"]
    hung = [r for r in results if r[5] == "TIMEOUT"]
    diff = [r for r in results if r[5] == "DIFF"] if args.jq_gate else []
    print(
        f"ACCEPT_WRONG outside the documented residual: {len(fresh)}; documented residual: "
        f"{len(residual)} (known sources included); of a non-register `$x` (#4588): {len(residual_nav)}"
    )
    if args.breakdown:
        by = collections.Counter(
            (r[0].split("/")[0], r[0].split("/")[2]) for r in residual_nav
        )
        for (outer, source), n in sorted(by.items()):
            print(f"  residual {outer} {source}: {n}")
    print(f"lost matches against --base: {len(lost)}; candidate timeouts: {len(hung)}")
    if args.jq_gate:
        print(f"DIFF against jq (--jq-gate): {len(diff)}")
    for title, section in (("ACCEPT_WRONG", fresh), ("LOST MATCH", lost), ("TIMEOUT", hung), ("DIFF", diff)):
        for name, _sn, prog, doc, oracle, klass, cand, _b in section[: args.show]:
            print(f"{title} {name} [{klass}]\n   {prog}\n   in={doc} jq={oracle} build={cand}")
    if args.wrapped or args.deep:
        # `main` is not clean on these rows: only what the candidate moved for the worse counts.
        worse = [r for r in results if r[5] == "ACCEPT_WRONG" and r[7] not in (None, "ACCEPT_WRONG")]
        print(f"newly ACCEPT_WRONG against --base: {len(worse)}")
        for name, _sn, prog, doc, oracle, klass, cand, _b in worse[: args.show]:
            print(f"NEW ACCEPT_WRONG {name}\n   {prog}\n   in={doc} jq={oracle} build={cand}")
        return 1 if worse or lost or hung or diff else 0
    return 1 if fresh or lost or hung or diff or len(residual_nav) > RESIDUAL_NAV_MAX else 0


sys.exit(main())
