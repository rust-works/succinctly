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

**The alphabet is part of the claim** (#2041): a pool that cannot emit the
shape a bug lives in proves nothing. `--self-test` prints the pools so a
shrink is visible.

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
]
NAV = [".a", ".c", ".x", ".x.a", ".b?", ".arr[0]?", ".arr[]?"]
LITERAL = ["5", "null", "true", "\"z\""]
PASSTHROUGH = ["select(true)", "if true then . else 1 end", "try . catch 1", "(label $out | .)", ". as $q | ."]
MOVES = ["([.a] | first)", "{k: .a}", "[.a]", "(tojson | fromjson)", "([1] | first)"]
USES = ["$v", "$v.b?", "($v | select(true))", "(if true then $v else 1 end)", "($v | .b?)",
        "$v[0:1]?", "(.a[0:1]? | $v)", "(.arr[-1]? | $v)", "recurse(if . == $v and type == \"object\" then $v.b? else empty end)",
        "$v as $w | $w", "reduce (1) as $i (.; $v)", "reduce (1) as $i (.a; $v)", "reduce (1) as $i (.a; 5 | $v)",
        "foreach (1) as $i (0; $v; .)", "($v, .b?)", "(.x | (.a | $v))", "(.x | (.c as $z | $v))",
        "path(.a | $v)", "(($v | .b?) as $w | .b? | $w)", "(.b? as $z | $v)", "($v | $v)"]

def stage(rng, v):
    r = rng.random()
    if r < 0.4: return rng.choice(NAV)
    if r < 0.55: return rng.choice(LITERAL)
    if r < 0.7: return rng.choice(PASSTHROUGH)
    if r < 0.8: return rng.choice(MOVES)
    return rng.choice(USES).replace("$v", v)

def program(rng):
    n_bind = rng.choice([1, 1, 2])
    parts = []; prev = None
    for i in range(n_bind):
        v = f"$v{i}"
        cands = [s for s, needs in SOURCES if not needs or prev]
        src = rng.choice(cands).replace("$p", prev or "$v0")
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
    if je == se and jo == so: return "agree"
    if je != 0 and se == 0: return "fabricate"
    if je == 0 and se != 0: return "refuse-only"
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
        for name, pool in [("SOURCES", [s for s, _ in SOURCES]), ("NAV", NAV), ("LITERAL", LITERAL),
                           ("PASSTHROUGH", PASSTHROUGH), ("MOVES", MOVES), ("USES", USES)]:
            print(f"{name} ({len(pool)}): " + " ; ".join(pool))
        return 0
    rng = random.Random(a.seed)
    kinds = ["agree", "fabricate", "mismatch", "refuse-only", "fabricate-baseline", "mismatch-baseline", "timeout"]
    counts = {k: 0 for k in kinds}
    examples = {k: [] for k in kinds}
    for _ in range(a.n):
        d = json.dumps(doc(rng)); f = program(rng)
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
            print(f"[{c}] {f}\n    on {d}\n    jq[{j[0]}]: {j[1].strip()[:120]}\n    sc[{s[0]}]: {s[1].strip()[:120]}")
    return 1 if counts["fabricate"] or counts["mismatch"] else 0

if __name__ == "__main__":
    sys.exit(main())
