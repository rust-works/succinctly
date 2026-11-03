#!/usr/bin/env python3
"""Differential grid for a fold source that destructures a frozen `$x` inside another fold (#5030).

A `reduce`/`foreach` whose SOURCE destructures a frozen `$x` (`$x as {b:$q} | .`, `try ($x | . as
{b:$q} | .)`, `first($x as {b:$q} | .)`, ...) and which sits in the body of another fold is judged against a
path register the inner fold does not have: jq raises on the step that reads the pattern's access
(`near attempt to access element "b"`) where a build that routes the register only at depth 0 answers, and
a `del`/`|=`/`=` then writes through the answer (#5172, #5197). The other grids hold no operand that is an
outer `$x` destructured in a fold body (`jq-fold-body-register-grid-sweep.py` is outer fold x inner fold x
source; this is the inner fold's own bind site x INIT x source wrapper x fold kind x UPDATE), so this one
is what gates the class.

Each row is judged against jq (`/usr/bin/jq`) on stdout, exit code and the first line of stderr (minus its
`jq: error (at ...): ` prefix), for the two builds. Judging stdout and exit code alone missed a message
regression that a pinned test caught (#5197), so a different message is a mismatch here.

  ok            both builds match jq
  fixed         NEW matches jq, BASE did not
  REGRESS       BASE matched jq, NEW does not (a lost match)
  still_wrong   neither matches, and they agree with each other
  changed_wrong neither matches, and they disagree: the change moved a wrong answer

Exit status 1 when any row is REGRESS, when NEW answers (exit 0) a row jq refuses and BASE did not
(a newly silent accept: `del`/`|=`/`=` write through it), or when NEW times out (counted apart, since two
hung builds are otherwise just `still_wrong`). A row jq itself times out on is skipped. `changed_wrong`
alone does not fail the run: read it. `still_wrong` was large when this grid was written, since both builds worded many of jq's
errors differently; on `main` at #5552 every row of the default draw matches jq, message included.
`--jq-gate` also fails the run on any row where NEW does not match jq (stdout, exit code and message):
with NEW passed as BASE too, that is the gate against jq alone, and it is how `succinctly dev compare jq
--dimension sweeps` runs this script (#5552). No residual is documented, so a residual that becomes
accepted has to be listed here first.

Usage: scripts/jq-nested-fold-source-sweep.py BASE NEW [N [SEED]] [--show N] [--dump FILE] [--jobs N] [--jq JQ]
  BASE, NEW  succinctly builds (the change's parent, normally `main`, and the build under test)
  N          rows to draw (default 6000; duplicates collapse, so fewer are run); about 3 spawns a row
  SEED       the draw's seed (default 1); a given N and SEED is the same row set on every machine
  --dump     write every row that is not `ok` (row, jq, base, new) as JSON to FILE

N=4000 SEED=1 is the grid of the #5030 evidence comment. On `main` 227929b16 335 of its rows differ from jq
on stdout and exit code (the comment's count) and 664 once the stderr line counts too.
"""
import argparse
import collections
import concurrent.futures
import json
import random
import re
import subprocess
import sys
import time

DOCS = [
    '{"a":{},"b":{}}',
    '{"a":{"b":1,"c":{"b":3}},"b":2}',
    '{"a":{"b":1},"c":{"b":2}}',
    '{"a":null,"b":null}',
    '{"a":[1],"b":0}',
    "null",
    '{"a":{"c":null},"b":{"b":4}}',
    '{"a":false,"b":{"b":1}}',
    '[{"b":1},{"b":2}]',
    '{"a":{"b":null},"b":1}',
    '{"a":{"a":{"b":1}},"b":{"b":1}}',
]
# what `$x` is bound to
BINDS = [".a.c", ".a.b", ".a[0]", ".nope", ".a", ".", ".b", ".a?"]
# the inner fold's INIT
INITS = [".a", ".", ".b", ".a.b", "null", ".nope", ".a | tostring", ".a.c", "1", ".a?"]
# the inner fold's SOURCE, each destructuring (or navigating) `$x` somewhere
WRAPS = [
    "$x as {b:$q} | .",
    "try ($x as {b:$q} | .)",
    "($x as {b:$q} | .)?",
    "$x | . as {b:$q} | .",
    "$x as [$q] | .",
    "$x as {b:$q} | $q",
    "try ($x | . as {b:$q} | .)",
    "try ($x | .a as {b:$q} | .)",
    "$x | .a as {b:$q} | .",
    "(1, ($x as {b:$q} | .))",
    "try reduce ($x as {b:$q} | .) as $w (0; .)",
    "first($x as {b:$q} | .)",
]
# the inner fold's UPDATE (for a `foreach`, also its EXTRACT)
UPDATES = [".", ".c?", ".b", "null", "$y"]
# the fold the inner one sits in; `G` is the inner fold, the last is no outer fold at all
OUTERS = [
    "reduce 1 as $i (.; G)",
    "foreach 1 as $i (.; G; .)",
    "foreach 1 as $i (.; .; G)",
    "reduce (1,2) as $i (.; G)",
    "foreach (1,2) as $i (.; G; .)",
    "reduce .a as $i (.; G)",
    "foreach .b as $i (.; G; .)",
    "foreach 1 as $i (.a; G; .)",
    "G",
]
# the write (or path) context; `F` is `BIND as $x | OUTER`
CONTEXTS = [
    "path(F)",
    "[path(F)?]",
    "del(F)",
    "(F) |= 9",
    'try path(F) catch "C"',
    "(F) = 9",
]
KINDS = ["reduce", "foreach", "foreach2"]


def make_row(rng):
    # The order of these draws is the seed's meaning: reordering them changes every row set.
    bind = rng.choice(BINDS)
    init = rng.choice(INITS)
    wrap = rng.choice(WRAPS)
    update = rng.choice(UPDATES)
    kind = rng.choice(KINDS)
    if kind == "reduce":
        fold = f"reduce ({wrap}) as $y ({init}; {update})"
    elif kind == "foreach":
        fold = f"foreach ({wrap}) as $y ({init}; {update}; .)"
    else:
        fold = f"foreach ({wrap}) as $y ({init}; .; {update})"
    outer = rng.choice(OUTERS).replace("G", fold)
    program = rng.choice(CONTEXTS).replace("F", f"{bind} as $x | {outer}")
    return rng.choice(DOCS), program


def rows(n, seed):
    rng = random.Random(seed)
    return sorted({make_row(rng) for _ in range(n)})


def run(argv, program, doc):
    """(stdout, exit code, first stderr line without its location prefix)."""
    try:
        p = subprocess.run(argv + ["-c", program], input=doc.encode(), capture_output=True, timeout=20)
    except subprocess.TimeoutExpired:
        return ("TIMEOUT", -1, "")
    err = p.stderr.decode(errors="replace").strip().split("\n")[0]
    err = re.sub(r"^jq: error \(at [^)]*\): ", "", err)
    return (p.stdout.decode(errors="replace"), p.returncode, err)


def classify(base, new, oracle):
    base_ok, new_ok = base == oracle, new == oracle
    if base_ok and new_ok:
        return "ok"
    if new_ok:
        return "fixed"
    if base_ok:
        return "REGRESS"
    return "still_wrong" if base == new else "changed_wrong"


def progress_line(done, total, elapsed):
    rate = done / elapsed if elapsed > 0 else 0.0
    left = int(round((total - done) / rate)) if rate > 0 else 0
    hours, rest = divmod(left, 3600)
    eta = f"{hours}h{rest // 60:02d}m" if hours else f"{rest // 60}m{rest % 60:02d}s"
    return f"  {done}/{total} ({elapsed:.0f}s, {rate:.0f} rows/s, ETA {eta})"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("base", help="succinctly build without the change (normally main)")
    ap.add_argument("new", help="succinctly build under test")
    ap.add_argument("n", nargs="?", type=int, default=6000, help="rows to draw (default 6000)")
    ap.add_argument("seed", nargs="?", type=int, default=1, help="the draw's seed (default 1)")
    ap.add_argument("--jq", default="/usr/bin/jq", help="the pinned oracle (jq 1.7.1)")
    ap.add_argument("--show", type=int, default=6, help="rows printed per section")
    ap.add_argument("--dump", help="write every row that is not ok to this file as JSON")
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument(
        "--jq-gate",
        action="store_true",
        help="also fail on any row where NEW differs from jq (the gate without a base: pass NEW as BASE too, #5552)",
    )
    args = ap.parse_args()

    grid = rows(args.n, args.seed)

    def one(row):
        doc, program = row
        oracle = run([args.jq], program, doc)
        if oracle[0] == "TIMEOUT":
            return None
        base = run([args.base, "jq"], program, doc)
        new = run([args.new, "jq"], program, doc)
        return row, oracle, base, new

    counts = collections.Counter()
    new_timeouts = new_accepts = 0
    shown = collections.defaultdict(list)
    kept = []
    skipped = 0
    started = time.time()
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        for i, rec in enumerate(pool.map(one, grid), 1):
            if i % 1000 == 0:
                print(progress_line(i, len(grid), time.time() - started), file=sys.stderr)
            if rec is None:
                skipped += 1
                continue
            _row, oracle, base, new = rec
            klass = classify(base, new, oracle)
            if new[0] == "TIMEOUT":
                new_timeouts += 1
            if oracle[1] != 0 and new[1] == 0 and base[1] != 0:
                new_accepts += 1
                shown["NEWACC"].append(rec)
            counts[klass] += 1
            if klass != "ok":
                kept.append(rec)
                shown[klass].append(rec)
    print(f"{dict(sorted(counts.items()))} rows {len(grid) - skipped} of {len(grid)} drawn "
          f"(N={args.n} SEED={args.seed}; jq timed out on {skipped})")
    if args.dump:
        with open(args.dump, "w") as out:
            json.dump(kept, out)
    print(f"NEW timeouts: {new_timeouts}; newly accepted where jq refuses: {new_accepts}")
    for klass in ("REGRESS", "NEWACC", "changed_wrong", "fixed"):
        for (doc, program), oracle, base, new in shown[klass][: args.show]:
            print(f"{klass} {doc} | {program}\n   jq   {oracle}\n   base {base}\n   new  {new}")
    # No residual is documented, so against jq alone any row NEW does not match is unexpected.
    jq_wrong = sum(n for klass, n in counts.items() if klass not in ("ok", "fixed"))
    if args.jq_gate:
        print(f"rows where NEW differs from jq (--jq-gate): {jq_wrong}")
        for klass in ("still_wrong",):
            for (doc, program), oracle, base, new in shown[klass][: args.show]:
                print(f"{klass} {doc} | {program}\n   jq   {oracle}\n   new  {new}")
    failed = counts["REGRESS"] or new_timeouts or new_accepts or (args.jq_gate and jq_wrong)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
