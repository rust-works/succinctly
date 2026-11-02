#!/usr/bin/env python3
"""Differential sweep for a `foreach` that is a fold's whole UPDATE and reads the outer
fold's frozen `$var` (#4499).

`foreach .a as $w (.; foreach (1,2) as $i (1; try ($w|.b); .); .)`: jq's path register sits
on `.a` (the outer SOURCE navigated there and nothing since moved it), `$w` is that node, so
the inner step navigates it and the paths are `["a","b"]`. The inner fold's register is not
the one it tracks, and #3984 withholds it from a fold that mentions a marker, so the row
refused. The inner fold now reads the enclosing fold's register, stated by value and
position, when it is that fold's *whole* UPDATE (`FoldRegister::positioned`).

The grid is the shape's neighbours, not only the shape: outer kind x outer source x inner
INIT/source/UPDATE/EXTRACT x write context x document, so a variant that must stay refused
(EXTRACT-nested, a comma beside the fold, a moving INIT or source, a reduce inside) is
judged against jq too. Rows are classified against jq (`/usr/bin/jq`) per build, as
`jq-path-register-sweep.py` does: MATCH, ACCEPT_WRONG (jq refused, the build answered: the
dangerous one), REFUSE_WRONG (the safe direction) and DIFF. Exit status 1 when the candidate
has an ACCEPT_WRONG or DIFF row the `--base` build did not answer identically, or lost a match
the `--base` build had (without `--base` every such row counts).

Usage: scripts/jq-nested-fold-outer-var-sweep.py --bin BIN [--base BIN] [--jq JQ] [--show N]
"""
import argparse
import collections
import concurrent.futures
import os
import subprocess
import sys

OUTERS = [
    "foreach .a as $w (.; {inner}; .)",
    "foreach .a as $w (.; {inner}; {inner_x})",
    "foreach .a as $w (0; {inner}; .)",
    "foreach .a? as $w (.; {inner}; .)",
    "foreach .a as $w (.; ({inner}, 1); .)",
    "foreach .a as $w (.; ({inner}) | .; .)",
    "foreach (.a, .z) as $w (.; {inner}; .)",
    "reduce .a as $w (.; {inner})",
    "reduce .a as $w (0; {inner})",
    "reduce (.a, .z) as $w (.; {inner})",
    # #4779: the positions the fold reads the register in beyond the whole UPDATE -- under a
    # `try`/`?`, as the whole EXTRACT (after an UPDATE that leaves or moves the register), and as
    # a comma item -- each next to the siblings and wrappers that must keep refusing.
    "foreach .a as $w (.; try ({inner}); .)",
    "foreach .a as $w (.; ({inner})?; .)",
    "foreach .a as $w (.; try ({inner}) catch 5; .)",
    "foreach .a as $w (.; try ({inner}) catch $w; .)",
    "foreach .a as $w (.; try ({inner}) catch .; .)",
    "foreach .a as $w (.; ({inner}) | .; {inner})",
    "foreach .a as $w (.; .; {inner})",
    "foreach .a as $w (.; .b?; {inner})",
    "foreach .a as $w (.; .b; {inner})",
    "foreach .a as $w (.; 5; {inner})",
    "foreach .a as $w (.; $w; {inner})",
    "foreach .a as $w (.; $w|.b; {inner})",
    "foreach .a as $w (.; try ($w|.b); {inner})",
    "foreach .a as $w (0; .; {inner})",
    "foreach .a as $w (.; foreach (1,2) as $j (1; try ($w|.b); .); {inner})",
    "foreach (.a, .z) as $w (.; .; {inner})",
    "foreach .a? as $w (.; .; {inner})",
    "foreach .a as $w (.; .; try ({inner}))",
    "foreach .a as $w (.; .; ({inner})?)",
    "foreach .a as $w (.; .; ({inner}, 1))",
    "foreach .a as $w (.; .; try ({inner}) catch 5)",
    "foreach .a as $w (.; ({inner}, .b); .)",
    "foreach .a as $w (.; (.b, {inner}); .)",
    "foreach .a as $w (.; (.b?, {inner}); .)",
    "foreach .a as $w (.; ({inner}, $w|.b); .)",
    "foreach .a as $w (.; ({inner}, ($w|.c)?); .)",
    "foreach .a as $w (.; ($w[0]?, {inner}); .)",
    "foreach .a as $w (.; (1, {inner}); .)",
    "foreach .a as $w (.; ([{inner}] | .[]?); .)",
    "foreach .a as $w (.; ({inner}, error); .)",
    "foreach .a as $w (.; try ({inner}, error); .)",
    "foreach .a as $w (.; ({inner}, empty); .)",
    "foreach .a as $w (.; ({inner}, {inner}); .)",
    "foreach .a as $w (.; ({inner}) // .b; .)",
    "foreach .a as $w (.; .; ({inner}, .b))",
    "foreach .a as $w (.; .b?; ({inner}, $w))",
    "foreach .a as $w (.; label $out | ({inner}, break $out); .)",
    "reduce .a as $w (.; try ({inner}))",
    "reduce .a as $w (0; try ({inner}))",
    "reduce .a as $w (.; ({inner}, .b))",
    "reduce .a as $w (.; (.b?, {inner}))",
    "reduce .a as $w (0; ({inner}, 1))",
    "reduce (.a, .z) as $w (.; try ({inner}, .b?))",
]
INNERS = [
    "foreach (1,2) as $i (1; try ($w|.b); .)",
    "foreach (1,2) as $i (1; $w|.b; .)",
    "foreach (1,2) as $i (1; try ($w|.c); .)",
    "foreach (1,2) as $i (1; try ($w|.b?); .)",
    "foreach (1,2) as $i (1; try ($w|.b); $w)",
    "foreach (1,2) as $i (1; .; try ($w|.b))",
    "foreach (1,2) as $i (1; try ($w|.b), 5; .)",
    "foreach (1,2) as $i ($w; .b; .)",
    "foreach (1,2) as $i ($w; try .b; .)",
    "foreach (1,2) as $i (.b; try ($w|.b); .)",
    "foreach .b? as $i (1; try ($w|.b); .)",
    "foreach (1,2) as $i (1; try (.b | $w); .)",
    "foreach (1,2) as $i (1; try ($w|.b|.c); .)",
    "foreach (1,2) as $i (null; try ($w|.b); .)",
    "foreach (1,2) as $i (1; try ($w|.b) // 7; .)",
    "foreach (1,2) as $i (1; reduce (1,2) as $j (1; try ($w|.b)); .)",
    "reduce (1,2) as $i (1; try ($w|.b))",
    "foreach (1,2) as $i (1; try ($w|.b); foreach (1,2) as $j (1; try ($w|.b); .))",
]
DOCS = [
    '{"a":{"b":1,"c":[1,2]},"z":{"b":3}}',
    '{"a":{"b":null},"z":0}',
    '{"a":[1,2],"z":0}',
    '{"a":null,"z":0}',
]
CONTEXTS = ["path({e})", "[path({e})]", "del({e})", "({e}) = 9", "({e}) |= 9"]


def programs():
    for outer in OUTERS:
        for inner in INNERS:
            body = outer.replace("{inner_x}", inner).replace("{inner}", inner)
            for context in CONTEXTS:
                yield context.replace("{e}", body)


def run(argv, program, doc):
    try:
        p = subprocess.run(argv + ["-c", program], input=doc, capture_output=True,
                           text=True, timeout=20)
    except subprocess.TimeoutExpired:
        return ("TIMEOUT", -1)
    return (p.stdout, p.returncode)


def classify(got, oracle):
    if got == oracle:
        return "MATCH"
    if oracle[1] != 0 and got[1] == 0:
        return "ACCEPT_WRONG"
    if oracle[1] == 0 and got[1] != 0:
        return "REFUSE_WRONG"
    return "DIFF"


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--bin", required=True, help="the build under test")
    ap.add_argument("--base", help="a build without the change, normally main")
    ap.add_argument("--jq", default="/usr/bin/jq")
    ap.add_argument("--show", type=int, default=10)
    ap.add_argument("--jobs", type=int, default=8)
    args = ap.parse_args()
    for build in (args.bin, args.base):
        if build and not os.access(build, os.X_OK):
            print(f"not an executable build: {build}", file=sys.stderr)
            sys.exit(2)

    rows = [(doc, prog) for prog in programs() for doc in DOCS]

    def one(row):
        doc, prog = row
        oracle = run([args.jq], prog, doc)
        cand = run([args.bin, "jq"], prog, doc)
        base = run([args.base, "jq"], prog, doc) if args.base else None
        return row, oracle, cand, base

    counts = collections.Counter()
    bad, inherited, lost, fixed = [], [], [], []
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        for row, oracle, cand, base in pool.map(one, rows):
            if oracle[0] == "TIMEOUT":
                continue
            c = classify(cand, oracle)
            counts[c] += 1
            if c in ("ACCEPT_WRONG", "DIFF"):
                (bad if cand != base else inherited).append((row, oracle, cand, base))
            if base is not None:
                b = classify(base, oracle)
                if b == "MATCH" and c != "MATCH":
                    lost.append((row, oracle, cand, base))
                if b != "MATCH" and c == "MATCH":
                    fixed.append((row, oracle, cand, base))

    def show(title, items):
        print(f"{title}: {len(items)}")
        for (doc, prog), oracle, cand, base in items[: args.show]:
            print(f"  {doc} {prog}\n    jq={oracle} cand={cand} base={base}")

    print(f"{len(rows)} rows; candidate: {dict(counts)}")
    show("candidate ACCEPT_WRONG/DIFF the base did not have", bad)
    print(f"candidate ACCEPT_WRONG/DIFF identical to the base's: {len(inherited)}")
    show("lost a match vs base", lost)
    print(f"flipped to MATCH vs base: {len(fixed)}")
    return 1 if bad or lost else 0


if __name__ == "__main__":
    sys.exit(main())
