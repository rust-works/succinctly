#!/usr/bin/env python3
"""Check the stripped release binary's size against a committed per-target budget (#7390).

    python3 scripts/binary_size.py check --binary target/release/succinctly [--target TRIPLE]

#7384 found the stripped release binary at 6x v0.7.0's size with nothing in CI to flag the growth.
This strips a copy of the binary with plain `strip` (as `release.yml` does before archiving it),
and compares its size with `tests/data/binary-size-budget.json`, keyed by the target triple
(`rustc -vV`'s host line unless `--target` names one):

- more than `tolerance` above the budget fails, printing the measured size and the edit that
  accepts it. An intentional increase bumps the committed number in the same PR, so the reviewer
  sees it;
- more than `tolerance` below it passes with a warning, printing the edit that tightens it, so the
  slack a size cut frees is not left for later growth to spend unseen;
- a target with no budget fails, printing the entry to add.

CI runs it in the Linux `Perf Regression Guard` legs (`.github/workflows/ci.yml`), whose release
build is the shipped command, `cargo build --release --features cli`. The committed numbers are the
sizes those legs measured: a different toolchain, linker or `strip` moves the size by a percent or
two, so refresh a budget from the number a CI run prints, not from a local build.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BUDGET = os.path.join(ROOT, "tests", "data", "binary-size-budget.json")


def host_target():
    out = subprocess.run(["rustc", "-vV"], capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        if line.startswith("host: "):
            return line[len("host: ") :].strip()
    raise RuntimeError("`rustc -vV` printed no host line")


def stripped_size(binary, strip="strip"):
    """The size in bytes of a stripped copy of `binary`; the binary itself is left alone."""
    with tempfile.TemporaryDirectory() as tmp:
        copy = os.path.join(tmp, os.path.basename(binary))
        shutil.copyfile(binary, copy)
        subprocess.run([strip, copy], check=True)
        return os.path.getsize(copy)


def mb(n):
    return f"{n / 1e6:.2f} MB"


def verdict(measured, budget, tolerance):
    """`"over"`, `"under"` or `"ok"`: where `measured` falls against `budget` +/- `tolerance`."""
    if measured > budget * (1 + tolerance):
        return "over"
    if measured < budget * (1 - tolerance):
        return "under"
    return "ok"


def check(measured, target, config, budget_path):
    """`(exit code, lines to print)` for a measured stripped size."""
    rel = os.path.relpath(budget_path, ROOT)
    tolerance = config["tolerance"]
    entry = config["targets"].get(target)
    if entry is None:
        return 1, [
            f"::error::no size budget for {target} in {rel}; the stripped binary is {measured} bytes ({mb(measured)})",
            f'Add it under "targets": "{target}": {{"bytes": {measured}, "measured": "<where it was measured>"}}',
        ]
    budget = entry["bytes"]
    change = (measured - budget) / budget
    line = (
        f"{target}: stripped binary {measured} bytes ({mb(measured)}), budget {budget} bytes ({mb(budget)}), "
        f"{change:+.2%} (tolerance {tolerance:.0%})"
    )
    edit = f'In {rel}, set "{target}" -> "bytes" to {measured} (and "measured" to the run that printed it).'
    v = verdict(measured, budget, tolerance)
    if v == "over":
        return 1, [
            f"::error::the stripped release binary for {target} grew {change:+.2%} past its budget (#7390)",
            line,
            "If the growth is intended, accept it in this PR: " + edit,
        ]
    if v == "under":
        return 0, [
            f"::warning::the stripped release binary for {target} is {change:+.2%} under its budget; tighten it (#7390)",
            line,
            "To keep the freed slack from being spent unseen: " + edit,
        ]
    return 0, [line]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    c = sub.add_parser("check", help="strip a copy of the binary and compare it with the budget")
    c.add_argument("--binary", required=True, help="the release binary (left unstripped)")
    c.add_argument("--target", help="target triple to look up (default: rustc's host)")
    c.add_argument("--budget", default=BUDGET, help="budget file (default: %(default)s)")
    c.add_argument("--strip", default="strip", help="strip program (default: %(default)s)")
    args = parser.parse_args(argv)

    with open(args.budget, encoding="utf-8") as f:
        config = json.load(f)
    target = args.target or host_target()
    measured = stripped_size(args.binary, args.strip)
    code, lines = check(measured, target, config, args.budget)
    for line in lines:
        print(line)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write("### Stripped binary size (#7390)\n\n")
            f.write("".join(f"    {line}\n" for line in lines) + "\n")
    return code


if __name__ == "__main__":
    sys.exit(main())
