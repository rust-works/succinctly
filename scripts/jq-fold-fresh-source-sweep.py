#!/usr/bin/env python3
"""Differential sweep for `yields_only_fresh_values` (#3489, #3745).

A destructuring bind over a *fresh* source in a `reduce`/`foreach` source is
path-tracked by jq, so its pattern step raises; a source that merely preserves jq's
register (`select(true)`, `first(.)`, `.`, ...) is destructured without a refusal.
`yields_only_fresh_values` (src/jq/eval.rs) routes the first group through the
resolver and leaves the second to the by-value drive. This sweep compares
`stdout` + exit code against jq for every source x pattern x form x document, and
fails when

- a source the allow-list names (FRESH) differs from jq: the list is wrong, or the
  resolver is, for it; or
- a register-preserving control (CONTROL) differs from jq where `--base` (a build
  without the change, normally `main`) agrees with jq: a regression.

Usage: scripts/jq-fold-fresh-source-sweep.py [--bin BIN] [--base BIN] [--jq JQ]
"""
import argparse
import collections
import concurrent.futures
import subprocess
import sys

DOCS = ['{"a":1}', '[1,2]', '[]', '{}', 'null', '[[1],[2]]', '{"a":[1]}', '"s"',
        '[{"a":1}]', '[[1]]', '[1]', '{"a":{"b":1}}']
FRESH = ['keys', 'keys_unsorted', 'to_entries', 'map(.)', 'paths', 'paths(true)',
         'flatten', 'flatten(1)', 'flatten(0)', 'tostream', 'sort', 'sort_by(.)',
         'reverse', 'group_by(.)', 'unique', 'unique_by(.)', 'transpose',
         '(keys | .)', '(to_entries | . | .)']
CONTROL = ['select(true)', 'first(.)', '(.|.)', 'limit(1; .)', '.', 'getpath([])',
           'add', 'first', 'last', '.[0]?', 'tostring', '[.]', '([.] | add)',
           '[.[]?]', 'values', 'arrays', 'objects', '.[]?', 'min', 'max',
           'min_by(.)', 'max_by(.)', '(.[]? | select(true))']
PATTERNS = ['[$a]', '{a:$a}', '[$a,$b]']
FORMS = [
    'path(reduce ({s} as {p} | $a) as $k (.; .))',
    'path(foreach ({s} as {p} | $a) as $k (.; .; .))',
    'del(reduce ({s} as {p} | $a) as $k (.; .))',
    '(reduce ({s} as {p} | $a) as $k (.; .)) |= 5',
    '(foreach ({s} as {p} | $a) as $k (.; .; .)) = 5',
    'path(reduce ({s} as {p} | 1) as $k (.; .))',
]


def run(cmd, query, doc):
    try:
        r = subprocess.run(cmd + ["-c", query], input=doc + "\n", capture_output=True,
                           text=True, timeout=20)
    except subprocess.TimeoutExpired:
        return ("TIMEOUT", -1)
    return (r.stdout, r.returncode)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default="target/debug/succinctly")
    ap.add_argument("--base")
    ap.add_argument("--jq", default="jq")
    ap.add_argument("--jobs", type=int, default=8)
    args = ap.parse_args()
    cand = [args.bin, "jq"]
    base = [args.base, "jq"] if args.base else None
    rows = [(group, s, p, f, d)
            for group, srcs in (("FRESH", FRESH), ("CONTROL", CONTROL))
            for s in srcs for p in PATTERNS for f in FORMS for d in DOCS]

    def one(row):
        group, s, p, f, d = row
        q = f.format(s=s, p=p)
        j = run([args.jq], q, d)
        c = run(cand, q, d)
        b = run(base, q, d) if base else None
        return row, q, j, c, b

    tally = collections.Counter()
    failures = []
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        for row, q, j, c, b in pool.map(one, rows):
            group = row[0]
            agree = c == j
            tally[(group, "match" if agree else "differ")] += 1
            if group == "FRESH" and not agree:
                failures.append(("FRESH differs from jq", q, row[4], j, c))
            if group == "CONTROL" and not agree and b is not None and b == j:
                failures.append(("CONTROL regressed", q, row[4], j, c))
    for k, v in sorted(tally.items()):
        print(k, v)
    print(f"failures: {len(failures)}")
    for f in failures[:20]:
        print(f)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
