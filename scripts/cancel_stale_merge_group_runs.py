#!/usr/bin/env python3
"""Cancel the workflow runs of merge groups the queue has destroyed (#7552).

    python3 scripts/cancel_stale_merge_group_runs.py [--repo OWNER/REPO] [--dry-run]

When the merge queue ejects an entry or a PR leaves it, the queue deletes the affected groups'
`gh-readonly-queue/<base>/pr-N-<sha>` branches and builds new groups, but the runs it started for
the deleted groups go on running (no workflow's `concurrency` cancels a `merge_group` run, and a
shared group would cancel live groups too, since the queue tests several at once). Their result
can never be used, yet each still holds its macOS jobs. `.github/workflows/cancel-stale-merge-groups.yml`
runs this on every `merge_group` event (each rebuild creates groups, so it fires just after the
stale runs appear), on `pull_request` `dequeued` and by hand.

A run is stale when it was started by `merge_group`, has not completed, and its head branch is not
among the live `gh-readonly-queue/` refs. The runs are listed BEFORE the refs: a group created in
between has its branch in the ref list, so it is never taken for a destroyed one. A run of this
sweep's own workflow is never cancelled. A token that may not cancel (a `pull_request` event from
a fork has a read-only one) is reported and the script exits 0; so does a run that completed
before its cancel arrived.

Needs `gh` on `PATH` with a token (`GH_TOKEN` in CI). `--dry-run` lists without cancelling.
"""

import argparse
import json
import os
import subprocess
import sys

QUEUE_PREFIX = "gh-readonly-queue/"
ACTIVE_STATUSES = ("queued", "in_progress", "waiting", "pending", "requested")
SELF_WORKFLOW = ".github/workflows/cancel-stale-merge-groups.yml"


class GhError(Exception):
    def __init__(self, args, stderr):
        super().__init__(f"gh {' '.join(args)}: {stderr.strip()}")
        self.stderr = stderr


def gh(args):
    """stdout of `gh <args>`, or GhError."""
    out = subprocess.run(["gh", *args], capture_output=True, text=True, check=False)
    if out.returncode != 0:
        raise GhError(args, out.stderr or out.stdout)
    return out.stdout


def json_lines(text):
    return [json.loads(line) for line in text.splitlines() if line.strip()]


def active_merge_group_runs(repo, run=gh):
    """Every non-completed `merge_group` run, one dict per run id."""
    runs = {}
    for status in ACTIVE_STATUSES:
        out = run(
            [
                "api",
                "--paginate",
                f"repos/{repo}/actions/runs?event=merge_group&status={status}&per_page=100",
                "--jq",
                ".workflow_runs[] | {id, name, path, status, head_branch}",
            ]
        )
        for r in json_lines(out):
            runs.setdefault(r["id"], r)
    return list(runs.values())


def live_queue_branches(repo, run=gh):
    """The branch names of the queue's live groups (`gh-readonly-queue/main/pr-...`)."""
    try:
        out = run(["api", "--paginate", f"repos/{repo}/git/matching-refs/heads/{QUEUE_PREFIX}", "--jq", ".[].ref"])
    except GhError as e:
        # The run list for this repository has just been read, so a 404 here means no ref matched.
        if "HTTP 404" in e.stderr:
            return set()
        raise
    return {line.strip().removeprefix("refs/heads/") for line in out.splitlines() if line.strip()}


def stale_runs(runs, live, own_run_id=None, own_workflow=SELF_WORKFLOW):
    """The runs to cancel: a queue branch that is gone, and not this sweep's own run or workflow."""
    out = []
    for r in runs:
        branch = r.get("head_branch") or ""
        if not branch.startswith(QUEUE_PREFIX) or branch in live:
            continue
        if own_run_id is not None and str(r["id"]) == str(own_run_id):
            continue
        if (r.get("path") or "").split("@")[0] == own_workflow:
            continue
        out.append(r)
    return sorted(out, key=lambda r: r["id"])


def main(argv=None, run=gh, environ=None):
    environ = os.environ if environ is None else environ
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--repo", default=environ.get("GITHUB_REPOSITORY") or "rust-works/succinctly")
    p.add_argument("--dry-run", action="store_true", help="list the stale runs, cancel nothing")
    p.add_argument("--own-run-id", default=environ.get("GITHUB_RUN_ID"), help="never cancel this run")
    args = p.parse_args(argv)

    # Order matters: runs first, then refs (see the module docstring).
    runs = active_merge_group_runs(args.repo, run)
    live = live_queue_branches(args.repo, run)
    stale = stale_runs(runs, live, args.own_run_id)
    print(f"{len(runs)} active merge_group run(s), {len(live)} live queue branch(es), {len(stale)} stale")
    for b in sorted(live):
        print(f"  live: {b}")
    for r in stale:
        what = f"run {r['id']} ({r.get('name')}, {r.get('status')}) of destroyed group {r['head_branch']}"
        if args.dry_run:
            print(f"  would cancel {what}")
            continue
        try:
            run(["api", "-X", "POST", f"repos/{args.repo}/actions/runs/{r['id']}/cancel", "--silent"])
        except GhError as e:
            if "HTTP 403" in e.stderr or "Resource not accessible" in e.stderr:
                print(f"::notice::this token may not cancel runs (a fork's read-only token?); stopping: {e}")
                return 0
            # 409: the run completed, or is already being cancelled, since it was listed.
            print(f"::warning::could not cancel {what}: {e}")
            continue
        print(f"::notice::cancelled {what}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
