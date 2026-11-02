#!/usr/bin/env python3
"""Flag a CI job whose time jumps well past its recent level (#5867).

A `Test` leg can get several times slower and stay green: the `no-std` legs went from about 40 s
to 266-593 s of lib-test time after #5232 (#5845) and stayed there nine days, and the macOS
`cli-gated` leg crept from 10.6 to 15.4 minutes in a week (#5494). Every merge-group run waits on
its longest leg, so the cost is paid on every merge. Nothing failed, so nothing said so.

    python3 scripts/ci_leg_times.py report [--as-of ISO] [--issue]
    python3 scripts/ci_leg_times.py thresholds             # print the thresholds in use

`report` reads the jobs of the successful `merge_group` runs of `ci.yml` through the Actions API
(`gh api`, so it needs `GH_TOKEN` and `actions: read`). It compares the median time of each job in
the newest `RECENT` runs against its median over `BASELINE_RUNS` runs sampled evenly from the
`DAYS` before them, and flags a job that is both `FACTOR` times and `MIN_ABS` seconds slower. The
median over several runs is what keeps runner noise out: adjacent merge-group runs measured the
same `no-std` leg at 4.8 and 6.9 minutes, and a single run would flag that. A job needs
`MIN_BASELINE` baseline samples and `MIN_RECENT` recent ones, and a baseline median of at least
`MIN_SECONDS`, so a leg that is new, or that only aggregates others, is never flagged.

This is a report, not a required check: it exits 0 whatever it finds, and 2 only when the Actions API cannot be read (the workflow job is `continue-on-error`). The markdown goes to stdout
(and to `$GITHUB_STEP_SUMMARY` in a workflow). With `--issue` a flag also opens one tracking issue
labelled `ci-slowdown`, or adds a comment to the one that is open, at most one comment a day. Closing
that issue is an acknowledgement: nothing new is filed for 14 days. The baseline follows a slowdown that is
left alone, so the report stops by itself within a few days.

`--as-of` ends the window at a past time, which replays a regression: the one this was written for
is flagged within a day or two of 2026-10-25 (the replay is in #5867's PR).
"""

import argparse
import json
import os
import re
import statistics
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta, timezone

REPO = os.environ.get("GITHUB_REPOSITORY", "rust-works/succinctly")
WORKFLOW = "ci.yml"
EVENT = "merge_group"

RECENT = 4  # newest runs whose median is the "now"
BASELINE_RUNS = 30  # runs sampled for the baseline
DAYS = 14  # how far back the baseline reaches
FACTOR = 1.5  # slower by this ratio ...
MIN_ABS = 120  # ... and by this many seconds
MIN_BASELINE = 8  # baseline samples a job needs
MIN_RECENT = 3  # recent samples a job needs
MIN_SECONDS = 60  # a job whose baseline median is shorter is not worth a report

COMMENT_GAP = timedelta(hours=20)  # between comments on the tracking issue
SILENCE = timedelta(days=14)  # a tracking issue closed this recently is an acknowledgement
ATTEMPTS = 3  # tries for one `gh api` read (a GitHub 5xx is not a finding)
MAX_LEFT_OUT = 0.25  # of the runs: more unreadable than this and the report is not trusted
CLOSED_SEEN = 30  # closed tracking issues looked at (`gh` lists them by creation, not by when they closed)
TRANSIENT = re.compile(r"HTTP 5\d\d|timed? ?out|connection|EOF|temporar|try again", re.IGNORECASE)
LABEL = "ci-slowdown"
TITLE = "ci: a CI job got much slower than its recent level"


def parse_time(text):
    return datetime.strptime(text, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)


def is_read(args):
    """Whether `gh <args>` only reads: `gh api` with no method, field or input flag."""
    return args[:1] == ("api",) and not any(
        a in ("-X", "--method", "-f", "-F", "--raw-field", "--field", "--input") or a.startswith("--method=")
        for a in args[1:]
    )


def gh(*args):
    """`gh <args>`'s stdout. A read through the API is retried when it failed the way a flaky network or a
    GitHub 5xx fails (a 404 or a rate limit is not going to get better); a write is never retried."""
    attempts = ATTEMPTS if is_read(args) else 1
    for attempt in range(attempts):
        out = subprocess.run(["gh", *args], capture_output=True, text=True, check=False)
        if out.returncode == 0:
            return out.stdout
        if attempt + 1 < attempts and TRANSIENT.search(out.stderr):
            time.sleep(2 * (attempt + 1))
        else:
            break
    raise RuntimeError(f"gh {' '.join(args)}: {out.stderr.strip()}")


def api(path, **params):
    query = "&".join(f"{k}={v}" for k, v in params.items())
    return json.loads(gh("api", f"{path}?{query}" if query else path))


def fetch_runs(as_of, days=DAYS):
    """Successful merge-group runs of `ci.yml` created in `[as_of - days, as_of]`, newest first."""
    floor = as_of - timedelta(days=days)
    runs, page = [], 1
    while True:
        body = api(
            f"repos/{REPO}/actions/workflows/{WORKFLOW}/runs",
            event=EVENT,
            status="success",
            per_page=100,
            page=page,
        )
        batch = body.get("workflow_runs", [])
        for run in batch:
            created = parse_time(run["created_at"])
            if created > as_of:
                continue
            if created < floor:
                return runs
            runs.append(run)
        if len(batch) < 100:
            return runs
        page += 1


def fetch_jobs(run_id):
    """`{job name: seconds}` of the jobs of one run that succeeded and took time."""
    seconds, page = {}, 1
    while True:
        body = api(f"repos/{REPO}/actions/runs/{run_id}/jobs", per_page=100, filter="latest", page=page)
        jobs = body.get("jobs", [])
        for job in jobs:
            if job.get("conclusion") != "success" or not job.get("started_at") or not job.get("completed_at"):
                continue
            seconds[job["name"]] = (parse_time(job["completed_at"]) - parse_time(job["started_at"])).total_seconds()
        if len(jobs) < 100:
            return seconds
        page += 1


def sample_evenly(items, count):
    """`count` items spread over `items` (all of them when there are fewer), in the order given."""
    if len(items) <= count:
        return list(items)
    step = len(items) / count
    return [items[int(i * step)] for i in range(count)]


def analyse(recent, baseline):
    """The flags for `recent` and `baseline`, each a list of `{job: seconds}` (one per run).

    Returns `[(job, baseline median, recent median)]`, the slowest-growing first.
    """
    flags = []
    for job in {name for run in recent for name in run}:
        now = [run[job] for run in recent if job in run]
        before = [run[job] for run in baseline if job in run]
        if len(now) < MIN_RECENT or len(before) < MIN_BASELINE:
            continue
        was, is_now = statistics.median(before), statistics.median(now)
        if was < MIN_SECONDS:
            continue
        if is_now >= FACTOR * was and is_now - was >= MIN_ABS:
            flags.append((job, was, is_now))
    return sorted(flags, key=lambda f: f[2] - f[1], reverse=True)


def minutes(seconds):
    return f"{seconds / 60:.1f} min"


def render(flags, recent_runs, baseline_runs, as_of):
    lines = [
        f"## CI job times as of {as_of:%Y-%m-%d %H:%M} UTC",
        "",
        f"Median of the newest {recent_runs} successful `merge_group` runs of `{WORKFLOW}` against the "
        f"median of {baseline_runs} runs sampled from the {DAYS} days before. A job is flagged when it is at "
        f"least {FACTOR}x and {MIN_ABS} s slower.",
        "",
    ]
    if not flags:
        lines.append("No job is flagged.")
        return "\n".join(lines) + "\n"
    lines += ["| job | baseline median | recent median | ratio |", "|-----|----------------:|--------------:|------:|"]
    for job, was, is_now in flags:
        lines.append(f"| `{job}` | {minutes(was)} | {minutes(is_now)} | {is_now / was:.1f}x |")
    lines += [
        "",
        "Runner noise is large (the same leg has measured 4.8 and 6.9 minutes in adjacent runs), which is why this "
        "compares medians over several runs. A flagged job is a prompt to read its log: `finished in` lines "
        "name the test binary that grew. This is a report, not a check; it does not block the merge queue (#5867).",
    ]
    return "\n".join(lines) + "\n"


def report(as_of, issue):
    runs = fetch_runs(as_of)
    recent_runs, rest = runs[:RECENT], runs[RECENT:]
    baseline_runs = sample_evenly(rest, BASELINE_RUNS)
    left_out = []

    def jobs_of(run):
        try:
            return fetch_jobs(run["id"])
        except RuntimeError as e:  # after the retries: leave this run out rather than lose the report
            print(f"ci_leg_times: run {run['id']} left out: {e}", file=sys.stderr)
            left_out.append(run["id"])
            return None

    with ThreadPoolExecutor(max_workers=8) as pool:  # each is a `gh api` round trip of a few seconds
        jobs = list(pool.map(jobs_of, recent_runs + baseline_runs))
    if len(left_out) > MAX_LEFT_OUT * len(jobs):
        raise RuntimeError(f"{len(left_out)} of {len(jobs)} runs could not be read; not reporting on the rest")
    recent_jobs, baseline_jobs = jobs[: len(recent_runs)], jobs[len(recent_runs) :]
    recent = [j for j in recent_jobs if j is not None]
    baseline = [j for j in baseline_jobs if j is not None]
    flags = analyse(recent, baseline)
    text = render(flags, len(recent), len(baseline), as_of)
    sys.stdout.write(text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write(text)
    if issue and flags:
        file_issue(text, as_of, [job for job, _, _ in flags])
    return 0


def acknowledged(jobs, as_of):
    """Whether every job in `jobs` is named by a `ci-slowdown` issue closed in the last `SILENCE`.

    Closing the tracking issue is the acknowledgement, so a step that has been looked at (a leg that was
    deliberately made longer) stays quiet while the baseline catches up. It covers the jobs that issue named:
    a different job that regresses meanwhile is still filed. `gh` lists issues by creation, so several are read.
    """
    closed = gh(
        "issue", "list", "--repo", REPO, "--label", LABEL, "--state", "closed", "--limit", str(CLOSED_SEEN),
        "--json", "closedAt,body",
    ).strip()
    named = ""
    for issue in json.loads(closed or "[]"):
        if not issue.get("closedAt"):
            continue
        # A close after `as_of` (a replay of an earlier day) is not an acknowledgement yet.
        if timedelta(0) <= as_of - parse_time(issue["closedAt"]) < SILENCE:
            named += issue.get("body") or ""
    return bool(jobs) and all(f"`{job}`" in named for job in jobs)


def file_issue(text, as_of, jobs=()):
    """One open tracking issue: created if there is none, else commented on at most once a day."""
    open_issues = json.loads(
        gh("issue", "list", "--repo", REPO, "--label", LABEL, "--state", "open", "--json", "number,comments,title")
    )
    if not open_issues:
        if acknowledged(jobs, as_of):
            print(f"Every flagged job is named by a `{LABEL}` issue closed within {SILENCE.days} days: not filing another.")
            return
        gh("label", "create", LABEL, "--repo", REPO, "--description", "a CI job got much slower", "--force")
        gh("issue", "create", "--repo", REPO, "--title", TITLE, "--label", LABEL, "--body", text)
        return
    number = open_issues[0]["number"]
    created = gh("api", "--paginate", f"repos/{REPO}/issues/{number}/comments?per_page=100", "--jq", ".[].created_at").split()
    # Less than a day, but the daily run starts a few minutes later each time it is delayed, so a
    # gap of 23h58m is a new day.
    if created and as_of - parse_time(created[-1]) < COMMENT_GAP:
        return
    gh("issue", "comment", str(number), "--repo", REPO, "--body", text)


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("command", choices=["report", "thresholds"])
    parser.add_argument("--as-of", help="end the window at this UTC time (2026-10-27T00:00:00Z)")
    parser.add_argument("--issue", action="store_true", help="open or update the tracking issue on a flag")
    args = parser.parse_args(argv)
    if args.command == "thresholds":
        for name in ("RECENT", "BASELINE_RUNS", "DAYS", "FACTOR", "MIN_ABS", "MIN_BASELINE", "MIN_RECENT", "MIN_SECONDS"):
            print(f"{name} = {globals()[name]}")
        return 0
    as_of = parse_time(args.as_of) if args.as_of else datetime.now(timezone.utc).replace(microsecond=0)
    try:
        return report(as_of, args.issue)
    except RuntimeError as e:
        print(f"ci_leg_times: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
