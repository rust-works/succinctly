#!/usr/bin/env python3
"""Program-dimension differential sweep for `--seq` records read through the owned door (#6620).

`jq-seq-oracle-sweep.py` runs one program (`.`) over random bytes, and `.` is answered by the
identity shortcut (#5624) before the owned door is asked, so a clean sweep says nothing about the
door. This sweep crosses programs with record streams that hold a diagnostic, which is what sends
a `--seq` run through the queue's per-record conversion and the door, and compares **stdout, stderr
and exit status together** against the pinned jq.

Each case is a stream of 1-4 random JSON records (one case in ten: 100-400, to cross the stretches the door closes for) (objects with `name`/`id`/`age`, nested arrays,
spelled-out numbers, `nan`, repeated keys, non-ASCII strings, a record 300 levels deep) with one
truncated record placed at a random position, run through one program with `-c` (some with
`-r`). With `--base BASE` it also runs the binary from before the change, so that a row where the
base matched jq and the binary under test does not is reported as a regression, and a row where it
is the other way round as a fix; a divergence both share is not this change's, and is only
counted.

Usage:
  cargo build --release --features cli
  ./scripts/jq-seq-door-sweep.py [--seed N] [--cases N] [--base BASE] [--oracle JQ] [--file | --both] [binary]

Exits 1 when a regression is found (or, without `--base`, when any case differs from jq), and 2 on a
setup error. `succinctly dev compare jq --dimension sweeps` runs it with `--both` and no base
(`comparison/jq/manifest.toml`, id `seq-door`), where it is a plain oracle sweep: every case must
match jq 1.7.1.
"""

import argparse
import collections
import os
import pathlib
import random
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

RS = b"\x1e"
REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent

# Faults: a record cut off inside a value, which jq warns about and reads on from.
FAULTS = [RS + b"[1,\n", RS + b'{"a":\n', RS + b'"abc\n', RS + b"{\n", RS + b"tru\n"]

RECORDS = [
    b'{"id":1,"name":"Alice","age":31,"tags":["a","b"]}',
    b'{"id":2,"name":"Bob","age":29,"tags":[]}',
    b'{"name":"x","id":3}',
    b'{"id":4,"age":50,"name":null}',
    b'{"a":{"b":[1,2,{"c":null}]},"name":"deep"}',
    b'{"a":1,"a":2,"name":"dup"}',
    b'{"id":1.50,"name":"num","age":1e2}',
    b'{"id":100000000000000000000,"name":"big","age":-0}',
    b'{"id":0.1,"name":"\xc3\xa9\xe2\x82\xac","age":0}',
    b'{"id":nan,"name":"nan","age":3}',
    b"[1,2,3]",
    b'[[1],[2,3],{"name":"in"}]',
    b"[]",
    b"{}",
    b'"a string"',
    b"12",
    b"-1.5e-3",
    b"null",
    b"true",
    b'{"name":"\\ud83d\\ude00","id":5}',
    b'{"name":"\\ud800","id":6}',
    b"[" * 300 + b"]" * 300,
    b'{"a":[{"b":1},{"b":2},{"b":3}],"name":"list","id":7,"age":40}',
]

PROGRAMS = [
    ".name", ".id", ".", "[.id,.name]", "keys", "select(.age>30)|.name", "{id,name}", ".x=1",
    "to_entries|length", "del(.name)", ".[0]", ".[]?", "length", "type", "tojson", "..", "[..]|length",
    "paths", "[paths]|length", ".a.b", ".a[0].b?", "map(.+1)?", "map_values(.)?", "tostring",
    ".name|ascii_downcase?", "getpath([\"a\"])", "has(\"name\")?", "to_entries?", "with_entries(.)?",
    "[.[]?]|length", "first(.[]?)", "limit(1;.[]?)", "[limit(2;.[]?)]", "try error catch .",
    "error", "error(\"x\")", "debug", "debug|.name", "input", "[.,input]", "try input catch .",
    ".name, (try (input|.name) catch \"x\")", "[inputs]|length", "input_line_number",
    "$__loc__", "@text", "@json", "@base64?", "tojson|fromjson", "walk(.)", "sort?", "unique?",
    "add?", "any?", "all?", "min?", "reverse?", "flatten?", "env|type", "input_filename",
    "ltrimstr(\"a\")", "select(.id)|.id", "select(.id==1)", "if .id then .id else empty end",
    "(.id,.name)", "[.id, .name, .age]", "[.[0], .[1], .[-1]]", "[.a, .b.c, .zz]", "[., .]", "[.id, \"x\", 3, null]",
    "[.name, [.id], {k: .age}]", "[(.id, .name)]", "[.id, (.name, .age)]", "has(\"id\")", "has(0)", "has(\"name\")?",
    "[.id, .a[0]]", "[.id, .name?]", "[.id, .name] | length", "keys | length", ".id as $x|$x", "if type==\"array\" then .[0]? else keys? end", "if type==\"object\" then .name else length? end", "reduce .[]? as $x (0;.+1)", "[.[]?|select(type==\"number\")]",
    "splits(\"a\")?", "ascii?", "implode?", "tojson|length", "halt", "halt_error", "empty", "not",
    "del(.a)", ".a=1", ".name|=.", ".id+=1", ".[]?|=1", "to_entries[]?", "limit(0;.)", "first",
    "last?", "nth(0)?", "range(2)", "$ENV|type", "now|type", "splits(\"x\")?",
]

FLAG_SETS = [["-c"], ["-c"], ["-c"], ["-r"], ["-c", "-e"]]


def resolve_oracle(given):
    if given:
        return given
    for candidate in ("/usr/bin/jq", "jq"):
        try:
            out = subprocess.run([candidate, "--version"], capture_output=True, text=True)
        except OSError:
            continue
        if out.stdout.strip().startswith("jq-1.7.1"):
            return candidate
    # A setup error exits 2, not 1: the compare harness reads 1 as "diverges" (comparison/jq/manifest.toml).
    print("error: no jq 1.7.1 found; pass --oracle", file=sys.stderr)
    sys.exit(2)


def run(argv, data, as_file=False):
    # stdin from a file, never a pipe (see jq-seq-oracle-sweep.py). `as_file` names the file as an
    # argument instead, which is the other route a stream reaches the reader by (#6620).
    with tempfile.NamedTemporaryFile(delete=False) as handle:
        handle.write(data)
        path = handle.name
    try:
        with open(path, "rb") as stdin:
            try:
                done = subprocess.run(
                    argv + [path] if as_file else argv,
                    stdin=subprocess.DEVNULL if as_file else stdin,
                    capture_output=True,
                    timeout=30,
                )
            except subprocess.TimeoutExpired:
                return ("timeout", b"", b"")
        out, err = done.stdout, done.stderr
        if as_file:
            # The file's name is in a diagnostic or `input_filename`; make it comparable.
            err = err.replace(path.encode(), b"<file>")
            out = out.replace(path.encode(), b"<file>")
        return (done.returncode, out, err)
    finally:
        os.unlink(path)


def make_cases(count, seed):
    rng = random.Random(seed)
    cases = []
    for _ in range(count):
        # One case in ten is long enough for the door's backoff: 4 declines close it for 64 records,
        # then 128, so a stream of a few hundred records crosses a closed stretch and its probes.
        count = rng.randint(100, 400) if rng.random() < 0.1 else rng.randint(1, 4)
        records = [RS + rng.choice(RECORDS) + b"\n" for _ in range(count)]
        fault = rng.choice(FAULTS)
        records.insert(rng.randint(0, len(records)), fault)
        stream = b"".join(records)
        if rng.random() < 0.15:
            stream = stream.rstrip(b"\n")
        cases.append((rng.choice(PROGRAMS), rng.choice(FLAG_SETS), stream))
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?", default=str(REPO_ROOT / "target/release/succinctly"))
    parser.add_argument("--base", help="the binary from before the change, to tell a regression from a shared divergence")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--cases", type=int, default=3000)
    parser.add_argument("--oracle")
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--file", action="store_true", help="give each stream as a file argument, not on stdin")
    parser.add_argument("--both", action="store_true", help="run every case on stdin and as a file argument")
    args = parser.parse_args()
    if not os.access(args.binary, os.X_OK):
        print(f"error: succinctly binary not found at {args.binary}", file=sys.stderr)
        return 2
    jq = resolve_oracle(args.oracle)

    def one(work):
        (program, flags, stream), as_file = work
        argv = flags + ["--seq", program]
        want = run([jq] + argv, stream, as_file)
        got = run([args.binary, "jq"] + argv, stream, as_file)
        before = run([args.base, "jq"] + argv, stream, as_file) if args.base else None
        return (program, flags, stream), want, got, before

    routes = [False, True] if args.both else [args.file]
    work = [(case, route) for case in make_cases(args.cases, args.seed) for route in routes]
    tally = collections.Counter()
    regressions, fixes, shared = [], [], []
    with ThreadPoolExecutor(args.jobs) as pool:
        for case, want, got, before in pool.map(one, work):
            tally["cases"] += 1
            if got == want:
                tally["match jq"] += 1
                if before is not None and before != want:
                    fixes.append((case, want, got, before))
            elif before is not None and before == want:
                regressions.append((case, want, got, before))
            else:
                shared.append((case, want, got, before))
    print(dict(tally))
    print(f"regressions (base matched jq, binary does not): {len(regressions)}")
    print(f"fixes (base differed from jq, binary matches):   {len(fixes)}")
    print(f"differ from jq with the base too:                {len(shared)}")
    for program, count in collections.Counter(c[0][0] for c in shared).most_common():
        print(f"    {count:4d}  {program}")

    def show(rows, label, limit):
        for (program, flags, stream), want, got, before in rows[:limit]:
            print(f"--- {label}: {' '.join(flags)} --seq {program!r}\n  stream {stream!r}")
            print(f"  jq    {want}")
            print(f"  this  {got}")
            if before is not None:
                print(f"  base  {before}")

    show(regressions, "REGRESSION", 20)
    show(fixes, "FIXED", 5)
    show(shared, "SHARED", 10 if not args.base else 3)
    if args.base:
        return 1 if regressions else 0
    return 1 if regressions or shared else 0


if __name__ == "__main__":
    sys.exit(main())
