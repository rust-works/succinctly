#!/usr/bin/env python3
"""Check the stripped release binary's size against its base and a committed per-target budget (#7390, #7959).

    python3 scripts/binary_size.py check --binary target/release/succinctly [--target TRIPLE]
        [--baseline-binary BASE] [--budget-mode enforce|report] [--commit-range RANGE]
    python3 scripts/binary_size.py track --binary BIN --target TRIPLE [--measured-at TEXT] [--issue]

#7384 found the stripped release binary at 6x v0.7.0's size with nothing in CI to flag the growth.
This strips a copy of the binary with plain `strip` (as `release.yml` does before archiving it),
and compares its size with `tests/data/binary-size-budget.json`, keyed by the target triple
(`rustc -vV`'s host line unless `--target` names one):

- more than `tolerance` above the budget fails (`--budget-mode report`: warns), printing the
  measured size and the edit that accepts it. An intended increase bumps the committed number in a
  PR of its own, so the reviewer sees it;
- more than `tolerance` below it passes with a warning, printing the edit that tightens it, so the
  slack a size cut frees is not left for later growth to spend unseen;
- a target with no budget fails, printing the entry to add.

A pull request is gated on its growth against its base, not on the budget (#7959): a committed
absolute number cannot tell one PR's growth from the drift of every PR before it. That drift ate the
tolerance until an unrelated PR would fail (+3.99% on x86_64, +4.12% on aarch64, in steps of at most
0.41%), and each PR that raised the number edited the same JSON line, so two in flight conflicted.
With `--baseline-binary`, a copy of the base's release binary (the one perf-guard compares against)
is stripped the same way, and the PR fails when it grew by more than `max_pr_growth` of the base. A
PR that means to grow past that says so with a line `[size-growth: ok #N]` in a commit message
(`--commit-range`) or in the PR body (`$PR_BODY`), as `[ledger keeps #N]` and `[no changelog]` do.
If the base binary is missing, the budget is enforced instead (the check fails towards checking).

The budget itself catches the slow drift no single PR trips, in three places:

- CI's pull-request and main-push runs report it (`--budget-mode report`), and never fail on it:
  failing every push to main would leave main red with no owner;
- `track`, run daily by `.github/workflows/scheduled.yml` on the binary main's newest push run
  uploaded, opens one tracking issue per target that is over (label `binary-size`), or rewrites the
  body of the one that is open;
- `release.yml` runs `check` (enforcing) on the Linux binaries it ships.

CI runs `check` in the Linux `Perf Regression Guard` legs (`.github/workflows/ci.yml`), whose release
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
REPO = os.environ.get("GITHUB_REPOSITORY", "rust-works/succinctly")
LABEL = "binary-size"  # the tracking issues `track --issue` opens


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

    With `report`, a size past the budget is a warning that exits 0: CI's pull-request and main-push
    runs, where the drift is not the change's own (a PR's growth is gated against its base)."""
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
            f"::warning::the stripped release binary for {target} is {change:+.2%} past its budget (a report here)",
            line,
            f"The scheduled binary-size job tracks this in a `{LABEL}` issue and release.yml fails on it.",
            "Shrink the binary, or accept the size in a PR of its own: " + edit,
        ]
    if v == "over":
        return 1, [
            f"::error::the stripped release binary for {target} grew {change:+.2%} past its budget (#7390)",
            line,
            "Shrink the binary, or accept the size in a PR of its own: " + edit,
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


def gh(*args):
    """`gh <args>`'s stdout; a failure is a RuntimeError naming the command."""
    out = subprocess.run(["gh", *args], capture_output=True, text=True, check=False)
    if out.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args[:2])}: {out.stderr.strip()}")
    return out.stdout


ANNOTATION = re.compile(r"^::\w+::")  # a workflow-command prefix, which an issue body shows as text


def issue_title(target):
    return f"ci: the stripped release binary for {target} is over its size budget"


def tracking_body(target, lines, measured_at):
    """The tracking issue's body: what was measured where, and how the issue ends."""
    shown = "".join(f"    {ANNOTATION.sub('', line)}\n" for line in lines)
    return (
        f"The scheduled `Binary size budget` job (`.github/workflows/scheduled.yml`, #7959) found main's stripped "
        f"release binary for `{target}` over its budget in `tests/data/binary-size-budget.json`:\n\n"
        f"{shown}\n"
        f"Measured: {measured_at}.\n\n"
        "No pull request failed on this: each one is held to `max_pr_growth` against its base binary, and this is "
        "the drift of many. `release.yml` fails while it lasts. Shrink the binary, or accept the size in a pull "
        "request of its own that edits the budget (taking the number from a CI log, not a local build) and closes "
        "this issue.\n\n"
        "Each daily run rewrites this body while the size stays over; nothing closes the issue automatically.\n"
    )


def file_tracking_issue(target, body):
    """Open `target`'s tracking issue, or rewrite the body of the open one; says which it did."""
    title = issue_title(target)
    listed = gh("issue", "list", "--repo", REPO, "--label", LABEL, "--state", "open", "--limit", "100", "--json", "number,title,body")
    for issue in json.loads(listed or "[]"):
        if issue.get("title") == title:
            number = str(issue["number"])
            if (issue.get("body") or "").strip() == body.strip():
                return f"tracking issue #{number} already says this"
            gh("issue", "edit", number, "--repo", REPO, "--body", body)
            return f"rewrote tracking issue #{number}"
    gh("label", "create", LABEL, "--repo", REPO, "--description", "the stripped release binary is over its size budget", "--force")
    url = gh("issue", "create", "--repo", REPO, "--title", title, "--label", LABEL, "--body", body).strip()
    return f"opened tracking issue {url}"


def track(measured, target, config, budget_path, measured_at, issue):
    """`(exit code, lines to print)` for main's binary: the budget enforced, filed as an issue when over.

    With `issue`, a size over the budget (or a target with none) opens or rewrites the target's tracking
    issue and exits 0, since the issue is the signal; without it, it exits 1 as `check` does."""
    code, lines = check(measured, target, config, budget_path)
    lines = lines + [f"Measured: {measured_at}."]
    if code and issue:
        lines.append(file_tracking_issue(target, tracking_body(target, lines[:-1], measured_at)))
        return 0, lines
    return code, lines


def emit(lines):
    for line in lines:
        print(line)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write("### Stripped binary size (#7390)\n\n")
            f.write("".join(f"    {line}\n" for line in lines) + "\n")


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
    t = sub.add_parser("track", help="enforce the budget on main's binary, filing a tracking issue when over")
    t.add_argument("--binary", required=True, help="main's release binary (left unstripped)")
    t.add_argument("--target", required=True, help="target triple to look up")
    t.add_argument("--budget", default=BUDGET, help="budget file (default: %(default)s)")
    t.add_argument("--strip", default="strip", help="strip program (default: %(default)s)")
    t.add_argument("--measured-at", default="an unnamed binary", help="where the binary came from, for the issue")
    t.add_argument("--issue", action="store_true", help="open or rewrite the target's tracking issue when over")
    args = parser.parse_args(argv)

    with open(args.budget, encoding="utf-8") as f:
        config = json.load(f)
    target = args.target or host_target()
    measured = stripped_size(args.binary, args.strip)
    if args.command == "track":
        try:
            code, lines = track(measured, target, config, args.budget, args.measured_at, args.issue)
        except RuntimeError as e:
            print(f"binary_size: {e}", file=sys.stderr)
            return 2
        emit(lines)
        return code
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
    emit(lines)
    return code


if __name__ == "__main__":
    sys.exit(main())
