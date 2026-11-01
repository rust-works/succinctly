#!/usr/bin/env python3
"""Fail a branch that references a `to-triage` ledger entry's issue but leaves the entry in (#5585).

`comparison/jq/divergences/<suite>/<probe>.toml` lists every known difference from the
reference jq, one entry per file (#5769). A `to-triage` entry carries the issue that tracks it. The branch that fixes that issue has to delete
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

LEDGER = "comparison/jq/divergences"
OLD_LEDGER = "comparison/jq/divergences.toml"  # the single file before #5769
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


_STRING_FIELD = re.compile(r'^(\w+)\s*=\s*"((?:[^"\\]|\\.)*)"\s*(?:#.*)?$', re.MULTILINE)
_ENTRY_HEADER = re.compile(r"^\[\[divergence\]\]\s*$", re.MULTILINE)
_ENTRY_PATH = re.compile(r"([A-Za-z0-9_-]+)/([A-Za-z0-9_-]+)(?:\.([0-9]+))?\.toml")


def entry_id(rel):
    """The id of the entry at `rel` (`suite/probe.toml`, `suite/probe.N.toml`), or None.

    `features/strftime.1.toml` is `features:strftime/1`; the id is the path (#5769).
    """
    m = _ENTRY_PATH.fullmatch(rel)
    if not m:
        return None
    suite, probe, arity = m.groups()
    return f"{suite}:{probe}" + (f"/{arity}" if arity else "")


def _triage_issue(fields):
    m = re.fullmatch(r"#(\d+)", fields.get("issue", "").strip())
    return int(m.group(1)) if fields.get("category") == "to-triage" and m else None


def to_triage_files(files):
    """`{issue number: [entry ids]}` for the `to-triage` entries that name an issue.

    `files` maps a path under the ledger directory to its text. Reads the entry's own shape
    (`key = "string"` lines) rather than importing `tomllib`, which the Python that macOS ships
    (3.9) lacks; the Rust loader is the strict parser, and CI's `Ledger drift` job runs it.
    """
    found = {}
    for rel in sorted(files):
        id_ = entry_id(rel)
        issue = _triage_issue(dict(_STRING_FIELD.findall(files[rel])))
        if id_ and issue is not None:
            found.setdefault(issue, []).append(id_)
    return found


def to_triage(text):
    """[`to_triage_files`] for the single-file ledger of before #5769 (`[[divergence]]` blocks)."""
    found = {}
    for block in _ENTRY_HEADER.split(text)[1:]:
        fields = dict(_STRING_FIELD.findall(block))
        issue = _triage_issue(fields)
        if issue is not None and "id" in fields:
            found.setdefault(issue, []).append(fields["id"])
    return found


def read_dir(root):
    """`{path under the ledger directory: text}` for the working tree."""
    base = os.path.join(root, LEDGER)
    files = {}
    for dirpath, _, names in os.walk(base):
        for name in names:
            path = os.path.join(dirpath, name)
            rel = os.path.relpath(path, base).replace(os.sep, "/")
            with open(path, encoding="utf-8") as f:
                files[rel] = f.read()
    return files


def read_tree(ref, root):
    """The same at `ref`, from git without a checkout. Empty when `ref` has no ledger directory."""
    listing = [
        path
        for path in git("ls-tree", "-r", "-z", "--name-only", ref, "--", LEDGER, cwd=root).split("\0")
        if path
    ]
    if not listing:
        return {}
    request = "".join(f"{ref}:{path}\n" for path in listing)
    out = subprocess.run(
        ["git", "cat-file", "--batch"], cwd=root, input=request.encode(), capture_output=True
    )
    if out.returncode != 0:
        raise GitError(f"git cat-file --batch: {out.stderr.decode().strip()}")
    data, files, pos = out.stdout, {}, 0
    for path in listing:
        end = data.index(b"\n", pos)
        header = data[pos:end].split()
        if len(header) != 3 or header[1] != b"blob":
            raise GitError(f"git cat-file: {data[pos:end].decode(errors='replace')}")
        size = int(header[2])
        files[path[len(LEDGER) + 1 :]] = data[end + 1 : end + 1 + size].decode("utf-8")
        pos = end + 1 + size + 1
    return files


def triage_at(ref, root):
    """`to_triage` of the ledger at `ref`: the directory, or the single file before #5769."""
    files = read_tree(ref, root)
    if files:
        return to_triage_files(files)
    try:
        return to_triage(git("show", f"{ref}:{OLD_LEDGER}", cwd=root))
    except GitError:
        return {}


def referenced_issues(branch, subjects):
    refs = {int(n) for n in BRANCH_REF.findall(branch or "")}
    for subject in subjects:
        refs.update(int(n) for n in SUBJECT_REF.findall(subject))
    return refs


def waived_issues(messages):
    return {int(n) for message in messages for n in WAIVER.findall(message)}


def stale_looking(at_base, at_head, branch, subjects, messages):
    """`[(issue, entry id)]` for each entry the branch should have deleted.

    `at_base` and `at_head` are `{issue: [entry ids]}` ([`to_triage_files`]).
    """
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
            f"{LEDGER}/ still lists an entry for an issue this branch references:",
            *lines,
            "",
            "If the branch fixes the issue, delete the entry's file in the same change; CI's `Ledger drift`",
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
        at_base = triage_at(base, root)
        if os.path.isdir(os.path.join(root, LEDGER)):
            at_head = to_triage_files(read_dir(root))
        else:
            # A branch cut before #5769 still has the single file.
            with open(os.path.join(root, OLD_LEDGER), encoding="utf-8") as f:
                at_head = to_triage(f.read())
    except (GitError, OSError) as e:
        print(f"ledger_refs: {e}", file=sys.stderr)
        return 2

    problems = stale_looking(at_base, at_head, branch, subjects, messages)
    if problems:
        print(report(problems), file=sys.stderr)
        return 1
    print("ledger_refs: no to-triage entry is left behind by this branch")
    return 0


if __name__ == "__main__":
    sys.exit(main())
