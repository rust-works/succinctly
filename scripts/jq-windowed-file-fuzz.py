#!/usr/bin/env python3
"""Differential fuzz of the windowed plain route of `succinctly jq` against its whole-file route (#6920, #7048).

A plain filter (no `-n`, nothing that reads `input`) over one regular file of 4 MiB or more is read a
window at a time (`WindowedFile`, `src/bin/succinctly/jq_runner/window.rs`); the same bytes on stdin are
read whole. Each case here builds a random stream of 4.3 to 7.5 MiB, runs one random program over it both
ways, and compares stdout, stderr (the file's name replaced by `<stdin>`) and the exit status. The two routes
are one implementation of jq's answer, so any difference is a bug in the window.

The streams mix the shapes that decide where a window ends: documents of every kind, pretty-printed ones that
span lines, two on a line with no separator, blank lines, CRLF and ` \\n` separators, a document bigger than a
window, and (for half the cases) one or two mutations at a random position, usually a line start: a stray
close bracket, a bad literal, an unterminated object or string, bytes that are not UTF-8, a NUL, a BOM in the
middle, a lone CR, trailing garbage, a document cut off, no final line break, a very deep document, a lone
surrogate, a raw control character in a string. A BOM at the front and a run of blank lines at the front are
added to some.

This is not a comparison with jq: both sides are `succinctly jq`. For that, `scripts/jq-grammar-fuzz.py`
and `succinctly dev compare jq` are the tools. A stream under 4 MiB would be read whole on both sides and
prove nothing, so a case that comes out shorter is a bug of the generator and stops the run.

Usage:
    cargo build --release --features cli,simd,regex,serde
    scripts/jq-windowed-file-fuzz.py [--bin PATH] [-n N] [--seed S] [--work DIR] [--programs K]
Each case takes about 0.2 s with a release build. Exit 1 on any difference; the stream that differed is kept
in `--work` as `windowed-fuzz-<seed>-<case>.json`, and the case is printed with the mutations applied.
`--work` defaults to a temporary directory; a debug build is several times slower per case.
"""
import argparse
import os
import random
import subprocess
import sys
import tempfile
import time

MIN_BYTES = 4 * 1024 * 1024

# The programs: identity (the raw-bytes fast path), a field read, a scalar pass-through, a select, a walk,
# an exit-status filter and a field length. Each is "plain": none reads `input`.
PROGRAMS = [
    ["-c", "."],
    ["-c", 'if type == "object" then (.id // .n // .k // 0) else . end'],
    ["-r", "tojson"],
    ["-c", 'select(type == "object" and ((.id // 0) % 4001 == 0)) | .id'],
    ["-c", "[.. | numbers] | length"],
    ["-e", "select(.id == 12345) | .name"],
    ["-c", ".name | length"],
]

MUTATIONS = [
    "stray_close", "bad_literal", "unterminated_obj", "unterminated_str", "invalid_utf8", "nul", "bom_mid",
    "cr_only", "trailing_garbage", "truncate_mid_doc", "no_final_newline", "deep", "lone_surrogate",
    "control_in_string",
]


def document(rng, i):
    """One document (or an odd gap) of a stream; `i` makes the ids distinct."""
    kind = rng.randrange(14)
    if kind == 0:
        return '{"id":%d,"name":"user %d","tags":["a","b"],"o":{"x":[1,2,{"y":null}]}}' % (i, i)
    if kind == 1:
        return '{"id":%d,"name":"caf\u00e9 \u20ac \U0001f600 \\u00e9 \\"q\\" \\\\"}' % i
    if kind == 2:
        return "[%d,%d,[%d,{\"k\":%d}]]" % (i, i + 1, i + 2, i)
    if kind == 3:
        return "%d" % i
    if kind == 4:
        return '"str%d"' % i
    if kind == 5:
        return ["true", "false", "null", "-0", "1.5e-3", "12345678901234567890"][i % 6]
    if kind == 6:
        return '{\n  "id": %d,\n  "name": "pretty %d",\n  "xs": [\n    1,\n    2\n  ]\n}' % (i, i)
    if kind == 7:
        return '{"id":%d}{"id":%d}' % (i, i + 1)
    if kind == 8:
        return '{"id":%d} {"id":%d}  ' % (i, i + 1)
    if kind == 9:
        return ""
    if kind == 10:
        return "   "
    if kind == 11:
        return '{"n":%d,"s":"%s"}' % (i, "x" * rng.choice([0, 10, 200, 5000]))
    if kind == 12:
        return "[" + ",".join(str(j) for j in range(rng.choice([0, 3, 100, 2000]))) + "]"
    return '{"id":%d,"name":"user %d"}' % (i, i)


def mutate(rng, data, notes):
    for _ in range(rng.choice([0, 0, 0, 1, 1, 2])):
        kind = rng.choice(MUTATIONS)
        # A cut-off document ends the stream, so it sits past the minimum size: before it the case would
        # be read whole on both sides and prove nothing.
        low = MIN_BYTES + 1 if kind == "truncate_mid_doc" and len(data) > MIN_BYTES + 1 else 0
        pos = rng.randrange(low, len(data))
        if rng.random() < 0.7:  # most faults sit at a line start
            newline = data.rfind(b"\n", 0, pos)
            pos = max(newline + 1, low) if newline >= 0 else low
        inserted = {
            "stray_close": b"}\n",
            "bad_literal": b"nul\n",
            "unterminated_obj": b'{"a":[1,2,\n',
            "unterminated_str": b'"abc\n',
            "invalid_utf8": b'{"id":1,"name":"x\xffy\xe2\x82"}\n',
            "nul": b'{"id":1,"name":"a\x00b"}\n',
            "bom_mid": b'\xef\xbb\xbf{"id":1}\n',
            "cr_only": b'{"id":1}\r{"id":2}\r',
            "trailing_garbage": b"12x\n",
            "truncate_mid_doc": b'{"id":7,"name":"tr',
            "no_final_newline": b"",
            "deep": b"[" * rng.choice([100, 300, 1000]),
            "lone_surrogate": b'{"id":1,"s":"\\ud800"}\n',
            "control_in_string": b'{"id":1,"s":"a\tb"}\n',
        }[kind]
        if kind == "no_final_newline":
            data = data.rstrip(b"\r\n")
        elif kind == "truncate_mid_doc":
            data = data[:pos] + inserted
        else:
            data = data[:pos] + inserted + data[pos:]
        notes.append("%s@%d" % (kind, pos))
    return data


def build(rng):
    """A random stream of at least MIN_BYTES, and the list of what was done to it."""
    target = rng.randrange(MIN_BYTES + 150_000, 7_500_000)
    separator = rng.choice(["\n", "\n", "\n", "\r\n", "\n\n", " \n"]).encode()
    parts, size, i = [], 0, 0
    while size < target:
        piece = document(rng, i).encode() + separator
        parts.append(piece)
        size += len(piece)
        i += 1
    notes = []
    data = mutate(rng, b"".join(parts), notes)
    if rng.random() < 0.15:
        data = b"\xef\xbb\xbf" + data
        notes.append("bom_front")
    if rng.random() < 0.05:
        data = b"\n" * rng.choice([1, 5, 10000]) + data
        notes.append("blank_prefix")
    return data, notes


def run(binary, args, stdin=None):
    done = subprocess.run([binary, "jq"] + args, stdin=stdin, capture_output=True, timeout=300)
    return done.stdout, done.stderr, done.returncode


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--bin", default="target/release/succinctly", help="the succinctly binary")
    parser.add_argument("-n", type=int, default=200, help="how many cases")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--work", default=None, help="a directory for the streams (default: a temporary one)")
    args = parser.parse_args()
    if not os.path.exists(args.bin):
        sys.exit("no binary at %s: build with `cargo build --release --features cli,simd,regex,serde`" % args.bin)
    work = args.work or tempfile.mkdtemp(prefix="windowed-fuzz-")
    os.makedirs(work, exist_ok=True)
    rng = random.Random(args.seed)
    path = os.path.join(work, "windowed-fuzz-%d.json" % args.seed)
    differences, started = 0, time.time()
    for case in range(args.n):
        data, notes = build(rng)
        if len(data) < MIN_BYTES:
            sys.exit("generator bug: case %d is %d bytes, under the windowed route's minimum" % (case, len(data)))
        with open(path, "wb") as handle:
            handle.write(data)
        program = rng.choice(PROGRAMS)
        windowed = run(args.bin, program + [path])
        with open(path, "rb") as handle:
            whole = run(args.bin, program, stdin=handle)
        windowed_err = windowed[1].replace(path.encode(), b"<stdin>")
        if (windowed[0], windowed_err, windowed[2]) != (whole[0], whole[1], whole[2]):
            differences += 1
            kept = os.path.join(work, "windowed-fuzz-%d-%d.json" % (args.seed, case))
            with open(kept, "wb") as handle:
                handle.write(data)
            first = next(
                (at for at, (a, b) in enumerate(zip(windowed[0], whole[0])) if a != b),
                min(len(windowed[0]), len(whole[0])),
            )
            print(
                "DIFFERENCE case=%d program=%s mutations=%s size=%d exit=%d/%d stdout_differs_at=%s kept=%s"
                % (case, program, notes, len(data), windowed[2], whole[2],
                   first if windowed[0] != whole[0] else "-", kept)
            )
            if windowed_err != whole[1]:
                print("  file :", windowed_err[:300])
                print("  stdin:", whole[1][:300])
    print("%d cases, %d differences, seed %d, %.0fs" % (args.n, differences, args.seed, time.time() - started))
    sys.exit(1 if differences else 0)


if __name__ == "__main__":
    main()
