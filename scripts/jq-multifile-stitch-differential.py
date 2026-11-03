#!/usr/bin/env python3
"""Randomised differential for jq's multi-file input stream (#4305, #4316, #4335, #4349, #4352,
#4358, #4376, #5936): the byte stream jq builds from several files, with a token open where a file ends,
a parse error caught by `try input catch`, and the lines `input_line_number` names.

jq reads its input files as ONE concatenated byte stream, with a line count and 4095-byte reads
per file; succinctly indexes each file on its own and stitches an open token onto the next file
(`stitch_json_file_seams` in `src/bin/succinctly/jq_runner.rs`). Every seam bug found so far came
from a stream no hand-written row had: a value split across files with a newline inside it, an
invalid UTF-8 byte in the moved bytes, a malformed container that closes, a number run into a
keyword, a fault met in bytes that came from the previous file. This generates such streams,
splits them at random bytes into two or three files, and runs each under the pinned `jq` and the
build under test with programs that catch every error, so the output says what was read, in what
order, and on which line.

Classes, per stream and program, against jq:

    match       same exit code and stdout
    differ      the candidate disagrees with jq

With `--base BINARY` (a build of `main`), every differing run is attributed:

    fixed       base differs from jq, the candidate matches
    regression  base matches jq, the candidate differs  -- the one that fails the run
    both        neither matches

so a change is judged by what it moved, not by the residual it leaves. A deliberately nasty pool
(`--pool nasty`) still differs from jq in some runs; read the totals against the same pool on
`main`, never against zero.

Usage (debug builds are fine; run it under `memcap.py` like any other succinctly run):

    cargo build --features cli
    python3 scripts/jq-multifile-stitch-differential.py --candidate target/debug/succinctly
    python3 scripts/jq-multifile-stitch-differential.py --candidate target/debug/succinctly \\
        --base /path/to/main/target/debug/succinctly --pool nasty --seed 7 --streams 600

Exit status 1 if any regression is found (needs `--base`), or if any run differs and there is no
`--base` and `--fail-on-differ` is set.
"""
import argparse
import os
import random
import subprocess
import sys
import tempfile

ORACLE = "/usr/bin/jq"

# Tokens that, glued together and cut at random bytes, make the streams. Newlines inside values
# are the point: they are what the per-file line count and the 4095-byte reads disagree about.
CLEAN = [
    "1", "23", "true", "null", '"ab"', "[1,\n2]", '{"a":\n1}', "[]", "{}", '"a\\nb"', "[\n]",
]
# What the clean pool leaves out: malformed containers and closers, a fault inside a string,
# strings that end in a backslash or never close, adjacent bare tokens, and invalid or
# truncated UTF-8 (every one of these was a bug).
NASTY = CLEAN + [
    "}", "[1 2]", "{1:2}", "[1}", "{]", '"\\', '"x\ny"', "[", '{"a":', '"a', "23null", "\n",
    '"\xff"', '"\xc3"', '"a\xe2\x82"', '"\xe2\x82\xac"',
]
SEPARATORS = [" ", "\n", "", "\t"]

# Every program bounds itself with limit(): `repeat(try input catch ...)` never ends, because the
# catch swallows the end-of-input break.
PROGRAMS = [
    ["-nc", '[limit(8; repeat(try input catch "E"))]'],
    [
        "-nc",
        '[limit(8; repeat(try (input | [., input_line_number]) '
        'catch ["E", input_line_number]))]',
    ],
    # The parser's own message names the line and column it stands on (#5923, #5936).
    ["-nc", "[limit(8; repeat(try input catch .))]"],
]


def run(binary, program, names, timeout):
    # `jq` is the oracle; the candidate is `succinctly jq`.
    argv = [binary] if binary == ORACLE else [binary, "jq"]
    try:
        r = subprocess.run(argv + program + names, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return ("timeout", b"")
    return (r.returncode, r.stdout)


def make_stream(rng, pool, max_tokens):
    out = ""
    for _ in range(rng.randint(2, max_tokens)):
        out += rng.choice(pool) + rng.choice(SEPARATORS)
    # Latin-1 so a "\xff" in the pool is one byte, not the two of its UTF-8 spelling.
    return out.encode("latin-1")


def split(rng, data, files):
    cuts = sorted(rng.randint(0, len(data)) for _ in range(files - 1))
    return [data[a:b] for a, b in zip([0] + cuts, cuts + [len(data)])]


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--candidate", help="build under test (succinctly binary)")
    ap.add_argument("--base", help="build of main, to attribute differences")
    ap.add_argument("--pool", choices=["clean", "nasty"], default="clean")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--streams", type=int, default=250)
    ap.add_argument("--files", type=int, choices=[2, 3, 4], default=0, help="0: either, at random")
    ap.add_argument("--max-tokens", type=int, default=6)
    ap.add_argument("--timeout", type=float, default=20.0)
    ap.add_argument("--show", type=int, default=5, help="runs to print per class")
    ap.add_argument("--fail-on-differ", action="store_true")
    ap.add_argument("--self-test", action="store_true", help="print the pools and exit")
    args = ap.parse_args()

    pool = CLEAN if args.pool == "clean" else NASTY
    if args.self_test:
        print(f"pool {args.pool}: {len(pool)} tokens")
        for token in pool:
            print("  ", repr(token))
        print(f"{len(PROGRAMS)} programs")
        return 0
    if not args.candidate:
        ap.error("--candidate is required")
    if not os.path.exists(ORACLE):
        print(f"{ORACLE} (jq 1.7.1) is required", file=sys.stderr)
        return 2

    rng = random.Random(args.seed)
    counts = dict(match=0, differ=0, fixed=0, regression=0, both=0)
    shown = dict(differ=0, fixed=0, regression=0, both=0)

    def report(kind, programs_text, parts, oracle, base, cand):
        counts[kind] += 1
        if shown[kind] < args.show:
            shown[kind] += 1
            print(f"{kind.upper()} {programs_text}")
            print(f"  files  {parts!r}")
            print(f"  jq     {oracle}")
            if base is not None:
                print(f"  base   {base}")
            print(f"  cand   {cand}")

    with tempfile.TemporaryDirectory() as tmp:
        for _ in range(args.streams):
            data = make_stream(rng, pool, args.max_tokens)
            parts = split(rng, data, args.files or rng.choice([2, 3]))
            names = []
            for index, part in enumerate(parts):
                path = os.path.join(tmp, f"f{index}.json")
                with open(path, "wb") as handle:
                    handle.write(part)
                names.append(path)
            for program in PROGRAMS:
                oracle = run(ORACLE, program, names, args.timeout)
                cand = run(args.candidate, program, names, args.timeout)
                text = program[1][:40]
                if args.base is None:
                    if cand == oracle:
                        counts["match"] += 1
                    else:
                        report("differ", text, parts, oracle, None, cand)
                    continue
                base = run(args.base, program, names, args.timeout)
                if cand == oracle and base == oracle:
                    counts["match"] += 1
                elif cand == oracle:
                    report("fixed", text, parts, oracle, base, cand)
                elif base == oracle:
                    report("regression", text, parts, oracle, base, cand)
                else:
                    report("both", text, parts, oracle, base, cand)

    total = sum(counts[k] for k in ("match", "differ", "regression", "both", "fixed"))
    print(
        f"runs {total}: match {counts['match']}, differ {counts['differ']}, "
        f"fixed {counts['fixed']}, regression {counts['regression']}, both {counts['both']}"
    )
    if args.base is not None and counts["regression"]:
        return 1
    if args.base is None and args.fail_on_differ and counts["differ"]:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
