#!/usr/bin/env python3
"""Exhaustive differential sweep for a fold SOURCE that navigates and then
computes (#2159) -- `path(foreach (.a|tostring) as $k (.; .a))`.

jq's path register is moved by every `INDEX` a `foreach` source runs, and a
non-navigating stage after the navigation (`tostring`, `length`, ...) only
stops it moving *further*. UPDATE/EXTRACT then navigate against that moved
register. `drive_fold_source` used to inspect only each element's final
`PathBranch.trackable`, so such an element looked like a literal that never
touched the register (#2159); `MovedRegister` (`src/jq/eval.rs`) now carries
where it went.

Every case is `path(F)` and `(F) = 9`, over 7 documents (a `null`, `true`,
nested and array-valued ones -- `null`/`bool` are `jv_identical` to a
same-kind register wherever it sits, which is what makes the *positive* face
of the bug reachable), for `foreach` and `reduce`, and is run through the
pinned oracle and the built binary, then classified by direction:

    same              same exit, same stdout
    FABRICATE         jq refuses, succinctly answers -- what `=`/`del()` write through
    REJECT            jq answers, succinctly refuses -- safe, reported as a count
    both-ok-differ    both answer, differently
    both-error-differ both refuse, with different stdout

**The alphabet is part of the claim** (#2041): 92 source shapes -- every
navigation x every non-navigating tail, plus the shapes the fix must NOT
change (a literal, an `as` source, an untaken navigating branch). Run
`--self-test` to print the pools.

Two-sided staleness gate, as the other oracle sweeps have: the run fails on
any FABRICATE, and on any divergence outside `KNOWN_RESIDUAL_SOURCES` -- the
sources whose remaining divergences are tracked (#3459: an opaque builtin
after the navigation; #3460: pointer identity). A residual source that
*stops* diverging also fails, so the list cannot go stale.

Usage:
    cargo build --release --features cli
    ./scripts/jq-fold-source-register-sweep.py [--bin PATH] [--jq PATH] [--show K] [--self-test]
    (60k cases, two process spawns each: minutes, not seconds -- not run in CI.)
Exit 1 on any FABRICATE, any unexpected divergence, or a stale residual entry.
"""
import argparse, collections, concurrent.futures as cf, itertools, re, subprocess, sys

INPUTS = [
    '{"a":1,"b":{"c":2}}', '{"a":null,"b":null}', '{"a":[1,2],"b":"s"}', '[1,2]', 'null',
    '{"a":{"a":1},"b":{"c":2}}', '{"a":true,"b":true}',
]
NAV = ['.a', '.b', '.a.c', '.[]', '.a?', '.[0]', '(.a,.b)', '.a[]?']
TAIL = ['tostring', 'length', 'keys', 'type', 'tojson', '1', '[.]|.[0]', 'not', 'first', '(.,.)']
SRCS = ['1', '.a', '.[]'] + [f'{n}|{t}' for n in NAV for t in TAIL]
SRCS += [
    '(.a|tostring)|.', '.a|tostring|tonumber?', 'if true then (.a|length) else 1 end',
    '.a as $x | 1', '(.a|tostring), 1', '1, (.a|tostring)', 'first(.a)', 'add?', 'empty',
]
UPDS = ['.', '.a', '.b', '.[0]', '$k', '1']
INITS = ['.', '.b', '1', 'null']
KINDS = ['foreach', 'reduce']
FORMS = ['path({k} ({s}) as $k ({i}; {u}))', '({k} ({s}) as $k ({i}; {u})) = 9']

# Sources whose remaining divergences are tracked, not fixed. `first`/`add?`
# index inside the builtin, so the register's position is unknowable to the
# resolver (#3459); the `tostring` rows are jq's pointer identity on a string
# and on the accumulator (#3460).
KNOWN_RESIDUAL_SOURCES = {
    '.a|first', '.b|first', '.a.c|first', '.[0]|first', '.[]|first', '.a?|first',
    '(.a,.b)|first', 'add?', '.b|tostring', '1, (.a|tostring)',
}


def run(cmd, inp):
    try:
        p = subprocess.run(cmd, input=inp, capture_output=True, text=True, timeout=20)
        return p.returncode, p.stdout
    except Exception as e:  # noqa: BLE001 -- a timeout is reported, not fatal
        return -9, str(e)


def source_of(program):
    return re.search(r'(?:foreach|reduce) \((.*?)\) as \$k', program).group(1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--bin', default='./target/release/succinctly')
    ap.add_argument('--jq', default='/usr/bin/jq')
    ap.add_argument('--show', type=int, default=3)
    ap.add_argument('--self-test', action='store_true')
    args = ap.parse_args()
    cases = [
        (form.format(k=k, s=s, i=i, u=u), inp)
        for k, s, u, i in itertools.product(KINDS, SRCS, UPDS, INITS)
        for form in FORMS
        for inp in INPUTS
    ]
    if args.self_test:
        print(f'{len(SRCS)} sources, {len(UPDS)} updates, {len(INITS)} inits, '
              f'{len(INPUTS)} documents, {len(cases)} cases')
        for s in SRCS:
            print('  ', s)
        return 0

    def one(case):
        f, inp = case
        j = run([args.jq, '-c', f], inp)
        s = run([args.bin, 'jq', '-c', f], inp)
        if -9 in (j[0], s[0]):
            return 'timeout', f, inp, j, s
        if j == s:
            return 'same', f, inp, j, s
        if j[0] != 0 and s[0] == 0:
            return 'FABRICATE', f, inp, j, s
        if j[0] == 0 and s[0] != 0:
            return 'REJECT', f, inp, j, s
        return ('both-error-differ' if j[0] != 0 else 'both-ok-differ'), f, inp, j, s

    counts, diffs = collections.Counter(), []
    with cf.ThreadPoolExecutor(8) as ex:
        for r in ex.map(one, cases, chunksize=32):
            counts[r[0]] += 1
            if r[0] not in ('same', 'timeout'):
                diffs.append(r)
    print(f'=== {len(cases)} cases ===')
    for k, v in counts.most_common():
        print(f'  {v:6}  {k}')

    residual_seen = set()
    bad = []
    for cat, f, inp, j, s in diffs:
        src = source_of(f)
        if cat == 'FABRICATE' or src not in KNOWN_RESIDUAL_SOURCES:
            bad.append((cat, f, inp, j, s))
        else:
            residual_seen.add(src)
    stale = KNOWN_RESIDUAL_SOURCES - residual_seen
    for cat, f, inp, j, s in bad[:args.show]:
        print(f'[{cat}] {f}   in={inp}\n   jq({j[0]})=[{j[1].strip()}]  succinctly({s[0]})=[{s[1].strip()}]')
    if counts['timeout']:
        print(f'  ({counts["timeout"]} timed out)')
    if bad:
        print(f'FAIL: {len(bad)} FABRICATE / unexpected divergences', file=sys.stderr)
    if stale:
        print(f'FAIL: residual sources no longer diverge (remove from KNOWN_RESIDUAL_SOURCES): '
              f'{sorted(stale)}', file=sys.stderr)
    return 1 if bad or stale else 0


if __name__ == '__main__':
    sys.exit(main())
