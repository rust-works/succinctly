#!/usr/bin/env python3
"""Differential sweep for a nested fold over a frozen `$var` in a `try` (#3984).

A `reduce`/`foreach` that mentions a frozen `$v` inside a `try`, in a fold's EXTRACT or
after a stage that leaves jq's path register where it was (`length`, a literal), is
judged against a register the fold does not have. jq finds the first navigation of `$v`
intact (its error, where it raises, comes later, outside the `try`), so a refusal
caught by the `try` drops the write or path jq makes. `FoldRegister::withhold_untracked_register`
makes that refusal loud. This is the grid #3984 found no committed sweep had: sibling
x nested fold x write context x accumulator x document, plus the same bodies behind a
`. as $x | SIBLING | BODY` pipe stage.

Rows are classified against jq (`/usr/bin/jq`) per build, as `jq-path-register-sweep.py`
does: MATCH, ACCEPT_WRONG (jq refused, the build answered: the dangerous one),
REFUSE_WRONG (the safe direction) and DIFF. Exit status 1 when the candidate has any
ACCEPT_WRONG or DIFF row, or any row that is worse than `--base`'s.

Usage: scripts/jq-nested-fold-frozen-var-sweep.py --bin BIN [--base BIN] [--jq JQ] [--show N]
"""
import argparse
import collections
import concurrent.futures
import subprocess
import sys

SIBLINGS = ["1", "length", '"s"', ".b", "$v", "($v|.b?)", "null", "true", "(.|.)", "[1]", "{}"]
NESTED = [
    "try (foreach 1 as $i ($v; .; .b))",
    "try (reduce 1 as $i ($v; .b))",
    "try (reduce $v as $i (0; $i.b))",
    "try (foreach 1 as $i (0; .; $v | .b))",
    "try (reduce 1 as $i (0; $v | .b))",
    "try (foreach 1 as $i ($v; .; .))",
    "try ($v | .b?)",
    "try (foreach 1 as $i ($v; .; length))",
    "try (foreach 1 as $i (0; .; .b))",
]
DOCS = ['{"a":{"b":1,"c":[1,2]},"z":0}', '{"a":{"b":null},"z":0}', '{"a":[1,2],"z":0}']
CONTEXTS = ["del({e})", "({e}) = 9", "[path({e})]"]
ACCUMULATORS = ["[3,1,2]", "0"]


def programs():
    for sibling in SIBLINGS:
        for nested in NESTED:
            for context in CONTEXTS:
                for acc in ACCUMULATORS:
                    body = f"foreach .a as $v ({acc}; (($v | .b?), {sibling}); {nested})"
                    yield context.format(e=body)
                # The same shapes behind a pipe stage that leaves the register in place.
                stage = f". as $x | {sibling.replace('$v', '$x')} | {nested.replace('$v', '$x')}"
                yield context.format(e=stage)


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
    ap.add_argument("--show", type=int, default=6)
    ap.add_argument("--jobs", type=int, default=6)
    args = ap.parse_args()
    cand = [args.bin, "jq"]
    base = [args.base, "jq"] if args.base else None
    rows = [(p, d) for p in programs() for d in DOCS]

    def evaluate(row):
        program, doc = row
        oracle = run([args.jq], program, doc)
        if oracle[0] == "TIMEOUT":
            return None
        return (program, doc, oracle, run(cand, program, doc),
                run(base, program, doc) if base else None)

    totals = collections.Counter()
    worse, bad = [], []
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        for rec in pool.map(evaluate, rows):
            if rec is None:
                continue
            program, doc, oracle, got, was = rec
            c = classify(got, oracle)
            totals[("candidate", c)] += 1
            if c in ("ACCEPT_WRONG", "DIFF"):
                bad.append(rec)
            if was is not None:
                b = classify(was, oracle)
                totals[("base", b)] += 1
                # A lost match, even in the safe direction, is reported; a row that became
                # dangerous is a failure.
                if b == "MATCH" and c != "MATCH":
                    worse.append(rec)
    print(f"{len(rows)} rows")
    for key in sorted(totals):
        print(f"  {key[0]:9} {key[1]:13} {totals[key]}")
    for title, recs in (("candidate ACCEPT_WRONG/DIFF", bad), ("lost a match vs base", worse)):
        print(f"{title}: {len(recs)}")
        for program, doc, oracle, got, was in recs[:args.show]:
            print(f"  {doc} {program}\n    jq={oracle} cand={got} base={was}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
