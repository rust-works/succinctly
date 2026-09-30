#!/usr/bin/env python3
"""Oracle sweep for jq assignment's right-hand side and target ordering (#3448).

Cross-products path x operator x right-hand side x consumer, runs every program
under `succinctly jq -nc` and under the pinned `/usr/bin/jq` with stdin
"1 2 3 4 5 6" (so `input` in the target or the right side is observable), and
compares (stdout, `debug` trace lines, exit code). Error *messages* are not
compared: their wording is tracked elsewhere, and the exit code says whether
both raised.

The point of the sweep is the shapes a hand-picked matrix misses: a consumer
that stops early (`first`, `limit`, `isempty`) over a right side or a target
with a side effect, and the per-output re-resolution of the target. On the
eager route it replaced, 1,215 of 5,940 cases diverged; the lazy generator has
none.

usage:
    cargo build --features cli
    scripts/jq-assign-rhs-oracle-sweep.py [--show N] [binary]

`binary` defaults to target/debug/succinctly. Every divergence is written to
`.ai/scratch/assign-rhs-divergences.txt`; the first N (default 10) are echoed.
Exit status is 1 if there is any divergence.
"""
import itertools
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

ORACLE = "/usr/bin/jq"
STDIN = b"1 2 3 4 5 6"
DOC = '{"a":1,"b":[1,2,3]}'

PATHS = [
    ".a",
    ".b[0]",
    ".b[]",
    '.[("a","b")]',
    '(.|debug("P"))["a"]',
    '.[(.|debug("P")|"a")]',
    ".b[(0,1)]",
    "(.a,.b[0])",
    '.[(input|tostring|"a")]',
    '.b[(0,(1|debug("P")))]',
]
OPS = ["=", "|=", "+=", "-=", "*=", "//="]
RHS = [
    "1",
    "(1,2)",
    '(1,(2|debug("R")))',
    "(1,input)",
    "input",
    "empty",
    '(1,error("x"))',
    '(error("x"),1)',
    '(.a|debug("S"))',
    '((1,2)|debug("R"))',
    '((1,2,3)|debug("R"))',
]
CONSUMERS = [
    "{X}",
    "[{X}]",
    "first({X})",
    "[limit(2; {X})]",
    "isempty({X})",
    "[{X}] | length",
    'try {X} catch "c"',
    "({X})?",
    "[limit(1; {X})], input",
]


def observe(argv):
    try:
        r = subprocess.run(argv, input=STDIN, capture_output=True, timeout=10)
    except subprocess.TimeoutExpired:
        return ("TIMEOUT", "", -1)
    err = r.stderr.decode(errors="replace")
    trace = "".join(line + "\n" for line in err.splitlines() if line.startswith('["DEBUG:"'))
    return (r.stdout.decode(errors="replace"), trace, r.returncode)


def main():
    args = sys.argv[1:]
    show = 10
    if "--show" in args:
        i = args.index("--show")
        show = int(args[i + 1])
        del args[i : i + 2]
    binary = args[0] if args else "target/debug/succinctly"
    if not os.access(binary, os.X_OK):
        sys.exit(f"error: {binary} not found; run: cargo build --features cli")
    if not os.access(ORACLE, os.X_OK):
        sys.exit(f"error: pinned oracle {ORACLE} not found")

    programs = [
        cons.replace("{X}", f"{DOC} | {path} {op} {rhs}")
        for path, op, rhs, cons in itertools.product(PATHS, OPS, RHS, CONSUMERS)
    ]

    def one(program):
        return (
            program,
            observe([ORACLE, "-nc", program]),
            observe([binary, "jq", "-nc", program]),
        )

    with ThreadPoolExecutor(max_workers=8) as pool:
        results = list(pool.map(one, programs))
    bad = [(p, want, got) for p, want, got in results if want != got]
    print(f"cases={len(programs)} divergences={len(bad)}")
    os.makedirs(".ai/scratch", exist_ok=True)
    with open(".ai/scratch/assign-rhs-divergences.txt", "w") as out:
        for p, want, got in bad:
            out.write(f"{p}\n  jq  : {want!r}\n  succ: {got!r}\n")
    for p, want, got in bad[:show]:
        print(p)
        print("  jq  :", want)
        print("  succ:", got)
    sys.exit(1 if bad else 0)


main()
