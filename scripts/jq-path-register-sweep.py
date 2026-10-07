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

**Size.** The full grid is about 2,952,000 rows (`--list-axes` prints the exact count:
237 operands, each also swept as a bare pipe stage since #3361, across 37 contexts),
which takes hours on a loaded machine. Judge a change with
`--operand` over the operands it touches (83,187 rows for 14 of them took about
22 minutes at `--jobs 6` on a box at load 100) plus a seeded `--sample`, and run
the whole grid only when the change reaches every operand.

This is a verification tool, not a CI gate. The pinned `*_3456` rows in
`tests/jq_cli_tests.rs` are what CI enforces (one per round-2 category); run
this after touching `PathBranch::register`, `Frame::register_loss`, or any
path-mode producer/consumer of them in `src/jq/eval.rs`.

Usage:
  cargo build --release --features cli
  ./scripts/jq-path-register-sweep.py --candidate target/release/succinctly \\
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
    "try last(error) catch .",
    "try last(.a) catch input",
    "limit(1; try last(.a) catch .)",
    "limit(1; (last(.a), select(.)))",
    "limit(1; (.a | select(.)))",
    "nth(0; (.a | last(.a)))",
    "limit(1; first(.a))",
    "[numbers]",
    "[strings]",
    "[select(.)]",
    "[numbers, strings]",
    "[.[]? | numbers]",
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
    "nth(0; ..)",
    "(.. | .)",
    "(.a | ..)",
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
    ("var-rebind-nav", "del(.a? as $y | try ({X}) | try ($y | .b))"),  # round 2
    ("nested", "del(.a? | ({X}))"),
    ("alt-pattern", "del(. as [$q] ?// $q | ({X}))"),  # round 2: ?// retry
    ("alt-pattern-pipe", "del(. as [$q] ?// $q | ({X}) | .c)"),  # round 2: stale downstream
    ("reduce-source", "del(reduce .[]? as $k (.; {X}))"),  # round 2: SOURCE navigates
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
    ("reduce-update-try-path", "path(reduce .a? as $k (0; try ({X})))"),
    ("reduce-update-try-del", "del(reduce .a? as $k (0; try ({X})))"),
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


def build_rows(operands=None, stage_only=False):
    """The grid and the chain rows, separately: (label, input, program).

    The chain rows are the timing axis and are never sampled away. `operands`
    restricts the grid to a subset of OPERANDS (the `--operand` flag): a fast
    targeted run while iterating, never the number a gate is judged on.
    """
    ctx = dict(CONTEXTS)
    rows = []
    chains = []
    for operand in OPERANDS if operands is None else operands:
        for shape in shapes_for(operand, stage_only):
            for cname, template in CONTEXTS:
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
    grid, chains = build_rows(args.operand or None, args.stage_only)
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
                print(f"  {i}/{len(rows)} ({time.time() - started:.0f}s)", file=sys.stderr)

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
