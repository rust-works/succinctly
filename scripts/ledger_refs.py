#!/usr/bin/env python3
"""Fail a branch that references a `to-triage` ledger entry's issue but leaves the entry in (#5585).

`comparison/jq/divergences.toml` lists every known difference from the reference jq. A
`to-triage` entry carries the issue that tracks it. The branch that fixes that issue has to delete
the entry in the same change: CI's `Ledger drift` job fails on "listed but no longer diverging"
otherwise, after a push, a build and a wait (#5583 needed a third commit for exactly this).

    python3 scripts/ledger_refs.py check [--base REF] [--branch NAME]

A branch references issue N when its name contains `issue-N` or when a commit subject in
`BASE..HEAD` contains `#N`. The check fails for each `to-triage` entry with `issue = "#N"` for a
referenced N that exists both at the merge base and at HEAD, so a branch that files the entry
itself is not asked to delete it. Only `to-triage` entries are checked: a `by-design` entry keeps
its issue link on purpose.

A branch that mentions the issue without fixing it (a partial fix, a follow-up) states so with a
line `[ledger keeps #N]` in any commit message in the range.

BASE defaults to the merge base of HEAD and `origin/main`; BRANCH to the current branch.
Exit 0 when clean, 1 on a stale-looking entry, 2 on a usage or git error.
"""

import argparse
import os
import re
import subprocess
import sys
import tomllib

LEDGER = "comparison/jq/divergences.toml"
DEFAULT_BASE = "origin/main"

BRANCH_REF = re.compile(r"(?<![0-9A-Za-z])issue-(\d+)(?![0-9])")
SUBJECT_REF = re.compile(r"#(\d+)(?![0-9])")
WAIVER = re.compile(r"^\s*\[ledger keeps #(\d+)\]\s*$", re.MULTILINE)


class GitError(Exception):
    pass


def git(*args, cwd=None):
    out = subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, text=True, check=False
    )
    if out.returncode != 0:
        raise GitError(f"git {' '.join(args)}: {out.stderr.strip()}")
    return out.stdout


def to_triage(text):
    """`{issue number: [entry ids]}` for the `to-triage` entries that name an issue."""
    found = {}
    for entry in tomllib.loads(text).get("divergence", []):
        if entry.get("category") != "to-triage":
            continue
        m = re.fullmatch(r"#(\d+)", (entry.get("issue") or "").strip())
        if m:
            found.setdefault(int(m.group(1)), []).append(entry["id"])
    return found


def referenced_issues(branch, subjects):
    refs = {int(n) for n in BRANCH_REF.findall(branch or "")}
    for subject in subjects:
        refs.update(int(n) for n in SUBJECT_REF.findall(subject))
    return refs


def waived_issues(messages):
    return {int(n) for message in messages for n in WAIVER.findall(message)}


def stale_looking(base_text, head_text, branch, subjects, messages):
    """`[(issue, entry id)]` for each entry the branch should have deleted."""
    at_base = to_triage(base_text) if base_text else {}
    at_head = to_triage(head_text) if head_text else {}
    refs = referenced_issues(branch, subjects) - waived_issues(messages)
    return sorted(
        (issue, entry)
        for issue in refs
        for entry in at_head.get(issue, [])
        if entry in at_base.get(issue, [])
    )


def report(problems):
    lines = []
    for issue, entry in problems:
        lines.append(
            f"  {entry}: a `to-triage` entry for #{issue}, which this branch references"
        )
    return "\n".join(
        [
            f"{LEDGER} still lists an entry for an issue this branch references:",
            *lines,
            "",
            "If the branch fixes the issue, delete the entry in the same change; CI's `Ledger drift`",
            "job fails on a listed entry that no longer differs. If it only mentions the issue",
            "(a partial fix or a follow-up), put `[ledger keeps #N]` on its own line in a commit",
            "message to say so.",
        ]
    )


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    check = sub.add_parser("check", help="fail on a to-triage entry the branch should have deleted")
    check.add_argument("--base", help=f"merge-base ref (default: merge base with {DEFAULT_BASE})")
    check.add_argument("--branch", help="branch name (default: the current branch)")
    args = ap.parse_args(argv)

    try:
        root = git("rev-parse", "--show-toplevel").strip()
        base = args.base or git("merge-base", "HEAD", DEFAULT_BASE).strip()
        branch = args.branch
        if branch is None:
            branch = git("rev-parse", "--abbrev-ref", "HEAD", cwd=root).strip()
        subjects = git("log", "--format=%s", f"{base}..HEAD", cwd=root).splitlines()
        messages = git("log", "--format=%B%x00", f"{base}..HEAD", cwd=root).split("\0")
        try:
            base_text = git("show", f"{base}:{LEDGER}", cwd=root)
        except GitError:
            base_text = ""
        with open(os.path.join(root, LEDGER), encoding="utf-8") as f:
            head_text = f.read()
    except (GitError, OSError) as e:
        print(f"ledger_refs: {e}", file=sys.stderr)
        return 2

    problems = stale_looking(base_text, head_text, branch, subjects, messages)
    if problems:
        print(report(problems), file=sys.stderr)
        return 1
    print("ledger_refs: no to-triage entry is left behind by this branch")
    return 0


if __name__ == "__main__":
    sys.exit(main())
