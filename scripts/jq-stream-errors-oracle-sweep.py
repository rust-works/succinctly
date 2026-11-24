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

Two jq 1.8.2 behaviours that `succinctly jq` follows on purpose cannot agree with the pinned jq 1.7.1
(#7890), and each has one named rule below, which is all this script sets aside:

- `oracle_reruns_slurped_program` (rule A; #7523, #7807): a `-s` row is not compared when jq 1.7.1
  itself runs the program more than once for that input, which is its slurp loop giving out the
  held closing event or the fault at the end of the text after the array. The oracle decides;
  nothing about the subject's output does.
- `separate_nan_payload` (rule B; #7522): a `nan` token is never joined to a digit (`nan1` is
  written `nan 1`), so no input holds a NaN with a payload. No row is skipped for it.

The run prints how many rows each rule touched and exits 2 when a rule touches more than its
ceiling share of the rows (`ADOPTED_CEILINGS`), so a rule cannot quietly swallow the sweep.
`--no-adopted-rules` turns both off and classes every difference by the rule that explains it.

Usage: scripts/jq-stream-errors-oracle-sweep.py [binary] [--seed N] [--count N] [--oracle PATH]
           [--no-adopted-rules]
"""

import argparse
import os
import pathlib
import random
import re
import subprocess
import sys
import tempfile

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent

TOKENS = [
    "[", "]", "{", "}", ",", ":", '"a"', '"', "\\", "1", "2.5", "-", "true", "null", "nul", "nan",
    '"\\u00e9"', "\n", "\n", " ", "[]", "{}", '"k":', "x", "\x00", "\xff", "[[", "]]", "},", "],",
    "12345678", '"long string value"',
]

# No `tokenbuf[1]` exposure (#7256, #7525): jq 1.7.1's `check_literal` reads `tokenbuf[1]` for a bare `n`
# whatever an earlier token left there (uninitialised heap before any token has written it, `u` after
# `nul` or `"iu"`), where jq 1.8.2 and `succinctly jq` never read it. TOKENS has no one-byte `n`
# (`nul`, `nan` and `null` each write index 1 themselves) and joins tokens only by concatenation, so a
# lone `n` token cannot arise. `scripts/jq-seq-oracle-sweep.py` skips the shape; add the same skip here
# if a bare `n` is ever added to TOKENS.

assert "n" not in TOKENS, "a bare `n` token reads jq 1.7.1's stale tokenbuf byte (#7256, #7525)"

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

# --- The two jq 1.8.2 behaviours `succinctly jq` follows and jq 1.7.1 cannot agree with (#7890) ---

# The greatest share of a run's rows each rule may touch before the run fails. Measured over seeds 1
# to 20 at the default count (2,600 rows a seed) against jq 1.7.1: rule A skipped 0 to 6 rows a seed
# (0.23% at most, 27 of the 52,000 in all), and rule B rewrote the input of 117 to 234 rows a seed
# (9.0% at most). Each ceiling is about three times the greatest share seen.
ADOPTED_CEILINGS = {"slurp-rerun": 0.0075, "nan-payload": 0.27}
# A short run (`--count 5`) has too few rows for a share to mean much, and one input is three `-s`
# rows: a rule may always touch this many.
ADOPTED_FLOOR_ROWS = 6

# Rule A's probe: under `-s` a program that reads nothing prints one `0` each time jq runs it.
SLURP_PROBE = ["-c", "-s", "0"]


def oracle_reruns_slurped_program(probe_stdout):
    """Rule A (#7523, #7807): whether the oracle ran a `-s` program more than once, read off the
    stdout of `oracle --stream-errors -c -s 0 <the row's input>`.

    jq 1.7.1's slurp loop gives the array out at the call that reads the last byte, so whatever its
    parser still holds, the closing event of a container that ends the text or the fault the end of
    the text raises, comes out after the array as a further input, and the program runs again for
    it: `2[` is `[[[],2]]` and then `["Unfinished JSON term at EOF at line 1, column 2",[0]]`.
    jq 1.8.2 reads once more before it closes the array (jqlang/jq#3279) and so keeps that event in
    it, and `succinctly jq` follows 1.8.2 for `--stream-errors -s`: docs/compliance/jq/limitations.md,
    "`--stream --slurp` keeps the last closing event in the array, as jq 1.8.2 does". Such a row
    cannot agree with jq 1.7.1, so it is not compared. The rule reads the oracle's output only: a
    `-s` row the oracle runs once is always compared, whatever the subject prints.
    """
    return probe_stdout.count(b"\n") > 1


_NAN_THEN_DIGIT = re.compile(rb"nan(?=[0-9])")


def has_nan_payload(datas):
    """Whether the row's input, its files read one after another as jq reads them, holds `nan`
    directly followed by a digit."""
    return _NAN_THEN_DIGIT.search(b"".join(datas)) is not None


def separate_nan_payload(datas):
    """Rule B (#7522): the row's files with a space between every `nan` and a digit that follows it,
    in one file or across the end of one and the start of the next.

    jq 1.7.1 reads a NaN with a payload (`nan1`, `-nan12345678`) as a number and prints `null`;
    jq 1.8.0 refuses it as a security fix (CVE-2024-53427) and `succinctly jq` follows:
    docs/compliance/jq/limitations.md, "A NaN with a payload (`NaN1`, `nan123`, `-sNaN10`) is not a
    number", and the ledger entry `cli:shtest-cve-2023-50268-nan-payload`. TOKENS joins by
    concatenation, so `nan` then `1` or `12345678` spells one (`nan` then `2.5` is no number in
    either release and is separated with them, to keep the rule one line). Separating the two
    keeps every row compared (skipping those inputs would drop about 6% of the rows) and draws
    nothing from the generator, so every other input of a seed is the one it was before the rule.
    """
    separated = []
    for data in datas:
        data = _NAN_THEN_DIGIT.sub(b"nan ", data)
        if separated and separated[-1].endswith(b"nan") and data[:1].isdigit():
            data = b" " + data
        separated.append(data)
    assert not has_nan_payload(separated), separated
    return separated


def over_ceiling(touched, rows):
    """The rules (by name) that touched more of `rows` than `ADOPTED_CEILINGS` allows."""
    return sorted(
        rule
        for rule, count in touched.items()
        if count > max(ADOPTED_FLOOR_ROWS, int(rows * ADOPTED_CEILINGS[rule]))
    )


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
    parser.add_argument(
        "--no-adopted-rules",
        action="store_true",
        help="compare every row, the ones that follow jq 1.8.2 included (#7890), and class each "
        "difference by the rule that explains it",
    )
    args = parser.parse_args()

    ours = [args.binary, "jq", "--stream-errors"]
    oracle = [args.oracle, "--stream-errors"]
    rng = random.Random(args.seed)
    scratch = tempfile.mkdtemp(prefix="jq-stream-errors-sweep-")
    rows = differences = unexplained = 0
    # Rows each rule touched, and how many of those differ when `--no-adopted-rules` compares them.
    touched = {"slurp-rerun": 0, "nan-payload": 0}
    touched_differ = {"slurp-rerun": 0, "nan-payload": 0}
    for _ in range(args.count):
        datas = [generate(rng) for _ in range(rng.choice([1, 1, 2, 3]))]
        if rng.random() < 0.3:
            datas[0] += b"\n"
        nan_payload = has_nan_payload(datas)
        if nan_payload and not args.no_adopted_rules:
            datas = separate_nan_payload(datas)
        paths = []
        for index, data in enumerate(datas):
            path = os.path.join(scratch, f"f{index}.json")
            with open(path, "wb") as handle:
                handle.write(data)
            paths.append(path)
        for program in PROGRAMS:
            stdin = datas[0] if len(paths) == 1 and rng.random() < 0.5 else None
            tail = [] if stdin is not None else paths
            rows += 1
            touched["nan-payload"] += nan_payload
            slurp_rerun = "-s" in program and oracle_reruns_slurped_program(
                run(oracle, SLURP_PROBE + tail, stdin)[0]
            )
            touched["slurp-rerun"] += slurp_rerun
            if slurp_rerun and not args.no_adopted_rules:
                continue
            got = run(ours, program + tail, stdin)
            want = run(oracle, program + tail, stdin)
            if got != want:
                differences += 1
                touched_differ["slurp-rerun"] += slurp_rerun
                touched_differ["nan-payload"] += nan_payload and not slurp_rerun
                explained = args.no_adopted_rules and (slurp_rerun or nan_payload)
                unexplained += not explained
                if unexplained <= 10 and not explained:
                    print(f"DIFF {program} {'stdin' if stdin is not None else 'files'}")
                    print(f"  input: {[d[:200] for d in datas]!r}")
                    print(f"  jq   : {want!r}")
                    print(f"  ours : {got!r}")
    print(f"{rows} runs, {differences} differences")
    a, b = touched["slurp-rerun"], touched["nan-payload"]
    if args.no_adopted_rules:
        print(
            f"adopted rules off: slurp-rerun flags {a} rows, {touched_differ['slurp-rerun']} of them differ; "
            f"nan-payload flags {b} rows, {touched_differ['nan-payload']} of them differ and are not "
            f"slurp-rerun rows; {unexplained} differences in rows no rule flags"
        )
        return 1 if differences else 0
    print(
        f"adopted jq 1.8.2 behaviours: slurp-rerun skipped {a} rows ({a / max(rows, 1):.2%}, ceiling "
        f"{ADOPTED_CEILINGS['slurp-rerun']:.2%}); nan-payload separated `nan` from a digit in the input of "
        f"{b} rows ({b / max(rows, 1):.2%}, ceiling {ADOPTED_CEILINGS['nan-payload']:.2%}), none skipped"
    )
    over = over_ceiling(touched, rows)
    if over:
        # Exit 2, not 1: `dev compare jq` reads 1 as a divergence, which a ledger entry could list,
        # and anything else as an error, which nothing can (comparison/jq/manifest.toml).
        print(f"FAIL: over its ceiling share of the rows: {', '.join(over)}")
        return 2
    return 1 if differences else 0


if __name__ == "__main__":
    sys.exit(main())
