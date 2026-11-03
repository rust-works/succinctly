#!/usr/bin/env python3
"""Differential grid for a `try` around a nested `foreach` whose source destructures a `$x`, in a `foreach` UPDATE (#5208).

`. as $x | foreach OUTER as $i (INIT; try (foreach (SRC) as $y (0; EXTRACT; $y)) catch HANDLER; .)`: after a
step whose handler hands the root on (`catch $i`, `catch $x`, `catch .`), the next step's state is the root, which
is jq's register and the node `$x` is, so a destructure of `$x` passes `path_intact` where it is value-equal and a
step that is not a plain `INDEX` away raises where it is not. The resolver reads its own refusal of such a step
as a guess nothing catches (#3267), so a pattern that does not raise was refused with `Invalid path expression
with result <document>`, and `del`/`|=` failed where jq writes. `jq-nested-fold-source-sweep.py` has no handler
that hands the state on, so it cannot gate this: the grid here is outer source x INIT x handler x `$x` bind x
source (pattern, wrapper) x nested fold shape x write context x document.

Each row is judged against jq (`/usr/bin/jq`) on stdout, exit code and the first line of stderr, for two builds:

  ok            both builds match jq
  fixed         NEW matches jq, BASE did not
  REGRESS       BASE matched jq, NEW does not (a lost match)
  still_wrong   neither matches, and they agree with each other
  changed_wrong neither matches, and they disagree: the change moved a wrong answer

Exit status 1 when any row is REGRESS, when NEW answers (exit 0) a row jq refuses and BASE did not (a newly
silent accept: `del`/`|=`/`=` write through it), or when NEW times out. A row jq itself times out on is skipped.
`changed_wrong` alone does not fail the run: read it. `still_wrong` was large when this grid was written, since
both builds worded many of jq's errors differently; on `main` at #5552 every row of the default draw matches jq, message
included. `--jq-gate` also fails the run on any row where NEW does not match jq (stdout, exit code and message):
with NEW passed as BASE too, that is the gate against jq alone, and it is how `succinctly dev compare jq
--dimension sweeps` runs this script (#5552). No residual is documented, so a residual that becomes
accepted has to be listed here first.

Usage: scripts/jq-foreach-update-state-register-sweep.py BASE NEW [N [SEED]] [--show N] [--dump FILE] [--jobs N] [--jq JQ]
  BASE, NEW  succinctly builds (the change's parent, normally `main`, and the build under test)
  N          rows to draw (default 6000; duplicates collapse, so fewer are run); about 3 spawns a row
  SEED       the draw's seed (default 1); a given N and SEED is the same row set on every machine
  --dump     write every row that is not `ok` (row, jq, base, new) as JSON to FILE
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
    '{"a":{"a":null}}',
    '{"a":null}',
    '{"a":{"a":null},"b":1}',
    '{"a":{"b":1}}',
    '{"a":[1]}',
    "null",
    '{"a":false}',
    '{"a":{"a":{"a":1}}}',
    '[{"a":1}]',
    '{"a":{"c":null}}',
    '{"a":{"a":{"c":1}}}',
    '{"a":{"a":1},"b":{"a":1}}',
]
# what `$x` is bound to
BINDS = [". as $x", ". as $x", ".a as $x", ".b as $x", "(.a // .) as $x"]
# the outer fold's source: the root is the register on the steps that emit `.`
OUTERS = ["(.,.)", "(.,.,.a)", "(.,.a)", "(.a,.)", "(.,.a,.)", ".", ".a", "(.,.,.)", "(.a,.a)", "(.,1,.)"]
INITS = ["0", "null", "[]", ".", "false", "1"]
# what the handler of the `try` around the nested fold hands on as the next state
HANDLERS = ["$i", "$i", "$x", ".", "null", "0", "$i | .a?", "($i, $i)", "($i, 0)", "($x, $i)"]
# the nested fold's source: a destructure of `$x`, behind no wrapper, a `try` or a `?`
PATTERNS = [
    "$x as {a:$q} | .",
    "$x as {a:$q} | .",
    "$x as {a:{a:$b}} | .",
    "$x as {a:{c:$b}} | .",
    "$x as [$q] | .",
    "$x as {a:$q,b:$r} | .",
    "$x as {b:$q} | .",
    "$x | . as {a:$q} | .",
    "$x as {a:$q} | $q",
    "$x as {a:$q} | $i",
    "$x as {a:$q} ?// [$q] | .",
]
WRAPPERS = ["{P}", "{P}", "try ({P})", "({P})?"]
# the nested fold's INIT, UPDATE and EXTRACT
NESTED = [
    ("0", ".a?", "$y"),
    ("0", ".a?", "$y"),
    ("0", ".", "$y"),
    ("0", ".", "."),
    (".", ".a?", "$y"),
    ("0", "empty", "$y"),
    ("0", ".a", "."),
    ("0", ".", ".a?"),
]
# the write (or path) context; `F` is the whole fold
CONTEXTS = [
    "path(F)",
    "[path(F)?]",
    "del(F)",
    "(F) |= 9",
    'try path(F) catch "C"',
    "(F) = 9",
    # a `?`/`try` around the target swallows a catchable raise, so a raise the resolver makes and jq does not turns
    # into a write of a prefix of jq's paths (the unmasked-residual shape #5189 refused to ship)
    "del((F)?)",
    "((F)?) |= 9",
    "del(try (F) catch empty)",
]


def make_row(rng):
    # The order of these draws is the seed's meaning: reordering them changes every row set.
    bind = rng.choice(BINDS)
    outer = rng.choice(OUTERS)
    init = rng.choice(INITS)
    handler = rng.choice(HANDLERS)
    source = rng.choice(WRAPPERS).replace("{P}", rng.choice(PATTERNS))
    n_init, n_update, n_extract = rng.choice(NESTED)
    inner = f"foreach ({source}) as $y ({n_init}; {n_update}; {n_extract})"
    fold = f"{bind} | foreach {outer} as $i ({init}; try ({inner}) catch ({handler}); .)"
    program = rng.choice(CONTEXTS).replace("F", fold)
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
    print(f"NEW timeouts: {new_timeouts}; newly accepted where jq refuses: {new_accepts}")
    if args.dump:
        with open(args.dump, "w") as out:
            json.dump(kept, out)
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
