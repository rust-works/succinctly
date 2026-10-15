#!/usr/bin/env python3
"""Directional differential sweep for jq's path register (#3456, #3289, #3428).

Runs a generated grid of path-mode programs through the pinned jq oracle
(`/usr/bin/jq` 1.7.1) and one or more succinctly builds, then classifies each
row per build by comparing **stdout and exit code** with jq's.

**Why directional.** Path mode has one dangerous failure: succinctly
*accepts* -- and so `del`/`=`/`|=` *writes* -- where jq refuses. A refusal jq
does not make only costs an answer (ADR-0018's refuse-only direction), so the
two directions are not weighed alike. The grid is what three review rounds on
#3289 kept finding holes in, so the axes here are deliberately wider than any
one review's: operands x combinators x inputs x contexts, with the round-2
categories (`try ... catch`, `?//`, array-copy identity, wrapper stacks,
retried alternatives after a pipe, SOURCE-navigating `reduce`/`foreach`, long
`and` chains) as explicit context and operand axes, not a hoped-for
by-product.

**Builds.** `--candidate` is the build under test. Each `--base LABEL=PATH`
is a reference build; the *first* one is the directional base the exit code
is judged against. Typical use while changing the register contract:

  --base pre3425=<build at 452940f6e> --base main=<build of main>

so a row can be attributed to #3425's own change (pre3425 vs main) or to the
candidate (main vs candidate).

**Row classes** (per build, against jq):

  MATCH         same stdout and exit code
  ACCEPT_WRONG  jq refused (exit != 0), the build succeeded -- the dangerous one
  REFUSE_WRONG  jq succeeded, the build refused (safe direction)
  DIFF          both succeeded with different output, or both failed with
                different exit codes
  TIMEOUT       the build exceeded --timeout (an O(N^3) gate shows up here), or
                printed more than OUTPUT_CAP bytes or outgrew RSS_CAP (killed
                there, #3893); a plain timeout is re-run serially once before it
                is believed, since a loaded box can time out a row a quiet one
                answers (a capped run is not: it ends the same way on any box)
  ORACLE_TIMEOUT  jq itself timed out, or never stopped printing or outgrew RSS_CAP
                (an operand that is infinite on some input, `recurse(.a)?` on
                `null`): the row says nothing, so the builds are not run on it

A row where the two sides both fail with the same exit code still needs the
same stdout to be a MATCH: `path(...)` can print results before it errors.

**FAIL** (exit status 1) is any row where the candidate is worse than the
first base: newly ACCEPT_WRONG, newly DIFF, newly REFUSE_WRONG where the base
matched jq (a lost match, even in the safe direction: a refuse-only fallback
has leaked under `try` before, #3186/#3267, and the producer migration is
expected to flip nothing), newly TIMEOUT, or a build that is ACCEPT_WRONG /
DIFF in both but writes a *different* document (a wrong write that moved).
Every other flip -- including every improvement -- is *reported*, and each
needs its own oracle capture before it is claimed: this script proves the
direction, not the reason.

**Reading a clean run.** A build is only as good as the rows it can reach. The
summary prints the per-class totals for every build, so "0 FAIL" over a grid
where the base is already ACCEPT_WRONG on thousands of rows reads differently
from "0 FAIL" over a clean base. A benchmark cannot measure a shape it does
not generate: add the generator pattern here before claiming a shape is safe.

**Size.** The full grid is millions of rows; `--list-axes` prints the exact count
(793 operands, 7 companions, 12 inputs and 37 contexts give 10,219,347 rows as of #4187,
and each operand is also swept as a bare pipe stage since #3361). The count grows with
every operand a change to the register adds, so read it from `--list-axes` rather than
from here.

Measured rate (#4110): about 200 rows/s with one base and the candidate, debug builds,
`--jobs 18` (the default, one per core) on an 18-core Apple M5 Max at load average ~8.
The #4110 report saw about 50 rows/s on a box at load 100 and 130 idle. At 200 rows/s the full grid is
about 13 hours and `--stage-only` over every operand (636,027 rows) about 53 minutes, so
the full grid is a once-per-contract-change run, not a per-PR one. The progress line
prints the running rows/s and an ETA; trust the first one after a few thousand rows.

**Recommended gate.** `--stage-only` keeps every context but only the operand as a
bare pipe stage and negated, dropping its `and`/`or` shapes (about 1/15 of the rows).
That is enough to iterate on, and not enough to judge a stage-level rule by (see
`shapes_for`), so:

  1. While iterating: `--stage-only --operand ...` over the operands the change touches
     (47,000 rows for 55 operands took about 6 minutes on a quiet box), or
     `--stage-only` alone over every operand (636,027 rows, about 53 minutes).
  2. To judge the change: `--operand ...` without `--stage-only` over the touched
     operands (83,187 rows for 14 operands took about 22 minutes at `--jobs 6` on a
     box at load 100), plus a seeded `--sample N --seed S` over the whole grid
     (`--sample 6000 --seed 7` is about 30 seconds). Keep the seed in the PR so the same
     rows can be re-run.

Run the whole grid only when the change reaches every operand.

**Debug builds.** Build the candidate and every base without `--release`. The pinned
release profile (fat LTO, one codegen unit) takes much longer to build than the sweep
saves, and each row compares stdout and exit code, which a debug build produces
identically. The one thing it changes is speed: a debug build is slower, so a
`TIMEOUT` is re-run serially (see above) and an operand that is slow rather than
cubic can time out here and not in a release build.

This is a verification tool, not a CI gate. The pinned `*_3456` rows in
`tests/jq_cli_tests.rs` are what CI enforces (one per round-2 category); run
this after touching `PathBranch::register`, `Frame::register_loss`, or any
path-mode producer/consumer of them in `src/jq/eval.rs`.

Usage:
  cargo build --features cli
  ./scripts/jq-path-register-sweep.py --candidate target/debug/succinctly \\
      --base main=/path/to/main-build [--base pre3425=/path/to/older-build]
  ./scripts/jq-path-register-sweep.py --candidate ... --sample 2000 --seed 7
  ./scripts/jq-path-register-sweep.py --candidate ... --list-axes
  ./scripts/jq-path-register-sweep.py --candidate ... --operand any --operand 'all'
  ./scripts/jq-path-register-sweep.py --candidate ... --stage-only   # quick, not a gate
  ./scripts/jq-path-register-sweep.py --candidate ... --json out.jsonl   # rows stream to out.jsonl.partial
"""

import argparse
import concurrent.futures
import itertools
import json
import os
import pathlib
import random
import subprocess
import sys
import threading
import time

ORACLE = "/usr/bin/jq"
ORACLE_VERSION = "jq-1.7.1"

# The most stdout one run may produce before it is killed and classed a timeout. A
# real row prints a few hundred bytes; 4 MiB is far past that and is reached by an
# infinite jq output in well under a second, so nothing near it is ever buffered.
OUTPUT_CAP = 4 * 1024 * 1024

# The most resident memory one run may hold, and how often a run still going is
# checked (the first check is after POLL_SECONDS, so a run that ends sooner is never
# sampled). A real row stays under a few tens of MB; jq grows ~7 GB/s on the
# infinite operands, so a coarser poll would let a run reach tens of GB first.
RSS_CAP = 256 * 1024 * 1024
POLL_SECONDS = 0.05

# ---------------------------------------------------------------------------
# Axes
# ---------------------------------------------------------------------------

# Operands. `and`/`or`/`-` are not subexps in jq, so each of these moves (or
# does not move) jq's path register differently inside them. The by-value
# group is the #3428 family: jq navigates inside them, the resolver
# evaluates them by value.
OPERANDS = [
    # native navigation
    ".",
    ".a",
    ".b",
    ".[0]",
    ".[]?",
    "..",
    ".[0:]",
    ".[0:0]",
    # (#3734) a slice under a postfix `?`: a branch resolved under it carries
    # `Optional(Slice)`, which the reduce accumulator's whole-array-slice rule
    # (#3504) must read through. The three cover the full slice (the register),
    # a partial one and an empty one (fresh values), the shapes that rule splits.
    ".[0:]?",
    ".[1:]?",
    ".[0:0]?",
    "getpath([\"a\"])",
    "select(true)",
    # (#3780) a navigation of the accumulator that then hands the accumulator
    # back: under a navigating `reduce` source jq raises on the navigation, and
    # the last output (`.`) used to relocate to the root, so `del` deleted the
    # document. Neither operand was covered before.
    "(.a, .)",
    "(.b, .)",
    # (#3797) the same navigation one level down: inside a collect, the left of a
    # `//`, `first(f)` and `last(f)`, all of which run their body whatever it yields,
    # so jq raises on `.a` there too. Followed by the contexts' own `$x`/`$k` reads,
    # the accumulator used to come back as the root and `del` deleted the document.
    "[.a]",
    "(.a // .)",
    "first(.a)",
    "last([.a])",
    "last((.a // .))",
    # #3811: navigation inside executed stream/label/definition bodies, reached
    # by the reduce-source context as well as the other path/write contexts.
    "limit(2; .a, .)",
    "nth(1; .a, .)",
    "label $l | .a, .",
    "def f: .a, .; f",
    "def f(g): g; f(.a, .)",
    "[limit(1; .a)]",
    "first(label $l | .a)",
    "(limit(1; .a) // .)",
    # ... and the wrappers around `.` that must NOT be refused for it: jq answers
    # `[]` for each (review of #3780: a "contains a navigation" test refused them)
    "first(.)",
    "limit(1; .)",
    "(. // .a)",
    "((.a)?, .)",
    "if true then . else .a end",
    # cannot navigate / cannot move the register
    "true",
    "null",
    "5",
    "empty",
    "tostring",
    "length",
    # (#4295) `del`/`delpaths`/`setpath` hand their input back as the same `jv` when the
    # path set is empty, so `path()` accepts them and a `$x` bound to the input still names
    # it. A non-empty set builds a fresh value and stays a refusal: `del(.[]?)` is empty over
    # a scalar and not over `[1]`, `del(.b?)` over `[1]` and not over `{"a":1}`.
    "del(empty)",
    "del(.[]?)",
    "del(.b?)",
    "del(.a?)",
    "del(.b)",
    "del(.[0]?)",
    "delpaths([])",
    "delpaths([[\"a\"]])",
    "setpath([]; .)",
    "setpath([]; 1)",
    "setpath([\"a\"]; .)",
    "del((empty))",
    "delpaths(([]))",
    "setpath(([]); (.))",
    # by-value operands jq navigates inside (#3428)
    "first",
    "last",
    "any",
    "all",
    "nth(0)",
    "range(2)",
    "paths",
    "first(.[]?)",
    "limit(1; .[]?)",
    "isempty(.[]?)",
    "[.[]?]",
    # (#3749) `any(gen; cond)`/`all(gen; cond)`: a deciding element leaves jq's
    # register where `gen` put it and `path()` accepts a boolean identical to
    # it, so the resolver now accepts these where it used to refuse. That is the
    # dangerous direction, so the arm needs rows: a `cond` that cannot move the
    # register (the verdict is stated) and one that can (a loss, still refused).
    "any(.[]?; .)",
    "all(.[]?; .)",
    "any(.[]?; .a)",
    "all(.[]?; .a)",
    # (#3763) the bare and one-argument spellings jq defines over `.[]`, and
    # `isempty(g)` (already above), now take the same arm: a `cond` that cannot
    # move the register (`. == true`, the verdict is stated) and one that can
    # (`.a`, a loss), for each of any and all. `any` and `all` are above.
    "any(. == true)",
    "all(. == true)",
    "any(.a)",
    "all(.a)",
    # (#3763 review) an any/all that decides nothing claims jq's register stayed
    # at its entry, which is only sound when `cond` cannot raise a path error: jq
    # path-checks `cond`, this resolver runs it by value. These put a `cond` that
    # raises in jq (`.[]?` and `.a?` on a computed `1`, `unique_by(.)`) under a
    # generator that decides nothing, bare and wrapped in `isempty`, which is
    # what let a later `$x` re-establish the root and a `del` delete it.
    # They were the by-value `cond` hole (#3757): jq's path error on a
    # *computed* element was invisible, and `any(1; .[]?) or true` or
    # `.. // .a` wrote where jq exits 5. #3757 resolves such a `cond` as
    # `gen | cond`: `--operand` over the any/all/isempty operands, against a build
    # of `main` without it, went from ACCEPT_WRONG rows to none, with no
    # regression. The counts move with the grid, so rerun rather than compare.
    "any(1; .[]?)",
    "any(true; .[0]?)",
    "all(unique_by(.))",
    "isempty(any(1; .[]?))",
    "isempty(all(1; .a?))",
    # (#4623) a verdict UPDATE of a `foreach` that navigates its source: a comma of verdicts and an
    # operand wrapped in `first`/`limit`, which leave jq's register where they began.
    "(isempty(1), isempty(2))",
    "(isempty(1), 5)",
    "(isempty(1), isempty(.a?))",
    "isempty(first(1))",
    "isempty(limit(1; 1, 2))",
    "any(first(1); .)",
    "all(1; first(.))",
    "isempty(first(.a?))",
    "isempty(path(.[]?))",
    # (#4641) the verdict operands #4623 still refused: a `path()` operand, a pipe that can emit
    # nothing before it navigates, `and`/`or` over verdicts, a `select` or `first(.)` as `cond`, a
    # `def` wrapper, and the controls that must stay refused (an operand that navigates before it
    # can emit nothing, a verdict whose operand navigates, a `select` whose condition raises).
    "isempty(empty | .a)",
    "isempty(.a? | empty)",
    "isempty(1) and isempty(2)",
    "isempty(1) or isempty(.a?)",
    "isempty(.a) and isempty(1)",
    "isempty(1) and isempty(first(.a?))",
    "any(true; select(.))",
    "all(1; select(.))",
    "any(1; first(.))",
    "all(1; select(.a))",
    "isempty(select(.a))",
    "(def f: isempty(empty); f)",
    "(def f: isempty(.a?); f)",
    # (#4499) an `isempty` over a fold: the operand fold is one flow, not siblings.
    "isempty(foreach (1,2) as $i (1; try .a; .))",
    "isempty(first(foreach (1,2) as $i (null; try .a; .)))",
    "isempty(reduce (1,2) as $i (1; try .a))",
    # (#4499 review) the shapes the `isempty(FOLD)` fix leaves refused: an inner EXTRACT that reads the
    # loop variable and a fold as the `cond` of `any`.
    "isempty(foreach (1,2) as $i (1; try .a; $i))",
    "any(foreach (1,2) as $i (1; try .a; .); .)",
    # (#3757) a `cond` outside the by-value allowlist is resolved as `gen | cond`
    # in jq mode: the stage rules for `select`/`first`, a navigating chain, an
    # update assignment and `unique_by` (which raises on a computed element), and
    # a computed generator under a navigating `cond`, the shape that wrote where
    # jq exits 5.
    "any(.[]?; select(.))",
    "all(.[]?; first(.))",
    "any(.[]?; .a | .b)",
    "all(.[]?; .a += 1)",
    "any(.[]?; unique_by(.))",
    "any(1; .a)",
    "any(true; select(.))",
    # (#3757 review) a computed generator element can still be jq's register: a
    # pass-through builtin returns the value it was given (`ltrimstr("x")` on a
    # non-string, `tostring` on a string) and jq then raises no path error in
    # `cond`, where a live stage assuming "computed means not the register" raised
    # one an enclosing `try` swallowed. And an opaque generator (`input_line_number`,
    # `now`) states no register at all, which used to trip the stage's entry
    # assertion in a debug build. `first(2, 3)` against `2` is the pair that tells a
    # generator the allowlist cannot prove from one it can.
    "any(ltrimstr(\"x\"); .[0])",
    "all(ltrimstr(\"x\"); first)",
    "any(.a | tostring; .[0]?)",
    "any(.a | ltrimstr(\"x\"); .a?)",
    "all(tostring; .a?)",
    "any(input_line_number; .a)",
    "any(now; .a)",
    "any(first(2, 3); .a?)",
    "any(2; .a?)",
    # (#3757 review) a `?//` inside `gen` retries after a failed alternative and
    # resolves a later one, which supersedes whatever the abandoned alternative's
    # element stashed: the first alternative yields a number `cond` cannot
    # navigate (or, for the by-value `cond`, raises on), the second the element
    # itself. A stash that outlived the retry raised the first one's error.
    "any((.[0]? | length) as $p ?// $q | if $p != null then $p else .[0]? end; .a?)",
    "any((.[0]? | tostring | length) as $p ?// $q | if $p != null then $p else .[0]? end; if type == \"number\" then error(\"x\") else . end)",
    # wrappers and control flow around the above
    "try .a",
    "(.a // .b)",
    "if . then .a else .b end",
    "(. as $v | .a)",  # `as` extends right: unparenthesised it swallows the combinator
    # (#3456 B2b) by-value builtins jq runs without moving the register: a C
    # builtin's arguments are subexps, and a jq-defined one built on `reduce` or
    # an array collect backtracks its source. A static `LostAt` is too coarse
    # for these, so each is a lost match the day a by-value operand stops
    # keeping the eager route.
    "has(\"a\")",
    "keys",
    "add",
    "map(.)",
    # Left in place by jq and not named by `cannot_move_register`: the boundary,
    # where a lost register must refuse loudly and a `try` must not swallow it.
    "sort",
    "to_entries",
    "flatten",
    # (#3456) the producers a wrong `Unmoved` would hurt most, now that an
    # and/or operand's stated register is read as given: a generator that emits
    # from inside a fork (`label`, `limit`, `first(f)`, `nth`), the folds,
    # recursion, a comma, a `try` with a handler, and a pipe that navigates and
    # then computes.
    "(reduce .[]? as $k (.; .))",
    # (#3710) a fold whose UPDATE computes: nothing in it moves the register, so a
    # `$x` frozen before it still names the register after (`reduce 1 as $i (.;
    # .a = $i) | $x.k` is `["k"]`). The contrasts jq raises on or this resolver
    # still refuses: a SOURCE or INIT that navigates, a destructuring pattern.
    "(reduce 1 as $i (.; .a = $i))",
    "(reduce (1,2) as $i (.; 5))",
    "(reduce 1 as $i (.; {a: 1}))",
    "(reduce empty as $i (.; 5))",
    "(reduce 1 as $i (.; del(.a)))",
    "(reduce .[]? as $i (.; 5))",
    "(reduce 1 as $i (.a; 5))",
    "(reduce 1 as $i (.; .a))",
    "(reduce 1 as [$i] (.; 5))",
    "(foreach .[]? as $k (.; .; .))",
    "(label $o | (.a, break $o))",
    "limit(2; .a, 1)",
    "first(.a, 1)",
    "nth(1; .[]?, 1)",
    "recurse(.[]?)",
    "(.a, 1)",
    "(try error(\"x\") catch .)",
    "(.a | tostring)",
    "(.[0] | length)",
    "(getpath([\"a\"]) // 1)",
    # (#3456 B2b) a compound stage with a by-value leaf. jq backtracks to the
    # fork, so the by-value leaf's register is unmoved there while a sibling
    # leaf navigates.
    "(.a // 1)",
    "if . then 1 else .a end",
    "try 1 catch .a",
    "(def f: 5; f)",
    # (#3643) `last(f)` is `reduce f as $x (null; $x)`: its source backtracks, so
    # the register is where it entered even when `f` navigates. The contrast
    # rows are the generators that emit from inside the fork (`first(f)`,
    # `nth(n; f)`, `limit`), which move it and must stay refused.
    "last(.a)",
    "last(.a,.b)",
    "last(.[]?)",
    "last(empty)",
    "last(first(.a))",
    "last(.a | .b?)",
    # (#3767) `last(f)` inside a collect: the `[E]` claim reads through it, so `f` is
    # judged as the claim judges it. The contrasts are an `f` jq path-checks on a
    # computed value (`1 | .a`, `.a | tostring | .b`), which must keep raising, and
    # the generators the allowlist does not read (`first(f)`, `//`, `limit`).
    "[last(.a)]",
    "[last(.[]?)]",
    "[last(.a?, .b)]",
    "[last(empty)]",
    "[last(.a | .b?)]",
    "[last(1 | .a)]",
    "[last(.a | tostring | .b)]",
    "[last(.a | add)]",
    "[last(first(.a, .b))]",
    "[last(.a // .b)]",
    "[last(limit(1; .[]?))]",
    "[last(.[]? | select(.))]",
    "[try last(1 | .a)]",
    # (#3743) a fold's `?//` pattern over a source that is the register itself (`.`):
    # the first alternative's step can never succeed on the element, so jq raises and
    # retries whether or not the element is the register's node. The contrasts are a
    # source that is not the register (`.a`, a literal), a first step that can succeed
    # (`[$a]` over an array), no alternatives, and a computed key (no kind to judge).
    # The operands whose pattern succeeds by value (`{a:$a}` over a document with `a`,
    # `[[$a]]` over `[[1]]`) and the computed-key one are the known-refused class: the
    # walk's guess still refuses them where jq answers, so a delta there is expected.
    "(reduce . as [$a] ?// $a (0; .))",
    "(reduce . as {a:$a} ?// $a (0; .))",
    "(reduce . as {a:$a} ?// [$a] (0; .))",
    "(reduce . as [[$a]] ?// $a (0; .))",
    "(reduce . as [$a] ?// {a:$a} ?// $a (0; .))",
    "(reduce .a as [$a] ?// $a (0; .))",
    "(reduce .[]? as [$a] ?// $a (0; .))",
    "(reduce . as {(\"a\"):$a} ?// $a (0; .))",
    # (#3998) a computed key whose step can never succeed on the element (a string off an
    # array): jq raises it whatever its register is and `?//` retries, so the walk's
    # refusal is jq's own verdict, for `foreach` as for `reduce`.
    "(foreach . as {(\"a\"):$a} ?// $a (0; .; .))",
    "(reduce . as {(\"a\"):$a} ?// {(\"b\"):$a} ?// $a (0; .))",
    # (#4013) a fold INIT that navigates and then computes leaves jq's register on the
    # navigated node, off the root the untracked INIT's ambient describes: a pattern
    # walking from there and a `=`/`|=` through it wrote through the wrong node.
    "(foreach (null) as {a:$x} (.a|tostring; .; $x))",
    "(reduce (null) as {a:$x} (.a|tostring; $x))",
    "(foreach (null) as [$x] (.a|length; .; $x))",
    "(reduce .[]? as {a:$x} (.b|tostring; $x))",
    "(foreach (null) as {a:$x} (tostring; .; $x))",
    "(reduce . as [$a] (0; .))",
    # (#3744) a destructuring of the register itself as a `foreach` SOURCE: the pattern's
    # tracked index steps move jq's register onto the matched member, so an EXTRACT that
    # is not there refuses, and a bare `$k` the walk bound is a path. Under `reduce` jq
    # restores the register when it backtracks the source, so the by-value drive is right
    # there and the `reduce` rows are the must-not-change contrast.
    "(foreach (. as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as [$a] | .) as $k (.; .; .))",
    "(foreach (. as {a:{b:$a}} | .) as $k (.; .; .))",
    "(foreach (. as {a:$a} | $a) as $k (.; .; .))",
    "(foreach (. as {a:$a} | .a) as $k (.; .; $k))",
    "(foreach (. as {a:$a} | .) as $k (.; $k; .))",
    "(foreach (. as {a:$a} | .) as $k (.; .; $k))",
    "(foreach (. as {a:$a} | $a, .) as $k (.; .; .))",
    "(foreach (0, (. as {a:$a} | $a)) as $k (.; .; .))",
    "(foreach (. as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (. as {a:$a, b:$b} | .) as $k (.; .; .))",
    "(foreach (. as {$a} | .) as $k (.; .; .))",
    "(foreach (select(true) as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as {a:$a} | .) as [$z] (.; .; .))",
    "(reduce (. as {a:$a} | .) as $k (.; .))",
    "(reduce (. as [$a] | .) as $k (.; .))",
    "(reduce (. as {a:$a} | $a) as $k (.; .))",
    "try (foreach (. as {a:$a} | .) as $k (.; .; .)) catch 7",
    # (#4129) a destructuring bind or fold under a `try` with a handler: the bind's source is a
    # subexpression, so only a producer in its *body* can state a register, and a fold whose
    # pattern destructures still leaves its entry register to the handler's output. The
    # contrasts keep a producer (`.`) in the body, a handler that navigates, and a body that
    # navigates before the bind.
    "try (. as [$a] | $a) catch 7",
    "try (. as {a:$a} | $a) catch 7",
    "try (. as [$a] ?// $a | $a) catch 7",
    "try (. as [$a] | 1) catch 7",
    "try (. as [$a] | .) catch 7",
    "try (. as [$a] | .a) catch 7",
    "try (. as [$a] | $a) catch .a",
    "try (.a | . as [$a] | $a) catch 7",
    "try (reduce . as [$a] (0; .)) catch 7",
    "try (reduce . as {a:$a} (0; .)) catch 7",
    "try (foreach . as [$a] (0; .; .)) catch 7",
    "try (foreach . as {a:$a} ?// $a (0; .; .)) catch 7",
    "try (reduce . as [$a] (0; .)) catch .a",
    # (#4490) a producer in a destructure's body, and a bind behind a stage, under a `try` with a
    # handler: the handler's output states the `try`'s entry register whatever the body does, and
    # the body's own successful outputs must not be read as if they did. The contrasts put a
    # producer (`.`, `..`, a comma, an `if`, a nested `try`) in the body after a walk that moved the
    # register, a bind behind a navigating, identity or plain-bind stage, and a nested pattern.
    "try (. as [$a] | (., 1)) catch 7",
    "try (. as [$a] | if $a then . else 1 end) catch 7",
    "try (. as [$a] ?// $a | .) catch 7",
    "try (. as [[$a]] | .) catch 7",
    "try (. as {a:$a} | .) catch 7",
    "try (1 | . as [$a] | .) catch 7",
    "try (.a? | . as [$a] | .) catch 7",
    "try (.[0]? | . as [$a] | .) catch 7",
    "try (. | . as [$a] | .) catch 7",
    "try (. as $x | . as [$a] | .) catch 7",
    "try (. as [$a] | try . catch 1) catch 7",
    "try (. as [$a] | ..) catch 7",
    "try (. as [$a] | first(.)) catch 7",
    "try (. as {a:$a} ?// [$a] | .) catch 7",
    "try (.a | . as [$a] | .) catch 7",
    "try (.a | . as [$a] ?// $a | $a) catch 7",
    # (#4490) the same bind outside a `try`, under each wrapper that forwards a branch: its own
    # outputs state the register at the bind's entry, never one the walk moved.
    "((. as [$a] | .), 1)",
    "(1, (. as [$a] | .))",
    "first(. as [$a] | .)",
    "(. as [$a] | .)?",
    "if . then (. as [$a] | .) else 1 end",
    "((. as [$a] | .) // 1)",
    "limit(2; . as [$a] | .)",
    "(. as [$a] | ..)",
    "(. as [$a] | (., 1))",
    "(.a | . as [$a] | $a)",
    # (#4527) a computed key forks the walk, and the same shapes inside a fold UPDATE are where a
    # key that is an array wrote through a `try` handler's register.
    "try (. as {(.):$a} | .) catch 7",
    "try (. as {(try . catch 1):$a} | .) catch 7",
    "try ((.,1) | . as {(.):$a} | ..) catch 7",
    "try ((.,1) | . as {(try . catch 1):$a} | .) catch 7",
    # (#4527) a computed key that is an array on an array input is jq's subarray search: the step
    # succeeds and moves the register, where a refusal read as "the bind did not match" let a `try`
    # hand back its handler's register.
    "try (. as {(.):$a} | $a) catch 7",
    "try (. as {(.):$a} | 1) catch 7",
    "try ((.,1) | . as {(.):$a} | $a) catch 7",
    "(. as {(.):$a} | $a)",
    "(. as {(.):$a} | .)",
    "(. as {(.):$a} | $a, 1)",
    "(. as {(.):$a} ?// $a | $a)",
    "(. as {(.[0]):$a} | $a)",
    # (#4539) a computed key on a `?//` bind under a `try`: the walk's refused step could never
    # succeed on its input, so jq retries onto the bare alternative and the fold's emission raises.
    "try (. as {(.):$a} ?// $a | $a)",
    "try (. as {(.|tostring):$a} ?// $a | $a)",
    "try (. as {(.):$a} ?// $a | 5)",
    # (#4187) a plain bind in front of the destructure does not move the register (its source is a
    # subexpression), so the destructure behind it is the source's own; the contrasts bind a
    # different value, destructure something the bind produced, or follow it with a navigation.
    "(foreach (. as $x | . as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as $x | . as [$a] | .) as $k (.; .; .))",
    "(foreach (.a as $x | . as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as $x | . as {a:$a} | $x) as $k (.; .; .))",
    "(foreach (. as $x | . as {a:$a} | .a) as $k (.; .; $k))",
    "(foreach (. as $x | $x as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as $x | .) as $k (.; .; .))",
    "(foreach (. as $x | . as $y | . as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as $x ?// $y | . as {a:$a} | .) as $k (.; .; .))",
    # (#4578) the destructured variable is bound only by a later bare alternative: alternative one meets
    # it as the placeholder `null` and raises, and the retry makes it a type check on the value itself.
    "(foreach (. as $x ?// $y | $y as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as $x ?// $y | $y as [$a] | .) as $k (.; .; .))",
    "(reduce (. as $x ?// $y | $y as {a:$a} | .) as $k (.; .))",
    "(reduce (. as $x | . as {a:$a} | .) as $k (.; .))",
    "(reduce (. as $x | . as [$a] | $a) as $k (.; .))",
    "(foreach (. as $x | foreach . as {a:$a} (0; .; .)) as $k (.; .; .))",
    # (#4187) a `?//` chain on the bare register as a fold source: jq's register follows the first
    # alternative that matches and a failed one is retried from the fork, which the resolver's
    # per-alternative walk states; the by-value drive answered the root.
    # (#4420) a `?//` chain with a bare `$var` alternative behind the stages that hand the register
    # on (a comma, a pipe, an `if`, `try`, `first`, a `label`, a nested `foreach`): on a `null`
    # register jq moves it onto the matched member, as for the bare source (#4417). The contrasts
    # put a stage that is not the register in front (`1 |`, `(., 1) |`), bind the register first,
    # or put the chain under a `reduce`, which backtracks its source.
    "(foreach ((., .) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (. | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    # (#4424) a pipe source of the chain (`(.|.) as P ?// $v`): on a `null` register the pattern's
    # first step succeeds, as for the bare source. The contrasts are a register that is not
    # `null`, a chain of destructuring patterns only, a stage that is not the register, and a
    # `reduce`, which backtracks its source.
    "(foreach ((.|.) as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach ((.|.|.) as [$a] ?// {a:$a} ?// $b | .) as $k (.; .; .))",
    "(foreach ((.|.) as {a:$a} ?// [$a] | .) as $k (.; .; .))",
    # (#4484) the same chain where the fold around it decides whether `.` is the register: a
    # `foreach` INIT that moves it, a nested `reduce`, and `first`/`limit` as the UPDATE.
    "(foreach ((.|.) as {a:$a} ?// [$a] | .) as $k (.a; .; .))",
    "(reduce 1 as $j (.; foreach ((.|.) as {a:$a} ?// [$a] | .) as $k (.; .; .)))",
    "(first(foreach ((.|.) as {a:$a} ?// [$a] | .) as $k (.; .; .)))",
    "(limit(1; foreach ((.|.) as {a:$a} ?// [$a] | .) as $k (.; .; .)))",
    "(foreach ((.|.) as {a:$a} ?// $a | $a) as $k (.; $k; .))",
    "(foreach ((.|1) as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach ((.a|.) as {a:$a} ?// $a | .) as $k (.; .; .))",
    # (#4489) a source whose only movement is a destructuring bind, on an untracked entry: the
    # fold's emission is at the entry register when the bind fell back to its bare alternative, and
    # the contrasts where the body, UPDATE or EXTRACT navigates (jq refuses) or the bound value is
    # `null`/a literal that the register may be identical to
    "(foreach (null as {a:$a} ?// $a | 2) as $k (.; .; .))",
    "(foreach (1 as {a:$a} ?// $a | 2) as $k (.; .; .))",
    "(foreach ((.|.) as {a:$a} ?// $a | .a?) as $k (.; .; .))",
    "(foreach ((.|.) as {a:$a} ?// $a | .) as $k (.; .a; .))",
    "(foreach ((.|.) as {a:$a} ?// $a | .) as $k (.; .; .a))",
    "(foreach ((.|.) as {a:$a} ?// [$a] ?// $b | $b) as $k (.; .; .))",
    "(foreach ((.|.) as {a:$a} ?// $a | if true then . else .a end) as $k (.; .; .))",
    "(foreach ((.|.) as {a:$a} ?// $a | if true then .a else . end) as $k (.; .; .))",
    "(reduce ((.|.) as {a:$a} ?// $a | .) as $k (.; .))",
    "(foreach (foreach (. as {a:$a} ?// $a | .) as $j (.; .; .)) as $k (.; .; .))",
    "(foreach (if true then (. as {a:$a} ?// $a | .) else . end) as $k (.; .; .))",
    "(foreach (try (. as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (first(. as {a:$a} ?// $a | .)) as $k (.; .; .))",
    # (#4483) the chain directly under `first`/`limit` (jq's `first` is a `label`/`break`, which the
    # chain's `?//` catches as a body error, so value mode emits twice and path mode once). The
    # contrasts nest the wrapper (`try`, `nth`, `limit(2; ..)`, `last`), count with a generator (`limit(1,2; ..)`), put a stage that is not the
    # register in front of the chain, or put it under a `reduce`, which backtracks its source.
    "(foreach (limit(1; . as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (limit(2; . as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (limit(0; . as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (limit(1,2; . as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (limit(2,1; . as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (first(. as [$a] ?// $a | .)) as $k (.; .; .))",
    "(foreach (first(. as {a:$a} ?// [$a] ?// $a | .)) as $k (.; .; .))",
    "(foreach (first(. as {a:$a} ?// $a | $a)) as $k (.; .; .))",
    "(foreach (first(. as {a:$a} ?// $a | .a)) as $k (.; .; .))",
    "(foreach (try first(. as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (first(. as {a:$a} ?// $a | .)?) as $k (.; .; .))",
    "(foreach (nth(0; . as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (last(. as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (first(1 | . as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (first(.a | . as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (first(. as $x | . as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (first(first(. as {a:$a} ?// $a | .))) as $k (.; .; .))",
    "(foreach (first((. as {a:$a} ?// $a | .), 1)) as $k (.; .; .))",
    "(reduce (first(. as {a:$a} ?// $a | .)) as $k (.; .))",
    "(foreach (first(. as {a:$a} ?// $a | .)) as {b:$b} (.; .; .))",
    # (#4497) the same wrappers behind a def parameter, a nested fold UPDATE and a `reduce` UPDATE
    "(def f(g): foreach (first(g)) as $k (.; .; .); f(. as {a:$a} ?// $a | .))",
    "(def f(g): foreach (g) as $k (.; .; .); f(first(. as {a:$a} ?// $a | .)))",
    "(def f: foreach (first(. as {a:$a} ?// $a | .)) as $k (.; .; .); f)",
    "(foreach (1,2) as $i (.; foreach (first(. as {a:$a} ?// $a | .)) as $k (.; .; .); .))",
    "(reduce (1,2) as $i (.; first(. as {a:$a} ?// $a | .)))",
    "(reduce (1,2) as $i (.; foreach (first(. as {a:$a} ?// $a | .)) as $k (.; .; .)))",
    "(foreach (nth(1; . as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (first(. as {a:$a} ?// $a | .)?) as $k (.; .; $k))",
    "(foreach (try first(. as {a:$a} ?// $a | .)) as $k (.; .; $k))",
    "(foreach (try first(. as [$a] ?// $a | .)) as $k (.; .; .))",
    "(foreach (try limit(1; . as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (try nth(0; . as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (limit(1; . as {a:$a} ?// $a | .)?) as $k (.; .; .))",
    "(foreach (try first(. as {a:$a} ?// $a | .a)) as $k (.; .; .))",
    # (#4537) a consumer that refused the chain's destructured first emission must not stop its siblings
    "(first(. as {a:$a} ?// $a | .), .a)",
    "(nth(0; . as {a:$a} ?// $a | .), .a)",
    "(limit(1,2; . as {a:$a} ?// $a | .), .a)",
    "(foreach (first(. as {a:$a} ?// $a | .), .a) as $k (.; .; .))",
    "(foreach (1,2) as $i (.; first(. as {a:$a} ?// $a | .); .))",
    "(foreach (1,2) as $i (.; .; first(. as {a:$a} ?// $a | .)))",
    "(foreach (1,2) as $i (.; limit(1; . as {a:$a} ?// $a | .); $i))",
    "(reduce (1,2) as $i (.; nth(0; . as {a:$a} ?// $a | .)))",
    "(reduce (1,2) as $i (.; limit(1,2; . as {a:$a} ?// $a | .)))",
    "(foreach (first(try . as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (nth(0,1; . as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (1,2) as $i (.; .; first(try . as {a:$a} ?// $a | .)))",
    "(foreach (label $l | (. as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach ((. as {a:$a} ?// $a | .), 1) as $k (.; .; .))",
    "(foreach (1 | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach ((., 1) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (. as $x | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (.a | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(reduce (. | . as {a:$a} ?// $a | .) as $k (.; .))",
    # (#4424, second part) the register reaches the chain through a call to a `def` with
    # parameters, a comma that is the register on only some outputs, or a nested `foreach` over
    # `.`. The contrasts shadow the definition's name by a parameter, bind the argument and
    # destructure it, put a non-register stage in front, or nest the fold under a `reduce`.
    "(foreach (def f($x): . as {a:$a} ?// $a | .; f(1)) as $k (.; .; .))",
    "(foreach (def f(g): . as {a:$a} ?// $a | g; f(.)) as $k (.; .; .))",
    "(foreach (def f($x): . as {a:$a} ?// $a | $x; f(.a)) as $k (.; .; .))",
    "(foreach (def f($x): $x as {a:$a} ?// $a | .; f(.)) as $k (.; .; .))",
    # (#4485) a `$`-style parameter bound to the register by its argument is the register inside the
    # body (`f(.)`, `f((.|.))`), per call. The contrasts bind it to another node (`f(1)`, `f(.a)`), bind
    # the other parameter, or put the bind behind a nested definition or a pipe.
    "(foreach (def f($x): $x as {a:$a} | .; f(.)) as $k (.; .; .))",
    "(foreach (def f($x): $x as {a:$a} ?// [$a] | .; f(.)) as $k (.; .; .))",
    "(foreach (def f($x): $x as {a:$a} | .; f((.|.))) as $k (.; .; .))",
    "(foreach (def f($x): $x as {a:$a} | .; f(1)) as $k (.; .; .))",
    "(foreach (def f($x): $x as {a:$a} | .; f(.a)) as $k (.; .; .))",
    "(foreach (def f($x; $y): $y as {a:$a} | .; f(1; .)) as $k (.; .; .))",
    "(foreach (def f($x; $y): $y as {a:$a} | .; f(.; 1)) as $k (.; .; .))",
    "(foreach (def f($x): def g: $x as {a:$a} | .; g; f(.)) as $k (.; .; .))",
    "(foreach (def f($x): $x | . as {a:$a} | .; f(.)) as $k (.; .; .))",
    "(foreach (def f($x): [$x] as [{a:$a}] | .; f(.)) as $k (.; .; .))",
    "(foreach (def f($x): $x as {a:$a} | .; f(.), f(1)) as $k (.; .; .))",
    "(foreach (def f($x): $x as {a:$a} | .; f(1), f(.)) as $k (.; .; .))",
    "(foreach (def f($x): def g: $x as {a:$a} | .; g; f(null), f(.)) as $k (.; .; .))",
    "(foreach (def f($x): def g: $x as {a:$a} | .; g; f(.), f(null)) as $k (.; .; .))",
    "(reduce (def f($x): $x as {a:$a} | .; f(.)) as $k (.; .))",
    "(foreach (def g: .a; def f(g): . as {a:$a} ?// $a | g; f(.)) as $k (.; .; .))",
    "(foreach (def f($x): . as {a:$a} ?// [$a] | .; f(1)) as $k (.; .; .))",
    "(foreach (def f($x): .a; f(1) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(reduce (def f($x): . as {a:$a} ?// $a | .; f(1)) as $k (.; .))",
    "(foreach ((1, .) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach ((., .a) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach ((., 1) | . as {a:$a} ?// [$a] | .) as $k (.; .; .))",
    "(foreach ((., 1) | . as {a:$a} ?// $a | $a) as $k (.; .; .))",
    "(foreach (foreach (., 1) as $j (.; .; .) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (foreach . as $j (.; .; .) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (foreach .a as $j (.; .; .) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (foreach (., 1) as $j (.; .; $j) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    # (#4485) a nested `foreach` whose UPDATE or EXTRACT hands its loop variable on, and two stages
    # that are the register on only some outputs in one pipe: what follows a `foreach` resolves at
    # the marks of the fold it is in, not of the loop body it was emitted from.
    "(foreach (foreach . as $j (.; .; $j) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (foreach (., 1) as $j (.; $j; .) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (foreach (., 1) as $j (.; $j; $j) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (foreach (., 1) as $j (.; .; $j | .) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (foreach (., 1) as $j (.; .; .a) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (foreach (., 1) as $j (.; .; ., 1) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (foreach (., 1) as $j (.; .; .) | (., 1) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (foreach . as $j (.; .; .) | (., 1) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach (foreach . as $j (.; .; .) | (1, .) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach ((., 1) | (., 1) | . as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(foreach ((., 1) | (., 1) | . as {a:$a} ?// [$a] | .) as $k (.; .; .))",
    "(foreach ((., 1) | (1, .) | . as {a:$a} | .) as $k (.; .; .))",
    # (#4615) a bare-final chain as the source of a `foreach` reached through a `def` call with
    # arguments or a nested `foreach`, on an untracked entry (the `untracked-*` contexts) where the
    # fold's emission states the register the source's bind left. The contrasts navigate after the
    # bind, or end the chain in a pattern, and stay refused as in jq.
    "(foreach (def f(g): g; f((.|.) as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (def f(g): g; f(. as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (def f(g): g; f((.|.) as {a:$a} ?// $a | .a?)) as $k (.; .; .))",
    "(foreach (def f(g): g; f((.|.) as {a:$a} ?// [$a] | .)) as $k (.; .; .))",
    "(foreach (def f(g): g | g; f((.|.) as {a:$a} ?// $a | .)) as $k (.; .; .))",
    "(foreach (foreach ((.|.) as {a:$a} ?// $a | .) as $j (.; .; .)) as $k (.; .; .))",
    "(foreach (foreach ((.|1) as {a:$a} ?// $a | .) as $j (.; .; .)) as $k (.; .; .))",
    "(foreach (foreach ((.|.) as {a:$a} ?// $a | .) as $j (.; .; $j)) as $k (.; .; .))",
    "(foreach (foreach ((.|.) as {a:$a} ?// $a | .) as $j (.; .; .a)) as $k (.; .; .))",
    "(foreach (foreach ((.|.) as {a:$a} ?// $a | .) as $j (.; .a; .)) as $k (.; .; .))",
    "(foreach (foreach ((.|.) as {a:$a} ?// [$a] | .) as $j (.; .; .)) as $k (.; .; .))",
    "(foreach (foreach ((.|.) as {a:$a} ?// $a | .) as [$j] (.; .; .)) as $k (.; .; .))",
    "(reduce ((., 1) | . as {a:$a} ?// $a | .) as $k (.; .))",
    # (#4424) a pipe source of a chain of destructuring patterns only, outside a fold: its first
    # step is the register's whatever the pipe is, and the alternative that matches moves it. The
    # contrasts keep a bare alternative, a stage that is not the register, and a body that
    # navigates (which retries onto the next alternative).
    "((.|.) as {a:$a} ?// [$a] | .)",
    "((.|.) as {a:$a} ?// [$a] | $a)",
    "((.|.) as [$a] ?// {a:$a} | .a)",
    "((.|.) as {a:$a} ?// [$a] | .b)",
    "((.|.) as {a:$a} ?// {b:$a} | .)",
    "((.|1) as {a:$a} ?// [$a] | .)",
    "((.|.) as {a:$a} ?// $a | .)",
    "(((.|.) as {a:$a} ?// [$a] | .)?)",
    "(foreach (. as {a:$a} ?// [$a] | .) as $k (.; .; .))",
    "(foreach (. as [$a] ?// {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as {a:$a} ?// [$a] | $a) as $k (.; .; .))",
    "(foreach (. as {a:$a} ?// [$a] | .) as $k (.; $k; .))",
    "(foreach (. as {a:$a} ?// [$a] | .) as {b:$b} (.; .; .))",
    "(foreach (. as $x | . as {a:$a} ?// [$a] | .) as $k (.; .; .))",
    "(reduce (. as {a:$a} ?// [$a] | .) as $k (.; .))",
    "(reduce (. as [$a] ?// {a:$a} | .) as $k (.; .))",
    "(reduce (. as {a:$a} ?// $a | .) as $k (.; .))",
    # (#4278) the carriers #4187 left: a mixed `?//` chain with a destructure in its body, a plain
    # bind that is a later pipe stage, an `if` branch, a no-argument `def` call, and a `$x` the
    # spine bound to the register. The contrasts bind something else, shadow `$x`, emit a
    # member, or sit under a `reduce`, which backtracks its source. `(.|.) as {...}` is still
    # accepted where jq raises (it needs the resolver to read `(.|.)` as the register).
    "(foreach (. as $x ?// [$q] | . as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as {a:$a} ?// $z | . as {a:$b} | .) as $k (.; .; .))",
    "(foreach (. as {a:$a} ?// $z | $z) as $k (.; .; .))",
    "(foreach ((. as $x | .) | . as {a:$a} | .) as $k (.; .; .))",
    "(foreach ((. as $x | .) | . as {a:$a} | $a) as $k (.; .; .))",
    "(foreach (def f: . as {a:$a} | .; . as $x | f) as $k (.; .; .))",
    "(foreach (def f: . as {a:$a} | $a; f) as $k (.; .; .))",
    "(foreach (def f: .a; f | . as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as $x | if true then . as {a:$a} | . else . end) as $k (.; .; .))",
    "(foreach (if .a then . as {a:$a} | .a else . end) as $k (.; .; $k))",
    "(foreach (. as $x | $x as {a:$a} | .a) as $k (.; .; $k))",
    "(foreach (.a as $x | $x as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as $x | . as {a:$x} | $x as {a:$a} | .) as $k (.; .; .))",
    "(reduce (. as $x ?// [$q] | . as {a:$a} | .) as $k (.; .))",
    "(reduce (. as $x | $x as {a:$a} | .) as $k (.; .))",
    # ...and the review's: a bare alternative of a mixed chain holding the register, a `def`
    # body reading a `$x` bound around it, a call bound where the `def` was written (not a later
    # shadowing one), and a pass-through `if` or call as an earlier pipe stage.
    "(foreach (. as [$q] ?// $x | $x as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as $x ?// $y | $y as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as $x | def f: $x as {a:$a} | .; f) as $k (.; .; .))",
    "(foreach (def g: . as {a:$a} | .; def f: g; def g: .; f) as $k (.; .; .))",
    "(foreach (def g: .; def f: g; def g: . as {a:$a} | .; f) as $k (.; .; .))",
    "(foreach ((if true then . else . end) | . as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: .; f | . as {a:$a} | .) as $k (.; .; .))",
    # (#4320) a bound expression that is a call returning the register: a `def` whose body is
    # `.`, a `$x` bound around the definition, a body that is `.a`, a constant, a comma, or a
    # definition shadowed before the call, and the same under `reduce`.
    "(foreach (def f: .; f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as $x | def f: $x; f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: .; f as [$q] | .) as $k (.; .; .))",
    "(foreach (def f: .; f as {a:$a} | .a) as $k (.; .; $k))",
    "(foreach (def f: .a; f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: 7; f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: .; def f: 7; f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: ., .; f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: .; f as $v | .) as $k (.; .; .))",
    "(foreach (def f: .; def g: f; g as {a:$a} | .) as $k (.; .; .))",
    "(reduce (def f: .; f as {a:$a} | .) as $k (.; .))",
    # (#4320, eighth part) a `reduce` whose source destructures the register, as a stage of a
    # `reduce` UPDATE (the nested `reduce` is vetted like a nested `foreach`): the contrasts are a
    # source that goes on to navigate, an INIT that is not the register, an UPDATE that binds
    # again, a `reduce` in the nested `reduce`'s UPDATE, a register an earlier stage moved, a
    # second `reduce` after the first (jq restores the register on exit), and a `?//` chain.
    "(reduce (. as {a:$a} | .a) as $k (.; .))",
    "(reduce (. as {a:$a} | .) as $k (1; .))",
    "(reduce (. as {a:$a} | .) as $k (.; . as {b:$b} | .))",
    "(reduce (. as {a:$a} | .) as $k (.; reduce (. as {b:$b} | .) as $j (.; .)))",
    "(reduce (. as {a:$a} | .) as $k (.; .) | reduce (. as {b:$b} | .) as $j (.; .))",
    "(reduce (first(. as {a:$a} | .)) as $k (.; .))",
    "(reduce (def f: {a:1}; f as {a:$a} | .) as $k (.; .))",
    # (#4494) a top-level `reduce` source that destructures a call whose `def` builds its result
    # raises at the pattern's step whatever the register is. The contrasts are a bare alternative to
    # retry onto (`?//  $a`), the register itself (`def f: .`) and the value-mode spelling. The
    # `{a:1} | . as {a:$a}`, parameterised-call and `//` rows are not reached yet (#4522) and are
    # wrong accepts.
    "(reduce (def f: {a:1}; f as {a:$a} ?// $a | .) as $k (.; .))",
    "(reduce ((def f: {a:1}; f) as {a:$a} | .) as $k (.; .))",
    "(reduce (def g: {a:1}; def f: g; f as {a:$a} | .) as $k (.; .))",
    "(reduce (def f: {a:1}; f | . as {a:$a} | .) as $k (.; .))",
    "(reduce (def f: {a:1}; (f, f) as {a:$a} | .) as $k (.; .))",
    "(reduce (def f: [1]; f as [$q] | .) as $k (.; .))",
    "(reduce (def f: {a:1}; f as {a:$a} | $a) as $k (.; .))",
    "(reduce (def f: {a:1}; if . then f as {a:$a} | . else . end) as $k (.; .))",
    "(reduce ({a:1} | . as {a:$a} | .) as $k (.; .))",
    "(reduce (def f(x): {a:1}; f(.) as {a:$a} | .) as $k (.; .))",
    "(reduce (def f: {a:1}; (f as {a:$a} | .) // .) as $k (.; .))",
    # (#4522) the residual shapes: a `?//` chain of destructuring patterns, a path-expression INIT
    # (the source raises whatever INIT is), a fresh value reached through a variable or a closure
    # argument, a nested fold, and the contrasts a fresh destructure must not disturb (a chain with
    # a bare alternative, a variable bound to the register).
    "(reduce (def f: {a:1}; f as [$a] ?// {a:$a} | .) as $k (.; .))",
    "(reduce (def f: {a:1}; f as [$a] ?// $a | .) as $k (.; .))",
    "(reduce (def f: {a:1}; f as {a:$a} | .) as $k (.a; .))",
    # (#4581) a path-expression INIT moved the register off the document: a variable or call bound to the
    # document is then not the register, as a fresh value is not
    "(reduce (def f: {a:1}; . as $x | f as $y | $y | $x as {a:$a} | .) as $k (.a; .))",
    "(reduce (. as $d | {a:1} | $d | . as {a:$a} | .) as $k (.a; .))",
    "(reduce (def f($x): $x; f(.) as {a:$a} | .) as $k (.a; .))",
    "(reduce (. as {a:$a} | .) as $k (.a; .))",
    "(reduce (. as $x | $x as {a:$a} | .) as $k (.a; .))",
    "(reduce (def f: {a:1}; f as {a:$a} | .) as $k (.[]?; .))",
    "(reduce (def f: {a:1}; f as {a:$a} | .) as $k (tostring; .))",
    "(reduce (def f: {a:1}; f as $x | $x as {a:$a} | .) as $k (.; .))",
    "(reduce (def f: {a:1}; . as $x | $x as {a:$a} | .) as $k (.; .))",
    "(reduce (def g(x): x; g(def f: {a:1}; f as {a:$a} | .)) as $k (.; .))",
    "(reduce (def g(x): x as {a:$a} | .; g({a:1})) as $k (.; .))",
    "(reduce (def g($x): $x as {a:$a} | .; g({a:1})) as $k (.; .))",
    "(reduce (def g(x): x as {a:$a} | .; g(.)) as $k (.; .))",
    "(reduce (def g(x): x as {a:$a} | .; g(.), g({a:1})) as $k (.; .))",
    "(reduce (def g(x): x as {a:$a} | .; g({a:1}), g(.)) as $k (.; .))",
    "(reduce (def g(x): x as {a:$a} | .; g(.), g(.)) as $k (.; .))",
    # (#4583) the register destructured twice in a pipe: the first destructure moves it, so the second raises
    "(reduce (def g(x): x as {a:$a} | .; g(.) | g(.)) as $k (.; .))",
    "(reduce (def g(x): x as {a:$a} | .; g(.) | g(.) | g(.)) as $k (.; .))",
    "(reduce (def g(x): x as {a:$a} | .; g(.) | g({a:1}) | g(.)) as $k (.; .))",
    "(reduce (def g: . as {a:$a} | .; g | g) as $k (.; .))",
    "(reduce (. as {a:$a} | . as {a:$b} | .) as $k (.; .))",
    "(reduce ((. as {a:$a} | .) | (. as {a:$b} | .)) as $k (.; .))",
    "(reduce (. as {a:$a} | (. as {a:$b} | .)) as $k (.; .))",
    "(reduce (. as [$a] | . as [$b] | .) as $k (.; .))",
    "(reduce ((. as {a:$a} | .), .) as $k (.; .))",
    "(reduce (if . then (. as {a:$a} | .) else . end | . as {a:$b} | .) as $k (.; .))",
    "(reduce ((. as {a:$a} | .), 1 | . as {a:$b} | .) as $k (.; .))",
    "(reduce (((. as {a:$a} | .) | .) | . as {a:$b} | .) as $k (.; .))",
    "(reduce (((. as {a:$a} | .), .) | . as {a:$b} | .) as $k (.; .))",
    "(reduce (def g(x): x as {a:$a} | .; (g(.), g(.)) | g(.)) as $k (.; .))",
    "(reduce (def f: {a:1}; def g(x): x; g(f) as {a:$a} | .) as $k (.; .))",
    "(reduce (def f: {a:1}; f as $x ?// $y | $y as {a:$a} | .) as $k (.; .))",
    "(reduce 1 as $i (.; reduce (def f: {a:1}; f as {a:$a} | .) as $j (.; .)))",
    "(reduce (1,2) as $i (.; reduce (def f: {a:1}; f as {a:$a} | .) as $j (.; .)))",
    # (#4566) the residual of #4522: a fresh value piped into a destructure of `.` (a literal, a
    # variable, a builtin's result, a call that hands `.` on), a parameterised call or a forwarded
    # closure as the bound expression, a `//` operand, and a `?//` chain whose bare alternative the
    # body destructures. The contrasts are a destructure that is never reached (`empty`,
    # `select(false)`, the truthy side of `//`), the register behind a call, and a variable bound
    # before the fresh value.
    "(reduce (def f: {a:1}; f as $x | $x | . as {a:$a} | .) as $k (.; .))",
    "(reduce (keys | . as [$a] | .) as $k (.; .))",
    "(reduce (def f($x): $x; f({a:1}) as {a:$a} | .) as $k (.; .))",
    "(reduce (def f: {a:1}; def g(x): x; g(f) as {a:$a} | .) as $k (.; .))",
    "(reduce (def f: {a:1}; null // (f as {a:$a} | .)) as $k (.; .))",
    "(reduce (def f: {a:1}; . // (f as {a:$a} | .)) as $k (.; .))",
    "(reduce (def f: {a:1}; (empty | f as {a:$a} | .) // .) as $k (.; .))",
    "(reduce (def f: {a:1}; f as $x ?// $y | $y as {a:$a} | .) as $k (.; .))",
    "(reduce (def f: {a:1}; f as $x ?// $y | $x as {a:$a} | .) as $k (.; .))",
    "(reduce (def f: {a:1}; f as $x ?// $y | $y as {a:$a} | .) as $k (.a; .))",
    "(reduce (def f($x): $x; f(f({a:1})) as {a:$a} | .) as $k (.; .))",
    "(reduce (({a:1} // 1) as {a:$a} | .) as $k (.; .))",
    "(reduce ({a:1} | if . then . else 1 end | . as {a:$a} | .) as $k (.; .))",
    "(reduce (def g(x): x | . as {a:$a} | .; g(. as $d | {a:1})) as $k (.; .))",
    "(reduce (def f: {a:1}; f as $x ?// $y | . as {a:$a} | .) as $k (.; .))",
    "(reduce (def g(x): x as {a:$a} | .; g(.) | g({a:1})) as $k (.; .))",
    "(reduce (def g(x): x as {a:$a} | .; g({a:1}) | g(.)) as $k (.; .))",
    "(reduce (def g(x): x as {a:$a} | .; g(.) | g(.)) as $k (.; .))",
    "(reduce ({a:1} | select(false) | . as {a:$a} | .) as $k (.; .))",
    "(reduce ({a:1} | .a? // . | . as {a:$a} | .) as $k (.; .))",
    "(reduce (. as $d | {a:1} | $d as {a:$a} | .) as $k (.; .))",
    "(reduce (. as $d | {a:1} | . as {a:$a} | $d | .) as $k (.; .))",
    "(reduce (def f: {a:1}; f as $x | . as {a:$a} | .) as $k (.; .))",
    "(reduce (def f(x): x; f(.) as {a:$a} | .) as $k (.; .))",
    "(reduce (def f($x): $x; f(.) as {a:$a} | .) as $k (.; .))",
    "(reduce (def f: ., {a:1}; f as {a:$a} | .) as $k (.; .))",
    # (#4582) a comma fresh on only some outputs, then a destructure of `.`: the source is generated
    # whole, so the fresh item reaches it and raises. And `$__loc__`, the object the evaluator builds.
    # The contrasts: a `first`/`limit(1; ..)` that stops before the fresh item, an `if` that takes one
    # branch at run time, an array pattern, a body that is not `.`, and a `foreach`.
    "(reduce (({a:1}, .) | . as {a:$a} | .) as $k (.; .))",
    "(reduce ((., {a:1}) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (({a:1}, 1) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (({a:1}, .) | . as [$a] | .) as $k (.; .))",
    "(reduce (({a:1}, .) | . as {a:$a} | $a) as $k (.; .))",
    "(reduce (({a:1}, .) | . as {a:$a} | .) as $k (.a; .))",
    "(reduce (first((., {a:1}) | . as {a:$a} | .)) as $k (.; .))",
    "(reduce (limit(1; (., {a:1}) | . as {a:$a} | .)) as $k (.; .))",
    "(reduce (if .a then {a:1} else . end | . as {a:$a} | .) as $k (.; .))",
    "(reduce (($__loc__, .) | . as {file:$a} | .) as $k (.; .))",
    "(reduce ($__loc__ | . as {file:$a} | .) as $k (.; .))",
    "(reduce (keys | $__loc__ | . as {file:$a} | .) as $k (.; .))",
    "(reduce ($__loc__ | . as {file:$a} ?// $a | .) as $k (.; .))",
    "(foreach (({a:1}, .) | . as {a:$a} | .) as $k (.; .; .))",
    # (#4597) the same comma behind `first`/`limit`/`nth(0; ..)`, which generate only the operand's first
    # output: raises when that is the fresh item, answers when it is the register. A `limit(n; ..)` reads
    # its first n outputs (the fresh item is among them or it is not), `try`/`?` every one, `nth(k; ..)`
    # the one at index k; see the second block below.
    "(reduce (first({a:1}, .) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (first(., {a:1}) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (first({a:1}) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (limit(1; {a:1}, .) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (limit(1; ., {a:1}) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (limit(2; ., {a:1}, .) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (nth(0; {a:1}, .) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (first({a:1}, .) | . as [$a] | .) as $k (.; .))",
    "(reduce (first({a:1}, .) | . as {a:$a} | $a) as $k (.; .))",
    "(reduce (try ({a:1}, .) | . as {a:$a} | .) as $k (.; .))",
    "(foreach (first({a:1}, .) | . as {a:$a} | .) as $k (.; .; .))",
    # (#4597, second part) the same fresh comma behind a wrapper that generates every item (`try`, `?`),
    # a `limit(n)` or a nested comma whose leading outputs hold it, `nth(k; ..)`, a `reduce` stage that
    # hands `.` on, and `isempty`/`any`/`all` around the destructure. The contrasts are a wrapper that stops
    # before the fresh item (`limit(2; ., ., {a:1})`, `first((., {a:1}), 2)`, `nth(1; {a:1}, .)`), an item that
    # cuts the generation first (`error`, `break`), and a `reduce` stage that does not return its input.
    "(reduce (try (., {a:1}) | . as {a:$a} | .) as $k (.; .))",
    "(reduce ((., {a:1})? | . as {a:$a} | .) as $k (.; .))",
    "(reduce (limit(2; ., ., {a:1}) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (limit(3; ., ., {a:1}) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (limit(2; {a:1}, ., .) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (first(({a:1}, .), 2) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (first((., {a:1}), 2) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (nth(1; ., {a:1}) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (nth(1; {a:1}, .) | . as {a:$a} | .) as $k (.; .))",
    "(reduce ((., {a:1}) | reduce . as $x (.; .) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (({a:1}, .) | reduce .[]? as $x (.; .) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (({a:1}, .) | reduce . as $x (.; .+1) | . as {a:$a} | .) as $k (.; .))",
    "(reduce ((., {a:1}) | isempty(. as {a:$a} | .)) as $k (.; .))",
    "(reduce (({a:1}, .) | any(. as {a:$a} | .; .)) as $k (.; .))",
    "(reduce (({a:1}, .) | all(. as {a:$a} | .; .)) as $k (.; .))",
    "(reduce ((., error(\"x\"), {a:1}) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (try (., error(\"x\"), {a:1}) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (label $f | (., break $f, {a:1}) | . as {a:$a} | .) as $k (.; .))",
    # (#4600) the same sources inside another fold's body, under a value-mode operand (`-(..)`, `[..] | .[0]`)
    # that a `try` makes visible: jq raises its own catchable path error at the fresh item. The contrasts are a
    # source whose raise is not certain (an `if`, a `try` inside the source, an `error` item, a `?//` chain, a
    # `null` item), a `foreach` source (a consumer can stop before the fresh item), and an UPDATE that
    # navigates, raises or counts.
    "(reduce 1 as $i (.; -(reduce ((., {a:1}) as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce (({a:1}, .) as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce ((., {a:1}) | . as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; [reduce ((., {a:1}) as {a:$a} | .) as $j (.; .)] | .[0]))",
    "(reduce 1 as $i (.; -(reduce ((., {a:1}) as {a:$a} | .) as $j (.; .a))))",
    "(reduce 1 as $i (.; -(reduce ((., {a:1}) as {a:$a} | .) as $j (.; error(\"u\")))))",
    "(reduce 1 as $i (.; -(reduce ((., {a:1}) as {a:$a} | .) as $j (0; . + 1))))",
    "(reduce 1 as $i (.; -(reduce ((., {a:1}) as [$a] | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce ((., {a:1}) as {a:$a} | $a) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce ((., {a:1}) as {a:$a} ?// $a | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce ((., null) as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce ((., error(\"x\"), {a:1}) as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce (if . then ((., {a:1}) as {a:$a} | .) else . end) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce (try ((., {a:1}) as {a:$a} | .)) as $j (.; .))))",
    "(reduce 1 as $i (.; -(foreach ((., {a:1}) as {a:$a} | .) as $j (.; .; .))))",
    "(reduce 1 as $i (.; -(reduce (def f: ., {a:1}; f as {a:$a} | .) as $j (.; .a))))",
    "(reduce 1 as $i (.; -(reduce (def f: ., {a:1}; f as {a:$a} | .) as $j (0; . + 1))))",
    # (#4600) the wrappers and predicates of the second #4597 block as the nested source's generator: a fresh
    # item certainly among the outputs raises jq's catchable error, which the resolver now models (the
    # `certain_fresh_destructure` claim), and one that is not (`first(., {a:1})`, `limit(2; ., ., {a:1})`)
    # answers. The contrasts are an `error`/`break` ahead of the fresh item inside a wrapper or behind
    # `isempty`/`any`/`all`, which hides it (jq answers, and so does the build).
    "(reduce 1 as $i (.; -(reduce (try (., {a:1}) | . as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce (label $f | (., {a:1}) | . as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce (limit(2; ., {a:1}, .) | . as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce (limit(2; ., ., {a:1}) | . as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce (limit(2; (., {a:1}) | .) | . as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce (first(., {a:1}) | . as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce (first({a:1}, .) | . as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce (nth(1; ., {a:1}) | . as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce (nth(1; {a:1}, .) | . as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce (((., {a:1}) | .) | . as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce ((., {a:1}) | isempty(. as {a:$a} | .)) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce (({a:1}, .) | any(. as {a:$a} | .; .)) as $j (.; .))))",
    "(reduce 1 as $i (.; -(reduce ((., {a:1}) | all(. as [$a] | .; .)) as $j (.; .))))",
    "(reduce 1 as $i (.; reduce (try (., error(\"x\"), {a:1}) | isempty(. as {a:$a} | .)) as $k (.; .)))",
    "(reduce 1 as $i (.; reduce (label $f | (., break $f, {a:1}) | any(. as {a:$a} | .; .)) as $k (.; .)))",
    # (#4667) `first(., X)`, `limit(1; ., X)` and `nth(0; ., X)` emit their comma's first leaf only, so with `.`
    # first they are the register once and the destructure after them meets it: in the body of a `reduce` whose
    # source moved the register it raises, as a bare `. as {a:$a}` does. Contrasts: a `try`/`?` around the
    # destructure catches the raise (answered), and a `try`/`?` around the `first` alone does not (the
    # residual, #4676: `try first(., X) | D` and `first(., X)? | D` are still accepted where jq raises).
    "(reduce (nth(0; ., {a:1}) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (first(., {a:1}) | . as [$a] | .) as $k (.; .))",
    "(reduce (first(., {a:1}) | . as {a:$a} | $a) as $k (.; .))",
    "(reduce (first(first(., {a:1}), 2) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (first((., 1), {a:1}) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (first(.) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (first(., {a:1}) | . | . as {a:$a} | .) as $k (.; .))",
    "(reduce (first(., {a:1}) | . as {b:$b} | .) as $k (.; .))",
    "(reduce (first(., {a:1}) | . as {a:$a} ?// $a | .) as $k (.; .))",
    "(reduce (try (first(., {a:1}) | . as {a:$a} | .)) as $k (.; .))",
    "(reduce ((first(., {a:1}) | . as {a:$a} | .)?) as $k (.; .))",
    "(reduce (try first(., {a:1}) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (first(., {a:1}) | (. as {a:$a} | .)?) as $k (.; .))",
    "(reduce (first(., {a:1}, .) | . as {a:$a} | .) as $k (.; .))",
    "(reduce (limit(2; ., {a:1}) | . as {a:$a} | .) as $k (.; .))",
    "(foreach (first(., {a:1}) | . as {a:$a} | .) as $k (.; .; .))",
    "(foreach (limit(1; ., {a:1}) | . as {a:$a} | .) as $k (.; .; .))",
    "(foreach (nth(0; ., {a:1}) | . as {a:$a} | .) as $k (.; .; .))",
    "(reduce (def f: .|tostring; f as {a:$a} | .) as $k (.; .))",
    "(reduce (def f: ., .; f as {a:$a} | .) as $k (.; .))",
    # (#4522) a call whose `def` hands the register on next to a fresh value, and a fresh
    # destructure inside another fold's body: the register output is jq's own (a reduce backtracks
    # it) and only the fresh one raises. The contrasts are a body that is the register on every
    # output and a fold whose source cannot reach the destructure.
    "(reduce (def f: {a:1}, .; f as {a:$a} | .) as $k (.; .))",
    "(reduce (def f: ., {a:1}; f as {a:$a} | .) as $k (.a; .))",
    "(reduce (def f: (., {a:1}) | .; f as {a:$a} | .) as $k (.; .))",
    "(reduce (def f: ., {a:1}; f as {a:$a} ?// $a | .) as $k (.; .))",
    "(reduce (def f: ., {a:1}; f as [$a] ?// {a:$a} | .) as $k (.; .))",
    "(reduce (def f: ., {a:1}; f as {a:$a} | $a) as $k (.; .))",
    "(reduce (def f: ., {a:1}; f as $x | $x as {a:$a} | .) as $k (.; .))",
    "(reduce 1 as $i (.; reduce (def f: ., {a:1}; f as {a:$a} | .) as $j (.; .)))",
    "(reduce (def f: {a:1}; f as {a:$a} | .) as $k (.; reduce (def g: {b:1}; g as {b:$b} | .) as $j (.; .)))",
    "(reduce 1 as $i (.; foreach (def f: {a:1}; f as {a:$a} | .) as $j (.; .; .)))",
    "(foreach (1,2) as $i (.; reduce (def f: {a:1}; f as {a:$a} | .) as $j (.; .); .))",
    "(foreach 1 as $i (.; .; reduce (def f: {a:1}; f as {a:$a} | .) as $j (.; .)))",
    "(reduce 1 as $i (.; . as $x | reduce (def f: {a:1}; f as {a:$a} | .) as $j (.; $x)))",
    "(reduce 1 as $i (.; reduce (def f: {a:1}; f as {a:$a} ?// $a | .) as $j (.; .)))",
    "(reduce 1 as $i (.; -(reduce (def f: ., {a:1}; f as {a:$a} | .) as $j (.; .))))",
    "(reduce 1 as $i (.; [reduce (def f: ., {a:1}; f as {a:$a} | .) as $j (.; .)] | .[0]))",
    "(reduce 1 as $i (.; (reduce (def f: ., {a:1}; f as {a:$a} | .) as $j (.; .)) | length))",
    "(-(reduce (def f: ., {a:1}; f as {a:$a} | .) as $j (.; .)))",
    "((reduce (def f: ., {a:1}; f as {a:$a} | .) as $j (.; .)) | length)",
    "([reduce (def f: ., {a:1}; f as {a:$a} | .) as $j (.; .)] | .[0])",
    "(reduce (def f: .; try f as {a:$a} | .) as $k (.; .))",
    "(reduce (.[] as $a | .) as $k (.; .))",
    "(reduce (try (. as {a:$a} | .)) as $k (.; .))",
    "(reduce ((.|.) as {a:$a} | .) as $k (.; .))",
    "(reduce (. as {a:$a} | .) as {a:$q} ?// $q (.; .))",
    # (#4506) a `?//` chain as the nested fold's own LOOP pattern, bound to `$q` and not `$x` (the grid
    # skips an operand with a free `$x` outside a context that binds it, which left the row above out
    # of most contexts). A chain whose last alternative is a bare variable over a no-op UPDATE cannot
    # raise whichever alternative binds, so the first alternative's pattern step is retried even where
    # the walk can only guess at jq's `path_intact`. The contrasts are a last alternative that can
    # fail, an UPDATE that navigates or reads `$q`, an INIT that is not the register, a chain of
    # three, a source that navigates or fans out, and a `foreach` (whose EXTRACT is emitted with the
    # register the pattern moved).
    "(reduce . as {a:$q} ?// $y (.; .))",
    "(reduce (. as {a:$a} | .) as [$q] ?// {b:$y} ?// $z (.; .))",
    "(reduce (. as {a:$a} | .) as {a:$q} ?// [$q] (.; .))",
    "(reduce (. as {a:$a} | .) as {a:$q} ?// {b:$q} (.; .))",
    "(reduce (. as {a:$a} | .) as {a:$q} ?// $q (.; .a))",
    "(reduce (. as {a:$a} | .) as {a:$q} ?// $q (.; $q))",
    "(reduce (. as {a:$a} | .) as {a:$q} ?// $q (1; .))",
    "(reduce (. as {a:$a} | .) as {a:$q} ?// $q (.a; .))",
    "(reduce (.a, .) as {a:$q} ?// $q (.; .))",
    "(reduce (. as {a:$a} | .a) as {a:$q} ?// $q (.; .))",
    "(reduce (. as {a:$a} | .) as {a:$q, b:$y} ?// $z (.; . | .))",
    "(reduce (. as {a:$a} | .) as [$q] ?// $q (.; .) | .a)",
    "(foreach (. as {a:$a} | .) as {a:$q} ?// $q (.; .; .))",
    "(foreach (. as {a:$a} | .) as {a:$q} ?// $q (.; .; $q))",
    "(reduce (. as {a:$a} ?// {b:$b} | .) as $k (.; .))",
    "(. as {a:$q} | reduce (. as {b:$b} | .) as $k (.; .))",
    "(. as {a:$q} | reduce (def f: .; f as {b:$b} | .) as $k (.; .))",
    "(foreach (def f: .; try f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: .; (f, .) as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: .; (f, f) as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: .; (f, 7) as {a:$a} | .) as $k (.; .; .))",
    # (#4320, seventh part) a call whose body returns a fresh value, or the register and a fresh
    # value: jq raises at the pattern's first step, in a `reduce` UPDATE too (contrasts to the
    # comma, nested-`def`, captured-`$x` and `try` calls above, which it answers there).
    "(foreach (def f: {a:1}; f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: ., {a:1}; f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: .|tostring; f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: {a:1}; try f as {a:$a} | .) as $k (.; .; .))",
    # (#4452) a call whose `def` builds its result (the sweep's first rows are #4320's seventh
    # part): the contrasts are a body that may be the register (`tostring`), a shadowed
    # definition, a bare `$v` alternative (no step), a `first`/`limit` that stops before the
    # fresh leaf, and a comma of the register and a fresh value written out.
    "(foreach (def f: keys; f as [$a] | .) as $k (.; .; .))",
    "(foreach (def f: {a:1}; def g: f; g as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: .; def f: {a:1}; f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: {a:1}; def f: .; f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: {a:1}; f as $v | .) as $k (.; .; .))",
    "(foreach (def f: {a:1}, .; first(f) as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: {a:1}, .; limit(1; f) as {a:$a} | .) as $k (.; .; .))",
    "(foreach ((., {a:1}) as {a:$a} | .) as $k (.; .; .))",
    "(reduce (def f: {a:1}; f as {a:$a} | .) as $k (.; .))",
    "(foreach (def f: [.]; f as [$a] | .) as $k (.; .; .))",
    "(foreach (def f: {a:1}; def g: f | .; g as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: {a:1}; def g: . | f; g as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: .; def g: f | .; g as {a:$a} | .) as $k (.; .; .))",
    "(reduce 1 as $k (.; reduce (def f: {a:1}; f as {a:$a} | .) as $j (.; .)))",
    # (#4320) a `$x` bound outside the `foreach` that is the register at it: certified by
    # marker identity, so it is the source's own destructure as `. as {a:$a}` is. The contrasts
    # are an outer `$x` bound to a member, a `reduce` (which backtracks its source), an outer
    # `$x` rebound by a pattern, and one the source only reads.
    "(. as $x | foreach ($x as [$q] | .) as $k (.; .; .))",
    # (#4514) a held `$x` behind a pipe or an `if`: the register as a bare `$x` is. The contrasts
    # are a pipe that ends in a navigation, a held variable that is a member, and an `if` with
    # a branch that is not the register.
    "(. as $x | foreach (($x|.) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach ((.|$x) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach (($x|.|.) as [$q] | .) as $k (.; .; .))",
    "(. as $x | foreach ((if $x then $x else . end) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach ((if true then $x else .a end) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach (($x|.a) as {a:$a} | .) as $k (.; .; .))",
    "(.a as $x | foreach (($x|.) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach ((.a|$x) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach (($x, $x) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach (first($x) as {a:$a} | .) as $k (.; .; .))",
    # (#4320, sixth part) a `$x` behind a nested wrapper, `first` over a comma whose first leaf it
    # is, `nth`, `label`, and the commas that emit a non-register leaf too (contrasts).
    "(. as $x | foreach (first(first($x)) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach (limit(1; first($x)) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach (first($x, 1) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach (nth(0; $x) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach ((label $f | $x) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach (((first($x))?) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach (last($x, 1) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach (limit(2; $x, 1) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach ($x as {a:$a} | .a) as $k (.; .; $k))",
    "(. as $x | foreach ($x as $v | .) as $k (.; .; .))",
    "(. as $x | reduce ($x as {a:$a} | .) as $k (.; .))",
    "(.a as $x | foreach ($x as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach (. as {a:$x} | $x as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach ($x | .a) as $k (.; .; .))",
    # ...and the second review's: a name two chain alternatives bind, a `$x` behind `first` or a
    # comma, a `$x` bound outside the source (read since #4320's second part when it is the register), and a run of calls to one
    # pass-through `def`.
    "(foreach (. as $a ?// {a:$a} | $a as {a:$b} | .) as $k (.; .; .))",
    "(foreach (. as $x ?// [$x] | $x as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as $x | first($x) as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as $x | ($x, $x) as {a:$a} | .) as $k (.; .; .))",
    "(. as $x | foreach ($x as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: .; f | f | f | f | . as {a:$a} | .) as $k (.; .; .))",
    # ...and the third's: a register-held `$x` as a stage, a closure argument as the source, a
    # `$x` behind `try`, and a nested fold destructuring an outer fold's item (jq answers).
    "(foreach (. as $x | $x | . as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as $x | if true then $x else . end | . as {a:$a} | .) as $k (.; .; .))",
    "(foreach (. as $x | (try $x) as {a:$a} | .) as $k (.; .; .))",
    "(def h(q): foreach (q as {a:$a} | .) as $k (.; .; .); h(.))",
    "(reduce .[]? as $it (.; foreach ($it as {a:$a} | .) as $k (.; .; .)))",
    "(foreach ((.|.) as {a:$a} | .) as $k (.; .; .))",
    # ...and the same inside a nested fold: a nested `foreach` over `.` moves the register
    # for the outer one, a nested `reduce` does not (it is backtracked), so the latter is
    # the contrast that must keep the by-value drive.
    "(foreach (foreach . as {a:$a} (0; .; .)) as $k (.; .; .))",
    "(foreach (reduce . as {a:$a} (0; .)) as $k (.; .; .))",
    "(foreach (foreach . as [$a] (.; .; .)) as $k (.; .; .))",
    # (#3853) the nested folds #3744 left by value. An inner fold whose own SOURCE
    # destructures the register moves it, and the inner loop pattern's step then runs on
    # an element the register is no longer at: jq raises there, inside a `reduce` too.
    # Contrasts: a bare `$a` loop pattern performs no step, and `reduce . as [$a]` over the
    # register itself is backtracked, so neither raises.
    "(reduce (reduce (. as [$q] | .) as [$a] (0; .)) as $x (.; .))",
    "(reduce (reduce (. as {a:$q} | .) as {a:$a} (0; .)) as $x (.; .))",
    "(reduce (reduce (. as [$q] | .) as $a (0; .)) as $x (.; .))",
    "(reduce (reduce . as [$a] (0; .)) as $x (.; .))",
    "(foreach (reduce (. as [$q] | .) as [$a] (0; .)) as $x (.; .; .))",
    "(foreach (foreach (. as [$q] | .) as [$a] (0; .; .)) as $x (.; .; .))",
    "(foreach (foreach . as {a:$a} (0; .; .)) as $x (.; .; .))",
    "(reduce (foreach . as {a:$a} (0; .; .)) as $x (.; .))",
    "(foreach (foreach . as [$a] (0; .; .)) as $x (.; .; .))",
    "(foreach (foreach . as [$a] (.; .; .)) as $k (.; .; .)) or .a",
    "try ((foreach (. as {a:$a} | .) as [$z] (.; .; .)) and true) catch 7",
    # (#3939) a nested `foreach` that leaves the register on a `null`/boolean member: the outer
    # EXTRACT is `jv_identical` to it by kind, so the path is that member's, not the root's.
    # A navigating source, a deeper pattern, a comma source and an INIT that navigated first.
    "(foreach (foreach .a as $a (0; .; .)) as $x (.; .; .))",
    "(foreach (foreach . as {a:{b:$a}} (0; .; .)) as $x (.; .; .))",
    "(foreach (foreach (., .) as {a:$a} (0; .; .)) as $x (.; .; .))",
    "(foreach (foreach . as {a:$a} (0; .; .)) as $x (.a; .; .))",
    "(foreach (foreach . as {a:$a} (0; .; .)) as $x (.; .; .a))",
    # A literal INIT does not move the register, so a `true`/`false`/`null` member is the
    # accumulator's node by kind. The `?//` chain is the recorded gap: it is left to the
    # by-value drive (`routes_destructuring`), so it still answers the root where jq does not.
    "(foreach (foreach . as {a:$a} (0; .; .)) as $x (true; .; .))",
    "(foreach (foreach . as {a:$a} (0; .; .)) as $x (null; .; .))",
    "(foreach (foreach . as {a:$a} ?// [$a] (0; .; .)) as $x (.; .; .))",
    # (#3953) a `reduce` UPDATE that hands the accumulator's own node back after a register
    # destructure: jq backtracks the whole UPDATE, so the final check reads INIT's register. The
    # contrasts return a destructured `$var`, destructure with an array pattern, or follow the
    # bind with a navigation. The dangerous direction is a `reduce` whose own source navigated
    # (the `reduce-update-first-source`/`computed-key` contexts), where `.` is not the register.
    # Review of the first version: shapes that deleted or overwrote the document where jq raises.
    "(reduce 1 as $j (.; foreach .[]? as $e (.; .; .)))",
    "(. as {b:$b} | foreach 1 as $e (.; .; .))",
    "(foreach .[]? as $j ((. as {a:$a} | .); .; .))",
    "(. as {a:$a} | foreach first(. as {a:$a} | .) as $j (.; .; .))",
    "(foreach (1,2) as $x (.; (foreach (select($x == 2)) as $y (.; .; .)); .))",
    "(. as {a:$a} | .)",
    "(. as [$a] | .)",
    "(. as {a:$a} | $a)",
    "((. as {a:$a} | .) | .a)",
    "(foreach (. as {a:$a} | .) as $x (.; .; .))",
    # (#3883) a nested `foreach` whose source navigates leaves the register on each element it
    # visits; an outer `null`/boolean accumulator is identical to a `null`/boolean member by kind.
    # Contrasts: an UPDATE that navigates, an EXTRACT that computes, a `reduce` that backtracks.
    "(foreach (foreach .[]? as $x (0; .+1)) as $k (null; .))",
    "(foreach (foreach .[]? as $x (0; .+1; .+2)) as $k (null; .a))",
    "(foreach (foreach .[] as $x (0; .+1; tostring)) as $k (true; .))",
    "(foreach (foreach .a as $x (0; .+1)) as $k (null; .))",
    "(foreach (foreach .[]? as $x (0; .a?)) as $k (null; .))",
    "(foreach (foreach .[]? as $x (0; .+1; .a?)) as $k (null; .))",
    "(foreach (foreach .[]? as $x (.a; .+1)) as $k (null; .))",
    "(foreach (foreach .[]? as [$q] (0; .+1)) as $k (null; .))",
    "(foreach (reduce .[]? as $x (0; .+1)) as $k (null; .))",
    # (#3940) a leading stage that only hands the register on, ahead of a register destructure.
    # Contrasts: a bare loop variable, a nested `reduce`, a navigated leading stage.
    "(foreach ((.|.) | foreach . as {a:$a} (0; .; .)) as $x (.; .; .))",
    "(foreach (.|.|foreach . as {a:$a} (0; .; .)) as $x (.; .; .))",
    "(foreach ((., .) | foreach . as {a:$a} (0; .; .)) as $x (.; .; .))",
    "(foreach ((.|.) | . as {a:$a} | .) as $x (.; .; .))",
    # Known gap (#3956): a no-op bind head. Routing it through the resolver turned a refused
    # `or`-under-`try` delete into a silent no-op (the resolver does not model `(.|.) as` as
    # the register), so it stays by value until the resolver does.
    "(foreach ((.|.) as {a:$a} | .) as $x (.; .; .))",
    "(foreach ((., .) as {a:$a} | .) as $x (.; .; .))",
    "(reduce ((.|.) | . as [$q] | .) as [$a] (0; .))",
    # Known gap (#3955 review): other stages that hand the register on (`select(true)`,
    # `first(.)`, `try`) hide the destructure the same way and are still read by value.
    "(foreach ((.|select(true)) | foreach . as {a:$a} (0; .; .)) as $x (.; .; .))",
    "(foreach ((.|.) | foreach . as [$a] (0; .; .)) as $x (.; .; .))",
    "(foreach ((.|.) | foreach . as $a (0; .; .)) as $x (.; .; .))",
    "(foreach ((.|.) | reduce . as {a:$a} (0; .)) as $x (.; .; .))",
    "(foreach (.a | foreach . as {a:$a} (0; .; .)) as $x (.; .; .))",
    # (#3790) a fold whose SOURCE is the register itself: `.` does not move jq's register
    # and `$k` is that very node, so a bare `$k` is a path (any value, not only null). The
    # contrasts are a navigated source (`.a`, `.[]?`) and a computed one (`tostring`);
    # `reduce` over `.` binds the same element.
    "(foreach . as $k (0; $k; .))",
    "(foreach . as $k (0; .; $k))",
    "(foreach . as $k (0; $k.a?; .))",
    "(foreach . as $k (.; $k | .a?; .))",
    "(foreach . as $k (0; try $k; .))",
    "(foreach . as $k (0; ($k | .a?) // $k; .))",
    "(foreach (., .) as $k (0; $k; .))",
    "(foreach (. | .) as $k (0; $k; .))",
    "(foreach . as $k (0; first($k); .))",
    "(foreach tostring as $k (0; $k; .))",
    "(foreach .a? as $k (0; $k; .))",
    "(foreach .[]? as $k (0; $k; .))",
    "(reduce . as $k (0; $k))",
    "(reduce . as $k (.; $k))",
    "(reduce . as $k (0; $k.a?))",
    "(foreach . as [$a] (0; $a; .))",
    # ...a fold over `.` nested in another fold's UPDATE (not recognised there: the accumulator
    # may have been moved off the register by the outer source), and one piped after a fold.
    "(reduce .a? as $x (.; foreach . as $k (0; $k; .)))",
    "(reduce first as $x (.; foreach . as $k (0; ($k | .a?) // $k; .)))",
    "(foreach . as $x (.; .; .) | foreach . as $k (0; $k; .))",
    "try (foreach . as $k (0; $k; .)) catch 7",
    "(reduce (foreach . as {a:$a} (.; .; .)) as $k (.; .))",
    "(reduce . as [[$a]] (0; .))",
    "try (reduce . as [[$a]] (0; .)) catch 7",
    "(reduce . as [[$a]] ?// [$a] (0; .))",
    "(reduce . as {a:{b:$a}} ?// $a (0; .))",
    "try (reduce . as {a:[[$a]]} (0; .)) catch 7",
    "(foreach . as [$a] ?// $a (0; .; .))",
    "(foreach . as {a:$a} ?// [$a] (0; .; $a))",
    "(foreach . as [$a] ?// $a (0; $a; .))",
    "(foreach .a as [$a] ?// $a (0; .; .))",
    "try (reduce . as [$a] ?// $a (0; .)) catch 7",
    "try ((reduce . as [$a] ?// $a (0; .)), .b[0]) catch 1",
    # (#4048) a destructuring `reduce` leaves jq's register where it entered: the pattern's
    # steps run inside the backtracked loop, so only INIT's navigation survives the fold.
    # The contrasts are what must stay refused or moved: an element that is not the
    # register (a constructed object or array), an UPDATE that reads the bound name or
    # hands back the register, a navigating INIT, and a navigating source.
    "(reduce . as {a:$a} (0; $a))",
    "(reduce . as {a:$a} ?// $a (0; $a))",
    "(reduce {a:1} as {a:$a} ?// $a (0; .))",
    "(reduce [[1]] as [[$a]] ?// $a (0; .))",
    "(reduce .a? as {b:$b} ?// $b (0; .))",
    "(reduce .[]? as {a:$a} (.; .))",
    "(reduce . as {a:$a} ?// $a (.a?; .))",
    "(reduce . as {a:$a} ?// $a (0; .a?))",
    "(reduce . as {a:$a} ?// $a (.; $a))",
    "(reduce . as [$a, $b] ?// {a:$a} (.; .))",
    "first(.a)",
    "nth(0; .a)",
    "limit(1; .a)",
    # (#3653) `last(f)` keeps jq's register through the wrappers that add no
    # movement of their own: `?`, `try` with no `catch`, and `first(...)`
    # (which emits from inside `last`, where the register is back where it
    # entered). The contrasts are the wrappers that are not admitted, and so stay
    # refused where jq answers (a compound stage; `limit`/`nth` of a `last`
    # and a `try ... catch` with a non-navigating handler were contrasts until #3767 admitted them), and the same
    # wrappers around a `f` that navigates, which move it and must stay refused.
    "last(.a)?",
    "(last(.a))?",
    "try last(.a)",
    "first(last(.a))",
    "first(last(.a))?",
    "last(.[]?)?",
    "(try last(.a) catch .)",
    "limit(1; last(.a))",
    "nth(0; last(.a))",
    "first(.a)?",
    "first(first(.a))",
    # (#3653) `select(f)` and the type filters are `if f then . else empty end`
    # over a subexp condition: they pass their input through at the register
    # whatever `f` navigates. `(.a | select(.))` is the contrast whose
    # navigation precedes the select, which moved the register.
    "select(.)",
    "select(.a)",
    "select(.a?)",
    "select(false)",
    "select((true, true))",
    "select(first(.[]?))",
    "(last(.a) | select(.))",
    "(.a | select(.))",
    # (#3653) the wrappers read through for `select` and the type filters too.
    "select(.)?",
    "try select(.)",
    "first(select(.))",
    "(numbers)?",
    "try numbers",
    "first(numbers)",
    # (#3767) `limit(n; E)` and `nth(n; E)` emit from inside `E`, so the inner stage
    # decides: `last(f)`, `select(f)` and the type filters leave the register where
    # it entered, and `.a` moves it. `limit(1; .a)`, `nth(0; .a)` (above), the
    # handler, the compound inner and the navigation-before-`select` rows are the
    # contrasts that must stay refused. `[numbers]` is the `[E]` collect of a type
    # filter, which the array allowlist now names beside `select`.
    "limit(1; select(.))",
    "limit(2; select(.))",
    "limit(-1; select(.))",
    "limit(0; select(.))",
    "nth(0; select(.))",
    "nth(1; select(.))",
    "limit(1; numbers)",
    "nth(0; values)",
    "limit(2; last(.a, .))",
    "limit(1; select(.a))",
    "limit(1; first(last(.a)))",
    "limit(1; try select(.))",
    "limit(1; select(.)?)",
    "first(limit(1; last(.a)))",
    "nth(0; limit(1; select(.)))",
    "limit(1; limit(1; last(.a)))",
    # (#3767 Part 3) a `try E catch H` over a register-keeping `E`: the handler runs after a
    # backtrack that restores the register, so the inner stage decides. A handler that
    # navigates (`catch .a`, `catch (.a | $x)`), a `E` that navigates (`.a`, `first(.a)`),
    # and a compound inner (`,` `|`) are the contrasts that stay refused.
    "try last(.a) catch .",
    "try last(.a) catch 7",
    "try last(.a) catch $x",
    "try last(.a) catch error(\"z\")",
    "try last(.a) catch .a",
    "try select(.) catch .",
    "try select(.a) catch 7",
    "try numbers catch .",
    "try values catch $x",
    "(try select(.) catch .)?",
    "first(try select(.) catch .)",
    "try (try last(.a) catch 1) catch .",
    "try (try last(.a) catch .a) catch 2",
    "try first(select(.)) catch .",
    "limit(1; try select(.) catch .)",
    "try .a catch .",
    "try first(.a) catch 7",
    "try (last(.a), error(\"e\")) catch .",
    "try (select(.) | error(\"z\")) catch .",
    "try error(\"z\") catch .",
    # (#3891) a catch handler that is a `?`-wrapped navigation. `INDEX_OPT` swallows a
    # type error but not a path error, and the caught payload may or may not be the
    # register's own node: `error` / `error(.)` raise it, `error("x")` / `error({..})`
    # a different one, so each pairing is its own operand.
    "try error catch (.a)?",
    "try error catch .a?",
    "try error catch (.a | .b)?",
    "try error catch (.a // 1)?",
    "try error catch first(.a)?",
    "try error(.) catch (.a)?",
    "try error(\"x\") catch (.a)?",
    "try error({\"a\":1}) catch (.a)?",
    "try error catch (try .a catch 0)",
    # (#4019) a body that raises its own input through more than a bare `error`, so the
    # handler's payload is the register's node (a scalar one included), and the same
    # shapes whose condition can raise or whose stage navigates first, which are not.
    "try error catch .",
    "try error(.) catch .",
    "try (error | error) catch (.a)?",
    "try (error(.) | error(\"x\")) catch ((.a)? // 5)",
    "try (error(.) | error(\"x\")) catch .a",
    "try (error(.) | error(\"x\")) catch .",
    "try (if . == 1 then error(.) else . end) catch .",
    "try (if . == 1 then error(.) else . end) catch ((.a)? // 5)",
    "try (if true then error else . end) catch .a?",
    "try (if . == null then . else error end) catch (.a)?",
    "try (if .a then error(.) else . end) catch ((.a)? // 5)",
    "try (., error) catch (.a)?",
    "try (error, .) catch .[0]?",
    "try (select(true) | error) catch (.a)?",
    "try (select(. == 1) | error) catch .",
    "try (select(.a) | error) catch .a",
    "try (.a | error) catch .",
    "try (.[0]?, error) catch (.[0])?",
    "try error([.][0]) catch (.a)?",
    # (#4146) `error(msg)` is `msg | error`: a message that navigates is path-checked.
    "try error(.b) catch .",
    "try error(.a) catch (.a)?",
    "try error(.b | tostring) catch .",
    "try error(.[0]) catch 7",
    # (#4019 review) `. // X` is `X` on a null/false register, so a body raising it does not
    # raise the register's own node.
    "try error(. // {\"a\":1}) catch .a",
    "try ((. // 1) | error) catch .",
    "try (if true then error(. // 5) else . end) catch (.a)?",
    "try ((. // 1), error) catch .",
    "try (select(. // 1) | error) catch (.a)?",
    "try last(error) catch .",
    "try last(.a) catch input",
    # (#4127) a body that raises a `$y` frozen at the `try`'s own node, however it steps off
    # the node first, so the handler's payload is the register's node; and the contrasts that
    # raise a message, another node or a value built by hand, which are not.
    "try (. as $y | .a | error($y)) catch (.a)?",
    "try (. as $y | if .a then error($y) else empty end) catch (.a)?",
    "try (. as $y | (., error($y))) catch (.a)?",
    "try (. as $y | .[0] | error($y)) catch .[0]?",
    "try (. as $y | .a | . as $z | error($z)) catch (.a)?",
    "try (. as $y | .a | error({\"a\":1})) catch (.a)?",
    "try (. as $y | if .a then error($y) else error({\"a\":1}) end) catch (.a)?",
    # (#3767 Part 4) a `first`/`limit`/`nth` over a stage that navigates nothing leaves the
    # register where it was, and the same three around a type filter inside a `[E]` collect
    # are path-checked as in a pipe. The contrasts navigate (`first(.a)`, `[limit(1; .a)]`),
    # sit behind a navigating tail (`[first(.a) | .b]`), or are compound (`first((5, .a))`)
    # and must stay refused where jq refuses.
    "first(5)",
    "first(\"s\")",
    "limit(1; 5)",
    "limit(2; 5)",
    "limit(0; 5)",
    "limit(-1; 5)",
    "nth(0; 5)",
    "nth(1; 5)",
    "first(null)",
    "first($x)",
    "first(empty)",
    "first({a: .a})",
    "first(5)?",
    "try first(5)",
    "first(limit(1; 5))",
    "limit(1; first(5))",
    "first((5, .a))",
    "first((.a, 5))",
    "limit(1; (5, .a))",
    "nth(0; (.a, 5))",
    "first(error(\"z\"))",
    "first(.a | 5)",
    # a wrapper over a stage the leaf verdict (`leaves_register_in_place`) admits but
    # `cannot_move_register` does not list (#4108): by-value builtins and
    # `input_line_number`, bare and under each wrapper. `path(.a)`, `first(path(.a))` and
    # `first(path(.a, .k))` are the contrast: `path(f)` is not in the leaf verdict (#4118),
    # so those stay refused where jq answers, and a row that moves is a change to read
    "first(length)",
    "first(to_entries)",
    "limit(1; sort)",
    "first(path(.a))",
    "first(input_line_number)",
    "nth(0; add)",
    "first(map(.))",
    "first(reverse)",
    "limit(1; min)",
    "first(flatten)",
    "first(path(.a, .k))",
    "path(.a)",
    "input_line_number",
    "[first(numbers)]",
    "[first(strings)]",
    "[first(select(.))]",
    "[first(values)]",
    "[limit(1; numbers)]",
    "[limit(2; select(.))]",
    "[limit(0; numbers)]",
    "[nth(0; numbers)]",
    "[nth(1; values)]",
    "[first(last(.a))]",
    "[first(first(numbers))]",
    "[limit(1; first(select(.)))]",
    "[first(.a)]",
    "[first(.a, 5)]",
    "[first(.[]?)]",
    "[first(.a | numbers)]",
    "[first(numbers) | .a]",
    "[first(numbers), .a]",
    "[limit(1; .a)] | .[0]",
    "[nth(0; .a)]",
    "[first(5)]",
    "[first(.a?)]",
    "[first(try .a catch 1)]",
    "[first(getpath([\"a\"]))]",
    "[first(with_entries(.))]",
    "[first(.a = 1)]",
    "[limit(1; .a |= 1)]",
    "[first(def f: .a; f)]",
    "limit(1; try last(.a) catch .)",
    "limit(1; (last(.a), select(.)))",
    "limit(1; (.a | select(.)))",
    "nth(0; (.a | last(.a)))",
    "limit(1; first(.a))",
    # (#3767 Part 5) a compound stage (`,` `//` `if`, a parenthesised pipe) whose every branch
    # leaves the register alone: jq backtracks to each branch's fork, so the register after
    # the whole is back at entry. The contrasts have a branch that navigates (`.a`, `first(.a)`,
    # `(.a)?`, `getpath`), a navigating tail (`| .a`) or a `def` call (never admitted), and
    # must stay refused where jq refuses.
    "(select(.), select(.))",
    "(select(.), (select(.) | select(.)))",
    "(select(.) // (select(.) | last(.a)))",
    "(select(.), 7)",
    "(empty, select(.))",
    "(last(.a), last(.k))",
    "(select(.) // 7)",
    "(select(.) // select(.))",
    "(last(.a) // select(.))",
    "(null // select(.))",
    "if . then select(.) else last(.a) end",
    "if .k then select(.) else 7 end",
    "if select(.) then 1 else 2 end",
    "(select(.) | select(.))",
    "(select(.) | (select(.), select(.)))",
    "(select(.) | last(.a))",
    "first(select(.), 3)",
    "limit(2; (select(.), 3))",
    "[(select(.), select(.))]",
    "try (select(.), error(\"e\")) catch .",
    # (#4151) a `catch` handler that is a compound of `select` stages navigates nothing at
    # stage level (`handler_navigates_nothing`); a branch that navigates keeps it refused.
    "try (select(.), error(\"e\")) catch (select(.), 7)",
    "try last(.a) catch (select(.), 7)",
    "try last(.a) catch select(.)",
    "try select(.) catch (numbers // 7)",
    "try (select(.), error(\"e\")) catch (select(.) | 7)",
    "try (select(.), error(\"e\")) catch (if . then select(.) else 7 end)",
    "try (select(.), error(\"e\")) catch (select(.), .a)",
    "try (select(.), error(\"e\")) catch (select(.) | .a)",
    "try (select(.), error({\"a\":1})) catch (select(.), .a)",
    "try (select(.), error(.)) catch (select(.), 7)",
    "try (select(.), error(null)) catch (select(.), 7)",
    "try error(.) catch (select(.), 7)",
    "try error(.) catch select(.)",
    "try error($x) catch (select(.), 7)",
    "try (error(.) | 1) catch (select(.), 7)",
    "(try (select(.), error(\"e\")) catch (select(.), 7))?",
    "first(try (select(.), error(\"e\")) catch (select(.), 7))",
    "try (select(.), error(\"e\")) catch ((select(.), 7) // 8)",
    "try (select(.), error(\"e\")) catch (select(.), last(.a))",
    # (#4309) a `catch` handler that navigates, over a body that can raise only a fresh
    # message string (`body_raises_only_fresh_messages`): it never runs on a document value.
    "try last(.a) catch .a",
    "try last(.a) catch .a.b",
    "try last(.a) catch .[0]",
    "try last(.a) catch .[]",
    "try last(.a) catch (.a | .b)",
    "try last(.a) catch .[1:]",
    "try select(.a) catch .a",
    "try numbers catch .a",
    "try (select(.), last(.a)) catch .a",
    "try last(.a[]) catch .a",
    "try last(.a) catch .a?",
    "try last(.a) catch (.a // 5)",
    "try last(.a) catch (.a, 7)",
    "try last(.a) catch last(.a)",
    "try last(.a) catch ($x | .a)",
    "try last(.a) catch ($x | .a.b)",
    "try error(.) catch .a",
    "try error($x) catch .a",
    "try (last(.a), error(.)) catch .a",
    "try (last(.a), error({\"a\":1})) catch .a",
    "try last(error(.)) catch .a",
    "try last(.a | error) catch .a",
    "try (.a | last(.b)) catch .a",
    "try first(.a) catch .a",
    "(try last(.a) catch .a)?",
    "first(try last(.a) catch .a)",
    "limit(1; try last(.a) catch .a)",
    "[try last(.a) catch .a]",
    "try (try last(.a) catch .a) catch .a",
    "try last(.a) catch (try .a catch 7)",
    "(select(.), (select(.) // 7))?",
    "(select(.), .a)",
    "(select(.) // .a)",
    "(.a // select(.))",
    "if .k then select(.) else .a end",
    "if .k then .a else select(.) end",
    "(select(.) | .a)",
    "(select(.), last(.a) | .b)",
    "(select(.), first(.a))",
    "(select(.), (.a)?)",
    "(select(.), getpath([\"a\"]))",
    "(select(.), (def g: .a; g))",
    "(def g: select(.); g)",
    # (#4109) a zero-arity `def` call is as register-keeping as its body (`def_call_keeps_register`).
    # Known-refused where jq answers (the safe direction, listed in limitations.md): a call with
    # an argument (`g(.a)`), a call under an `[E]` collect (`[g]`), and a call asked past the
    # shared 16-body budget; a body that navigates (`.a`, `first(.a)`, `| .b`) raises as in jq.
    "(def g: last(.a); g)",
    "(def g: select(.a); g)",
    "(def g: numbers; g)",
    "(def g: try last(.a) catch .; g)",
    "(def g: (last(.a), select(.)); g)",
    "(def g: .a; g)",
    "(def g: first(.a); g)",
    "(def g: last(.a); g | g)",
    "(def g: last(.a); def h: g; h)",
    "(def g: last(.a); def h: g | .a; h)",
    "(def g: last(.a) | .b; g)",
    "(def g(f): last(f); g(.a))",
    "(def g: last(.a); first(g))",
    "(def g: last(.a); g)?",
    "(def g: last(.a); try g catch .)",
    "(def g: last(.a); [g])",
    "(def g: last(.a); (g, 7))",
    "(def g: last(.a); (g // 7))",
    "(def g: last(.a); if . then g else 7 end)",
    "(def g: last(.a); g | select(.))",
    "(def g: select(.); g | last(.a))",
    "(def g: last(.a); (def g: .a; g))",
    "(def g: (last(.a), .a); g)",
    "(def g: try (last(.a), error(.)) catch .a; g)",
    "(def g: last(.a); def h: (g, 5); h)",
    "(def g: last(.a); g | last(.b))",
    "(def g: last(error(.)); g)",
    "(def g: reduce .[]? as $i (.; last(.a)); g)",
    "[numbers]",
    "[strings]",
    "[select(.)]",
    "[numbers, strings]",
    "[.[]? | numbers]",
    # (#4152) an `[E]` collect as a branch of a compound stage, read by the same rule as
    # the bare collect (`collect_keeps_register`). The contrasts that must stay refused:
    # a navigation after the collect, a `getpath` inside it, and the shapes that wrap a
    # collect (`try`, `?`, `first`), which the bare stage refuses too.
    "(select(.), [numbers])",
    "([numbers], select(.))",
    "([numbers], 7)",
    "(select(.), ([numbers]))",
    "([numbers], [strings])",
    "if .k then [numbers] else select(.) end",
    "if .k then select(.) else [numbers] end",
    "([numbers] // 7)",
    "(select(.) // [numbers])",
    "(select(.) | [numbers])",
    "([numbers] | select(.))",
    "([numbers] | length)",
    "(select(.), ([numbers] | length))",
    "(select(.), [.a])",
    "(select(.), [.a?])",
    "([.[]?], select(.))",
    "(select(.), [last(.a)])",
    "(select(.), [numbers] | .a)",
    "([numbers] | .a)",
    "(select(.), [numbers, getpath([\"a\"])])",
    "(select(.), [with_entries(.)])",
    "(select(.), [walk(.)])",
    "(select(.), [.k |= . + 1])",
    "(select(.), try [numbers] catch 7)",
    "try (select(.), [numbers]) catch 7",
    "try (select(.), [numbers]) catch .a",
    "try (select(.), error(\"e\")) catch [numbers]",
    "first(select(.), [numbers])",
    "limit(2; (select(.), [numbers]))",
    # (#3653 review) a `last`/`select` whose output *is* the register: `.` and, in
    # the contexts that bind it, `$x`. `select` hands its input through as the
    # very value it received, so the register keeps its identity; `last(f)`'s
    # result is the very value `f` last emitted, so `last(.)`/`last($x)` keep it
    # too (#3766 fixed what #3643's arm lost). A context that does not bind `$x`
    # makes both sides fail to compile, which is a MATCH.
    "last(.)",
    "last($x)",
    "try last($x)",
    "last($x)?",
    "first(last($x))",
    # (#3766) which of a generator's outputs is the register: only the last one
    # decides, and only when it is the entry node itself. `last(.a, .)` is the
    # shape a navigating `f` in a reduce UPDATE turned into an accepted root
    # (`del(reduce .[]? as $k (.; last(.a, .)))` deleted the document before the
    # arm was restricted to an `f` that navigates nothing); `last(first(.))` and
    # `last(select(true))` navigate nothing but are outside the
    # `cannot_move_register` allowlist, so they keep the copy.
    "last(.a, .)",
    "last(., .a)",
    "last(first(.))",
    "last(select(true))",
    "last(last(.))",
    "last(limit(1; .))",
    # An `f` the reduce walk cannot read may still navigate before its last output,
    # the root: `del(reduce .[]? as $k (.; last(limit(2; .a, .))))` deleted the document
    # once `last(f)` stopped refusing an `f` that may navigate (#3786 review).
    # `first(f)` stops at its first output: `first(., .a)` never runs `.a`.
    "first(., .a)",
    "last(limit(2; .a, .))",
    "last(nth(1; .a, .))",
    "last(label $l | .a, .)",
    "last(def f: .a, .; f)",
    "select($x)",
    "try select($x)",
    "numbers",
    "objects",
    "arrays",
    "strings",
    "values",
    "nulls",
    "booleans",
    "iterables",
    "scalars",
    # (#3738) a fold's own loop variable used inside its UPDATE/EXTRACT, where the
    # generator in the body (`.[]?`, `..`) used to make the fold decline to hand
    # the body its register, so a `try` around it caught the resulting refusal
    # and the UPDATE yielded nothing. The comma and `//` rows are the contrasts
    # that must keep refusing (sibling branches see the register unevenly), and the
    # Index/Slice chains that `is_single_path_chain` takes from `is_navigation_node`.
    # A context that does not bind `$k` skips these rows (see `build_rows`).
    "$k",
    "($k | .[]?)",
    "($k | ..)",
    "($k | .[0])",
    "($k | .a?)",
    "($k | .[]? | .a?)",
    "first($k | .[]?)",
    "try ($k | .[]?)",
    "try ($k | ..)",
    "try ($k | .a)",
    "($k, 1)",
    "(($k | .[]?), $k)",
    "($k | .[]?) // $k",
    "($k | .[0] | .[]?)",
    "($k | .[0:1])",
    "($k | .[] | .[0:1])",
    "try ($k | .a[0] | .[]?)",
    "try ($k | .[] | .[0:1])",
    # (#3361) the rest of the by-value builtins jq defines over a backtracked
    # source or never lets touch the register: `walk(f)` and `map(f)` qualify
    # only for an `f` that navigates nothing (`walk(.a)` and `map(.a)` are the
    # contrasts: jq path-checks `f` against the elements, a by-value stage does
    # not).
    "walk(.)",
    "walk(tostring)",
    "walk(.a)",
    "map(tostring)",
    "map(.a)",
    # (#3711) promoted: `reverse` is a collect that backtracks, `min`/`max` are C
    # calls, `min_by`/`max_by`/`group_by`/`sort_by` are C calls over a `map([f])`
    # argument (a subexp, so any `f` is fine), `flatten(n)` and `join(s)` are
    # `reduce .[]` like `add`. `unique_by` is the contrast jq raises on (it
    # iterates a computed array), which must stay refused.
    "reverse",
    "min",
    "flatten(1)",
    "group_by(.)",
    "max",
    "min_by(.)",
    "max_by(.)",
    "group_by(.a)",
    "sort_by(.)",
    "sort_by(.a)",
    "flatten(0)",
    "join(\",\")",
    "unique_by(.)",
    # (#3347) `indices`/`index`/`rindex` are `.[$i]` / `.[[$i]]` on their input
    # (and `index`/`rindex` then index the array `_strindices` built), so on an
    # untracked value jq raises naming the *evaluated pattern*, which the leaf
    # resolvers now observe once. The patterns cover each type the access names
    # differently (a bool and a null against the array and string inputs, an
    # array, a string), a fork (jq raises on the first value), and an empty one
    # (nothing runs, so nothing raises).
    "indices(true)",
    "index(true)",
    "rindex(null)",
    "indices([true])",
    "index(\"a\")",
    "rindex(\"a\")",
    "indices(true, null)",
    "indices(empty)",
    # (#3890) on a *tracked* input the same three are the path step `.[$i]` / `.[[$i]]` (and
    # `index`/`rindex` then step `.[0]` / `.[-1:][0]` of the answer), so they move the register:
    # `path(.a | indices(1))` is `["a",[1]]`. An array, an object, `null` and a string input; a
    # pattern that is an array, a bool, a string, a fork, and none.
    "(.a | indices(true))",
    "(.a | index(true))",
    "(.a | rindex(true))",
    "(.a | indices([true]))",
    "(.a | indices(\"a\"))",
    "(.a | indices(true, null))",
    "(.a | indices(empty))",
    "(.[0] | index(1))",
    "(.[0] | rindex([1]))",
    "(.a | index(\"a\"))",
    "(.a.b | indices(1))",
    "(.a | indices(1) | .[0])",
    # a string input: `indices` of a string by a string only reads, and `index`/`rindex` then take
    # `.[0]` / `.[-1:][0]` of the positions, which is never the register (jq names the access).
    "(.key | indices(\"a\"))",
    "(.key | index(\"a\"))",
    "(.key | rindex(\"a\"))",
    "(.key | index(true))",
    "(\"abc\" | index(\"b\"))",
    "(\"abc\" | rindex(\"b\"))",
    "(.a | tostring | index(\"t\"))",
    "(.a | tostring | rindex(\"t\"))",
    # (#3360) `from_entries` is `map({...}) | add`: it always iterates the array
    # `map` built, so jq raises on every value it produces. The `walk` rows are
    # the zero-output case (`f` yields nothing, but an input that reaches an
    # object raises in `map_values` all the same) and the contrasts that stay
    # accepted wrongly where `f` navigates a computed array (`.a?`, `.[]?`).
    "from_entries",
    "walk(empty)",
    "walk(select(type != \"array\"))",
    "walk(.a?)",
    "walk(.[]?)",
    # (#3724) `map`/`any`/`all` inside an `[E]` collect, an `f` that navigates only the
    # register's own node (jq: `[]`, the collect backtracks) against the contrasts that
    # navigate a computed value (jq raises) and must stay refused: a construction, a
    # `tostring`/`sort`/`length` result, a fork mixing the two, a `try`/`//` (a different
    # route), and `walk`, whose trailing `f` always runs on a computed array.
    "[map(.a)]",
    "[map(.[0])]",
    "[map(first)]",
    "[map(last)]",
    "[map(.a | .b)]",
    "[map(.a?)]",
    "[map(.[]?)]",
    "[map(..)]",
    "[map(.a, .b)]",
    "[map(select(.a))]",
    "[map(numbers)]",
    "[map(if .a then .b else .[0] end)]",
    "[map(.a | length)]",
    "[map(.[.a?])]",
    "[map(.[0:1])]",
    "[any(.a)]",
    "[all(.a)]",
    "[any(.[0]?)]",
    "[all(.a | .b)]",
    "[any(select(.a))]",
    "[map({k:1} | .k)]",
    "[map(tostring | .a)]",
    "[map(.a | tostring | .b)]",
    "[map(.a, {k:1} | .k)]",
    "[map((.a, length) | .b)]",
    "[map(sort | .[0])]",
    "[map(.a | {k:1} | .k)]",
    "[any({k:1} | .k)]",
    "[all(.a | {k:1} | .k)]",
    "[map(try .a)]",
    "[map(.a // .b)]",
    "[map(first(.a))]",
    "[map(getpath([\"a\"]))]",
    "[map(.a | getpath([\"b\"]))]",
    "[walk(.a)]",
    "[map(.a)] | .[0]",
    "[any(.a)] | .a",
    "[map(.a)] | $x",
    "([map(.a)], .a)",
    "try [map({k:1} | .k)]",
    "[.[] | map(.a)]",
    "[map(map(.a))]",
    "[map(.a | map(.b))]",
    # (#3724, item 2) an `and`/`or`/unary minus inside an `[E]` collect whose operands
    # the narrower `register_movement_tracked` allowlist rejected and
    # `array_contents_are_checked` accepts: a bare `first`/`last`/`add`/`any`, a
    # `map(f)`, `last(f)`. jq's answer depends on the value (a short-circuit never
    # runs the right operand), so each is swept over inputs that decide both ways.
    "[first and true]",
    "[first and .[0]]",
    "[.[0] and first]",
    "[first or .a]",
    "[last and first]",
    "[add and 1]",
    "[any and all]",
    "[map(.) and true]",
    "[map(.a) and .[0]]",
    "[true or first]",
    "[-first]",
    "[-(add)]",
    "[(first and true) or last]",
    # ...and the other shapes `array_contents_are_checked` admits that an operand can now be
    # (the review of #3724 item 2): `try`, `if`, `recurse`, `walk`, `flatten`.
    "[(try .a) and true]",
    "[true or (try .a)]",
    "[(try (.a | tostring) catch .) and first]",
    "[true or (if .a then first else .[0] end)]",
    "[(recurse(.[]?)) and true]",
    "[true or (walk(.))]",
    "[(flatten) and first]",
    "[-(try .a)]",
    "[last(.a) and first]",
    "[.a? and first]",
    "[(.a, .[0]) and last]",
    # (#3865) a bare `map(f)` is `[.[] | f]` in jq, so an `f` that navigates a computed value
    # raises there, whatever the stage does with the result. The by-value leaf never raised, so
    # a stage that discarded the output accepted (`path(map({k:1} | .k) | empty)`). The
    # contrasts keep an `f` jq answers: `//`, `try`-free chains, a trailing non-navigating stage.
    "map({k:1} | .k)",
    "map(.a | {k:1} | .k)",
    "map(sort | .[0])",
    "map(.a // .b)",
    "map(.[0])",
    "map(first)",
    "map(.a | length)",
    "map(.[]?)",
    "map(select(.a))",
    "map(.a, {k:1} | .k)",
    "map(getpath([\"a\"]))",
    # ...with a `getpath` in `f`: it is never the first stage of `.[] | f`, so jq's path error inside
    # such an `f` is raised too (the review of #3865 found these rows still accepting).
    "map(getpath([\"a\"]) | {k:1} | .k)",
    "map({k:1} | .k, getpath([\"a\"]))",
    "map(error(\"x\"))",
    "map(empty)",
    # (#3580) a recursion's seed and a `catch` handler's output leave jq's register at
    # the stage's entry, one output at a time: a stage that bounds or forks a
    # recursion (`first`, `limit`, `,`, `if`, `//`) delivers the seed beside the outputs
    # that navigate, and the later outputs must keep refusing. The contrasts are the
    # wrappers that are not forwarders (a bind, an array, a pipe, a `def`, `nth`), the
    # same recursion behind a navigation, and a handler whose own body navigates.
    # (Not `skip`: jq 1.7.1 does not define it, so the oracle refuses every row.)
    "first(..)",
    "limit(1; ..)",
    "limit(2; ..)",
    "limit(0; ..)",
    "limit(2; recurse(.a))",
    "first(recurse(.a))",
    "first(try ..)",
    "first(.. | .)",
    "(., ..)",
    "(.., .)",
    "(.., 1)",
    "(1, ..)",
    "(.a, ..)",
    "(.., .a)",
    "if true then .. else 1 end",
    "if false then .. else 1 end",
    "if true then .. else .a end",
    "if .a then .. else . end",
    "if . then 1 else .. end",
    ".. // 3",
    "(try ..) // 3",
    "(.. | select(.)) // 3",
    "try recurse(.a) catch 7",
    "try recurse(.a) catch .",
    "try recurse(.a) catch .a",
    "try recurse(.a) catch $x",
    "try .. catch 7",
    "try (.a | error) catch 7",
    "try (.a | error) catch .",
    "try (.a | error) catch $x",
    "try error(1) catch .a",
    "try error(null) catch .",
    "(try ..)?",
    "recurse(.a)?",
    "limit(2; try recurse(.a) catch 7)",
    "[..]",
    "[first(..)]",
    ". as $q | ..",
    ". as [$q] | ..",
    ". as {a:$q} | ..",
    "def f: ..; f",
    # (#3914) a zero-argument `def` call forwards its body's statements as an inline body does;
    # an `if` branch that runs states the register it was entered with.
    "def f: try ..; f",
    "def f: first(..); f",
    "def f: ..; first(f)",
    "def f: (.a | ..); f",
    "def f: ..; def g: f; g",
    "def f(a): ..; f(1)",
    "if false then .. else (1, ..) end",
    "if true then (try ..) else 1 end",
    "if false then .. else $x end",
    "if false then .. else .a end",
    "nth(0; ..)",
    "(.. | .)",
    "(.a | ..)",
    # (#4123) a pipe nested in a forwarder whose last stages only hand the statement on: `.` and
    # `select(f)` after a seed producer, behind `if`, `try`, `first`, `limit`, a comma and a `def`.
    # The contrasts end in a stage that moves the register (`.a`), a type filter, or a `select`
    # whose input is not the producer's.
    "(.. | select(true))",
    "(.. | select(.))",
    "(.. | select(false))",
    "(.. | select(true) | select(true))",
    "(.. | . | select(true))",
    "(. | select(true))",
    "(.a | select(true))",
    "(.a? | select(true))",
    "(.. | .a)",
    "(.. | numbers)",
    "(.. | select(true) | .a?)",
    "first(.. | select(true))",
    "try (.. | select(true))",
    "limit(1; .. | select(true))",
    "if true then (.. | select(true)) else . end",
    "if false then 1 else (.. | .) end",
    "(1, (.. | select(true)))",
    "def f: .. | select(true); f",
    "def f: .. | .; f",
    "(try (.. | .) catch 7)",
    "(.. | select(.a?))",
    "(.. | .a?)",
    "(reduce (1,2) as $i (1; try ..))",
    "(reduce (1,2) as $i (1; ..))",
    "(reduce (1,2) as $i (1; try recurse(.a)))",
    "(reduce (1,2) as $i (1; try .a))",
    "(reduce (1,2) as $i (1; try (.a | error) catch 7))",
    "(reduce (1,2) as $i (.; try .a))",
    "(reduce (1,2) as $i (.a; try ..))",
    "(reduce .[]? as $i (1; try ..))",
    "(reduce (1,2) as $i (1; first(..)))",
    "(reduce (1,2) as $i (1; try ..) | .a?)",
    "(foreach (1,2) as $i (1; try ..; .))",
    "(foreach (1,2) as $i (1; try ..; $i))",
    "(foreach (1,2) as $i (1; try ..; .a?))",
    "(foreach (1,2) as $i (1; try ..; first(..)))",
    "(foreach (1,2) as $i (1; try recurse(.a); .))",
    "(foreach (1,2) as $i (1; try (.a | error) catch 7; .))",
    "(foreach (1,2) as $i (1; ..; .))",
    "(foreach (1,2) as $i (1; try .a; .))",
    "(foreach (1,2) as $i (.; try .a; .))",
    "(foreach (1,2) as $i (.a; try ..; .))",
    "(foreach .[]? as $i (1; try ..; .))",
    "(foreach (1,2) as $i (1; try ..; .) | .a?)",
    # (#4499) a fold as a sibling in a comma: jq forks the comma at its entry, so each side starts
    # from the register the comma was entered with and the fold's caught step still emits.
    "(foreach (1,2) as $i (1; try .a; .), 1)",
    "(1, foreach (1,2) as $i (1; try .a; .))",
    "(foreach (1,2) as $i (1; try .a; .), foreach (1,2) as $j (1; try .a; .))",
    "(reduce (1,2) as $i (1; try .a), 1)",
    "(foreach (1,2) as $i (null; .a; .), .a)",
    "(foreach (1,2) as $i (1; try .a; .), .)",
    "(foreach (1,2) as $i (1; try .a; .), (.a)?)",
    "(foreach (1,2) as $i (.; try .a; .), reduce (1,2) as $j (.; .))",
    # (#4499) a fold two levels down in a fold's UPDATE: the middle fold's register is untracked,
    # and the live `null`/boolean register its frame carries still reaches the innermost fold.
    "(foreach (1,2) as $i (1; try (reduce (1,2) as $j (1; try .a)); .))",
    "(foreach (1,2) as $i (1; (reduce (1,2) as $j (1; try .a)); .))",
    "(foreach (1,2) as $i (1; reduce (1,2) as $j (1; try .a); .))",
    "(foreach (1,2) as $i (1; try (foreach (1,2) as $j (1; try .a; .)); .))",
    "(reduce (1,2) as $i (1; reduce (1,2) as $j (1; try .a)))",
    "(foreach (1,2) as $i (null; foreach (1,2) as $j (null; .a; .); .))",
    "(foreach (1,2) as $i (0; foreach (1,2) as $j (1; try .a; .); .))",
    # (#4499 review) a pipe-bodied middle fold, and a fold sibling or nested fold reading a frozen `$k`
    # (#3984 withholds the register there, so a marker cannot re-establish against it).
    "(foreach (1,2) as $i (1; (reduce (1,2) as $j (1; try .a) | .); .))",
    "(foreach (1,2) as $i (1; (1 | reduce (1,2) as $j (1; try .a)); .))",
    "(foreach .a? as $k (0; (foreach (1,2) as $i (1; try ($k | .b?); .), 1); .))",
    "(foreach .a? as $k (0; (foreach (1,2) as $i (1; try ($k | .b?); .), foreach (1,2) as $j (1; try .a; .)); .))",
    "(foreach .a? as $k (0; foreach (1,2) as $i (1; reduce (1,2) as $j (1; try ($k | .b?)); .); .))",
    # (#4445, #4480, #4492) a fold over a `null` accumulator on an untracked entry, and a fold
    # nested in another fold's UPDATE: the live `null`/boolean register is the accumulator by kind,
    # whatever shape UPDATE has, and the inner fold's caught step still emits.
    "(reduce (1,2) as $q (null; reduce (3,4) as $r (.; .b)))",
    "(reduce (1,2) as $q (null; foreach (3,4) as $r (.; .b)))",
    "(reduce (1,2) as $q (null; foreach (3,4) as $r (.; .b; .)))",
    "(reduce (1,2) as $q (null; reduce (3,4) as $r (1; .b)))",
    "(reduce (1,2) as $q (null; null))",
    "(reduce (1,2) as $q (null; if $q == 1 then . else null end))",
    "(foreach (1,2) as $q (null; foreach (3,4) as $r (.; .b); .))",
    "(foreach (1,2) as $i (null; .a; .))",
    "(foreach (1,2) as $i (null; .; .))",
    "(foreach .a? as $k (0; foreach (1,2) as $i (1; try .a; .); .))",
    "(foreach .a? as $k (0; -(foreach (1,2) as $i (1; try .a; .)); .))",
    # (#4071) a `try` in UPDATE whose handler navigates a frozen `$k` may or may not have moved
    # jq's register (it runs only if the body raised), so a stage after it leaves the register
    # somewhere the resolver cannot name, and EXTRACT's refusal of `$k` is its guess.
    "(foreach .a? as $k (0; try (length | .) catch ($k | .b) | .; try ($k | .b?)))",
    "(foreach .a? as $k (0; try (length | .) catch ($k | .b) | .; $k | .b?))",
    "(foreach .a? as $k (0; try length catch ($k | .b) | .; try ($k | .b?)))",
    "(foreach .a? as $k (0; try (length | .) catch ($k | .a); try ($k | .b?)))",
    "(foreach .a? as $k (0; try (length | .) catch 7 | .; try ($k | .b?)))",
    "(foreach .a? as $k (0; (try (length | .) catch ($k | .b)) | (1, 2); try ($k | .b?)))",
    "(foreach .[]? as $k (0; try (length | .) catch ($k | .b) | .; try ($k | .b?)))",
    "(foreach .a? as $k (null; try (length | .) catch ($k | .b) | .; try ($k | .b?)))",
    "(foreach .a? as $k (.; try (length | .) catch ($k | .b) | .; try ($k | .b?)))",
    "(foreach .a? as $k (0; try (tostring | .) catch ($k | .b) | .; try ($k | .b?)))",
    "(foreach .a? as $k (0; try (length | .) catch ($k | .b) | .; $k))",
    # (#4071) the same gate, reached by any UPDATE the resolver cannot prove leaves the register
    # (`select`, a `def` call, a comma into either) rather than only by a navigating handler; and
    # the issue's own two shapes.
    "(foreach .a? as $k (0; select(true); try ($k | .b?)))",
    "(foreach .a? as $k (0; def f: .+1; f; try ($k | .b?)))",
    "(foreach .a? as $k (0; (., 1) | select(true); try ($k | .b?)))",
    "(foreach .a? as $k (0; (1, ($k | .b?)) | select(true); try ($k | .b?)))",
    "(foreach .a? as $k (0; ($k | .b?) | select(true); try ($k | .b?)))",
    "(foreach .a? as $k (0; ($k | .b?) | def f: 1; f; try ($k | .b?)))",
    "(foreach .a? as $k (0; reduce 1 as $q (.; .+1); try ($k | .b?)))",
    "(foreach .a? as $k (0; first(.+1); try ($k | .b?)))",
    "(foreach .a? as $k (null; select(true); try ($k | .b?)))",
    "(foreach .a? as $k (.; select(true); try ($k | .b?)))",
    "(foreach .a? as $k (null; ((now // sort) | ($k | .c?) // (first(($k | length)) | (1, 2))); try ($k | .b?)))",
    "(foreach .a? as $k (0; (try ((length | .)) catch ([($k | .b?)]) | if first(0) then (($k | .b?), true) else sort end); try ($k | .b?)))",
    # (#4252) an UPDATE output that navigated and then computed leaves jq's register on the node it
    # navigated to, which a `null`/`true`/`false` `$k` equals by kind; an object `$k` never does.
    "(foreach .a? as $k (0; ($k | .b?) | def f: 1; f; $k))",
    "(foreach .a? as $k (0; ($k | .[0]?) | def f: 1; f; try ($k | .[0]?)))",
    "(foreach .a? as $k (0; ($k | .b?.c?) | def f: 1; f; try ($k | .b?)))",
    "(foreach .a? as $k (null; ($k | .b?) | def f: 1; f; try ($k | .b?)))",
    "(foreach .a? as $k (0; (($k | .b?), now) | floor; try ($k | .b?)))",
    "(foreach .a? as $k (0; ($k | .b?) | def f: 1; f; try ($k | .b?) | .c?))",
    "(foreach .a? as $k (0; ($k | .[0:1]?) | def f: 1; f; try ($k | .[0:1]?)))",
    # (#3899) a destructuring `?//` bind whose body mixes a navigating and a
    # by-value part. A failed destructure is restored by the fork, so the bare
    # `$var` alternative that runs leaves jq's register where the stage entered
    # and each branch of the body states it itself; a destructure that succeeds
    # moves it. The by-value `true`/`false`/`null` body is #3859's.
    "(. as [$q] ?// $z | (.a?, true))",
    "(. as [$q] ?// $z | (true, .a?))",
    "(. as [$q] ?// $z | (.a?, false))",
    "(. as [$q] ?// $z | (.a?, null))",
    "(. as [$q] ?// $z | (.a?, 1))",
    "(. as [$q] ?// $z | (.[0]?, true))",
    "(. as [$q] ?// $z | (., true))",
    "(. as [$q] ?// $z | (.a, true))",
    "(. as {a:$q} ?// $z | (.a?, true))",
    "(. as {a:$q} ?// $z | (.a, null))",
    "(. as [$q] ?// $z | (.a? // true))",
    "(. as [$q] ?// $z | (.a // true))",
    "(. as [$q] ?// $z | if .a? then .a else true end)",
    "(. as [$q] ?// $z | if true then true else .a end)",
    "(. as [$q] ?// $z | try (.a, true) catch 1)",
    "(. as [$q] ?// $z | try (.a?, true) catch .a)",
    "(. as [$q] ?// $z | ((.a?, true), false))",
    "(. as [$q] ?// $z | (.a? | true))",
    "(. as [$q] ?// $z | ((.a?, true) | .b?))",
    "(. as [$q] ?// {a:$z} ?// $y | (.a?, true))",
    "(. as [$q] ?// $z | (. as [$r] ?// $w | (.a?, true)))",
    "(. as [$q] ?// $z | (.a?, $z))",
    "(. as [$q] ?// $z | (.a?, $q))",
    "(. as [$q] ?// $z | (empty, true))",
    "(. as [$q] ?// $z | (.[]?, true))",
    "(. as [$q] ?// $z | ({a:1}, true))",
    # (#4114) `isempty(g)` is `first((g | false), true)`: the `first`'s break is the
    # error a `?//` inside `g` swallows and retries past, so each retried
    # alternative answers `false` again and a retry that comes up dry leaves `g`
    # running to its end, which fires the trailing `true`. The by-value
    # `true`/`false` body is the case where the `false` is, or is not, identical
    # to the register the answer is checked against. Under `and`/`or` on an untracked
    # entry the identity `true` rides the frame's register, which these operands pin.
    "isempty(. as $q ?// $z | if $q then true else empty end)",
    "isempty(. as $q ?// $z | if $q then empty else false end)",
    "isempty(. as $q ?// $z | empty)",
    "isempty(. as $q ?// $z | $q)",
    "isempty(. as [$q] ?// $z | if $q then true else empty end)",
    "isempty(. as [$q] ?// $z | (.a?, true))",
    "isempty(. as [$q] ?// $z | (empty, true))",
    "isempty(. as {a:$q} ?// $z | if $q then true else empty end)",
    "isempty(. as [$q] ?// {a:$z} ?// $y | if $z then true else empty end)",
    "isempty(. as $q ?// $z | (if $q then true else empty end, .))",
    "isempty(. as $q ?// $z | (if $q then true else empty end | .a?))",
    "isempty(first(. as $q ?// $z | if $q then true else empty end))",
    "isempty(limit(2; . as $q ?// $z | if $q then true else empty end))",
    "isempty(isempty(. as $q ?// $z | if $q then true else empty end))",
    # (#4354) a fold whose UPDATE is such a call: the arm states its answers and its identity
    # element itself, so the fold body keeps the register on its frame (`fans_out`). The
    # operands below put the `?//` in every position that exemption must not widen: a navigating
    # or loop-variable-reading body, a comma beside it, and `reduce` as well as `foreach`.
    "(foreach .[]? as $k (.; isempty(. as $q ?// $z | if $q then true else empty end); .))",
    "(foreach .[]? as $k (.; isempty(. as $q ?// $z | if $q then true else empty end) or (.a)?; .))",
    "(foreach .[]? as $k (.; isempty(. as $q ?// $z | if $q then true else empty end) or true; .b?))",
    "(foreach .[]? as $k (.; any(. as $q ?// $z | if $q then true else empty end; .); .))",
    "(foreach .[]? as $k (.; all(. as $q ?// $z | if $q then false else empty end; .) and (.a)?; .))",
    "(foreach .[]? as $k (.; isempty(. as [$q] ?// $z | ($k, .a?)); $k))",
    "(foreach .[]? as $k (.; isempty(. as $q ?// $z | ($q, $k)); .))",
    "(foreach .[]? as $k (.; isempty((. as $q ?// $z | .a), .b); .))",
    "(foreach .a? as $k (.; isempty(. as {a:$q} ?// $z | $q.b?); .))",
    "(foreach (1,2) as $k (.; isempty(. as $q ?// $z | if $q then true else empty end); $k))",
    "(reduce .[]? as $k (.; isempty(. as $q ?// $z | if $q then true else empty end)))",
    # `any`/`all` stay out of the exemption: a bind in their `cond` runs a live stage against the
    # carried register (a debug-assertion panic when they were exempted).
    "(foreach .[]? as $k (.; all(.; . as [$q] ?// $z | $z); .))",
    "(foreach .[]? as $k (.; any(.; . as [$q] ?// $z | $z); .b?))",
    "(foreach .[]? as $k (.; all(.; . as {a:$q} ?// $z | $q); .))",
    "(foreach .[]? as $k (.; any(. as [$q] ?// $z | .a; .); .))",
    "(reduce .[]? as $k (.; isempty(. as $q ?// $z | $k) or (.a)?))",
    # (#3826) a verdict stage behind a zero-arity `def` call, whose body the resolver resolves in
    # place (so its leaf states the register), and behind a `reduce`, which backtracks to INIT's
    # register whatever its source and UPDATE navigate. Each is a bare pipe stage under
    # `var-rebind` and the `untracked-*` contexts, an UPDATE under the `reduce-*` and `foreach-*`
    # contexts, and a comma sibling, and a decided element (`[true]`, `{"a":true}`) is the row
    # that must keep refusing because the verdict moved the register.
    "(def f: any; f)",
    "(def f: all; f)",
    "(def f: any(.a); f)",
    "(def f: any(.[]?; .a); f)",
    "(def f: isempty(.[]?); f)",
    "(def f: isempty(empty); f)",
    "(def f: any(empty; .); f)",
    "(def f: (any, any); f)",
    "(def f: (any, .a); f)",
    "(def f: first(any); f)",
    "(def f: any; def g: f; g)",
    "(def f: any; def g: (f, f); g)",
    "(def f: any; def g: 1; f | g)",
    "(def f: any; def f: .a; f)",
    "(def f(g): any; f(.a))",
    "(def f: f; 1)",
    "(reduce 1 as $i (.; any))",
    "(reduce 1 as $i (.; all))",
    "(reduce (1,2) as $i (.; any))",
    "(reduce empty as $i (.; any))",
    "(reduce .[]? as $i (.; any))",
    "(reduce .[]? as $i (.; all(.a)))",
    "(reduce .a? as $i (.; any))",
    "(reduce 1 as $i (.; isempty(.[]?)))",
    "(reduce 1 as $i (.; isempty(empty)))",
    "(reduce 1 as $i (.; any(.[]?; .a)))",
    "(reduce 1 as $i (.; any(empty; .)))",
    "(reduce .[]? as $i (.; isempty(empty)))",
    "(reduce .[]? as $i (.; isempty(.[]?)))",
    "(reduce .[]? as $i (.; isempty(.a?)))",
    "(reduce .[]? as $i (.; try .a))",
    "(reduce .[]? as $i (.; try .a catch 1))",
    "(reduce .[]? as $i (.; try .a catch false))",
    "(reduce .[]? as $i (.; try .[]? catch .))",
    "(reduce .[]? as $i (.; (.a)?))",
    "(reduce .[]? as $i (.; try isempty(.[]?) catch 1))",
    "(reduce .[]? as $i (.; try (.a | not) catch error))",
    "(reduce .[]? as $i (.; isempty(try .a)))",
    "(reduce .[]? as $i (.; ((.k)?, .)))",
    "(reduce .[]? as $i (.; ((try .k), .)))",
    "(reduce .[]? as $i (.; (., try .a)))",
    "(reduce .[]? as $i (.; (try .a catch 1, .)))",
    "(reduce .[]? as $i (.; (., try isempty(.a) catch 1)))",
    "(reduce .[]? as $i (.; (empty, try isempty(.a) catch 1)))",
    "(reduce .[]? as $i (.; (try .a catch 1, try .b catch 2)))",
    "(reduce .[]? as $i (.; (., (.a)?)))",
    "(reduce .[]? as $i (.; first((isempty(.a))?, 1, .a?)))",
    "(reduce .[]? as $i (.; .))",
    "(reduce .a? as $i (.; .))",
    "(reduce .[] as $i (.; .))",
    "(reduce 1 as $i (.a?; .))",
    "(reduce .[]? as $i (empty; .))",
    "(reduce .[]? as $i (first(.); .))",
    "(reduce .[]? as $i ((.)?; .))",
    "(reduce .[]? as $i (.; isempty(.[])))",
    "(reduce .[]? as $i (.; isempty(select(false), .a)))",
    "(reduce .[]? as $i (.; isempty(1, .a)))",
    "(reduce .[]? as $i (.; isempty(.)))",
    "(reduce .a? as $i (.; isempty(.[]?)))",
    "(reduce .a? as $i (.; any(empty; .)))",
    "(reduce 1 as $i (.a; any))",
    "(reduce 1 as $i (.; try any))",
    "(def f: reduce 1 as $i (.; any); f)",
    "(reduce 1 as $i (.; def f: any; f))",
    "(foreach 1 as $i (.; any; .))",
    "(foreach .[]? as $i (.; any; .))",
    "(foreach .[]? as $i (.; isempty(.[]?); .))",
    "(foreach .[]? as $i (.; try .a catch 1; .))",
    "(foreach 1 as $i (.; isempty(empty); .))",
    # (#4488) a `?//` chain under a fold. The fold's own UPDATE refuses a navigation of a value
    # that is not the register (a literal INIT, jq's own path error), which sends the chain to its
    # next alternative with the state `null`; and a chain that *is* the UPDATE falls back to its
    # bare `$x` with the register where the fold step left it (the failed alternatives were
    # backtracked). The contrasts: a body or INIT that navigates, a source that navigates, and a
    # `reduce`/`foreach` UPDATE whose chain destructures.
    "(foreach . as [$a] ?// $a ({\"a\":5}; .a; .))",
    "(foreach . as [$a] ?// $a ({\"a\":5}; .a; .a))",
    "(foreach .a as [$a] ?// $a ({\"a\":5}; .a; .))",
    "(foreach . as [$a] ?// $a (1; .a; .))",
    "(foreach . as [$a] ?// $a (.; .b; .))",
    "(foreach . as {b:$b} ?// $a ({\"a\":5}; .a; .))",
    "(foreach . as [$a] ?// [$a] ?// $a ({\"a\":5}; .a; .))",
    "(reduce . as [$a] ?// $a ({\"a\":5}; .a))",
    "(reduce . as [$a] ?// $a (1; .a))",
    "(reduce .a as [$a] ?// $a ({\"a\":5}; .a))",
    "try (foreach . as [$a] ?// $a ({\"a\":5}; .a; .)) catch 7",
    "try (foreach . as [$a] ?// $a ({\"a\":5}; .a; .a)) catch 7",
    "try (reduce . as [$a] ?// $a ({\"a\":5}; .a)) catch 7",
    "(. as [$a] ?// $a | 0)",
    "(. as [$a] ?// $a | $a)",
    "(. as {a:$a} ?// $a | 0)",
    "(. as [$a] ?// [$a] ?// $a | 0)",
    "(. as $a ?// [$a] | 0)",
    "(. as [$a] ?// $a | .a)",
    "(. as [$a] ?// $a | (0, .a))",
    "(.a as [$a] ?// $a | 0)",
    "(try (. as [$a] ?// $a | 0) catch 5)",
    "(reduce . as [$a] ?// $a (0; .))",
    "(reduce . as [$a] ?// $a (0; 1))",
    "(reduce (1, 2) as [$a] ?// $a (0; 1))",
    "(reduce .a? as [$a] ?// $a (0; 1))",
    "(reduce . as [$a] ?// $a (.a?; 1))",
    "(reduce . as [$a] ?// $a (0; .a?))",
    "(foreach . as [$a] ?// $a (0; 1; 2))",
    # (#4514) `limit(1; ., ..)` is its comma's first leaf, as `first(., ..)` is: the register, so a
    # destructure of it is the register's own. The contrasts: a first leaf that is not the register,
    # a count past one (the second leaf is not), and a count of zero.
    "(foreach (limit(1; ., 1) as {a:$a} | .) as $k (.; .; .))",
    "(foreach (limit(1; ., 1) as [$a] | .) as $k (.; .; .))",
    "(foreach (limit(1; ., 1) as $b | $b as {a:$a} | .) as $k (.; .; .))",
    "(foreach (limit(1; ., 1) as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(. as $x | foreach (limit(1; $x, 1) as {a:$a} | .) as $k (.; .; .))",
    "(foreach (limit(1; 1, .) as {a:$a} | .) as $k (.; .; .))",
    "(foreach (limit(2; ., 1) as {a:$a} | .) as $k (.; .; .))",
    "(foreach (limit(0; ., 1) as {a:$a} | .) as $k (.; .; .))",
    "(foreach (limit(0.5; ., 1) as {a:$a} | .) as $k (.; .; .))",
    "(foreach (limit(1; (., 1), 2) as {a:$a} | .) as $k (.; .; .))",
    "(reduce (limit(1; ., 1) as {a:$a} | .) as $k (.; .))",
    "(reduce (limit(1; ., 1) as {a:$a} | $a) as $k (.; .))",
    "(foreach (def f($x): $x as {a:$a} | .; f(limit(1; ., 1))) as $k (.; .; .))",
    "(foreach (def f: limit(1; ., 1); f as {a:$a} | .) as $k (.; .; .))",
    "(reduce (label $o | .) as $z (.; label $q | (. as {a:$a} | .)))",
    # (#4567) a `label` around a `reduce` UPDATE that destructures the register hands the body's
    # outputs on, so it is vetted as the body is. The contrasts: a `label` around a destructure that
    # goes on to navigate, two labels, a `label` around a nested fold, a `break` in the body, a
    # `label` around a comma, and a `foreach`.
    "(reduce (label $o | .) as $z (.; label $q | label $p | (. as {a:$a} | .)))",
    "(reduce (label $o | .) as $z (.; label $q | (. as {a:$a} | .a)))",
    "(reduce (label $o | .) as $z (.; label $q | (. as {a:$a} | ., break $q)))",
    "(reduce (label $o | .) as $z (.; label $q | ((. as {a:$a} | .), .)))",
    "(reduce (label $o | .) as $z (.; label $q | (. as {a:$a} | break $q)))",
    "(reduce (label $o | .) as $z (.; label $q | reduce (. as {a:$a} | .) as $j (.; .)))",
    "(reduce (label $o | .) as $z (.; label $q | foreach (. as {a:$a} | .) as $j (.; .; .)))",
    "(reduce (label $o | .) as $z (.; label $q | (. as {a:$a} | .) | (. as {b:$b} | .)))",
    "(reduce 1 as $z (.; label $q | (. as {a:$a} | .)))",
    "(reduce .[]? as $z (.; label $q | (. as {a:$a} | .)))",
    "(reduce (label $o | .) as {a:$z} (.; label $q | (. as {a:$a} | .)))",
    "(foreach (label $o | .) as $z (.; label $q | (. as {a:$a} | .); .))",
    "(reduce (label $o | .) as $z (.; label $q | (. as {a:$a} | .)) | .a)",
    "(reduce (label $o | .) as $z (.; . as {a:$a} | .))",
    "(foreach (label $o | .) as $z (.; . as {a:$a} | .; .))",
    # (#4514) a source whose FIRST output is the register and whose later outputs need not be: jq
    # runs the first output first, and its destructure moves the register before the second exists.
    # A comma as a call argument bound to a `$` parameter, a `limit` with a count past one, and
    # the contrasts: the register on the second output only, a first output that is not the
    # register, and a body that does not navigate. The last operand (a call with no arguments
    # returning such a source) is the open contrast: a routed `def f: ., {a:1}` refuses a negated
    # operand jq answers (#4452), so the walk does not read a call's body for its first output.
    "(foreach (def f($x): $x as {a:$a} | .; f((., 1))) as $k (.; .; .))",
    "(foreach (def f($x): $x as {a:$a} | .; f((1, .))) as $k (.; .; .))",
    "(foreach (def f($x): $x as {a:$a} | .; f(((., 1), 2))) as $k (.; .; .))",
    "(foreach (def f($x): $x as {a:$a} | .; f(limit(2; ., 1))) as $k (.; .; .))",
    "(foreach (def f($x): $x as {a:$a} | $a; f((., 1))) as $k (.; .; .))",
    "(foreach (def f($x): $x as [$a] | .; f((., 1))) as $k (.; .; .))",
    "(foreach (def f($x): $x | .; f((., 1))) as $k (.; .; .))",
    "(foreach (limit(2; ., 1) as {a:$a} | $a) as $k (.; .; .))",
    "(foreach (limit(2; ., 1) as [$a] | .) as $k (.; .; .))",
    "(foreach (limit(2; (., 1), 2) as {a:$a} | .) as $k (.; .; .))",
    "(foreach (limit(2; 1, .) as {a:$a} | .) as $k (.; .; .))",
    "(foreach (limit(2; ., 1) as $b | $b as {a:$a} | .) as $k (.; .; .))",
    "(foreach (limit(2; ., 1) as {a:$a} ?// $a | .) as $k (.; .; .))",
    "(. as $x | foreach (limit(2; $x, 1) as {a:$a} | .) as $k (.; .; .))",
    "(reduce (limit(2; ., 1) as {a:$a} | .) as $k (.; .))",
    "(foreach (def f: limit(2; ., 1); f as {a:$a} | .) as $k (.; .; .))",
    # (#4565) a call with no arguments whose body is such a source: a comma body is placed per leaf
    # (also behind a call to a call), the others are routed and refused.
    "(foreach (def f: (., 1); f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: (., {a:1}); def g: f; g as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: first((., 1)); f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: (., 1) | .; f as {a:$a} | .) as $k (.; .; .))",
    "(foreach (def f: (1, .); f as {a:$a} | .) as $k (.; .; .))",
    "(reduce (def f: (., 1); f as {a:$a} | .) as $k (.; .))",
    # The wrappers that hand the first output on: `nth(0; ..)`, a pipe with `.`, `try`, and the contrasts
    # that do not (`nth(1; ..)`, `limit(0; ..)`).
    "(foreach (nth(0; ., 1) as {a:$a} | $a) as $k (.; .; .))",
    "(foreach (((., 1) | .) as {a:$a} | $a) as $k (.; .; .))",
    "(foreach ((. | (., 1)) as {a:$a} | $a) as $k (.; .; .))",
    "(foreach (limit(2; (., 1) | .) as {a:$a} | $a) as $k (.; .; .))",
    "(foreach ((try (., 1)) as {a:$a} | $a) as $k (.; .; .))",
    "(foreach (nth(1; ., 1) as {a:$a} | $a) as $k (.; .; .))",
    "(foreach (limit(0; ., 1) as {a:$a} | $a) as $k (.; .; .))",
    "(foreach (def f($x): $x as {a:$a} | $a; f(limit(2; ., 1) | .)) as $k (.; .; .))",
]

# The other side of a two-operand shape. Chosen so that, against the inputs
# below, `R` sometimes navigates off a register `L` moved and sometimes
# lands on a value identical to it.
COMPANIONS = [
    ".b?",
    ".[0]",
    "true",
    ".a",
    # (#3456) a right operand wrapped in `?` is pruned, not refused, when its
    # first step fails on an untracked input, which on a lost register is a
    # guess that must not be silent; and a generator whose inner `?` jq catches
    # locally before it goes on to the children.
    "(.a)?",
    "(.a | .b)?",
    "(.. | .a?)",
]

INPUTS = [
    "null",
    "true",
    "[true]",
    # (#4507) an element the register can sit on that is `null`: a `null` bound by a failed
    # alternative is `jv_identical` to it, so a `reduce` over `.[]?` answers where `[true]` raises
    "[null]",
    "[]",
    "[[]]",
    "[[1],[1]]",
    '{"a":true}',
    '{"a":1,"b":2}',
    '{"a":[true]}',
    '{"a":false,"b":null}',
    # an object with the key `a` below the root: a descent (`..`) reaches a
    # value `.a` can navigate that the root is not
    '[{"a":1}]',
    # (#3360) entries `from_entries` accepts
    '[{"key":"a","value":1}]',
]

# Contexts wrap a shape `X` in a path-consuming expression. `X` is spliced in
# as-is; the templates that need it grouped (`|=`, `=`, `try`, `?`, `//`, `as`
# bodies) parenthesise it themselves. Round-2 categories are tagged.
CONTEXTS = [
    ("path", "path({X})"),
    ("collect-path", "[path({X})]"),
    ("del", "del({X})"),
    ("update", "({X}) |= 9"),
    ("assign", "({X}) = 9"),
    ("try-del", "del(try ({X}))"),
    ("try-catch", "del(try ({X}) catch .)"),  # round 2: catchable refusal
    ("try-catch-update", "(try ({X}) catch .) |= 9"),  # round 2
    ("optional", "del(({X})?)"),  # round 2: wrapper drops register
    ("first-wrap", "del(first({X}))"),  # round 2: wrapper
    ("alt-wrap", "del(({X}) // .a)"),  # round 2: wrapper
    ("var-rebind", "del(. as $x | ({X}) | $x)"),
    # (#3653) the stage is entered on an *untracked* value (a literal ran first),
    # so the register is carried by the pipe and only a stage the resolver
    # knows leaves it in place passes it on. `var-rebind` enters on the trackable
    # root, where a pass-through stage never needed the register restated, so it
    # cannot see a stage that drops it.
    ("untracked-path-scalar", "path(. as $x | 1 | ({X}) | $x)"),
    ("untracked-path-object", "path(. as $x | {a:{b:1}} | ({X}) | $x)"),
    ("untracked-del-scalar", "del(. as $x | 1 | ({X}) | $x.a?)"),
    ("untracked-del-object", "del(. as $x | {a:{b:1}} | ({X}) | $x.a?)"),
    # (#4615) the operand inside another fold's body, entered on an untracked value: `.` is the
    # accumulator there, which the enclosing source may have moved off the register.
    ("untracked-foreach-update", "path(. as $x | 1 | (foreach (1,2) as $j (.; ({X}); .)) | $x)"),
    ("untracked-foreach-update-try", "path(. as $x | 1 | (foreach (1,2) as $j (.; try ({X}); .)) | $x)"),
    ("untracked-foreach-extract", "path(. as $x | 1 | (foreach (1,2) as $j (.; .; ({X}))) | $x)"),
    ("untracked-del-foreach-update", "del(. as $x | 1 | (foreach (1,2) as $j (.; ({X}); .)) | $x.a?)"),
    ("untracked-reduce-update", "path(. as $x | 1 | (reduce (1,2) as $j (.; ({X}))) | $x)"),
    ("var-rebind-nav", "del(.a? as $y | try ({X}) | try ($y | .b))"),  # round 2
    ("nested", "del(.a? | ({X}))"),
    ("alt-pattern", "del(. as [$q] ?// $q | ({X}))"),  # round 2: ?// retry
    ("alt-pattern-pipe", "del(. as [$q] ?// $q | ({X}) | .c)"),  # round 2: stale downstream
    ("reduce-source", "del(reduce .[]? as $k (.; {X}))"),  # round 2: SOURCE navigates
    # (#4507) a SOURCE that navigates and then computes yields a node the register is not on
    ("reduce-source-null", "del(reduce (.[]? | null) as $k (.; {X}))"),
    ("reduce-source-tostring", "del(reduce (.[]? | tostring) as $k (.; {X}))"),
    ("foreach-source", "del(foreach .[]? as $k (.; {X}; .b?))"),  # round 2
    # (#3734) a fold whose SOURCE does not navigate: the whole-array-slice rule
    # (#3504) only applies there, and every context above either navigates in
    # the source or is not a fold, so none of them reached it.
    ("reduce-update-path", "path(reduce 1 as $k (.; {X}))"),
    ("reduce-update-del", "del(reduce 1 as $k (.; {X}))"),
    # ...and the SOURCE shapes that decide whether the rule may apply, which the
    # two above (a literal source) never reach: a computed key (`.[1+1]` moves
    # jq's register though no `Field`/`Iterate` is spelled), a bare `first`
    # (`.[0]` to jq, opaque to the fold driver), and a destructuring pattern
    # (it navigates). The review of #3734 found the first two accepting where jq
    # refuses, with and without a `?`: a context list that fixes the source
    # cannot see the gate that depends on it.
    ("reduce-update-computed-key", "path(reduce .[1+1] as $k (.; {X}))"),
    ("reduce-update-first-source", "path(reduce first as $k (.; {X}))"),
    ("reduce-update-pattern", "path(reduce [1] as [$q] (.; {X}))"),
    # (#3738) the fold's UPDATE under a `try`, with a source that navigates (so jq's
    # register has moved) and EXTRACT either the loop variable or `.`; the plain
    # row is the control without the `try`.
    ("foreach-update-try-var", "path(foreach .a? as $k (0; try ({X}); $k))"),
    ("foreach-update-try-dot", "path(foreach .a? as $k (0; try ({X}); .))"),
    ("foreach-update-try-del", "del(foreach .a? as $k (0; try ({X}); .))"),
    ("foreach-update-try-assign", "(foreach .a? as $k (0; try ({X}); .)) = 9"),
    ("foreach-update-plain", "path(foreach .a? as $k (0; {X}; .))"),
    # ...and the same body as the EXTRACT, and as a `reduce` UPDATE (every fold body
    # goes through `FoldRegister::resolve_sink`).
    ("foreach-extract-try-path", "path(foreach .a? as $k (0; .; try ({X})))"),
    ("foreach-extract-try-del", "del(foreach .a? as $k (0; .; try ({X})))"),
    ("foreach-extract-try-assign", "(foreach .a? as $k (0; .; try ({X}))) = 9"),
    # (#4488) the same body as UPDATE with the loop variable as EXTRACT, read back by `path`
    # (the register EXTRACT's `$k` is checked against) and written through.
    ("foreach-update-var-path", "path(foreach .a? as $k (0; {X}; $k))"),
    ("foreach-update-var-del", "del(foreach .a? as $k (0; {X}; $k))"),
    ("foreach-update-var-assign", "(foreach .a? as $k (0; {X}; $k)) = 9"),
    # (#4550) a container INIT: the accumulator is a computed container, which jq's `path_intact`
    # refuses at a pattern's first step whatever the register is, so `?//` retries it.
    ("foreach-update-try-var-arr", "path(foreach .a? as $k ([]; try ({X}); $k))"),
    ("foreach-update-try-dot-arr", "path(foreach .a? as $k ([]; try ({X}); .))"),
    ("foreach-update-try-del-obj", "del(foreach .a? as $k ({}; try ({X}); .))"),
    ("foreach-update-try-assign-arr", "(foreach .a? as $k ([1]; try ({X}); .)) = 9"),
    # ...a two-element source (step 2 starts from step 1's output) and INIT `.`, which IS the register
    # on the first step, so a retry there accepts what jq answers.
    ("foreach-update-try-var-arr2", "path(foreach (1,2) as $k ([]; try ({X}); $k))"),
    ("foreach-update-try-var-init-dot", "path(foreach .a? as $k (.; try ({X}); $k))"),
    ("reduce-update-try-path-arr2", "path(reduce (1,2) as $k ([]; try ({X})))"),
    ("reduce-update-try-path-init-dot", "path(reduce .a? as $k (.; try ({X})))"),
    ("reduce-update-try-path-arr", "path(reduce .a? as $k ([]; try ({X})))"),
    ("reduce-update-try-del-obj", "del(reduce .a? as $k ({}; try ({X})))"),
    ("reduce-update-try-path", "path(reduce .a? as $k (0; try ({X})))"),
    ("reduce-update-try-del", "del(reduce .a? as $k (0; try ({X})))"),
    # (#4440) a `foreach` whose SOURCE and INIT cannot move jq's register, so the first step runs
    # UPDATE (or, below, EXTRACT) with `.` on it, and a second step that does not. Every `foreach`
    # context above navigates in its source or starts from `0`, so none of them reached it.
    ("foreach-update-literal-path", "path(foreach (1,2) as $k (.; {X}; .))"),
    ("foreach-update-literal-del", "del(foreach (1,2) as $k (.; {X}; .))"),
    ("foreach-update-literal-assign", "(foreach (1,2) as $k (.; {X}; .)) = 9"),
    ("foreach-extract-literal-path", "path(foreach (1,2) as $k (.; .; {X}))"),
    ("foreach-extract-literal-del", "del(foreach (1,2) as $k (.; .; {X}))"),
    ("foreach-extract-literal-assign", "(foreach (1,2) as $k (.; .; {X})) = 9"),
    # (#4595) #4550's container INIT, with the `try` in EXTRACT instead of UPDATE: the state UPDATE stored
    # is a computed container, which `path_intact` refuses at a pattern's first step whatever the register
    # is, so `?//` retries it. INIT `.` and a `$r` bound to the register are on it, and retrying there
    # accepts what jq answers.
    ("foreach-extract-try-path-arr", "path(foreach .a? as $k ([]; .; try ({X})))"),
    ("foreach-extract-try-del-obj", "del(foreach .a? as $k ({}; .; try ({X})))"),
    ("foreach-extract-try-assign-arr", "(foreach .a? as $k ([1]; .; try ({X}))) = 9"),
    ("foreach-extract-try-path-arr2", "path(foreach (1,2) as $k ([]; .; try ({X})))"),
    ("foreach-extract-try-path-init-dot", "path(foreach .a? as $k (.; .; try ({X})))"),
    ("foreach-extract-try-path-r", "path(. as $r | foreach .a? as $k ([]; $r; try ({X})))"),
    # (#4610) a source that navigates the register onto the element, with INIT `.`: the root is no node of
    # the register there (only a `null`/boolean state is still identical to it by kind), so a pattern step on
    # the state is refused by `path_intact` and `?//` retries it.
    ("foreach-update-try-var-nav-init-dot", "path(foreach .[]? as $k (.; try ({X}); $k))"),
    ("foreach-update-try-dot-nav-init-dot", "path(foreach .[]? as $k (.; try ({X}); .))"),
    ("foreach-update-try-del-nav-init-dot", "del(foreach .[]? as $k (.; try ({X}); $k))"),
    ("foreach-update-try-assign-nav-init-dot", "(foreach .[]? as $k (.; try ({X}); $k)) = 9"),
    ("foreach-extract-try-path-nav-init-dot", "path(foreach .[]? as $k (.; .; try ({X})))"),
    ("foreach-update-try-var-nav-key-init-dot", "path(foreach .a? as $k (.; try ({X}); $k))"),
    # (#4611) a source that navigates the register onto the element, an UPDATE that is the operand
    # itself (no `try`), and an EXTRACT `$k` that IS that register: a builtin-call UPDATE
    # (`isempty(g)`, `any(g; c)`) left the register where it entered, so `$k` names a path.
    ("foreach-update-var-nav-path", "path(foreach .[]? as $k (0; {X}; $k))"),
    ("foreach-update-var-nav-init-dot", "path(foreach .[]? as $k (.; {X}; $k))"),
    ("foreach-update-var-nav-del", "del(foreach .[]? as $k (0; {X}; $k))"),
    ("foreach-update-var-nav-assign", "(foreach .[]? as $k (0; {X}; $k)) = 9"),
    ("foreach-update-var-nav-key-path", "path(foreach .a? as $k (0; {X}; $k))"),
]

# Long chains are the O(N^3) row: a timing axis, not a correctness one.
CHAIN_LENGTHS = [16, 64, 256]
CHAIN_CONTEXTS = ["path", "del", "update"]


def shapes_for(operand, stage_only=False):
    """Every combinator shape an operand takes part in.

    `stage_only` keeps just the operand as a bare pipe stage (and negated), about
    1/15 of the rows: a quick run to repeat while iterating. It does **not**
    bound a change to a stage-level rule (what `resolve_seq_stage` carries across
    a stage: #3643, #3653). The right operand of an `and`/`or` is resolved as a
    one-stage pipe seeded at the register (`resolve_from_restored_input`), so
    those rules reach the `and`/`or` shapes too: #3653's seeded sample flipped
    108 `and`/`or` rows beside 109 bare-stage ones. Judge a change on the full
    shapes (or a seeded `--sample` over them).
    """
    # (#3361) the operand alone, as a bare pipe stage: `. as $x | OP | $x` is
    # what the `var-rebind` context makes of it.
    yield f"({operand})"
    yield f"-({operand})"
    if stage_only:
        return
    for c in COMPANIONS:
        yield f"{operand} and {c}"
        yield f"{c} and {operand}"
        yield f"{operand} or {c}"
        yield f"{c} or {operand}"


def build_rows(operands=None, stage_only=False, contexts=None):
    """The grid and the chain rows, separately: (label, input, program).

    The chain rows are the timing axis and are never sampled away. `operands`
    restricts the grid to a subset of OPERANDS (the `--operand` flag): a fast
    targeted run while iterating, never the number a gate is judged on. `contexts` likewise
    restricts the grid to the named CONTEXTS (the `--context` flag).
    """
    ctx = dict(CONTEXTS)
    rows = []
    chains = []
    for operand in OPERANDS if operands is None else operands:
        for shape in shapes_for(operand, stage_only):
            for cname, template in CONTEXTS:
                if contexts and cname not in contexts:
                    continue
                # An operand that uses `$x` or `$k` *free* only means something where the
                # context binds it; elsewhere jq and the build both fail to compile, a
                # trivial MATCH that carries no signal (3.4% of the grid, #3653; `$k` since
                # #3738). An operand that binds its own (`reduce .[]? as $k ...`) is valid in
                # every context and is never skipped: a substring test on `$k` alone dropped
                # 15,120 such rows.
                if any(
                    v in shape and f"as {v}" not in shape and v not in template
                    for v in ("$x", "$k")
                ):
                    continue
                program = template.replace("{X}", shape)
                for doc in INPUTS:
                    rows.append((f"{cname}|{shape}", doc, program))
    for n in CHAIN_LENGTHS:
        for cname in CHAIN_CONTEXTS:
            chain = " and ".join([".a"] * n)
            program = ctx[cname].replace("{X}", chain)
            for doc in ('{"a":true}', '{"a":false}', "[true]"):
                chains.append((f"chain{n}|{cname}", doc, program))
    return rows, chains


# ---------------------------------------------------------------------------
# Running and classifying
# ---------------------------------------------------------------------------


# What a run that outlived `--timeout` returns. A run killed at a cap returns a note in
# the stdout slot instead, which is how the serial re-run tells load from the row.
DEADLINE_RESULT = ("", None)
# A build's result on a row whose oracle never finished: it was not run.
SKIPPED_RESULT = ("<not run: jq did not finish>", None)


def rss_bytes(pid):
    """Resident size of `pid` in bytes; 0 once it is gone or cannot be read.

    /proc where there is one (no process to spawn); `ps` on macOS. A failure to read
    is 0, never an exception: this runs inside a worker, and one raised error would
    abort a multi-hour sweep for the sake of a cap that only guards against a hog.
    """
    try:
        with open(f"/proc/{pid}/statm") as fh:
            return int(fh.read().split()[1]) * os.sysconf("SC_PAGE_SIZE")
    except (OSError, ValueError, IndexError):
        pass
    try:
        out = subprocess.run(
            ["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True
        ).stdout.strip()
    except OSError:
        return 0
    return int(out) * 1024 if out.isdigit() else 0


def run_one(argv, doc, timeout):
    """(stdout, exit code); exit code is None on timeout or a blown output/memory cap.

    A run that never ends can cost the harness far more than its `--timeout` (#3893):

    * stdout is read through a reader that stops at OUTPUT_CAP bytes and kills the
      process. `subprocess.run(capture_output=True)` buffers without bound, so an
      operand that is infinite on some input (`recurse(.a)?` on `null`, where jq
      prints ever longer paths, ~135 MB in 3 s) grew the harness until `--timeout`,
      and across `--jobs` workers that is gigabytes.
    * the process's own resident size is polled while it runs and it is killed past
      RSS_CAP. The same operand under `[path(...)]` or `del(...)` prints nothing and
      grows *jq* instead (~30 GB in 4 s); macOS cannot set RLIMIT_AS, so this is the
      only cap there. A row that finishes inside the first poll never pays for one.

    A capped row is a timeout (jq's own is `ORACLE_TIMEOUT`, a build's is `TIMEOUT`),
    with a note in the stdout slot.
    """
    with subprocess.Popen(
        argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL
    ) as p:
        chunks = []
        state = {"size": 0, "capped": False}

        def drain():
            while True:
                chunk = p.stdout.read1(65536)
                if not chunk:
                    return
                state["size"] += len(chunk)
                if state["size"] > OUTPUT_CAP:
                    state["capped"] = True
                    p.kill()
                    return
                chunks.append(chunk)

        def feed():
            try:
                p.stdin.write(doc.encode())
                p.stdin.close()
            except (BrokenPipeError, ValueError):
                pass  # the process answered (or refused) before it read its input

        deadline = time.monotonic() + timeout
        reader = threading.Thread(target=drain, daemon=True)
        reader.start()
        # Not on this thread: a child that reads nothing, given more than a pipe holds,
        # would block the write before the deadline or the memory poll began.
        threading.Thread(target=feed, daemon=True).start()
        note = None
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                break
            try:
                p.wait(timeout=min(left, POLL_SECONDS))
                break
            except subprocess.TimeoutExpired:
                if rss_bytes(p.pid) > RSS_CAP:
                    note = f"<resident size exceeded {RSS_CAP} bytes>"
                    break
        if p.poll() is None:
            p.kill()
            p.wait()
            reader.join()
            return (note, None) if note else DEADLINE_RESULT
        reader.join()
        if state["capped"]:
            return (f"<output exceeded {OUTPUT_CAP} bytes>", None)
        return (b"".join(chunks).decode("utf-8", "replace"), p.returncode)


def classify(oracle, result):
    """Class of one build's result against jq's."""
    if oracle[1] is None:
        return "ORACLE_TIMEOUT"
    if result[1] is None:
        return "TIMEOUT"
    if result == oracle:
        return "MATCH"
    if oracle[1] != 0 and result[1] == 0:
        return "ACCEPT_WRONG"
    if oracle[1] == 0 and result[1] != 0:
        return "REFUSE_WRONG"
    # Both succeeded with different output, or both failed with different
    # stdout or exit codes. The stderr message is deliberately not compared:
    # it is not what this sweep tests, and it differs by design where the
    # resolver refuses with its by-value "with result" text.
    return "DIFF"


def evaluate(row, builds, timeout):
    label, doc, program = row
    oracle = run_one([ORACLE, "-c", program], doc, timeout)
    classes = {}
    outputs = {}
    for name, path in builds:
        # A row jq never finishes says nothing about any build (ORACLE_TIMEOUT whatever it
        # prints), and an infinite operand is exactly where a build can also run away.
        if oracle[1] is None:
            res = SKIPPED_RESULT
        else:
            res = run_one([path, "jq", "-c", program], doc, timeout)
        outputs[name] = res
        classes[name] = classify(oracle, res)
    return {
        "label": label,
        "input": doc,
        "program": program,
        "oracle": oracle,
        "outputs": outputs,
        "classes": classes,
    }


def is_regression(rec, base, cand):
    """Whether the candidate is worse than the directional base on this row."""
    b, c = rec["classes"][base], rec["classes"][cand]
    if "ORACLE_TIMEOUT" in (b, c):
        return False
    if c == b:
        # Same class is not "no change" when the class is wrong: a write that
        # moved from one wrong target to another is invisible to the label.
        return c in ("ACCEPT_WRONG", "DIFF") and rec["outputs"][base] != rec["outputs"][cand]
    if c == "TIMEOUT":
        return True
    if c == "ACCEPT_WRONG":
        return True
    if c == "DIFF" and b == "MATCH":
        return True
    if c == "REFUSE_WRONG" and b == "MATCH":
        # Safe direction, but a match lost is still a flip worth failing on;
        # the docstring says why.
        return True
    return False


def progress_line(done, total, elapsed):
    """`  done/total (elapsed s, rate rows/s, ETA)`: the rate is the run's average so far,
    which is what a full run is planned around (#4110)."""
    rate = done / elapsed if elapsed > 0 else 0.0
    left = (total - done) / rate if rate > 0 else 0.0
    return f"  {done}/{total} ({elapsed:.0f}s, {rate:.0f} rows/s, ETA {format_duration(left)})"


def format_duration(seconds):
    seconds = int(round(seconds))
    hours, rest = divmod(seconds, 3600)
    minutes, secs = divmod(rest, 60)
    if hours:
        return f"{hours}h{minutes:02d}m"
    if minutes:
        return f"{minutes}m{secs:02d}s"
    return f"{secs}s"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--candidate", help="build under test")
    ap.add_argument(
        "--base",
        action="append",
        default=[],
        metavar="LABEL=PATH",
        help="reference build; repeatable; the first is the directional base",
    )
    ap.add_argument("--sample", type=int, default=0, help="run a seeded sample of N rows")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--timeout", type=float, default=20.0)
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 4)
    ap.add_argument("--show", type=int, default=25, help="max rows printed per section")
    ap.add_argument(
        "--json",
        metavar="PATH",
        help="write every non-MATCH row as JSON lines (they stream to PATH.partial while running)",
    )
    ap.add_argument(
        "--operand",
        action="append",
        default=[],
        metavar="TEXT",
        help="restrict the grid to this operand (exact text from --list-axes); repeatable",
    )
    ap.add_argument(
        "--context",
        action="append",
        default=[],
        metavar="NAME",
        help="restrict the grid to this context (its name in CONTEXTS); repeatable. The chain "
        "rows are unaffected",
    )
    ap.add_argument(
        "--stage-only",
        action="store_true",
        help="only the operand as a bare pipe stage (and negated), about 1/15 of the "
        "rows: a quick iteration run, not a gate (an and/or operand is a one-stage "
        "pipe too)",
    )
    ap.add_argument("--list-axes", action="store_true", help="print the grid's size and exit")
    args = ap.parse_args()

    unknown = [o for o in args.operand if o not in OPERANDS]
    if unknown:
        ap.error(f"--operand {unknown[0]!r} is not an operand; see --list-axes")
    known_contexts = {name for name, _ in CONTEXTS}
    unknown = [c for c in args.context if c not in known_contexts]
    if unknown:
        ap.error(f"--context {unknown[0]!r} is not a context; see CONTEXTS")
    grid, chains = build_rows(args.operand or None, args.stage_only, args.context or None)
    if args.list_axes:
        print(f"operands={len(OPERANDS)} companions={len(COMPANIONS)} inputs={len(INPUTS)} "
              f"contexts={len(CONTEXTS)}")
        print(f"rows={len(grid) + len(chains)} ({len(grid)} grid + {len(chains)} chain)")
        for operand in OPERANDS:
            print(f"  operand: {operand}")
        return 0
    if not args.candidate:
        ap.error("--candidate is required")

    builds = []
    for spec in args.base:
        if "=" not in spec:
            ap.error(f"--base wants LABEL=PATH, got {spec!r}")
        label, path = spec.split("=", 1)
        if label == "candidate" or label in [n for n, _ in builds]:
            ap.error(f"--base label {label!r} is reserved or repeated")
        builds.append((label, path))
    builds.append(("candidate", args.candidate))
    for name, path in builds:
        if not os.access(path, os.X_OK):
            print(f"error: {name} build {path!r} is not executable", file=sys.stderr)
            return 2
    ver = subprocess.run([ORACLE, "--version"], capture_output=True, text=True).stdout.strip()
    if not ver.startswith(ORACLE_VERSION):
        print(f"error: oracle is {ver!r}, expected {ORACLE_VERSION}*", file=sys.stderr)
        return 2

    rows = grid
    if args.sample and args.sample < len(rows):
        random.Random(args.seed).shuffle(rows)
        rows = rows[: args.sample]
    rows = rows + chains  # the timing axis is never sampled away
    print(f"{len(rows)} rows x {len(builds)} builds + oracle; {args.jobs} jobs", file=sys.stderr)

    started = time.time()
    records = []
    # Non-MATCH rows stream to PATH.partial as they finish, so a run that is
    # killed (an hour into the grid, #3653) keeps what it found; the final file
    # is still written once the serial re-run of timed-out rows has settled.
    partial = open(args.json + ".partial", "w") if args.json else None
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for i, rec in enumerate(pool.map(lambda r: evaluate(r, builds, args.timeout), rows), 1):
            records.append(rec)
            if partial and any(c != "MATCH" for c in rec["classes"].values()):
                partial.write(json.dumps(rec) + "\n")
                partial.flush()
            if i % 2000 == 0:
                print(progress_line(i, len(rows), time.time() - started), file=sys.stderr)

    # A timeout under `--jobs` parallelism can be load, not the row. Re-run
    # each such row alone, once, and believe that answer. A run killed at the
    # output or memory cap carries a note and is not retried: it ends the same
    # way on a quiet box, and re-running an infinite operand serially is minutes
    # of nothing (#3893).
    retry = [i for i, rec in enumerate(records)
             if DEADLINE_RESULT in [rec["oracle"], *rec["outputs"].values()]]
    if retry:
        print(f"re-running {len(retry)} timed-out row(s) serially", file=sys.stderr)
        for i in retry:
            records[i] = evaluate(rows[i], builds, args.timeout)

    names = [n for n, _ in builds]
    print("\nper-build class totals (vs jq 1.7.1):")
    classes = ["MATCH", "ACCEPT_WRONG", "REFUSE_WRONG", "DIFF", "TIMEOUT", "ORACLE_TIMEOUT"]
    print(f"  {'build':<12}" + "".join(f"{c:>15}" for c in classes))
    for n in names:
        counts = {c: 0 for c in classes}
        for rec in records:
            counts[rec["classes"][n]] += 1
        print(f"  {n:<12}" + "".join(f"{counts[c]:>15}" for c in classes))

    base = names[0] if len(names) > 1 else None
    cand = "candidate"
    regressions = []
    if base:
        regressions = [r for r in records if is_regression(r, base, cand)]
        improvements = [
            r for r in records
            if r["classes"][cand] != r["classes"][base] and not is_regression(r, base, cand)
        ]
        print(f"\ncandidate vs {base}: {len(regressions)} regression(s), "
              f"{len(improvements)} other flip(s)")
        for title, group in (("REGRESSIONS (fail)", regressions), ("other flips", improvements)):
            if not group:
                continue
            print(f"\n{title}, first {min(len(group), args.show)} of {len(group)}:")
            for r in group[: args.show]:
                print(f"  [{r['label']}] input={r['input']}")
                print(f"    program: {r['program']}")
                print(f"    jq       : {r['oracle']}")
                print(f"    {base:<9}: {r['outputs'][base]}  {r['classes'][base]}")
                print(f"    candidate: {r['outputs'][cand]}  {r['classes'][cand]}")
    else:
        print("\n(no --base given: totals only, no directional check)")

    if args.json:
        with open(args.json, "w") as fh:
            for rec in records:
                if any(c != "MATCH" for c in rec["classes"].values()):
                    fh.write(json.dumps(rec) + "\n")
        partial.close()
        os.remove(partial.name)

    if regressions:
        print(f"\nFAIL: {len(regressions)} regression(s) vs {base}")
        return 1
    print("\nOK" if base else "\ndone")
    return 0


if __name__ == "__main__":
    sys.exit(main())
