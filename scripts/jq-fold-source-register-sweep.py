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

**The alphabet is part of the claim** (#2041): 124 source shapes -- every
navigation x every non-navigating tail, the builtins that navigate without
spelling an INDEX (`..`, `recurse`, `getpath`, `walk`), slices, computed
index/slice keys (`.[1+1]`), a nested fold, and the shapes the fix must NOT
change (a literal, an `as` source, an untaken navigating branch). Every
fold binds a bare `$k`: destructuring patterns and `?//` chains are not in
the alphabet. Run `--self-test` to print the pools.

Two-sided staleness gate, as the other oracle sweeps have: the run fails on
any divergence outside `KNOWN_RESIDUALS` -- the (source, category) pairs whose
remaining divergences are recorded (#3460: pointer identity; a nested navigating
`foreach`, recorded in docs/compliance/jq/limitations.md) -- and on any recorded pair that
*stops* occurring, so the table cannot go stale. `--print-residuals` prints
the observed table for regenerating it after a deliberate change.

Usage:
    cargo build --release --features cli
    ./scripts/jq-fold-source-register-sweep.py [--bin PATH] [--jq PATH] [--show K] [--jobs N] [--self-test]
    (~80k cases, two process spawns each: tens of minutes at the default --jobs, not seconds --
    not run in CI.)
Exit 1 on any unexpected divergence or a stale residual entry.
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
# Sources that navigate without spelling an INDEX (the review of #2159 found
# `..|tostring` fabricating while the alphabet above could not emit it): the
# recursion/`getpath`/`walk` builtins, the bare `first`/`last`/`nth`/`map`
# builtins that index inside themselves, slices (a full slice is the same
# `jv`, a partial one is not), and controls that must not change.
SRCS += [
    '..|tostring', 'recurse|tostring', 'recurse(.[]?)|length', '..', 'recurse',
    'getpath(["a"])|tostring', 'getpath(["b","c"])|type', 'getpath(["a"])', 'walk(.)|tostring',
    'first|tostring', 'last|tostring', 'nth(0)|tostring', 'first', 'last', 'map(1)|length',
    'nth(1)', 'nth(0,1)|tostring', 'flatten|length', 'add|length', 'map(.)|length', 'map(.a?)|length',
    '.[0:1]|tostring', '.[0:]|tostring', '.a[0:1]?|length', '.[1:]|type', '.[0:2]',
    'range(2)|tostring', 'paths|tostring', 'keys|.[0]', 'to_entries|length', 'tojson|fromjson',
]
# Computed spellings of an INDEX (`.[1+1]`, `.["a"|ascii_downcase]`, `.[0:(1+1)]`, `.[[1]]`)
# move the register exactly as `.a`/`.[1]` do, but a key with no `.`/`.[]` of its own once
# slipped past `drive_fold_source`'s navigation gate and was driven by value (#2159). And a
# nested `foreach` emits its values off the resolver's root path whatever its own source
# navigated, so the fold reads them as literals (a tracked residual, see KNOWN_RESIDUALS).
SRCS += [
    '.[1+1]|tostring', '.["a"|ascii_downcase]|tostring', '.[0:(1+1)]|tostring',
    '.[[1]]|length', '.[1+1]', 'foreach .[]? as $x (0; .+1)', 'reduce .[]? as $x (0; .+1)',
]
UPDS = ['.', '.a', '.b', '.[0]', '$k', '1']
INITS = ['.', '.b', '1', 'null']
KINDS = ['foreach', 'reduce']
FORMS = ['path({k} ({s}) as $k ({i}; {u}))', '({k} ({s}) as $k ({i}; {u})) = 9']

# Divergences that remain, as `source -> the categories it may show`. Gated per
# (source, category) rather than per source, so an unrelated new divergence on
# a known source (a `both-ok-differ` where only a `REJECT` is tracked) fails
# the run instead of hiding behind the source's name. A pair that stops
# occurring fails too, so the table cannot go stale.
#
#   (#3459 closed the bare `first`/`last`/`nth`/`map`/`add`/`flatten` sources: they
#   are routed through the resolver like any other navigation, so none of them is
#   listed -- the sweep has no FABRICATE row left.)
#   `.b|tostring` -- jq's pointer identity: `tostring` of a string is the same `jv`, a
#   path-mode gap that is not specific to a fold (#3460's second half). The accumulator
#   carried across source elements, `..`/`recurse` and a full slice of the accumulator
#   are closed (#3460).
#   `recurse(.[]?)|length` -- a `both-error-differ` row: `path(foreach (recurse(.[]?)|length) as $k
#   (.; .a))` on `{"a":{"a":1},"b":{"c":2}}` raises at the second element in succinctly, one element
#   *before* jq does (a parameterised `recurse` followed by a computing stage; bare `..`/`recurse`
#   match since #3460).
#   `foreach .[]? as $x (0; .+1)` -- a nested navigating `foreach`: its values
#   come back off the resolver's root path, which `drive_fold_source` reads as
#   a lost register, so it refuses where jq may answer (it used to read as a
#   literal and FABRICATE).
KNOWN_RESIDUALS = {
    '.b|tostring': ['REJECT'],
    'foreach .[]? as $x (0; .+1)': ['REJECT'],
    'recurse(.[]?)|length': ['both-error-differ'],
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
    ap.add_argument('--jobs', type=int, default=8, help='parallel case runners')
    ap.add_argument('--self-test', action='store_true')
    ap.add_argument('--print-residuals', action='store_true',
                    help='print the observed (source -> categories) table and exit 0')
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
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        for r in ex.map(one, cases):
            counts[r[0]] += 1
            if r[0] not in ('same', 'timeout'):
                diffs.append(r)
    print(f'=== {len(cases)} cases ===')
    for k, v in counts.most_common():
        print(f'  {v:6}  {k}')

    observed = collections.defaultdict(set)
    for cat, f, inp, j, s_ in diffs:
        observed[source_of(f)].add(cat)
    if args.print_residuals:
        print('KNOWN_RESIDUALS = {')
        for src in sorted(observed):
            print(f'    {src!r}: {sorted(observed[src])!r},')
        print('}')
        return 0

    bad = [d for d in diffs if d[0] not in KNOWN_RESIDUALS.get(source_of(d[1]), ())]
    stale = sorted((src, cat) for src, cats in KNOWN_RESIDUALS.items() for cat in cats
                   if cat not in observed.get(src, ()))
    for cat, f, inp, j, s_ in bad[:args.show]:
        print(f'[{cat}] {f}   in={inp}\n   jq({j[0]})=[{j[1].strip()}]  succinctly({s_[0]})=[{s_[1].strip()}]')
    if counts['timeout']:
        print(f'  ({counts["timeout"]} timed out)')
    if bad:
        print(f'FAIL: {len(bad)} divergences outside KNOWN_RESIDUALS', file=sys.stderr)
    if stale:
        print(f'FAIL: KNOWN_RESIDUALS entries that no longer occur (remove them): {stale}',
              file=sys.stderr)
    return 1 if bad or stale else 0


if __name__ == '__main__':
    sys.exit(main())
