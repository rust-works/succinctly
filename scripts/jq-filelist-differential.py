#!/usr/bin/env python3
"""Randomised differential for a LIST of input files where one holds a fault and another cannot be
opened (#7750, #7751, #7752, #7753, #7754, #7755): `succinctly jq` against the pinned jq.

jq opens its files in turn, only when its parser has read every byte of the one before, prints
`Could not open file ...` (or `Is a directory`) when it gets there, stops its main loop one value
past such a failure, and stops at the first fault of a plain JSON text, or, under `try input`,
drops the rest of the line that held it and parses on. succinctly reads the files whole or a window
at a time, stitches a token a file ends inside onto the next one (#4305) and places each failure
by rules of its own; every bug this family has had came from a list no hand-written row had.

A text of top-level values, with a fault injected at a random byte (or none), is cut into 2-5 files
(sometimes 33-40, past the list the windowed route holds open), one or two paths that do not open
(missing, a directory, a mode-000 file) are put in the list, and each of the programs below runs on
both tools: stdout, stderr and the exit status must agree. For a run that differs, the same list
without those paths is run too (`control_same`): a difference that stays is not this family's (a
lazy read, a wording), and only the others count as `in-family`.

usage: jq-filelist-differential.py --sjq BIN [--jq /usr/bin/jq] [--cases N] [--seed S] [--jobs J]
       [--out F]        one JSON line per differing run, then the totals
       [--exhaustive]   every cut of a few small texts into two files x every failure position
       [--no-bads]      the same lists with every path opening (a control for the stitch itself)
       [--stdin P]      with probability P one of the pieces is standard input, named `-`
       [--big P]        with probability P the text is padded to 1.3-4.6 MB (the windowed routes;
                        use a release build)

Debug builds are fine for the default sizes; run it under `memcap.py` like any other succinctly run.
"""
import argparse, json, os, random, shutil, subprocess, sys, tempfile
from concurrent.futures import ProcessPoolExecutor

PROGRAMS = [
    ['-c', '.'], ['.'], ['-c', '-e', '.'], ['-r', '.'], ['-c', '[., input_filename]'],
    ['-c', 'input'], ['-c', 'try input catch "E"'], ['-c', '[., input]'],
    ['-c', '[., (try input catch "E")]'], ['-c', '[input_filename, input_line_number]'],
    ['-c', '-n', '[inputs]'], ['-c', '-n', 'input'], ['-c', '-n', 'first(inputs)'],
    ['-c', '-n', '[inputs?]'], ['-c', '-n', '[limit(50; repeat(try input catch "E"))]'],
    ['-c', '-n', 'reduce (inputs|[., input_filename]) as $x ([]; . + [$x])'],
    ['-c', '-n', '[limit(50; repeat(try (input|[., input_filename, input_line_number]) catch "E"))]'],
    ['-c', '-s', '.'], ['-c', '-s', 'length'], ['-c', '-s', 'input_filename'],
    ['-c', '-n', '-s', '[inputs]'],
    ['-c', '--stream', '.'], ['-c', '-n', '--stream', '[inputs]'], ['-c', '--seq', '.'],
    ['-R', '.'], ['-R', '-c', '-n', '[inputs]'], ['-c', '--stream-errors', '.'],
    ['-c', '-s', '--stream', '.'], ['-c', '-s', '--seq', '.'], ['-R', '-s', '.'],
    ['-c', '-s', '--stream-errors', '.'], ['-c', '-n', '--seq', '[inputs]'], ['-c', '-e', 'input'],
    ['-c', '-n', '--stream-errors', '[inputs]'], ['-c', '-n', '-R', '[limit(50; repeat(try input catch "E"))]'],
]

RECORDS = [b'{"a":%d}', b'%d', b'[%d,"s"]', b'"str%d"', b'null', b'true', b'[[%d]]', b'{"k":[%d,{"z":%d}]}']
FAULTS = [b'[x"]', b'x"', b'"', b'{"a":x"}', b'}', b',', b'nul', b']', b'tru', b'{', b'[', b'\n"', b'{"',
          b'1}', b'{"a",', b':', b'[1,', b'{0,"s"]', b'[1 2]', b'"a\tb"', b'2]', b'true}', b'[,]', b'{"a":1,}']
BADS = ['missing.json', 'dir.d', 'nope.json']
PAD_LINE = b'{"pad":12345,"s":"qqqqqqqqqqqqqqqqqqqqqqqqqqqqqq"}\n'


def make_text(rng, big=0.0):
    out = []
    for i in range(rng.randint(1, 7)):
        r = rng.choice(RECORDS)
        out.append(r.replace(b'%d', b'%d' % i, 1).replace(b'%d', b'%d' % (i + 1)))
        out.append(rng.choice([b'\n', b'\n', b' ', b'']))
    text = b''.join(out)
    if rng.random() < 0.85:
        p = rng.randint(0, len(text))
        text = text[:p] + rng.choice(FAULTS) + text[p:]
    if rng.random() < big:
        # Past the windowed routes' thresholds (1 MiB for the plain route, 4 MiB for the lazy one).
        pad = PAD_LINE * rng.choice([25000, 30000, 90000])
        text = pad + text if rng.random() < 0.5 else text + b'\n' + pad
    elif rng.random() < 0.08:
        pad = b''.join(b'{"pad":%d,"s":"qqqqqqqqqqqqqqqqqqqqqqqqqqqqqq"}\n' % i for i in range(rng.choice([150, 3000])))
        text = pad + text if rng.random() < 0.5 else text + b'\n' + pad
    return text


def cut(text, cuts):
    pieces, prev = [], 0
    for c in sorted(cuts) + [len(text)]:
        pieces.append(text[prev:c]); prev = c
    return pieces


def setup(d, pieces, bads, stdin_at):
    names = []
    for i, pc in enumerate(pieces):
        if i == stdin_at:
            names.append('-')
            continue
        nm = 'f%d.json' % i
        with open(os.path.join(d, nm), 'wb') as f:
            f.write(pc)
        names.append(nm)
    os.makedirs(os.path.join(d, 'dir.d'), exist_ok=True)
    p = os.path.join(d, 'nope.json')
    with open(p, 'wb') as f:
        f.write(b'7\n')
    os.chmod(p, 0)
    for pos, bad in bads:
        names.insert(min(pos, len(names)), bad)
    return names


def run(cmd, d, stdin=None):
    try:
        p = subprocess.run(cmd, cwd=d, capture_output=True, timeout=30,
                           input=stdin if stdin is not None else b'',
                           )
        return [p.stdout.decode('utf-8', 'replace'), p.stderr.decode('utf-8', 'replace'), p.returncode]
    except subprocess.TimeoutExpired:
        return ['', 'TIMEOUT', -1]


def norm(r, jq):
    return [r[0], r[1].replace(jq + ':', 'jq:'), r[2]]


def one(case):
    jq, sjq, prog, pieces, bads = case['jq'], case['sjq'], case['prog'], case['pieces'], case['bads']
    stdin_at = case.get('stdin_at')
    raw = [bytes.fromhex(p) for p in pieces]
    sin = raw[stdin_at] if stdin_at is not None else None
    d = tempfile.mkdtemp(prefix='mf1sw')
    try:
        names = setup(d, raw, bads, stdin_at)
        a = norm(run([jq] + prog + names, d, sin), jq)
        b = run([sjq, 'jq'] + prog + names, d, sin)
        control = None
        if a != b:
            # The same run without the paths that do not open: a difference that stays is not this
            # family's (a lazy read, a wording), and is reported as such.
            good = [n for n in names if n not in BADS]
            if good:
                control = norm(run([jq] + prog + good, d, sin), jq) == run([sjq, 'jq'] + prog + good, d, sin)
    finally:
        try:
            os.chmod(os.path.join(d, 'nope.json'), 0o600)
        except OSError:
            pass
        shutil.rmtree(d, ignore_errors=True)
    if a == b:
        return None
    short = lambda t: t if len(t) <= 2000 else t[:900] + '...%d bytes...' % len(t) + t[-900:]
    clip = lambda r: [short(r[0]), short(r[1]), r[2]]
    a, b = clip(a), clip(b)
    return {'prog': prog, 'names': names, 'pieces': [short(p.decode('latin1')) for p in raw],
            'jq': a, 'sjq': b, 'seed': case.get('seed'), 'control_same': control, 'stdin_at': stdin_at}


def random_cases(args):
    for it in range(args.seed, args.seed + args.cases):
        rng = random.Random(it)
        text = make_text(rng, args.big)
        k = rng.choice([2, 2, 3, 3, 4, 5]) if rng.random() > 0.08 else rng.randint(33, 40)
        pieces = cut(text, [rng.randint(0, len(text)) for _ in range(k - 1)])
        bads = [(rng.randint(0, len(pieces)), rng.choice(BADS)) for _ in range(rng.choice([1, 1, 1, 2]))]
        stdin_at = rng.randrange(len(pieces)) if rng.random() < args.stdin else None
        yield {'jq': args.jq, 'sjq': args.sjq, 'prog': rng.choice(PROGRAMS),
               'pieces': [p.hex() for p in pieces], 'bads': [] if args.no_bads else bads, 'seed': it,
               'stdin_at': stdin_at}


EXHAUSTIVE_TEXTS = [b'0 [x"]\n1\n2\n3\n', b'{"a",:2}\n3\n', b'1}\n2\n', b'[1,x]\n1\n', b'[{0,"s"]\nnul',
                    b'1\n{y\n2\n3\n', b'[1,\n}2]\n', b'"a"\n[1,2]\n{"b":3}\n', b'[0,]"s"] ']
EXHAUSTIVE_PROGS = [['-c', '.'], ['-c', 'try input catch "E"'], ['-c', '-n', '[inputs?]'],
                    ['-c', '-s', '.'], ['-c', '[., input_filename]'],
                    ['-c', '-n', '[limit(50; repeat(try (input|[., input_filename, input_line_number]) catch "E"))]']]


def exhaustive_cases(args):
    for text in EXHAUSTIVE_TEXTS:
        for c in range(len(text) + 1):
            pieces = cut(text, [c])
            for pos in range(3):
                for bad in ([None] if args.no_bads else BADS):
                    if bad is None and pos:
                        continue
                    for prog in EXHAUSTIVE_PROGS:
                        yield {'jq': args.jq, 'sjq': args.sjq, 'prog': prog,
                               'pieces': [p.hex() for p in pieces], 'bads': [] if bad is None else [(pos, bad)]}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--sjq', required=True)
    ap.add_argument('--jq', default='/usr/bin/jq')
    ap.add_argument('--cases', type=int, default=1000)
    ap.add_argument('--seed', type=int, default=0)
    ap.add_argument('--jobs', type=int, default=3)
    ap.add_argument('--out', default='-')
    ap.add_argument('--exhaustive', action='store_true')
    ap.add_argument('--no-bads', action='store_true')
    ap.add_argument('--big', type=float, default=0.0, help='share of texts padded past 1 MiB')
    ap.add_argument('--stdin', type=float, default=0.0)
    args = ap.parse_args()
    cases = list(exhaustive_cases(args) if args.exhaustive else random_cases(args))
    out = sys.stdout if args.out == '-' else open(args.out, 'w')
    n = diffs = family = 0
    with ProcessPoolExecutor(args.jobs) as pool:
        for r in pool.map(one, cases, chunksize=8):
            n += 1
            if r is not None:
                diffs += 1
                family += r['control_same'] is not False
                out.write(json.dumps(r) + '\n')
    print('cases %d diffs %d in-family %d' % (n, diffs, family), file=sys.stderr)
    if out is not sys.stdout:
        out.write(json.dumps({'cases': n, 'diffs': diffs, 'family': family}) + '\n')


if __name__ == '__main__':
    main()
