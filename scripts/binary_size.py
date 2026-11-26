#!/usr/bin/env python3
"""Check the stripped release binary's size against a committed per-target budget (#7390).

    python3 scripts/binary_size.py check --binary target/release/succinctly [--target TRIPLE]
        [--baseline-binary BASE] [--budget-mode enforce|report] [--commit-range RANGE]

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

Growth against the base, not only against the budget (spike): a committed absolute number cannot
tell one PR's growth from the drift of every PR before it. Cumulative growth eats the tolerance until
an unrelated PR fails, and each PR that raises the number edits the same JSON line, so two in flight
conflict. With `--baseline-binary`, a copy of the base's release binary (the one perf-guard compares
against) is stripped the same way, and the PR fails when it grew by more than `max_pr_growth` of the
base. The absolute budget stays, as a report on a PR (`--budget-mode report`: the same line, as a
warning) and a gate on a push to main, where it catches the slow drift no single PR trips. A PR that
means to grow past the threshold says so with a line `[size-growth: ok #N]` in a commit message
(`--commit-range`) or in the PR body (`$PR_BODY`), as `[ledger keeps #N]` and `[no changelog]` do.

CI runs it in the Linux `Perf Regression Guard` legs (`.github/workflows/ci.yml`), whose release
build is the shipped command, `cargo build --release --features cli`. The committed numbers are the
sizes those legs measured: a different toolchain, linker or `strip` moves the size by a percent or
two, so refresh a budget from the number a CI run prints, not from a local build.
"""

import argparse
import json
import os
import re
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


# A waiver is a line of its own naming the issue that explains the growth. An HTML comment in a PR
# body is the template's own instructions, so it never counts.
WAIVER = re.compile(r"^\s*\[size-growth: ok #(\d+)\]\s*$", re.MULTILINE)
HTML_COMMENT = re.compile(r"<!--.*?-->", re.DOTALL)


def find_waiver(texts):
    """The issue number of the first `[size-growth: ok #N]` line in `texts`, or None."""
    for text in texts:
        m = WAIVER.search(HTML_COMMENT.sub("", text or ""))
        if m:
            return m.group(1)
    return None


def commit_messages(commit_range):
    """The messages of the commits in `commit_range` (`git log`), or [] when git cannot say."""
    out = subprocess.run(
        ["git", "-C", ROOT, "log", "--format=%B%x00", commit_range], capture_output=True, text=True
    )
    return out.stdout.split("\x00") if out.returncode == 0 else []


def mb(n):
    return f"{n / 1e6:.2f} MB"


def verdict(measured, budget, tolerance):
    """`"over"`, `"under"` or `"ok"`: where `measured` falls against `budget` +/- `tolerance`."""
    if measured > budget * (1 + tolerance):
        return "over"
    if measured < budget * (1 - tolerance):
        return "under"
    return "ok"


def check(measured, target, config, budget_path, report=False):
    """`(exit code, lines to print)` for a measured stripped size.

    With `report`, a size past the budget is a warning that exits 0 and asks for no edit: on a PR the
    committed number is not this PR's to bump (its growth is gated against its base)."""
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
    if v == "over" and report:
        return 0, [
            f"::warning::the stripped release binary for {target} is {change:+.2%} past its budget (report only on a PR)",
            line,
            "Not this PR's to fix: refresh the budget in a PR of its own when main's push run fails it. " + edit,
        ]
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


def check_growth(measured, base, max_growth, waiver=None):
    """`(exit code, lines to print)` for a stripped size against the base's stripped size."""
    change = (measured - base) / base
    line = (
        f"stripped binary {measured} bytes ({mb(measured)}) against its base {base} bytes ({mb(base)}), "
        f"{change:+.2%} (limit {max_growth:+.2%} per PR)"
    )
    if change <= max_growth:
        return 0, [line]
    if waiver:
        return 0, [f"::notice::stripped binary growth {change:+.2%} accepted by [size-growth: ok #{waiver}]", line]
    return 1, [
        f"::error::this change grew the stripped release binary {change:+.2%} over its base, past {max_growth:.2%} per PR",
        line,
        "First look for what grew: `cargo bloat --release --features cli --crates`, or a new dependency or generic.",
        "If the growth is intended, put a line of its own in a commit message or the PR body naming the issue",
        "that explains it (the check reads both; a PR-body edit needs a re-run of the job):",
        "    [size-growth: ok #N]",
    ]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    c = sub.add_parser("check", help="strip a copy of the binary and compare it with the budget")
    c.add_argument("--binary", required=True, help="the release binary (left unstripped)")
    c.add_argument("--target", help="target triple to look up (default: rustc's host)")
    c.add_argument("--budget", default=BUDGET, help="budget file (default: %(default)s)")
    c.add_argument("--strip", default="strip", help="strip program (default: %(default)s)")
    c.add_argument("--baseline-binary", help="the base's release binary: fail when this one grew past max_pr_growth of it")
    c.add_argument("--max-growth", type=float, help="per-PR growth limit as a fraction (default: max_pr_growth in the budget file)")
    c.add_argument(
        "--budget-mode",
        choices=("enforce", "report"),
        default="enforce",
        help="`report` turns a budget failure into a warning (a PR, whose own growth is gated against its base); default: %(default)s",
    )
    c.add_argument("--commit-range", help="git range whose commit messages may carry a [size-growth: ok #N] waiver")
    args = parser.parse_args(argv)

    with open(args.budget, encoding="utf-8") as f:
        config = json.load(f)
    target = args.target or host_target()
    measured = stripped_size(args.binary, args.strip)
    budget_mode = args.budget_mode
    lines = []
    code = 0
    if args.baseline_binary:
        if os.path.exists(args.baseline_binary):
            max_growth = args.max_growth if args.max_growth is not None else config["max_pr_growth"]
            texts = [os.environ.get("PR_BODY", "")]
            texts += commit_messages(args.commit_range) if args.commit_range else []
            base = stripped_size(args.baseline_binary, args.strip)
            code, lines = check_growth(measured, base, max_growth, find_waiver(texts))
        else:
            # No base to compare with: the absolute budget is then the only gate, so it enforces.
            budget_mode = "enforce"
            lines = [f"::warning::no baseline binary at {args.baseline_binary}; checking the absolute budget only"]
    budget_code, budget_lines = check(measured, target, config, args.budget, report=budget_mode == "report")
    code = code or budget_code
    lines += budget_lines
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
