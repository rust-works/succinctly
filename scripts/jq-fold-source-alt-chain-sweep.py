#!/usr/bin/env python3
"""Differential sweep for a `?//` chain over a fresh value in a fold SOURCE (#3651).

In `path()` a destructuring pattern step is a tracked index, and a freshly built
value is never the register's node, so jq's `[[1]] as [$x] ?// $y | $x` raises in
alternative one and runs `$y`. `routes_fresh_destructuring` and `resolve_as_pattern`'s
`fresh_head` (src/jq/eval.rs) route such a chain through the resolver so
succinctly runs the same alternative. This sweep crosses

    source x `?//` chain x consumer/fold form x document

(about 4,300 rows) through jq and the candidate, and fails when a row differs from
jq on stdout, stderr (as a set of lines: a body's side-effect count is the claim)
or exit code. With `--base` (a build without the change, normally `main`) it also
reports how many rows the change fixed and fails on a regression: a row the base
matched and the candidate does not.

Usage: scripts/jq-fold-source-alt-chain-sweep.py --bin BIN [--base BIN] [--jq JQ]
"""
import argparse
import collections
import concurrent.futures
import re
import subprocess
import sys

SRCS = ['[[1]]', '[1]', '{"a":1}', '([1],[2])', '([[1]],5)', 'keys', '"s"', '1', '[]',
        '({"a":1},[1])']
CHAINS = [('[$x] ?// $y', ['x', 'y']), ('{a:$x} ?// $y', ['x', 'y']),
          ('[$x] ?// [$y]', ['x', 'y']), ('[$x] ?// [$y] ?// $z', ['x', 'y', 'z']),
          ('$x ?// $y', ['x', 'y']), ('[[$x]] ?// $y', ['x', 'y']),
          ('[$x] ?// {a:$y}', ['x', 'y']), ('$y ?// [$x]', ['x', 'y']),
          ('[$x] ?// $y ?// [$z]', ['x', 'y', 'z'])]
# UPDATE reads `$w` (the source's output), so which alternative bound is visible.
UPDATE = 'if ($w|tojson|length) > 6 then .a else .b end'
FORMS = [
    '[path(foreach ({s} as {p} | {b}) as $w (.; {u}))]',
    '[first(path(foreach ({s} as {p} | {b}) as $w (.; {u})))]',
    'first(path(foreach 1 as $v (.; foreach ({s} as {p} | {b}) as $w (.; {u}))))',
    '[limit(2; path(foreach ({s} as {p} | {b}) as $w (.; {u}; .)))]',
    '[path(reduce ({s} as {p} | {b}) as $w (.; {u}))]',
    'del(first(foreach ({s} as {p} | {b}) as $w (.; {u})))',
    'first(foreach 1 as $v (.; foreach ({s} as {p} | {b}) as $w (.; {u}))) = 5',
    '[first(path(foreach ({s} as {p} | {b}) as $w ((.a, .b); {u})))]',
    '[path(foreach ({s} as {p} | {b}) as $w (.a; .))]',
    '[path(foreach ({s} as {p} | ("S"|stderr|empty), {b}) as $w (.; {u}))]',
    '[first(path(foreach ({s} as {p} | ("S"|stderr|empty), {b}) as $w (.; {u})))]',
    'path(reduce ({s} as {p} | {b}) as $w (.[]?; .))',
]
DOCS = ['{"a":{"a":1,"b":{"c":2}},"b":[1,2,3]}', '[[1],[2]]', 'null', '{"a":null,"b":[]}']


def rows():
    for s in SRCS:
        for p, names in CHAINS:
            body = '[' + ','.join('$' + n for n in names) + ']'
            for f in FORMS:
                for d in DOCS:
                    yield f.format(s=s, p=p, b=body, u=UPDATE), d


def run(cmd, query, doc):
    try:
        r = subprocess.run(cmd + ['-c', query], input=doc + '\n', capture_output=True,
                           text=True, timeout=20)
    except subprocess.TimeoutExpired:
        return ('TIMEOUT',)
    err = re.sub(r' \(at <stdin>:\d+\)', '', r.stderr).replace('(not a string)', '')
    return (r.stdout, sorted(err.splitlines()), r.returncode)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--bin', default='target/debug/succinctly')
    ap.add_argument('--base')
    ap.add_argument('--jq', default='/usr/bin/jq')
    ap.add_argument('--jobs', type=int, default=8)
    args = ap.parse_args()

    def one(row):
        q, d = row
        return (row, run([args.jq], q, d), run([args.bin, 'jq'], q, d),
                run([args.base, 'jq'], q, d) if args.base else None)

    tally = collections.Counter()
    failures = []
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        for row, j, c, b in pool.map(one, list(rows())):
            tally['match' if c == j else 'differ'] += 1
            if c != j:
                failures.append(('differs from jq', row, j, c))
            if b is not None:
                if b != j:
                    tally['base differed'] += 1
                if c != j and b == j:
                    failures.append(('regressed against base', row, j, c))
    print(dict(tally))
    print(f'failures: {len(failures)}')
    for f in failures[:20]:
        print(f)
    return 1 if failures else 0


if __name__ == '__main__':
    sys.exit(main())
