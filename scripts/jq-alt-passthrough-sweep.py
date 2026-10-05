#!/usr/bin/env python3
"""Differential sweep for `?//` destructuring binds over a pass-through head (#3781).

jq takes a destructuring pattern's first step *on the register* when the bind source
passes `.` through (`select(true)`, `(.|.)`, `first(.)`, ...), so a body that
navigates under `try` is refused and caught: no paths, and `del`/`=`/`|=` leave the
document alone. `resolve_as_pattern` (src/jq/eval.rs) cannot always tell such a head
from a copy of the register, took the step's refusal for jq's, retried onto the next
alternative and delivered a tracked path. A retry that rests on that guess now refuses
a tracked output of a later alternative.

Every source x pattern x alternative x body x form x document row is run through jq,
the candidate and (optionally) a base build, and classified against jq:

  match / refuse (exit != 0 where jq answers) / ACCEPT_WRONG (exit 0 where jq refuses)
  / other (both exit 0 with different output: a wrong answer, e.g. a wrong write)

The sweep FAILS when the candidate has an ACCEPT_WRONG row the base does not, or (with
--base) a row that matched jq on base and is no longer `match` is NOT a refusal. Rows
that move from `match` to `refuse` are counted and listed: they are the documented cost
(docs/compliance/jq/limitations.md, #3781).

Usage: scripts/jq-alt-passthrough-sweep.py [--bin BIN] [--base BIN] [--jq JQ] [--jobs N]
"""
import argparse
import collections
import concurrent.futures
import subprocess
import sys

DOCS = ['{"a":{"b":1}}', '{"a":1}', '[1]', '[[1]]', 'null', '{"a":null}', '[]', '{}',
        '{"a":[1]}', '[[1],[2]]', '{"a":{"b":null}}']
SRCS = ['.', 'select(true)', 'select(.a)', '(.|.)', 'first(.)', 'limit(1; .)', '.a', '[.]',
        'keys', '{a:.}', '(. + {z:1})', '(.|select(true))', 'nth(0; .)', 'last(.)']
PATS = ['{a:$v0}', '[$v0]', '{a:{b:$v0}}', '[[$v0]]', '{a:[$v0]}']
ALTS = ['$v0', '{b:$v0}', '[$v0]']
BODIES = ['try .a', '(.a)?', '.a', '$v0', 'try $v0', '$v0.b?', 'try (.[]? | .b?)',
          'try .[0]', '.[0]?', 'empty', 'try ($v0 | .b)']
FORMS = ['del({s} as {p} ?// {alt} | {b})', '[path({s} as {p} ?// {alt} | {b})]',
         '({s} as {p} ?// {alt} | {b}) |= 5']


def run(cmd, query, doc):
    try:
        r = subprocess.run(cmd + ["-c", query], input=doc + "\n", capture_output=True,
                           text=True, timeout=20)
    except subprocess.TimeoutExpired:
        return ("TIMEOUT", -1)
    return (r.stdout, r.returncode)


def classify(got, jq):
    if got == jq:
        return "match"
    if got[1] != 0 and jq[1] == 0:
        return "refuse"
    if got[1] == 0 and jq[1] != 0:
        return "ACCEPT_WRONG"
    return "other"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default="target/debug/succinctly")
    ap.add_argument("--base")
    ap.add_argument("--jq", default="jq")
    ap.add_argument("--jobs", type=int, default=8)
    args = ap.parse_args()
    cand = [args.bin, "jq"]
    base = [args.base, "jq"] if args.base else None
    rows = [(s, p, a, b, f, d) for s in SRCS for p in PATS for a in ALTS for b in BODIES
            for f in FORMS for d in DOCS]

    def one(row):
        s, p, a, b, f, d = row
        q = f.format(s=s, p=p, alt=a, b=b)
        j = run([args.jq], q, d)
        c = run(cand, q, d)
        m = run(base, q, d) if base else None
        return q, d, j, c, m

    tally = collections.Counter()
    failures, costs = [], []
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        for q, d, j, c, m in pool.map(one, rows):
            cc = classify(c, j)
            mc = classify(m, j) if m is not None else None
            tally[(mc, cc)] += 1
            if cc == "ACCEPT_WRONG" and mc != "ACCEPT_WRONG":
                failures.append(("new accept-where-jq-refuses", q, d, j, c))
            if mc == "match" and cc != "match":
                (costs if cc == "refuse" else failures).append(
                    ("match -> " + cc, q, d, j, c))
    for k, v in sorted(tally.items(), key=str):
        print(k, v)
    print(f"match -> refuse (documented cost): {len(costs)}")
    print(f"failures: {len(failures)}")
    for f in failures[:20]:
        print(f)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
