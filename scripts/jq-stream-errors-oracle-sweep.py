#!/usr/bin/env python3
"""Randomised differential sweep for `jq --stream-errors` (#5474).

Generates malformed JSON text from a small token alphabet (structure bytes, literals, NUL, invalid
UTF-8, lines longer than jq's 4095-byte read), spreads it over one to three input files or stdin,
and runs each through the pinned jq oracle and the built succinctly binary under several programs.
It compares **stdout, exit status and stderr** together: stdout carries the `[message, path]`
inputs and the order `--slurp` puts them in, and stderr carries the `(at <file>:<line>)` marker that
says where jq's input stood when a program failed.

This is a verification tool, not a CI gate -- the rows in `tests/jq_cli/i5474.rs` and the unit
tests in `src/bin/succinctly/jq_stream_errors.rs` are what CI enforces. Run it after touching
`jq_stream_errors.rs` or the `--stream-errors` branch of `get_inputs`.

Usage: scripts/jq-stream-errors-oracle-sweep.py [binary] [--seed N] [--count N] [--oracle PATH]
"""

import argparse
import os
import pathlib
import random
import subprocess
import sys
import tempfile

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent

TOKENS = [
    "[", "]", "{", "}", ",", ":", '"a"', '"', "\\", "1", "2.5", "-", "true", "null", "nul", "nan",
    '"\\u00e9"', "\n", "\n", " ", "[]", "{}", '"k":', "x", "\x00", "\xff", "[[", "]]", "},", "],",
    "12345678", '"long string value"',
]

PROGRAMS = [
    ["-c", "[., input_filename, input_line_number]"],
    ["-c", "-s", "[., input_filename, input_line_number]"],
    ["-c", "-s", "."],
    ["-c", "-n", "[inputs]"],
    ["-c", "-n", "input, input"],
    ["-c", "-n", "[inputs] | error(\"y\")"],
    ["-c", "-n", "input | error(\"y\")"],
    ["-c", "-n", "input, input, input, input"],
    ["-c", "-s", "error(\"y\")"],
    ["-c", "error(\"y\")"],
    ["-c", "-e", ".[0]?"],
    ["-r", "-c", "(. | tojson)"],
    ["-c", "try error catch ."],
]


def generate(rng):
    text = "".join(rng.choice(TOKENS) for _ in range(rng.randint(1, 25)))
    if rng.random() < 0.15:
        text += "a" * rng.choice([4094, 4095, 4096, 8200])
    return text.encode("latin1")


def run(binary, args, stdin):
    done = subprocess.run(binary + args, input=stdin, capture_output=True, timeout=60)
    # The program name differs between the two binaries; nothing else in stderr should.
    stderr = done.stderr.replace(binary[0].encode(), b"jq")
    return done.stdout, done.returncode, stderr


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("binary", nargs="?", default=str(REPO_ROOT / "target/release/succinctly"))
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--count", type=int, default=200, help="inputs to generate")
    parser.add_argument("--oracle", default="/usr/bin/jq")
    args = parser.parse_args()

    ours = [args.binary, "jq", "--stream-errors"]
    oracle = [args.oracle, "--stream-errors"]
    rng = random.Random(args.seed)
    scratch = tempfile.mkdtemp(prefix="jq-stream-errors-sweep-")
    differences = 0
    for _ in range(args.count):
        datas = [generate(rng) for _ in range(rng.choice([1, 1, 2, 3]))]
        if rng.random() < 0.3:
            datas[0] += b"\n"
        paths = []
        for index, data in enumerate(datas):
            path = os.path.join(scratch, f"f{index}.json")
            with open(path, "wb") as handle:
                handle.write(data)
            paths.append(path)
        for program in PROGRAMS:
            stdin = datas[0] if len(paths) == 1 and rng.random() < 0.5 else None
            tail = [] if stdin is not None else paths
            got = run(ours, program + tail, stdin)
            want = run(oracle, program + tail, stdin)
            if got != want:
                differences += 1
                if differences <= 10:
                    print(f"DIFF {program} {'stdin' if stdin is not None else 'files'}")
                    print(f"  input: {[d[:200] for d in datas]!r}")
                    print(f"  jq   : {want!r}")
                    print(f"  ours : {got!r}")
    print(f"{args.count * len(PROGRAMS)} runs, {differences} differences")
    return 1 if differences else 0


if __name__ == "__main__":
    sys.exit(main())
