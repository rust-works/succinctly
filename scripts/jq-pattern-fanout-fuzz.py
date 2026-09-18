#!/usr/bin/env python3
"""Randomised differential fuzz for the destructuring matcher's fan-out
(#2872) -- computed keys in `SRC as PATTERN`, `reduce`/`foreach`'s own
pattern, and every path-position consumer (`path()`, `del()`, `=`, `|=`).

jq compiles a destructuring pattern into backtracking matcher bytecode: a
computed key is an ordinary generator fork, a later entry's key re-runs for
each output of an earlier one, array elements match right to left, and the
body runs at the end of every completed path through the matcher. Every
consequence of that model is a claim this fuzz can emit a shape for:

  * multi-output keys (`("a","b")`), zero-output keys (`empty`, `("a",
    empty)`), a key that raises after an output (`("a", error("E"))`) or
    before any (`(error("E"), "a")`), a key that breaks out of a label or
    halts, and a key with a side effect (`("K"|stderr|"a")`) whose *order*
    relative to the body's own side effects is the laziness claim;
  * nested patterns with computed keys at more than one level, arrays of
    object patterns (right-to-left order), the `{$k}` shorthand beside a
    computed entry, and duplicate names across entries (jq's first/last
    STOREV rule);
  * `?//` chains with the computed pattern as either alternative -- the
    retry rules are where a wrong answer is a fabrication rather than a
    refusal, and a catchable refusal at an intermediate alternative was
    exactly #2872's bug;
  * every consumer: value position under `first`/`limit`/`try`/`?`/`label`,
    both folds in value position (with and without EXTRACT, with `?//`),
    and path position under `path`/`del`/`=`/`|=` and the path-mode folds.

stderr is compared too, with the oracle's own `jq: error` diagnostics
stripped: a `stderr` marker written in the wrong order, or one written by a
key output jq never produced, is a mismatch even when stdout agrees.

Classification, as in `scripts/jq-bind-origin-fuzz.py`:

    agree        same exit, same stdout, same side-effect stream
    fabricate    jq refuses, succinctly answers  -- what `del()`/`=` write through
    mismatch     both answer, differently; or both refuse but emitted
                 different prefixes or side effects
    refuse-only  jq answers, succinctly refuses  -- safe, reported as a count
    refuse-early both refuse, and what succinctly emitted before refusing is
                 a strict prefix of what jq emitted
    both-reject  neither compiles the program (a pool bug, not evidence)

**The alphabet is part of the claim** (#2041): `--self-test` prints the
pools so a shrink is visible. `--baseline` takes a binary built before the
change under test; a finding it reproduces byte-for-byte is reported as
`*-baseline` and does not fail the run. Run against such a binary as
`--bin`, this fuzz must report fabricate/mismatch -- that is how the
alphabet was validated (every one of #2872's filed wrong answers shows up).

Usage:
    cargo build --release --features cli
    ./scripts/jq-pattern-fanout-fuzz.py [--bin PATH] [--jq PATH] [-n N] [--seed S] [--show K]
Exit 1 on any fabricate/mismatch.
"""
import argparse, json, random, re, subprocess, sys

DOCS = [
    {"a": 1, "b": 2},
    {"a": {"x": 1, "y": 2}, "b": {"x": 3, "y": 4}},
    {"a": [1, 2], "b": [3]},
    {"a": {"x": [1, 2]}, "b": {"x": [3, 4]}},
    [{"a": 1, "b": 2}, {"a": 3, "b": 4}],
    [[1, 2], [3, 4]],
    [1, 2, 3],
    None,
    {"a": None, "b": True},
    {"a": 1, "b": 1},
    5,
    "s",
]

# Key expressions. `$o` is a label every program with a `break` wraps in;
# `stderr` markers are the side-effect stream the laziness claim reads.
KEYS = ['(%s)' % k for k in [
    '"a"', '"b"', '"a","b"', '"b","a"', '"a","a"', '"a","b","x"',
    'empty', '"a", empty', 'empty, "b"',
    '"a", error("E")', 'error("E"), "a"', '"a", error("E"), "b"',
    '"a", break $o', 'break $o, "a"',
    '"a", halt_error(7)',
    '"K"|stderr|"a"', '"a", ("K"|stderr|"b")', '("K"|stderr|"a"), "b"',
    '"x","y"', '"x", "a"',
    # Spelled through an array so jq's compile-time "Cannot use <kind> as
    # object key" check on a *literal* key (constant-folded through `| .`
    # and arithmetic too; a separate, documented gap) does not reject the
    # program before the matcher ever runs.
    '[0,1][]', '[1,0][]', '[-1][0]', '[1.5][0]', '[1, "a"][]', '["a", 1][]',
    '[true][0]', '[null][0]',
    '.a', 'keys[]', 'keys_unsorted[]', '.[]|tostring',
]]
KEYS_BREAK = {k for k in KEYS if "break" in k}

# Bodies, by which variables they read. Every one is a generator of its own
# in at least one member, so the order of body outputs against key outputs
# is a claim too.
BODIES_1 = [
    '$q', '[$q]', '($q, 0)', '($q | .x?)', '($q | .[0]?)',
    'if $q == 1 then error("x") else $q end',
    'if $q == null then empty else $q end',
    '("B\\($q)"|stderr|$q)', '($q | tostring)', '(1, error("y"))', 'empty',
]
BODIES_2 = [
    '[$q,$r]', '($q, $r)', '($r, $q)', '($q + $r)', '("B\\([$q,$r])"|stderr|[$q,$r])',
    'if $q == 1 then $r else $q end',
]

def fill(rng, shape, vars_):
    """Fill a shape's `K` slots with key expressions and its `V` slots with
    the variable names, in order (cycling, so a two-`V` shape with one name
    binds it twice -- jq's duplicate-name rule is a claim too)."""
    keys, out, vi = [], [], 0
    for ch in shape:
        if ch == 'K':
            k = rng.choice(KEYS); keys.append(k); out.append(k)
        elif ch == 'V':
            out.append(vars_[vi % len(vars_)]); vi += 1
        else:
            out.append(ch)
    return ''.join(out), keys

SHAPES_1 = [
    '{K:$V}', '{K:{K:$V}}', '{"a":{K:$V}}', '{K:{a:$V}}',
    '[{K:$V}]', '{K:[$V]}', '[{K:$V}, $V]', '{K:$V, K:$V}',
]
SHAPES_2 = [
    '{K:$V, K:$V}', '{K:$V, b:$V}', '{a:$V, K:$V}', '{K:{K:$V, $V}}',
    '[{K:$V},{K:$V}]', '[$V,{K:$V}]', '{K:$V, $V}', '{$V, K:$V}', '{K:[$V,$V]}',
]

def pattern(rng, vars_):
    """A pattern binding the names in `vars_` (1 or 2), with at least one
    computed key somewhere in it."""
    shape = rng.choice(SHAPES_1 if len(vars_) == 1 else SHAPES_2)
    return fill(rng, shape, vars_)

def program(rng):
    d = rng.choice(DOCS)
    nvars = rng.choice([1, 1, 2])
    vars_ = ['q', 'r'][:nvars]
    pat, keys = pattern(rng, vars_)
    body = rng.choice(BODIES_1 if nvars == 1 else BODIES_2)
    alt = rng.random()
    if alt < 0.25:
        pat = f'{pat} ?// $z'
    elif alt < 0.35:
        pat = f'$z ?// {pat}'
    elif alt < 0.45:
        pat = f'{pat} ?// {{a:$q}}'
    elif alt < 0.5:
        pat = f'[$z] ?// {pat}'
    kind = rng.random()
    src = rng.choice(['.', '.', '.', '(., .)', '.[]?', '.a?', '(.a?, .b?)'])
    if kind < 0.45:
        bind = f'{src} as {pat} | {body}'
        wrap = rng.choice([
            '[%s]', '[%s]', '[first(%s)]', '[limit(2; %s)]', '[try (%s) catch "c"]',
            '[(%s)?]', '[isempty(%s)]', '[%s] | length', '(%s) as $v | [$v]',
        ])
        f = wrap % bind
    elif kind < 0.7:
        wrap = rng.choice(['[path(%s)]', '[path(%s)]', 'del(%s)', '(%s) |= .', '(%s) = 9',
                           '[path(first(%s))]', '[path(limit(2; %s))]', '[path((%s)?)]',
                           '[try path(%s) catch "c"]'])
        f = wrap % f'{src} as {pat} | {body}'
    else:
        fold = rng.choice([
            'reduce %s as %s (0; .+1)', '[reduce %s as %s (0; .+1)]',
            '[foreach %s as %s (0; .+1)]', '[foreach %s as %s (0; .+1; [., %s])]',
            '[limit(2; foreach %s as %s (0; .+1; [., %s]))]',
            '[first(foreach %s as %s (0; .+1; [., %s]))]',
            '[foreach %s as %s (0; if . == 1 then error("u") else .+1 end)]',
            '[path(foreach %s as %s (.; .; %s))]', '[path(reduce %s as %s (.; %s))]',
            '[path(limit(2; foreach %s as %s (.; .; %s)))]',
            'del(foreach %s as %s (.; .; %s))',
            '[foreach %s as %s (0; ("U\\(.)"|stderr) | .+1)]',
        ])
        fsrc = rng.choice(['.', '(., .)', '.[]?', '(.a?, .b?)'])
        n = fold.count('%s')
        body_ref = f'${vars_[0]}' if nvars == 1 else f'[${vars_[0]},${vars_[1]}]'
        f = fold % ((fsrc, pat) + ((body_ref,) * (n - 2)))
    if any(k in KEYS_BREAK for k in keys) or 'break $o' in f:
        f = f'[label $o | {f}]'
    return json.dumps(d), f

def run(cmd, data):
    try:
        p = subprocess.run(cmd, input=data.encode(), capture_output=True, timeout=30)
    except subprocess.TimeoutExpired:
        return 124, "TIMEOUT", ""
    # The side-effect stream only: an error diagnostic is stripped wherever
    # it lands (a `stderr` marker written just before one shares its line).
    err = re.sub(r"jq: error[^\n]*\n?", "", p.stderr.decode(errors="replace"))
    return p.returncode, p.stdout.decode(errors="replace"), err

def classify(j, s):
    (je, jo, jx), (se, so, sx) = j, s
    if je == 124 or se == 124: return "timeout"
    if je == 3 and se == 3: return "both-reject"
    if je == se and jo == so and jx == sx: return "agree"
    if je != 0 and se == 0: return "fabricate"
    if je == 0 and se != 0: return "refuse-only"
    if je != 0 and se != 0:
        # Both refuse: succinctly refusing *earlier* -- fewer outputs, fewer
        # side effects, each a prefix of jq's -- is the safe direction, and
        # so is the same refusal under a different diagnostic (a halt's
        # own stderr dump counts as jq's side effects here).
        jl, sl = jo.splitlines(), so.splitlines()
        if jl[:len(sl)] == sl and jx.startswith(sx):
            return "agree" if (jl == sl and jx == sx) else "refuse-early"
    return "mismatch"

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default="target/release/succinctly")
    ap.add_argument("--jq", default="/usr/bin/jq")
    ap.add_argument("--baseline", default=None,
                    help="a succinctly binary built from a commit that predates the change under test; "
                         "a fabricate/mismatch it reproduces byte-for-byte is reported as *-baseline "
                         "and does not fail the run (it is not this change's), but is still listed")
    ap.add_argument("-n", type=int, default=500)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--show", type=int, default=8)
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    pin = open("tests/data/jq-golden/JQ_VERSION").read().strip()
    version = subprocess.run([a.jq, "--version"], capture_output=True, text=True).stdout.strip()
    if not version.startswith(pin):
        sys.exit(f"error: {a.jq} is not the pinned oracle ({pin}): {version!r}")
    if a.self_test:
        for name, pool in [("DOCS", [json.dumps(d) for d in DOCS]), ("KEYS", KEYS),
                           ("BODIES_1", BODIES_1), ("BODIES_2", BODIES_2)]:
            print(f"{name} ({len(pool)}): " + " ; ".join(pool))
        rng = random.Random(a.seed)
        for _ in range(12):
            print("  e.g. " + program(rng)[1])
        return 0
    rng = random.Random(a.seed)
    kinds = ["agree", "fabricate", "mismatch", "refuse-only", "refuse-early", "both-reject",
             "fabricate-baseline", "mismatch-baseline", "timeout"]
    counts = {k: 0 for k in kinds}
    examples = {k: [] for k in kinds}
    for _ in range(a.n):
        d, f = program(rng)
        j = run([a.jq, "-c", f], d)
        s = run([a.bin, "jq", "-c", f], d)
        c = classify(j, s)
        if c in ("fabricate", "mismatch") and a.baseline:
            b = run([a.baseline, "jq", "-c", f], d)
            if b == s:
                c += "-baseline"
        counts[c] += 1
        if c != "agree" and len(examples[c]) < a.show:
            examples[c].append((d, f, j, s))
    print(f"seed={a.seed} n={a.n} " + " ".join(f"{k}={v}" for k, v in counts.items() if v or k in kinds[:4]))
    for c in kinds[1:]:
        for d, f, j, s in examples[c]:
            print(f"[{c}] {f}\n    on {d}\n    jq[{j[0]}]: {j[1].strip()[:120]} | {j[2].strip()[:60]}"
                  f"\n    sc[{s[0]}]: {s[1].strip()[:120]} | {s[2].strip()[:60]}")
    return 1 if counts["fabricate"] or counts["mismatch"] else 0

if __name__ == "__main__":
    sys.exit(main())
