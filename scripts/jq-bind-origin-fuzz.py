#!/usr/bin/env python3
"""Randomised differential fuzz for `$var` node identity in path position
(#2042, #1573) -- the companion of `scripts/jq-bind-origin-oracle-sweep.sh`.

Generates `path(...)`/`del(...)`/assignment programs that bind variables
from navigated positions and use them after navigation, literals,
passthroughs, register-moving computations, folds, nested pipes, nested
bindings and nested `path()` calls, over documents built so that equal
values at *different* nodes are common (siblings copied from each other,
rebuilt copies). Every shape is run through the pinned oracle and the
built succinctly binary and classified by direction:

    agree        same exit, same stdout
    fabricate    jq refuses, succinctly answers  -- what `del()`/`=` write through
    mismatch     both answer, differently (or both refuse with different
                 emitted prefixes)
    refuse-only  jq answers, succinctly refuses  -- safe, reported as a count
    refuse-early both refuse, and what succinctly emitted before refusing is
                 a strict prefix of what jq emitted: a refusal on an earlier
                 binding of a multi-output source (the same safe direction
                 as refuse-only, counted separately so it stays visible)

**The alphabet is part of the claim** (#2041): a pool that cannot emit the
shape a bug lives in proves nothing. `--self-test` prints the pools so a
shrink is visible.

#2649 widened the alphabet with *destructuring* binds, because
`SRC as PATTERN | BODY` moves the path register through every pattern step
and none of the #2042 shapes could emit one. `PATTERNS` adds, on top of the
plain `SRC as $v` bind every other pool assumes:

  * object patterns `{k:$v}` and nested ones `{k:{k2:$v}}` / `{k:{k:$v}}`;
  * array patterns `[$v]` and `[$v,$w]` -- jq runs array elements in
    *reverse* index order, so both the first and the second element are
    drawn as the variable the body uses;
  * multi-entry `{k:$v, k2:$w}` -- jq's second entry refuses unless the
    first step landed on a null/bool, so this is mostly a both-refuse shape
    and that is the point (the refusing half of the arm is a claim too);
  * the `{$k}` shorthand and `{$k:{k2:$v}}`, which jq compiles as *one*
    INDEX step with two binds;
  * `?//` chains of two alternatives, including a bare-variable alternative
    (`{a:$v} ?// $v`), whose retry rule is where a wrong answer would be a
    fabrication rather than a refusal.

The pattern's variable is then fed to the existing `USES` pool, so every
body shape (`$v`, `$v[0]`, `$v | .[]`, `$v.k`, folds, nested `path()`,
comma, recurse) and every wrapper (`path`, `del`, `=`, `|=`) already
applies to it -- the write shapes come for free from `program()`'s wrapper
draw. Sources are drawn from the same `SOURCES` pool as a plain bind, so a
pattern is exercised on `.`, on a prior binding `$x`, and on a navigation
`.k` (which jq refuses at the first step: the source is not the register).

#3036 added the ROUTES family, because every program above binds at the
head of a `path()`/`del()`/assignment and is answered by the generic
evaluator, whose funnels are where #2642's rebuilt-root check runs -- so
none of them could reach the routes on which the bind *and* the rebuild both
run inside `eval.rs`'s owned-value evaluator (a program mentioning
`input`/`inputs`/`input_line_number`, a fold's UPDATE, a `|=` right-hand
side, `with_entries`, a `catch` handler). A ROUTES program binds `. as $x`
outside any resolver, rebuilds (or passes through) the document, then
writes through `$x`, and wraps the whole thing in one of those routes; the
control routes (`first`, `[...]`, `label`, a `def`) and the bare twin are
drawn from the same pool so a route that changes the answer is visible next
to ones that must not. The stdin carries the document twice so `input` has
a second, equal-valued document to read.

#3049 adds a tracked-input array family. It collects a navigation from a
computed value and then discards the array, so an inner refusal cannot be
recovered by the terminal path check. The family also includes arrays jq
accepts and the still-deferred optional-navigation cases from #2764.

Usage:
    cargo build --release --features cli
    ./scripts/jq-bind-origin-fuzz.py [--bin PATH] [--jq PATH] [-n N] [--seed S] [--show K]
Exit 1 on any fabricate/mismatch.
"""
import argparse, json, random, subprocess, sys, copy

LEAVES = [1, 1, 2, "s", "s", None, True, False, {"b": 1}, {"b": 1}, [1], [{"b": 1}]]

def doc(rng):
    a = rng.choice(LEAVES)
    d = {"a": copy.deepcopy(a), "c": copy.deepcopy(a) if rng.random() < 0.7 else rng.choice(LEAVES),
         "d": rng.choice(LEAVES), "x": {"a": copy.deepcopy(a), "c": rng.choice(LEAVES)}}
    if rng.random() < 0.3:
        d["arr"] = [copy.deepcopy(a), copy.deepcopy(a), rng.choice(LEAVES)]
    return d

# Binding sources: (source, needs_prev) where `$p` is the previous binding.
SOURCES = [
    (".a", False), (".c", False), (".x.a", False), (".x", False), (".", False),
    ("(.a | .b?)", False), ("getpath([\"a\"])", False), ("first(.a)", False),
    ("([.a] | .[0])", False), ("(.a | tojson | fromjson)", False), ("(.a, .c)", False),
    (".arr[]?", False), (".arr[0]?", False), ("($p | .b?)", True), ("$p", True),
    ("([$p] | .[0])", True), ("(if true then $p else . end)", True),
    # #2042 review: error-catching wrappers around a navigation of a
    # computed value (the resolver's own refusal must not be catchable),
    # the recurse/getpath refusal kind, slices of every kind, negative
    # indices, and marker heads behind nested pipes or later in the pipe.
    ("(try (.a | tostring | .[0:1]) catch \"x\")", False), ("((.a | tostring | .[0:1])?)", False),
    ("(([1] | .[0])? // \"alt\")", False), ("(try ([1] | .[0]) catch \"c\")", False),
    ("([1,[2]] | ..)", False), ("([1] | getpath([]))", False), ("(.a | ..)", False),
    (".arr[0:1]?", False), (".arr[1:1]?", False), (".a[0:1]?", False), (".arr[-1]?", False),
    (".a[0:3]?", False), ("(.a | select(.b?))", False), ("(.a // 1)", False),
    ("(if .a then .a else .c end)", False), (".a?", False),
    ("($p.b? | .c?)", True), ("(($p | .b?) | .c?)", True), ("(.c | $p | .b?)", True),
    ("(try $p.b? catch \"X\")", True), ("(5 | $p | .b?)", True), ("($p | ..)", True),
    ("($p[0:1]?)", True),
    # #2978: every spelling `is_identity_passthrough` recognises, not just
    # bare `.` -- each now freezes `.` *with its position*, and the `//`
    # arm's soundness argument (the right operand only runs below a
    # `null`/`false` register) is a claim the pool has to be able to test.
    # `.` alone was 1 draw in 40, too rare to reach the getpath-stage uses
    # that consume the position.
    ("(. // 1)", False), ("(try . catch 1)", False), ("(if true then . else . end)", False),
    ("(if .a then . else . end)", False), ("(. // $p)", True), ("(if true then . else $p end)", True),
    # #2978 review: a `try` whose body can raise binds the *handler's* value
    # (or, catch-less, nothing -- which a following `//` turns into its
    # right operand), not `.`; the handler's value is drawn as a copy of the
    # document's own leaves so a position-blind or value-blind rule would
    # certify it against a real node.
    ("(try (if error(\"e\") then . else . end) catch {\"b\":1})", False),
    ("(try (if error(\"e\") then . else . end) catch $p)", True),
    ("((try (if error(\"e\") then . else . end)) // {\"b\":1})", False),
    ("(try (if .a then . else . end) catch 1)", False),
]
NAV = [".a", ".c", ".x", ".x.a", ".b?", ".arr[0]?", ".arr[]?"]
LITERAL = ["5", "null", "true", "\"z\""]
PASSTHROUGH = ["select(true)", "if true then . else 1 end", "try . catch 1", "(label $out | .)", ". as $q | ."]
MOVES = ["([.a] | first)", "{k: .a}", "[.a]", "(tojson | fromjson)", "([1] | first)"]
# #2889: identity-preserving stages jq keeps the *same* `jv` through --
# embedding `.` into a container and extracting it back out (an object/array
# member, a fold accumulator that returns its input, `add`/`min`/`max` of
# one element, an empty-operand `+`/`*`, a `getpath` readback). Drawn
# alongside PASSTHROUGH wherever it sits between a bind and its use, so a
# generated program can interleave embed stages with the plain passthroughs
# instead of only ever emitting one kind between `. as $x`/`SRC as $y` and
# the eventual `$x`/`$y` reference.
EMBEDS = [
    "({k:.} | .k)", "(. + {})", "([.] | add)", "(reduce empty as $i (.; .))",
    "([.,.] | .[1])", "({k:{j:.}} | .k.j)", "([.] | min)", "(. * {})",
    "(. + null)", "(null + .)", "([] + [.] | .[0])", "([[.]] | .[0][0])",
    "({k:.} | getpath([\"k\"]))",
    # #3183: the multi-element/object fold shapes #2889's review recovered
    # (`eval_owned_relocating_fold` widened to the owned route -- `add`/
    # `min`/`max` over more than the single-element `[.]` this pool already
    # had, and over an object) -- pinned by
    # `test_owned_embed_keeps_node_identity_2889`, but only exercised by that
    # fixed sweep, never by this differential fuzz. All three hold
    # regardless of `.`'s value: `[.,.] | max` compares `.` against itself,
    # a tie by construction whichever position the builtin's tie-break
    # keeps; `null`/`{a:.}`'s single value are folded through `add`'s
    # `null + x`, and jq's `+` treats a `null` operand as the *other* side's
    # identity for every type, not just when it numerically ties.
    #
    # The issue's two tie-break-specific rows (`[{a:1},.] | max`,
    # `[.,{a:1}] | min`) are deliberately not here: unlike the three above,
    # they embed `.` only when `.`'s *value* genuinely ties with the literal
    # `{a:1}`, which needs a document equal to that exact literal --
    # something this fuzzer's `doc()` generator does not produce, so drawing
    # them would silently test ordinary (non-tied) object comparison
    # instead almost every time (confirmed live: `null | [{a:1},.] | max`
    # answers the literal, not `.`, and `{"b":1} | [.,{a:1}] | max` answers
    # `.` itself, not the literal -- the opposite of `EMBEDS`'/`REBUILDS`'
    # own "holds for any document" contract every other entry in both pools
    # relies on). The tie-break path they were meant to add coverage for
    # remains covered only by the fixed-input Rust test above.
    "([.,.] | max)", "([null,.] | add)", "({a:.} | add)",
]
# #3182: a string or number literal is `Rc`-backed like a container, so the
# same placements keep a scalar's identity -- and so do the builtins real jq
# hands its input `jv` back from (KEEPS), while the ones that allocate their
# result must keep refusing (FRESH). Every leaf `doc()` draws (`1`, `"s"`,
# `true`, `null`) reaches these through NAV, and the `?` keeps a container
# input from turning a draw into an error. Drawn beside PASSTHROUGH/EMBEDS so
# a generated program interleaves them with the container-shaped stages.
SCALAR_KEEPS = [
    "tostring", "@text", "(ltrimstr(\"z\"))", "(rtrimstr(\"z\"))", "(sub(\"z\"; \"y\")?)",
    "(tonumber?)", "(abs?)", "getpath([])", "setpath([]; .)", "([.] | add)", "([.] | first)",
    "([.] | sort | .[0])", "([.] | unique | .[0])", "(if . then . else . end)",
]
SCALAR_FRESH = [
    "(ascii_downcase?)", "(. + \"\")?", "(\"\" + .)?", "(tojson | fromjson)", "(ltrimstr(\"s\"))",
    "(.[0:]?)", "(explode | implode)?", "(floor?)", "(. + 0)?", "(-(-.))?", "(tostring | tonumber?)",
    "(\"\\(.)\")", "([.] | join(\"\")?)",
]
USES = ["$v", "$v.b?", "($v | select(true))", "(if true then $v else 1 end)", "($v | .b?)",
        "$v[0:1]?", "(.a[0:1]? | $v)", "(.arr[-1]? | $v)", "recurse(if . == $v and type == \"object\" then $v.b? else empty end)",
        "$v as $w | $w", "reduce (1) as $i (.; $v)", "reduce (1) as $i (.a; $v)", "reduce (1) as $i (.a; 5 | $v)",
        "foreach (1) as $i (0; $v; .)", "($v, .b?)", "(.x | (.a | $v))", "(.x | (.c as $z | $v))",
        "path(.a | $v)", "(($v | .b?) as $w | .b? | $w)", "(.b? as $z | $v)", "($v | $v)",
        # #2649: a pattern variable is indexed/iterated/navigated as often as
        # it is used bare, and the register has to survive each of them.
        "$v[0]?", "($v | .[]?)", "($v | recurse)", "($v[0]?, $v)", "($v | first(.[]?))",
        # #2896: `getpath` as a *stage*, not just a bind source. jq's
        # `f_getpath` is the one navigating builtin that leaves the path
        # register untouched when its input is not the register, and whose
        # result is a pointer *into* its input -- so a `getpath` stage can
        # both preserve a live register and land the pipe back on it. None
        # of the pools above could emit one in stage position, so none of
        # these shapes were reachable at all ("the alphabet is part of the
        # claim", #2041).
        #
        # Drawn so both of the interesting halves are covered: a `getpath`
        # whose result lands *back on* the register (`["a"]` after a `.a`
        # navigation), and one that lands on a sibling holding an equal
        # value at a different node (`["c"]`, `["x","a"]`) -- the shape a
        # position-blind rule would wrongly accept. `getpath([])` is the
        # pure no-op arm; the navigating-argument and two-key forms cover
        # composition.
        "($v | getpath([]))", "($v | getpath([]) | .b?)",
        "($v | getpath([\"a\"]))", "($v | getpath([\"a\"]) | .b?)",
        "($v | getpath([\"c\"]) | .b?)", "($v | getpath([\"x\",\"a\"]) | .b?)",
        "($v | getpath([\"b\"]))", "($v | getpath([(\"a\")]) | .b?)",
        "(.a | $v | getpath([\"a\"]) | .b?)", "(.c | $v | getpath([\"c\"]) | .b?)",
        "(5 | getpath([]) | $v)", "({b:9} | getpath([\"b\"]) | $v)",
        "(.a | 5 | getpath([]) | $v)", "(.a | {b:9} | getpath([\"b\"]) | $v)",
        # Fold shapes are wrapped in `$v | ...` rather than written bare:
        # the bind is what this harness exists to exercise, and a pool entry
        # that never mentions `$v` leaves it dead (review finding on #2896).
        # The wrapper feeds the fold its ambient input and leaves the
        # accumulator/register composition under test unchanged.
        #
        # `reduce` takes exactly two clauses -- a three-clause spelling is a
        # parse error in *both* binaries, which `classify()` used to score
        # "agree". It is `foreach` that has EXTRACT.
        "($v | reduce (1) as $i (.; getpath([\"a\"])))",
        "($v | reduce (1) as $i (.; getpath([\"a\"]) | .b?))",
        "($v | foreach (1) as $i (.; getpath([\"a\"]); .b?))",
        "($v | foreach (1) as $i (.; getpath([\"c\"]); .b?))",
        "($v | foreach (1) as $i (.a; getpath([\"b\"]); .))",
        "($v | foreach (1,2) as $i (.; getpath([\"a\"]); .b?))",
        "foreach (1) as $i (.; $v | getpath([\"a\"]); .b?)",
        "($v | foreach .[]? as $i (.; getpath([\"a\"]); .b?))",
        # #2978: a *nested* bind from a marker source, used at a deeper
        # register inside the parentheses. An identity-passthrough bind now
        # carries the position `.` was frozen at, and a rebind from it
        # (`$v as $w`) must inherit the *marker's* position, never the
        # frame's -- `$v` bound at the root and rebound at `["a"]` is still
        # the root's node, so `getpath(["c"])` from it is `["c"]`, not
        # `["a","c"]`. The second form composes back onto the register.
        "(.a | ($v as $w | .c | $w | getpath([\"c\"]) | .b?))",
        "(.a | ($v as $w | .c | $w | getpath([\"a\",\"c\"]) | .b?))",
        "(.x | ($v as $w | .a | $w | getpath([\"x\",\"a\"]) | .b?))",
        # #3133: the register reaches a pipe nested under `try`/`if`/`,` and
        # a `catch` handler (jq restores it to the `try`'s entry). A marker
        # navigating inside such a pipe used to raise the resolver's own
        # refusal, which `try`/`?` then swallowed into a discarded write --
        # so both halves are drawn: the marker that *is* the register
        # (`$v` right after its own bind) and, through `SOURCES`' sibling
        # copies, one that merely equals it. The catch shapes pair a
        # payload that is the register (`error($v)`, `error(null)` on a
        # null register) with one that is a rebuilt copy.
        "try ($v | .b?)", "(try ($v | .b?) catch .)", "($v | .b?)?",
        "(if true then ($v | .b?) else 1 end)", "(($v | .b?), 1)",
        "try (($v | .[]?) | .b?)", "(try error($v) catch $v)",
        "(try error($v) catch ($v | .b?))", "(try error(null) catch ($v | .b?))",
        "(try error(1) catch $v)", "(5 | try error(1) catch $v)",
        "(try error(.) catch .)", "(try error({b:1}) catch .b?)",
        "(. as $q | 5 | $q as {a:$w} | try ($w | .b?))",
        "(try (. as {a:$w} | $w | .b?) catch .)",
        # #3177: the bound node navigated to *inside* `path()`'s own
        # argument, at a nested position of a container the body built.
        # Storage identity certifies it there (`marker_identical`), which
        # the resolver can only see once the owned re-entry hands it the
        # caller's own tree (`owned_path_door`). Drawn beside a rebuilt
        # sibling (`[$v,{}]`, jq refuses) so a value-only rule would be
        # caught, and with a tail after `path()` so the per-path re-entry
        # is exercised too.
        "([$v] | path(.[0] | $v))", "({k:$v} | path(.k | $v))",
        "([$v,$v] | path(.[1] | $v))", "([$v,{}] | path(.[1] | $v))",
        "([$v] | path(.[] | $v | .b?))", "([$v] | . | path(.[0] | $v))",
        "([$v,$v] | path(.[] | $v) | .[0])", "([$v] | . | max | path($v))",
        "([[$v]] | path(.[0][0] | $v))"]

# #2978: an optional navigation *prefix* before the first bind, so `. as $v`
# can be drawn below the invocation root. `program()` put every bind at the
# head of the pipe, which meant the fuzz could not emit the one shape the
# #2978 hazard lives in: an identity bind at `["x"]` whose position a wrong
# rule would take for `[]`, composing `getpath(["a"])` to the register's own
# path (`["x","a"]`) for a node that is really `["x","x","a"]`-shaped. The
# documents already hold `x.a` as a copy of `a` precisely so an equal value
# sits at a different node.
PREFIX = [".x", ".a", ".arr[]?", ".x.a"]
PREFIX_P = 0.3

# #2649 destructuring patterns: (pattern, the variable the body then uses).
# `V` is replaced by this bind's generated name, `W` by its sibling.
PATTERNS = [
    ("{a:V}", "V"), ("{c:V}", "V"), ("{x:V}", "V"), ("{b:V}", "V"),
    ("{a:{b:V}}", "V"), ("{x:{a:V}}", "V"), ("{x:{c:V}}", "V"),
    ("[V]", "V"), ("[V,W]", "V"), ("[V,W]", "W"), ("[[V]]", "V"),
    ("{a:V, c:W}", "V"), ("{a:V, c:W}", "W"), ("{a:V, a:W}", "W"),
    ("{$a}", "$a"), ("{$a:{b:V}}", "V"), ("{$a, c:V}", "V"),
    ("{a:[V]}", "V"), ("{arr:[V,W]}", "W"),
    # `?//` alternatives: the retry rule is the fabrication-prone half.
    ("{a:V} ?// [V]", "V"), ("[V] ?// {a:V}", "V"), ("{a:V} ?// V", "V"),
    ("{a:V, d:W} ?// {c:V}", "V"), ("{a:V} ?// {c:V}", "V"),
    ("V ?// {a:V}", "V"), ("{a:{b:V}} ?// {c:V}", "V"),
]
DESTRUCTURE_P = 0.5

# #2676: a fold's own loop-variable pattern (`reduce`/`foreach`'s `as
# PATTERN`) is compiled with the same tracked matchers `PATTERNS` above
# already exercises for a plain `SRC as PATTERN | BODY` bind, one level up
# -- reused verbatim here rather than duplicated, since the per-step rule
# (`walk_pattern`) is the very same function on both sides. `FOLD_INIT`
# gives the accumulator a genuinely varied starting shape (a literal, a
# navigation, the fold's own source restated) so both the register's own
# persistence rule (`reduce` never re-anchors after INIT; `foreach` does,
# per source element) and the `null`/`bool` identity exception get real
# coverage, not just the `INIT=.` shape the hand-written sweep leans on.
FOLD_INIT = [".", "0", "null", ".a", ".c", "[1]"]

def fold_program(rng):
    src, _ = rng.choice(SOURCES[:9])  # top-level fold source: no `$p` needed
    pat, use = rng.choice(PATTERNS)
    v = "$v0"
    pat = pat.replace("V", v).replace("W", "$w0")
    use = use.replace("V", v).replace("W", "$w0")
    init = rng.choice(FOLD_INIT)
    update = rng.choice(USES).replace("$v", use)
    if rng.random() < 0.5:
        body = f"reduce {src} as {pat} ({init}; {update})"
    else:
        extract = rng.choice(USES + ["."]).replace("$v", use)
        body = f"foreach {src} as {pat} ({init}; {update}; {extract})"
    wrap = rng.choice(["path(%s)", "path(%s)", "del(%s)", "(%s) = 9", "(%s) |= ."])
    return wrap % body

FOLD_P = 0.2

# #3188: a write whose *target* navigates to an embed of a root bind, over a
# container built after the bind -- the owned re-entry's write door. Every
# other family binds inside the wrapper, so none reaches it. Each construct
# carries navigations into it; the rebuilt ones (`{"a":1}` literals) are the
# must-refuse half: a value-equal copy shares no storage, and jq refuses.
EMBED_WRITE_CONSTRUCTS = [
    ("[.]", [".[0]", ".[]", ".[0]?", ".[-1]", ".[0,0]"]),
    ("[.,.]", [".[1]", ".[]", ".[0,1]", ".[1]?"]),
    ("{k:.}", [".k", ".[\"k\"]", ".[]", ".k?"]),
    ("[[.]]", [".[0][0]", ".[0][]", ".[][]"]),
    ("{k:{j:.}}", [".k.j", ".k[]"]),
    ("([.] + [.])", [".[]", ".[1]"]),
    ("[.,{\"a\":1}]", [".[]", ".[1]", ".[0]"]),
    ("[{\"a\":1}]", [".[0]", ".[]"]),
]
EMBED_WRITE_TAILS = ["", " | .a?", " | .c?", " | .[0]?", " | .x.a?"]
EMBED_WRITE_WRAPS = [
    "del(%s)", "(%s) = 9", "(%s) |= 5", "(%s) |= empty", "(%s) //= 3",
    "((%s) = 9) | length", "del(%s) | .[0]?", ". | del(%s)",
]

def embed_write_program(rng):
    construct, navs = rng.choice(EMBED_WRITE_CONSTRUCTS)
    target = rng.choice(navs) + " | $x" + rng.choice(EMBED_WRITE_TAILS)
    return f". as $x | {construct} | " + rng.choice(EMBED_WRITE_WRAPS) % target

# #3049: `[f]` keeps path tracking live even when its input is tracked.
# Earlier pools generated array values but not this tracked-input placement
# with navigation *inside* the constructor followed by an empty consumer.
TRACKED_ARRAY_INNERS = [
    "[[1] | .[0]]", "[.x | {k:1} | .k]",
    "[.x | tojson | fromjson | .a]", "[.x | to_entries[] | .key]",
    "[.a | length | .x?]", "[5 | .[]?]",
    "[.a]", "[.x.a]", "[paths]", "[to_entries]",
    "[.a[] | select(. == 1)]", "[limit(1; .a[])]",
    "[recurse(.[]?)]", "[[.a]]", "[.a, .x]",
    "[.x as $z | $z.a]", "[.x as $z | [.a, $z.a]]",
]
TRACKED_ARRAY_WRAPPERS = [
    "path(%s | empty)", "del(%s | select(false))",
    "(%s | empty) = 9", "(%s | empty) |= 9",
]
TRACKED_ARRAY_P = 0.12

def tracked_array_program(rng):
    inner = rng.choice(TRACKED_ARRAY_INNERS)
    # Both the document and a navigated child are tracked inputs.
    body = rng.choice(["%s", ".x | %s"]) % inner
    return rng.choice(TRACKED_ARRAY_WRAPPERS) % body

# #3036: routes into `eval.rs`'s own owned-value evaluator (see the module
# doc). `%s` is the body. The `input` route binds the *second* stdin
# document, a value-equal but different `jv` to jq -- the same document
# every other route binds directly.
ROUTES = [
    "input | %s", "(input_line_number | empty), (%s)", "input_line_number as $n | %s",
    "[.] | .[] |= (%s)", "reduce (1) as $i (.; %s)", "foreach (1) as $i (.; %s; .)",
    "with_entries(.value |= (%s))", "try error(.) catch (%s)", "try error({z:1}) catch (%s)",
    # Controls: routes that never leave the generic evaluator, and the bare twin.
    "first(%s)", "[%s]", "label $out | %s", "def f: %s; f", "%s",
]
# Stages between the bind and the write: rebuilds that jq allocates a new
# `jv` for (must refuse), and passthroughs jq keeps the same `jv` through
# (a refusal is safe, an answer must match). `DOC` is the document itself,
# spelled as a literal.
REBUILDS = [
    "(tojson | fromjson)", "({k: .} | .k)", "([.] | .[0])", "(. + {})", "DOC", "(DOC | .)",
    "(. as $q | $q)", "first(.)", "select(true)", "if true then . else 1 end", "(. // 1)",
    "(try . catch 1)", "(label $l | .)", "reduce empty as $i (.; .)", "([.] | first)",
    "limit(1; .)", "(. + null)", "(to_entries | from_entries)", "(with_entries(.))",
    "({a: .a, c: .c, d: .d, x: .x})", "(.x | {a: .a, c: .c})",
    # #2889: these ROUTES-family stages must keep refusing even though the
    # bare-embed twin some of them shadow (`{k: .} | .k`, `. + {}`, `. +
    # null`, `reduce empty as $i (.; .)` -- already above) is recovered by
    # the generic evaluator's own embed table; every route this pool feeds
    # (`input`, a fold's UPDATE, a `|=` RHS, `with_entries`, a `catch`
    # handler) runs through `eval.rs`'s owned-value evaluator instead, which
    # stays refuse-only until Stage B (docs/plan/jq-bind-origin-frame.md).
    "({} + .)", "(. + {a:1})", "([.[]])", "map(.)", "({k:{a:1}} | .k)",
]
# Writes and reads through the root marker; every key is present in `doc`.
ROOT_USES = [
    "($x.a = 9)", "del($x.a)", "path($x)", "($x.c) |= 5", "($x | .a) = 1", "path($x | .c)",
    "del($x | .d)", "($x.a, $x.c) = 2", "path($x, $x.a)", "($x.a? // $x.c) = 3",
]
ROUTE_P = 0.25

# #3037: a variable bound from a *navigated* position outside any resolver
# (`.a as $y`, `Origin::Untracked`) used where the path register really is
# that same node -- `.a as $y | .a | ($y.b) = 9` -- or, the trap, at a
# sibling holding an equal value (`.a as $y | .c | ...` with `.c` copied
# from `.a`, which `doc()` makes common). Every program above binds inside
# the resolver (`path(... as $y ...)`) or binds `.` itself, so the
# `Untracked` marker could reach a write only through ROUTES' identity
# bind; this family draws the bind source from `VALUE_BIND_SOURCES` and the
# stage that follows from the same pool, so same-node, sibling and
# equal-valued-sibling pairings all occur, then writes or reads through the
# marker via `VALUE_BIND_USES`. A rebuild between the two must still refuse.
VALUE_BIND_SOURCES = [".a", ".c", ".x.a", ".x.c", ".x", ".arr[0]?", ".arr[1]?", ".arr[-1]?", ".d", ".[]?"]
VALUE_BIND_USES = [
    "path($y)", "($y.b?) = 9", "del($y.b?)", "($y | .b?) = 9", "$y |= 5", "($y.b?) |= 5",
    "($y.b?) += 1", "($y.b?) //= 7", "path($y.b?)", "path($y | .b?)", "path($y[0]?)",
    "del($y[0]?)", "($y[0]?) = 9", "first(path($y))", "[path($y)]", "(path($y), path(.b?))",
    "try error(.) catch path($y)", "($y | path(.))",
]
VALUE_BIND_P = 0.15

def value_bind_program(rng, d):
    src = rng.choice(VALUE_BIND_SOURCES)
    # Mostly land on the same node again, sometimes on a sibling or an
    # ancestor/descendant, sometimes through a passthrough or a rebuild.
    r = rng.random()
    if r < 0.5:
        nav = src
    elif r < 0.8:
        nav = rng.choice(VALUE_BIND_SOURCES)
    else:
        nav = src + " | " + rng.choice(REBUILDS[:4] + PASSTHROUGH + EMBEDS + ["first(.)", "(. // 1)"])
    use = rng.choice(VALUE_BIND_USES)
    body = f"{src} as $y | {nav} | {use}"
    return rng.choice(ROUTES) % body

def route_program(rng, d):
    v = "$x"
    prefix = rng.choice(["", "", ".x | "])
    parts = [f". as {v}"]
    stages = []
    for _ in range(rng.choice([0, 1, 1, 2])):
        stage = rng.choice(REBUILDS)
        if prefix and "DOC" in stage:
            stage = stage.replace("DOC", json.dumps(d["x"]))
        stages.append(stage.replace("DOC", json.dumps(d)))
    use = rng.choice(ROOT_USES)
    if rng.random() < 0.25:
        # The use inside a `def` resolved before the rebuild runs: the marker
        # is substituted into the def body, and the call is what crosses the
        # rebuild (a `DefCall`'s body is not walked by `map_subexprs`).
        parts.append(f"def f: {use}; " + " | ".join(stages + ["f"]))
    else:
        parts.extend(stages)
        parts.append(use)
    body = prefix + " | ".join(parts)
    return rng.choice(ROUTES) % body

def stage(rng, v):
    r = rng.random()
    if r < 0.4: return rng.choice(NAV)
    if r < 0.55: return rng.choice(LITERAL)
    if r < 0.7: return rng.choice(PASSTHROUGH + EMBEDS + SCALAR_KEEPS + SCALAR_FRESH)
    if r < 0.8: return rng.choice(MOVES)
    return rng.choice(USES).replace("$v", v)

def program(rng):
    n_bind = rng.choice([1, 1, 2])
    parts = []; prev = None
    if rng.random() < PREFIX_P:
        parts.append(rng.choice(PREFIX))
    for i in range(n_bind):
        v = f"$v{i}"
        cands = [s for s, needs in SOURCES if not needs or prev]
        src = rng.choice(cands).replace("$p", prev or "$v0")
        if rng.random() < DESTRUCTURE_P:
            # #2649: a destructuring bind; the body uses the pattern's variable.
            pat, use = rng.choice(PATTERNS)
            w = f"$w{i}"
            parts.append(f"{src} as {pat.replace('V', v).replace('W', w)}")
            v = use.replace("V", v).replace("W", w)
        else:
            parts.append(f"{src} as {v}")
        for _ in range(rng.choice([0, 1, 1, 2])):
            parts.append(stage(rng, v))
        prev = v
    parts.append(rng.choice(USES).replace("$v", prev))
    for _ in range(rng.choice([0, 0, 1])):
        parts.append(stage(rng, prev))
    body = " | ".join(parts)
    wrap = rng.choice(["path(%s)", "path(%s)", "del(%s)", "(%s) = 9", "(%s) |= .", "[path(%s)]"])
    return wrap % body

def run(cmd, data):
    try:
        p = subprocess.run(cmd, input=data.encode(), capture_output=True, timeout=30)
    except subprocess.TimeoutExpired:
        # A shape either binary loops on (jq's `recurse` on a self-similar
        # value, say) is not a direction finding; counted, never failed on.
        return 124, "TIMEOUT"
    return p.returncode, p.stdout.decode(errors="replace")

def classify(j, s):
    (je, jo), (se, so) = j, s
    if je == 124 or se == 124: return "timeout"
    # A program neither binary can even compile is not evidence of anything,
    # but `je == se and jo == so` scored it "agree" -- so a pool entry with a
    # syntax error in it inflated the agreement count instead of being
    # noticed (review finding on #2896: a three-clause `reduce` in the
    # getpath-stage pool accounted for 164 of 6000 "agreements"). jq and
    # succinctly both exit 3 on a compile error.
    if je == 3 and se == 3: return "both-reject"
    if je == se and jo == so: return "agree"
    if je != 0 and se == 0: return "fabricate"
    if je == 0 and se != 0: return "refuse-only"
    if je != 0 and se != 0:
        jl, sl = jo.splitlines(), so.splitlines()
        if len(sl) < len(jl) and jl[:len(sl)] == sl: return "refuse-early"
    return "mismatch"

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default="target/release/succinctly")
    ap.add_argument("--jq", default="/usr/bin/jq")
    ap.add_argument("--baseline", default=None,
                    help="a succinctly binary built from a commit that predates the change under test; "
                         "a divergence it reproduces byte-for-byte is reported as *-baseline "
                         "and does not fail the run (it is not this change's), but is still listed")
    ap.add_argument("-n", type=int, default=500)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--show", type=int, default=8)
    ap.add_argument("--fold-p", type=float, default=None,
                    help="probability a program is a fold (default FOLD_P). A fold-only fix's "
                         "changed code is reachable only from a fold, so a stock run draws too "
                         "few to see its rare shapes -- #3145's review found a fabricated write "
                         "at --fold-p 1.0 that 18,000 stock programs had missed")
    ap.add_argument("--value-bind-p", type=float, default=None,
                    help="probability a program is a value-bind program (default VALUE_BIND_P, "
                         "scaled like the other routes when --fold-p is set). #3135's door opens "
                         "only for a navigated bind used by a resolver on an owned-rooted route, "
                         "so a stock run draws few of them -- run at 1.0 to weight the sweep onto it")
    ap.add_argument("--embed-write-p", type=float, default=0.0,
                    help="probability a program is an embed-write program (#3188): a root bind, a "
                         "container built from it, then del/assignment through a target navigating "
                         "to it. Off by default so existing seeds keep their stream; run at 1.0 to "
                         "weight the sweep onto the owned write door")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    pin = open("tests/data/jq-golden/JQ_VERSION").read().strip()
    version = subprocess.run([a.jq, "--version"], capture_output=True, text=True).stdout.strip()
    if not version.startswith(pin):
        sys.exit(f"error: {a.jq} is not the pinned oracle ({pin}): {version!r}")
    if a.self_test:
        for name, pool in [("PREFIX", PREFIX), ("SOURCES", [s for s, _ in SOURCES]), ("NAV", NAV),
                           ("LITERAL", LITERAL), ("PASSTHROUGH", PASSTHROUGH), ("MOVES", MOVES),
                           ("EMBEDS", EMBEDS),
                           ("USES", USES), ("PATTERNS", [f"{p} -> {u}" for p, u in PATTERNS]),
                           ("ROUTES", ROUTES), ("REBUILDS", REBUILDS), ("ROOT_USES", ROOT_USES),
                           ("TRACKED_ARRAY_INNERS", TRACKED_ARRAY_INNERS),
                           ("TRACKED_ARRAY_WRAPPERS", TRACKED_ARRAY_WRAPPERS),
                           ("VALUE_BIND_SOURCES", VALUE_BIND_SOURCES), ("VALUE_BIND_USES", VALUE_BIND_USES),
                           ("EMBED_WRITE_CONSTRUCTS", [c for c, _ in EMBED_WRITE_CONSTRUCTS]),
                           ("EMBED_WRITE_WRAPS", EMBED_WRITE_WRAPS)]:
            print(f"{name} ({len(pool)}): " + " ; ".join(pool))
        return 0
    rng = random.Random(a.seed)
    # `--fold-p 1.0` drives every program through `fold_program`; the other
    # routes keep their own shares of what is left.
    fold_p = FOLD_P if a.fold_p is None else a.fold_p
    route_p = ROUTE_P * (1.0 - fold_p) / max(1.0 - FOLD_P, 1e-9)
    value_bind_p = (VALUE_BIND_P * (1.0 - fold_p) / max(1.0 - FOLD_P, 1e-9)
                    if a.value_bind_p is None else a.value_bind_p)
    kinds = ["agree", "fabricate", "mismatch", "refuse-only", "refuse-early", "both-reject",
             "fabricate-baseline", "mismatch-baseline", "refuse-only-baseline",
             "refuse-early-baseline", "timeout"]
    counts = {k: 0 for k in kinds}
    examples = {k: [] for k in kinds}
    for _ in range(a.n):
        dv = doc(rng)
        d = json.dumps(dv)
        r = rng.random()
        if a.embed_write_p and rng.random() < a.embed_write_p:
            f = embed_write_program(rng)
        elif r < TRACKED_ARRAY_P:
            f = tracked_array_program(rng)
        elif r < TRACKED_ARRAY_P + (1.0 - TRACKED_ARRAY_P) * route_p:
            # #3036: the document twice, so `input` reads a second copy.
            f, d = route_program(rng, dv), d + "\n" + d
        elif r < TRACKED_ARRAY_P + (1.0 - TRACKED_ARRAY_P) * (route_p + value_bind_p):
            # #3037: same ROUTES, so `input` needs its second copy too.
            f, d = value_bind_program(rng, dv), d + "\n" + d
        elif r < TRACKED_ARRAY_P + (1.0 - TRACKED_ARRAY_P) * (route_p + value_bind_p + fold_p):
            f = fold_program(rng)
        else:
            f = program(rng)
        j = run([a.jq, "-c", f], d)
        s = run([a.bin, "jq", "-c", f], d)
        c = classify(j, s)
        if c in ("fabricate", "mismatch", "refuse-only", "refuse-early") and a.baseline:
            b = run([a.baseline, "jq", "-c", f], d)
            if b == s:
                c += "-baseline"
        counts[c] += 1
        if c != "agree" and len(examples[c]) < a.show:
            examples[c].append((d, f, j, s))
    print(f"seed={a.seed} n={a.n} fold_p={fold_p:g} " + " ".join(f"{k}={v}" for k, v in counts.items() if v or k in kinds[:4]))
    for c in kinds[1:]:
        for d, f, j, s in examples[c]:
            print(f"[{c}] {f}\n    on {d}\n    jq[{j[0]}]: {j[1].strip()[:120]}\n    sc[{s[0]}]: {s[1].strip()[:120]}")
    return 1 if counts["fabricate"] or counts["mismatch"] else 0

if __name__ == "__main__":
    sys.exit(main())
