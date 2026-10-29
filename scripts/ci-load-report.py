#!/usr/bin/env python3
"""Measure CI runner load and macOS queue wait from the Actions jobs API (#3649).

#3649 cut CI's runner load in steps and judged each one by the same report: per-run
wall-clock and macOS-minutes, the worst macOS queue wait in a run, and the queue wait
of each macOS job *at a given load*, bucketed by how many macOS jobs of other runs were
queued or running when it was created. The load-adjusted table is the one that matters:
macOS runners are capped at five concurrent jobs, so a raw queue-wait percentile mostly
says how busy the window was, and two windows only compare at equal load.

Each `--window NAME=SINCE..UNTIL` is reported side by side. A run belongs to every window
its `created_at` falls in, so a week and its last day can be compared with what followed; load is counted over every fetched macOS job, including those
of runs created up to `--margin` hours before a window opens and of the workflows that
are not reported on (the scheduled bench, a release), since they hold the same slots.

    scripts/ci-load-report.py --cache .ai/scratch/ci-load \\
        --window before=2026-10-21T00:00:00Z..2026-10-28T12:17:00Z \\
        --window after=2026-10-28T12:17:00Z..2026-10-29T12:17:00Z

Definitions:
- queue wait = `started_at - created_at` of a job that ran (success or failure);
- run time = `completed_at - started_at`;
- a job holds a macOS slot over `[created_at, completed_at)` unless it was skipped;
- run wall-clock = last job's `completed_at` - run `created_at`, first attempts only;
- a macOS job is one whose labels name a `macos` runner.

Run-level rows count successful first-attempt runs with at least one macOS job, so a
cancelled or re-run run does not stretch a percentile; queue-wait and leg rows count
first-attempt jobs, since a re-run's jobs can carry an earlier attempt's timestamps. A job
still queued or running holds its slot to the end of the report. Needs `gh` (authenticated).
Jobs of a run whose jobs had all completed are cached in `--cache` and never refetched.
Standard library only.
"""

import argparse
import collections
import concurrent.futures
import heapq
import json
import os
import statistics
import subprocess
import sys
import time
from datetime import datetime, timedelta, timezone

REPO = "rust-works/succinctly"
# The workflows that run macOS jobs. Only `ci.yml` runs are reported on; the others
# count towards the load a `ci.yml` job queued behind.
WORKFLOWS = ("ci.yml", "scheduled.yml", "release.yml")
REPORTED_WORKFLOW = "ci.yml"
EVENTS = ("merge_group", "pull_request")
BUCKETS = ("0-1", "2-3", "4-5", "6-8", "9+")
JOB_FIELDS = ("name", "status", "conclusion", "created_at", "started_at", "completed_at", "labels", "run_attempt")
RUN_FIELDS = ("id", "status", "event", "conclusion", "created_at", "run_attempt")
# A job still queued or running holds its place in the load count until the report ends.
NOT_YET = datetime.max.replace(tzinfo=timezone.utc)


def parse_ts(text):
    return datetime.fromisoformat(text.replace("Z", "+00:00")) if text else None


def minutes(start, end):
    return (end - start).total_seconds() / 60


def percentile(values, fraction):
    """The value at index `round(fraction * (n - 1))` of the sorted values: the method
    every #3649 table used, kept so this report's figures compare with theirs."""
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(round(fraction * (len(ordered) - 1))))]


def is_macos(job):
    return any("macos" in label.lower() for label in job["labels"])


def ran(job):
    return job["conclusion"] in ("success", "failure") and job["started_at"] and job["completed_at"]


def holds_slot(job):
    return job["conclusion"] != "skipped"


def slot_end(job):
    return parse_ts(job["completed_at"]) or NOT_YET


def first_attempt(job):
    return job.get("run_attempt", 1) == 1


def unique_jobs(jobs):
    """A run's jobs with a later attempt's copies of an earlier attempt's jobs dropped.

    With `filter=all`, a re-run attempt can list a job it did not run again: same name,
    `started_at` and `completed_at` as the earlier attempt's, and a later `created_at`.
    Counting both would hold the slot twice."""
    seen = set()
    out = []
    for job in sorted(jobs, key=lambda job: job.get("run_attempt", 1)):
        key = (job["name"], job["started_at"], job["completed_at"])
        if job["started_at"] is not None and key in seen:
            continue
        seen.add(key)
        out.append(job)
    return out


def load_bucket(count):
    if count <= 1:
        return "0-1"
    if count <= 3:
        return "2-3"
    if count <= 5:
        return "4-5"
    if count <= 8:
        return "6-8"
    return "9+"


def other_run_in_flight(jobs):
    """For each job, how many jobs of *other* runs held a slot when it was created.

    `jobs` is a list of `(run_id, created, completed)`; the result is in the same order.
    A job counts another as in flight when `other.created <= created < other.completed`.
    """
    order = sorted(range(len(jobs)), key=lambda i: jobs[i][1])
    active = []  # heap of (completed, run_id)
    by_run = collections.Counter()
    counts = [0] * len(jobs)
    pos = 0
    while pos < len(order):
        created = jobs[order[pos]][1]
        group = []
        while pos < len(order) and jobs[order[pos]][1] == created:
            run_id, _, completed = jobs[order[pos]]
            heapq.heappush(active, (completed, run_id))
            by_run[run_id] += 1
            group.append(order[pos])
            pos += 1
        while active and active[0][0] <= created:
            _, run_id = heapq.heappop(active)
            by_run[run_id] -= 1
        total = sum(by_run.values())
        for index in group:
            counts[index] = total - by_run[jobs[index][0]]
    return counts


def summarize_run(run, jobs):
    """Run-level figures, or None for a run the run-level rows do not count."""
    if run["conclusion"] != "success" or run.get("run_attempt", 1) != 1:
        return None
    done = [job for job in jobs if ran(job)]
    mac = [job for job in done if is_macos(job)]
    if not mac:
        return None
    created = parse_ts(run["created_at"])
    last = max(done, key=lambda job: parse_ts(job["completed_at"]))
    return {
        "event": run["event"],
        "day": run["created_at"][:10],
        "legs": len(mac),
        "wall": minutes(created, parse_ts(last["completed_at"])),
        "macos_min": sum(minutes(parse_ts(j["started_at"]), parse_ts(j["completed_at"])) for j in mac),
        "runner_min": sum(minutes(parse_ts(j["started_at"]), parse_ts(j["completed_at"])) for j in done),
        "worst_macos_queue": max(minutes(parse_ts(j["created_at"]), parse_ts(j["started_at"])) for j in mac),
        # The `Test (macOS ARM64)` aggregate runs on Linux but finishes right after the
        # last macOS leg, so either one finishing last means macOS set the wall-clock.
        "macos_last": is_macos(last) or last["name"].startswith("Test (macOS"),
    }


def parse_window(text):
    name, _, span = text.partition("=")
    since, _, until = span.partition("..")
    if not (name and since and until):
        raise argparse.ArgumentTypeError(f"expected NAME=SINCE..UNTIL, got {text!r}")
    try:
        since, until = parse_ts(since), parse_ts(until)
    except ValueError as error:
        raise argparse.ArgumentTypeError(f"{text!r}: {error}") from None
    if since.tzinfo is None or until.tzinfo is None:
        raise argparse.ArgumentTypeError(f"{text!r}: give a timezone, e.g. 2026-10-28T12:17:00Z")
    since, until = since.astimezone(timezone.utc), until.astimezone(timezone.utc)
    if since >= until:
        raise argparse.ArgumentTypeError(f"{text!r}: the window ends before it starts")
    return name, since, until


def gh_json(path, attempts=4):
    """`gh api path`, retried: a fetch is a few thousand calls, and one dropped
    connection would otherwise end it."""
    for attempt in range(attempts):
        out = subprocess.run(["gh", "api", path], capture_output=True, text=True)
        if out.returncode == 0:
            return json.loads(out.stdout)
        if attempt + 1 < attempts:
            time.sleep(2 ** attempt)
    raise RuntimeError(f"gh api {path}: {out.stderr.strip()}")


def list_runs(workflow, since, until):
    """Every run of `workflow` created in `[since, until)`, one day per query: the runs
    endpoint returns at most 1,000 results per query, and a busy day is a few hundred."""
    runs = []
    day = since
    while day < until:
        end = min(day + timedelta(days=1), until)
        span = f"{day:%Y-%m-%dT%H:%M:%SZ}..{end - timedelta(seconds=1):%Y-%m-%dT%H:%M:%SZ}"
        page = 1
        while True:
            body = gh_json(
                f"repos/{REPO}/actions/workflows/{workflow}/runs"
                f"?created={span}&per_page=100&page={page}"
            )
            for run in body["workflow_runs"]:
                runs.append({key: run[key] for key in RUN_FIELDS} | {"workflow": workflow})
            if len(body["workflow_runs"]) < 100:
                break
            page += 1
        day = end
    # A run created while a day is being paged shifts the pages, so one can repeat.
    return list({run["id"]: run for run in runs}.values())


def fetch_jobs(run, cache):
    """Every attempt's jobs (`filter=all`): a re-run's earlier attempts held slots too.
    Cached per attempt, and only once the run itself has completed, since a run's
    `needs:`-gated jobs do not exist until their dependencies finish."""
    path = os.path.join(cache, "jobs", f"{run['id']}-{run['run_attempt']}.json")
    if os.path.exists(path):
        with open(path) as f:
            return json.load(f)
    jobs, page = [], 1
    while True:
        body = gh_json(f"repos/{REPO}/actions/runs/{run['id']}/jobs?filter=all&per_page=100&page={page}")
        jobs += [{key: job[key] for key in JOB_FIELDS} for job in body["jobs"]]
        if len(jobs) >= body["total_count"] or not body["jobs"]:
            break
        page += 1
    if run["status"] == "completed":
        with open(path + ".tmp", "w") as f:
            json.dump(jobs, f)
        os.replace(path + ".tmp", path)
    return jobs


def fetch(windows, margin_hours, cache, workers):
    os.makedirs(os.path.join(cache, "jobs"), exist_ok=True)
    since = min(w[1] for w in windows) - timedelta(hours=margin_hours)
    until = max(w[2] for w in windows)
    runs = [run for workflow in WORKFLOWS for run in list_runs(workflow, since, until)]
    with concurrent.futures.ThreadPoolExecutor(workers) as pool:
        jobs = dict(zip((run["id"] for run in runs), pool.map(lambda r: fetch_jobs(r, cache), runs)))
    return runs, jobs


def fmt(value):
    return f"{value:.1f}"


def table(header, rows):
    lines = ["| " + " | ".join(header) + " |", "|" + "|".join("---" for _ in header) + "|"]
    lines += ["| " + " | ".join(str(cell) for cell in row) + " |" for row in rows]
    return "\n".join(lines)


def report(windows, runs, jobs):
    # Windows may overlap (a week and its last day), so a run can belong to several.
    def windows_of(stamp):
        return [name for name, since, until in windows if since <= stamp < until]

    names = [w[0] for w in windows]
    run_windows = {run["id"]: windows_of(parse_ts(run["created_at"])) for run in runs}
    ci_runs = [run for run in runs if run["workflow"] == REPORTED_WORKFLOW and run_windows[run["id"]]]
    out = []

    durations = {name: (until - since).total_seconds() / 86400 for name, since, until in windows}
    rows = []
    for name in names:
        counts = collections.Counter(run["event"] for run in ci_runs if name in run_windows[run["id"]])
        rows.append([name, fmt(durations[name] * 24) + " h"]
                    + [f"{counts[e]} ({counts[e] / durations[name]:.0f}/day)" for e in EVENTS + ("push",)])
    out.append("**`ci.yml` runs per window**\n\n" + table(["window", "length"] + list(EVENTS + ("push",)), rows))

    summaries = collections.defaultdict(list)
    by_day = collections.defaultdict(list)
    for run in ci_runs:
        summary = summarize_run(run, jobs.get(run["id"], []))
        if summary:
            for name in run_windows[run["id"]]:
                summaries[(name, summary["event"])].append(summary)
            if summary["event"] == "merge_group":
                by_day[summary["day"]].append(summary["wall"])
    rows = []
    for event in EVENTS:
        for name in names:
            group = summaries[(name, event)]
            if not group:
                continue
            leg_counts = collections.Counter(s["legs"] for s in group)
            rows.append([
                event, name, len(group),
                ", ".join(f"{k}: {v}" for k, v in sorted(leg_counts.items())),
                f"{fmt(percentile([s['wall'] for s in group], .5))} / {fmt(percentile([s['wall'] for s in group], .9))}",
                fmt(statistics.mean(s["macos_min"] for s in group)),
                fmt(statistics.mean(s["runner_min"] for s in group)),
                f"{fmt(percentile([s['worst_macos_queue'] for s in group], .5))}"
                f" / {fmt(percentile([s['worst_macos_queue'] for s in group], .9))}",
                f"{sum(s['macos_last'] for s in group)} of {len(group)}",
            ])
    out.append("**Per run** (successful first attempts with macOS jobs; minutes)\n\n" + table(
        ["event", "window", "n", "macOS legs: runs", "wall p50 / p90", "macOS-min", "runner-min",
         "worst macOS queue p50 / p90", "macOS finished last"], rows))

    rows = [[day, len(walls), f"{fmt(percentile(walls, .5))} / {fmt(percentile(walls, .9))}"]
            for day, walls in sorted(by_day.items())]
    out.append("**`merge_group` wall-clock by day** (minutes)\n\n" + table(["day (UTC)", "n", "p50 / p90"], rows))

    mac = [(run_id, job) for run_id, run_jobs in jobs.items() for job in unique_jobs(run_jobs)
           if is_macos(job) and holds_slot(job)]
    in_flight = other_run_in_flight(
        [(run_id, parse_ts(job["created_at"]), slot_end(job)) for run_id, job in mac])
    # A re-run attempt's jobs can carry an earlier attempt's `started_at` (up to an hour
    # before their `created_at`), so only first-attempt jobs give queue-wait samples.
    # Every attempt still counts towards load.
    reported = {run["id"] for run in ci_runs}
    waits = collections.defaultdict(list)
    legs = collections.defaultdict(list)
    for (run_id, job), count in zip(mac, in_flight):
        if run_id not in reported or not first_attempt(job) or not ran(job):
            continue
        for name in run_windows[run_id]:
            waits[(name, load_bucket(count))].append(
                minutes(parse_ts(job["created_at"]), parse_ts(job["started_at"])))
            if job["conclusion"] == "success":
                legs[(job["name"], name)].append(minutes(parse_ts(job["started_at"]), parse_ts(job["completed_at"])))
    rows = []
    for bucket in BUCKETS:
        row = [bucket]
        for name in names:
            values = waits[(name, bucket)]
            row.append(
                f"n={len(values)}, mean {fmt(statistics.mean(values))}, p90 {fmt(percentile(values, .9))}, "
                f"{100 * sum(v > 1 for v in values) / len(values):.0f}% over 1 min"
                if values else "no data")
        rows.append(row)
    out.append("**macOS queue wait by load** (macOS jobs of other runs in flight at creation; minutes)\n\n"
               + table(["in flight"] + names, rows))

    rows = []
    for leg in sorted({leg for leg, _ in legs}):
        rows.append([leg] + [
            f"{fmt(percentile(legs[(leg, n)], .5))} / {fmt(percentile(legs[(leg, n)], .9))} (n={len(legs[(leg, n)])})"
            if legs[(leg, n)] else "-" for n in names])
    out.append("**macOS leg run time p50 / p90** (successful jobs; minutes)\n\n" + table(["job"] + names, rows))
    return "\n\n".join(out) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--window", action="append", type=parse_window, required=True,
                        metavar="NAME=SINCE..UNTIL", help="a window to report on (repeatable; ISO 8601 UTC)")
    parser.add_argument("--cache", required=True, help="directory for cached job lists")
    parser.add_argument("--margin", type=float, default=3,
                        help="hours of runs before the first window fetched for load only (default 3)")
    parser.add_argument("--workers", type=int, default=8, help="concurrent jobs-API requests (default 8)")
    args = parser.parse_args(argv)
    runs, jobs = fetch(args.window, args.margin, args.cache, args.workers)
    sys.stdout.write(report(args.window, runs, jobs))


if __name__ == "__main__":
    main()
