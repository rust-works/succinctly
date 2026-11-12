#!/usr/bin/env python3
"""Differential fuzz of the windowed plain route of `succinctly jq` against its whole-file route (#6920, #7048, #7024).

A plain filter (no `-n`, nothing that reads `input`) over regular files of 4 MiB or more between them is read a
window at a time (`WindowedFiles`, `src/bin/succinctly/jq_runner/window.rs`); the same files given as `/dev/fd/N`
(which the route declines) are read whole. Each case here builds a random stream of 4.3 to 7.5 MiB, cuts it into
`--files` files at random bytes (anywhere in a token as often as not, so a document is open where a file ends),
runs one random program over the files both ways, and compares stdout, stderr (the files' names replaced by
`<file N>`) and the exit status. The two routes are one implementation of jq's answer, so any difference is a
bug in the window. A regular file on standard input is windowed too since #7059, so it is no longer the whole-file
side. The programs avoid `input_line_number` and `input`, which leave the route: the line a document was read on
shows in the `(at <file N>:LINE)` of a runtime error. (The seeds of #7048's runs do not reproduce: the generator
now cuts the stream into files and picks among more programs.)

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
    scripts/jq-windowed-file-fuzz.py [--bin PATH] [-n N] [--seed S] [--work DIR] [--files K] [--cap-mb MB]
Each case takes about 0.2 s with a release build. Exit 1 on any difference (a timeout, a crash of the
script's own and a run that exceeds `--cap-mb` of address space count as one); the stream that differed is
kept in `--work` as `windowed-fuzz-<seed>-<case>-<file>.json` (one per file), and the case is printed with the mutations applied,
in the order they were applied (a later one can move an earlier one's position). `--work` defaults to a
temporary directory; a debug build is several times slower per case.

`--cap-mb` (default 8000) is an `RLIMIT_AS` on each run, which Linux enforces. macOS does not, so there
run the script under `.ai/scratch/memcap.py --cap-mb 24000 --` (the standing rule for every run of
succinctly).
"""
import argparse
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile
import time

MIN_BYTES = 4 * 1024 * 1024
TIMED_OUT = b"<timed out after 300s>"

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

# With several files (#7024) which file each document was read in, and on which line, is part of the answer.
FILE_PROGRAMS = [
    ["-c", '[input_filename, (.id? // .n? // .)]'],
    ["-c", 'select((.id? // 0) % 4001 == 0) | error("e")'],
    ["-c", "[input_filename, ([.. | numbers] | length)]"],
    ["-r", "input_filename"],
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


def big_document(rng):
    """A document bigger than a window (1 MiB): a window cannot hold it, so the reader has to read on."""
    if rng.random() < 0.5:
        count = rng.randrange(180_000, 400_000)
        return "[" + ",".join(str(n) for n in range(count)) + "]"
    count = rng.randrange(8_000, 25_000)
    return "{\n" + ",\n".join('  "k%d": [%d, "v%d", {"n": %d}]' % (n, n, n, n) for n in range(count)) + "\n}"


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
    data = b"".join(parts)
    if rng.random() < 0.3:  # one document bigger than a window, at a line start
        at = rng.randrange(len(data))
        at = data.rfind(b"\n", 0, at) + 1
        data = data[:at] + big_document(rng).encode() + separator + data[at:]
        notes.append("big_document@%d" % at)
    data = mutate(rng, data, notes)
    if rng.random() < 0.15:
        data = b"\xef\xbb\xbf" + data
        notes.append("bom_front")
    if rng.random() < 0.05:
        data = b"\n" * rng.choice([1, 5, 10000]) + data
        notes.append("blank_prefix")
    return data, notes


def cap(megabytes):
    """A `preexec_fn` that limits a run's address space (Linux enforces it; macOS does not)."""
    def limit():
        try:
            import resource
            resource.setrlimit(resource.RLIMIT_AS, (megabytes << 20, megabytes << 20))
        except (ImportError, ValueError, OSError):
            pass
    return limit


def run(binary, args, megabytes, stdin=None, pass_fds=()):
    """stdout, stderr and the exit status of one run; a run that hangs is reported as one that did."""
    try:
        done = subprocess.run([binary, "jq"] + args, stdin=stdin, capture_output=True, timeout=300,
                              preexec_fn=cap(megabytes), pass_fds=pass_fds)
    except subprocess.TimeoutExpired:
        return b"", TIMED_OUT, -9
    return done.stdout, done.stderr, done.returncode


def cut_into_files(rng, data, count):
    """`data` cut at `count - 1` random bytes, in order. Most cuts fall anywhere (inside a token, a
    string, a multi-byte character); some at a line start, and some leave a file empty."""
    cuts = []
    for _ in range(count - 1):
        at = rng.randrange(len(data) + 1)
        if rng.random() < 0.25:
            at = data.rfind(b"\n", 0, at) + 1
        if rng.random() < 0.1 and cuts:
            at = cuts[-1]
        cuts.append(at)
    cuts.sort()
    edges = [0] + cuts + [len(data)]
    return [data[a:b] for a, b in zip(edges, edges[1:])]


def named(output, names, descriptors):
    """`output` with each file's path, or its `/dev/fd/N`, replaced by `<file N>`."""
    for n, name in enumerate(names):
        output = output.replace(name.encode(), b"<file %d>" % n)
    by_fd = {fd: n for n, fd in enumerate(descriptors)}
    return re.sub(rb"/dev/fd/(\d+)", lambda m: b"<file %d>" % by_fd.get(int(m.group(1)), -1), output)


def run_files(args, rng, data, case, program, work, seed):
    """One case over several files: windowed (as paths) and whole (as descriptors), compared."""
    pieces = cut_into_files(rng, data, args.files)
    names = []
    for n, piece in enumerate(pieces):
        name = os.path.join(work, "windowed-fuzz-%d-%d.json" % (seed, n))
        with open(name, "wb") as handle:
            handle.write(piece)
        names.append(name)
    handles = [open(name, "rb") for name in names]
    try:
        descriptors = [handle.fileno() for handle in handles]
        for handle in handles:
            os.set_inheritable(handle.fileno(), True)
        windowed = run(args.bin, program + names, args.cap_mb)
        whole = run(args.bin, program + ["/dev/fd/%d" % fd for fd in descriptors], args.cap_mb,
                    pass_fds=descriptors)
    finally:
        for handle in handles:
            handle.close()
    windowed = (named(windowed[0], names, descriptors), named(windowed[1], names, descriptors), windowed[2])
    whole = (named(whole[0], names, descriptors), named(whole[1], names, descriptors), whole[2])
    return windowed, whole, names, [len(piece) for piece in pieces]


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--bin", default="target/release/succinctly", help="the succinctly binary")
    parser.add_argument("-n", type=int, default=200, help="how many cases")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--work", default=None, help="a directory for the streams (default: a temporary one)")
    parser.add_argument("--files", type=int, default=1,
                        help="cut each stream into this many files, 1 to 32, read as one stream (#7024)")
    parser.add_argument("--cap-mb", type=int, default=8000, help="RLIMIT_AS of each run, in MiB (Linux)")
    args = parser.parse_args()
    if not 1 <= args.files <= 32:
        sys.exit("--files is 1 to 32: the route holds at most 32 files open and takes no more")
    if not os.path.exists(args.bin):
        sys.exit("no binary at %s: build with `cargo build --release --features cli,simd,regex,serde`" % args.bin)
    work = os.path.abspath(args.work or tempfile.mkdtemp(prefix="windowed-fuzz-"))
    os.makedirs(work, exist_ok=True)
    rng = random.Random(args.seed)
    differences, started = 0, time.time()
    for case in range(args.n):
        data, notes = build(rng)
        if len(data) < MIN_BYTES:
            sys.exit("generator bug: case %d is %d bytes, under the windowed route's minimum" % (case, len(data)))
        program = rng.choice(PROGRAMS + FILE_PROGRAMS * 2)
        try:
            windowed, whole, names, sizes = run_files(args, rng, data, case, program, work, args.seed)
        except OSError as error:  # the binary could not be run at all: keep the stream and say so
            differences += 1
            kept = os.path.join(work, "windowed-fuzz-%d-%d.json" % (args.seed, case))
            with open(kept, "wb") as handle:
                handle.write(data)
            print("DIFFERENCE case=%d program=%s mutations=%s: could not run: %s kept=%s"
                  % (case, program, notes, error, kept))
            continue
        hung = TIMED_OUT in (windowed[1], whole[1])
        if hung or windowed != whole:
            differences += 1
            kept = [os.path.join(work, "windowed-fuzz-%d-%d-%d.json" % (args.seed, case, n))
                    for n in range(len(names))]
            for source, target in zip(names, kept):
                shutil.copyfile(source, target)
            first = next(
                (at for at, (a, b) in enumerate(zip(windowed[0], whole[0])) if a != b),
                min(len(windowed[0]), len(whole[0])),
            )
            print("DIFFERENCE case=%d program=%s mutations=%s sizes=%s exit=%d/%d stdout_differs_at=%s kept=%s"
                  % (case, program, notes, sizes, windowed[2], whole[2],
                     first if windowed[0] != whole[0] else "-", kept))
            if windowed[1] != whole[1]:
                print("  files:", windowed[1][:300])
                print("  fds  :", whole[1][:300])
    print("%d cases, %d differences, seed %d, %.0fs" % (args.n, differences, args.seed, time.time() - started))
    sys.exit(1 if differences else 0)


if __name__ == "__main__":
    main()
