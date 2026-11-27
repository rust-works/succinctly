#!/usr/bin/env python3
"""File the link-order list's regeneration as an issue once it is close to stale (#7894 follow-up).

    python3 scripts/link-order/watch.py [--issue] [--threshold PERCENT]

ci.yml's perf-guard job prints `link-order list: N listed, M found, K no longer resolve (S%, limit
5%)` on every run (`generate.py stale`), and fails on a share over the limit only on release branches:
a pull request that merely meets a stale list is warned, not failed, since the list goes stale from
every merge that renames a hot function and the fix (`generate.py linux` on x86_64 Linux, then
`generate.py macos` on Apple silicon) is no ordinary PR's to make, and two PRs that both regenerate it
conflict.

So this reads that line from the x86_64 perf-guard job of the newest completed `push` run of ci.yml
on `main`, prints it (and to `$GITHUB_STEP_SUMMARY`), and with `--issue`, once the share passes
THRESHOLD (default 4, a point below the limit: about 15 of the 1536 names the November 2026
regeneration listed), opens one tracking issue or rewrites the body of the open one.
Editing the body, not commenting, keeps a daily job from adding a comment a day.
scheduled.yml runs it.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from ci_leg_times import REPO, api, gh  # noqa: E402

TITLE = "perf(build): regenerate the link-order list (scripts/link-order/symbols.txt)"
JOB = "Perf Regression Guard (x86_64)"
THRESHOLD = 4.0
ESCAPES = "--allow-escape-sequences"
LINE = re.compile(r"link-order list: (\d+) listed, (\d+) found, (\d+) no longer resolve "
                  r"\(([\d.]+)%, limit ([\d.]+)%\)")


def parse(log):
    """(listed, found, unresolved, share, limit) from the last `link-order list:` line of a job
    log, or None when the job printed none (a run before #7894, or one that failed before the step)."""
    found = LINE.findall(log)
    if not found:
        return None
    listed, ok, gone, share, limit = found[-1]
    return int(listed), int(ok), int(gone), float(share), float(limit)


def job_log(job_id, run=gh):
    """A job's log. A job log holds terminal escapes (the runner colours each step's command), and a
    recent `gh` (the runners') refuses to print a response holding them unless passed ESCAPES, a flag
    older ones (2.96) reject: so ask without it, and again with it only when `gh` says to
    (2026-11-27's scheduled run failed on the first ask)."""
    path = f"repos/{REPO}/actions/jobs/{job_id}/logs"
    try:
        return run("api", path)
    except RuntimeError as e:
        if ESCAPES not in str(e):
            raise
        return run("api", ESCAPES, path)


def newest_reading(runs=10):
    """(run, counts) for the newest completed main `push` run of ci.yml whose x86_64 perf-guard job
    printed the line, or (None, None)."""
    page = api(f"repos/{REPO}/actions/workflows/ci.yml/runs", branch="main", event="push",
               status="completed", per_page=runs)
    for run in page.get("workflow_runs", []):
        jobs = api(f"repos/{REPO}/actions/runs/{run['id']}/jobs", per_page=100, filter="latest")
        for job in jobs.get("jobs", []):
            if job["name"] == JOB and job.get("conclusion") in ("success", "failure"):
                counts = parse(job_log(job["id"]))
                if counts:
                    return run, counts
    return None, None


def body(run, counts, threshold):
    listed, found, gone, share, limit = counts
    return (
        f"At {run['head_sha'][:9]} ({run['html_url']}) the x86_64 link resolves {found} of the {listed} names "
        f"in `scripts/link-order/symbols.txt`: {gone} no longer resolve, {share:.1f}% against "
        f"`generate.py stale`'s limit of {limit:g}% (this issue opens at {threshold:g}%).\n\n"
        "Every merge that renames, splits or adds a function on a hot path (or adds a closure ahead of others "
        "in one: `{closure#N}` renumbers) leaves it outside the ordered block, where it keeps a page of the "
        "binary resident in every run that calls it (#7894).\n\n"
        "Regenerate it in one PR, as `docs/guides/release.md` says: `generate.py linux` from a release build on "
        "x86_64 Linux with valgrind, then `generate.py macos` on Apple silicon, then time the new list against the "
        "old one interleaved with an A/A' control (`docs/guides/benchmarking.md`).\n\n"
        "Pull requests only warn on a stale list; release branches and release builds fail on it.\n\n"
        "Filed by `scripts/link-order/watch.py` (scheduled.yml), which rewrites this body while the issue is open."
    )


def decide(share, threshold, open_numbers):
    """'create', 'edit' or None."""
    if share <= threshold:
        return None
    return "edit" if open_numbers else "create"


def main(argv):
    issue = "--issue" in argv
    threshold = float(argv[argv.index("--threshold") + 1]) if "--threshold" in argv else THRESHOLD
    run, counts = newest_reading()
    if counts is None:
        print("no recent main push run of ci.yml printed a `link-order list:` line")
        return 0
    text = body(run, counts, threshold)
    print(text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(text + "\n")
    if not issue:
        return 0
    import json
    opened = json.loads(gh("issue", "list", "--repo", REPO, "--state", "open", "--search", f"in:title {TITLE}",
                           "--json", "number,title"))
    numbers = [i["number"] for i in opened if i["title"] == TITLE]
    action = decide(counts[3], threshold, numbers)
    if action == "create":
        gh("issue", "create", "--repo", REPO, "--title", TITLE, "--body", text)
    elif action == "edit":
        gh("issue", "edit", str(numbers[0]), "--repo", REPO, "--body", text)
    print(f"issue: {action or 'none needed'}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
