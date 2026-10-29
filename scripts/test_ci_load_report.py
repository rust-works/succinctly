#!/usr/bin/env python3
"""Unit tests for scripts/ci-load-report.py that need no network: the jobs are built
here, so these check the report's own arithmetic, not the Actions API. Run directly
(`python3 scripts/test_ci_load_report.py`) or via `python3 -m unittest
scripts/test_ci_load_report.py`; wired into CI's `perf-guard` job beside the other
script self-tests.

Imported via `importlib` rather than `import ci_load_report`: the script's filename has
a hyphen, which is not a valid Python module identifier.
"""

import argparse
import importlib.util
import json
import os
import pathlib
import tempfile
import unittest
from unittest import mock
from datetime import datetime, timedelta, timezone

_SCRIPT_PATH = pathlib.Path(__file__).parent / "ci-load-report.py"
_spec = importlib.util.spec_from_file_location("ci_load_report", _SCRIPT_PATH)
ci_load_report = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ci_load_report)

T0 = datetime(2026, 10, 28, 12, 0, tzinfo=timezone.utc)


def at(minute):
    return T0 + timedelta(minutes=minute)


def stamp(minute):
    return f"{at(minute):%Y-%m-%dT%H:%M:%SZ}"


def job(name, created, started, completed, conclusion="success", runner="macos-latest", attempt=1):
    return {
        "name": name, "status": "completed" if completed is not None else "in_progress",
        "conclusion": conclusion, "labels": [runner], "run_attempt": attempt,
        "created_at": stamp(created),
        "started_at": None if started is None else stamp(started),
        "completed_at": None if completed is None else stamp(completed),
    }


def run(run_id, created=0, event="merge_group", conclusion="success", attempt=1, status="completed"):
    return {"id": run_id, "status": status, "event": event, "conclusion": conclusion,
            "created_at": stamp(created), "run_attempt": attempt, "workflow": "ci.yml"}


class OtherRunInFlightTest(unittest.TestCase):
    def test_counts_only_jobs_of_other_runs(self):
        # Run 1's two legs never count each other; run 2's job sees both.
        jobs = [(1, at(0), at(10)), (1, at(0), at(10)), (2, at(5), at(8))]
        self.assertEqual(ci_load_report.other_run_in_flight(jobs), [0, 0, 2])

    def test_a_job_completed_at_creation_is_not_in_flight(self):
        # `[created, completed)`: a slot freed at the instant a job is created is free.
        jobs = [(1, at(0), at(5)), (2, at(5), at(9)), (3, at(4), at(6))]
        self.assertEqual(ci_load_report.other_run_in_flight(jobs), [0, 1, 1])

    def test_same_instant_jobs_of_other_runs_count_each_other(self):
        jobs = [(1, at(3), at(9)), (2, at(3), at(9)), (3, at(3), at(4))]
        self.assertEqual(ci_load_report.other_run_in_flight(jobs), [2, 2, 2])

    def test_matches_the_quadratic_definition(self):
        # The sweep against the definition it replaces, over overlapping, nested and
        # disjoint intervals in unsorted order.
        jobs = [
            (1, at(0), at(30)), (2, at(1), at(2)), (2, at(1), at(40)), (3, at(2), at(3)),
            (4, at(29), at(31)), (5, at(30), at(35)), (1, at(31), at(32)), (6, at(50), at(51)),
            (7, at(10), at(60)), (8, at(10), at(11)), (3, at(35), at(36)), (9, at(0), at(1)),
        ]

        def quadratic(i):
            run_id, created, _ = jobs[i]
            return sum(1 for o_run, o_created, o_done in jobs
                       if o_run != run_id and o_created <= created < o_done)

        self.assertEqual(ci_load_report.other_run_in_flight(jobs), [quadratic(i) for i in range(len(jobs))])


class HelpersTest(unittest.TestCase):
    def test_buckets(self):
        self.assertEqual([ci_load_report.load_bucket(n) for n in (0, 1, 2, 3, 4, 5, 6, 8, 9, 30)],
                         ["0-1", "0-1", "2-3", "2-3", "4-5", "4-5", "6-8", "6-8", "9+", "9+"])

    def test_percentile_is_nearest_rank(self):
        values = [5, 1, 4, 2, 3]
        self.assertEqual(ci_load_report.percentile(values, .5), 3)
        self.assertEqual(ci_load_report.percentile(values, .9), 5)
        self.assertEqual(ci_load_report.percentile([7], .9), 7)

    def test_is_macos(self):
        self.assertTrue(ci_load_report.is_macos(job("x", 0, 0, 1, runner="macos-latest")))
        self.assertTrue(ci_load_report.is_macos(job("x", 0, 0, 1, runner="macOS-arm64")))
        self.assertFalse(ci_load_report.is_macos(job("x", 0, 0, 1, runner="ubuntu-24.04-arm")))

    def test_skipped_jobs_hold_no_slot(self):
        self.assertFalse(ci_load_report.holds_slot(job("x", 0, 0, 0, conclusion="skipped")))
        self.assertTrue(ci_load_report.holds_slot(job("x", 0, None, 4, conclusion="cancelled")))
        # Still queued: it holds its place in the load count until the report ends.
        queued = job("x", 0, None, None, conclusion=None)
        self.assertTrue(ci_load_report.holds_slot(queued))
        self.assertEqual(ci_load_report.slot_end(queued), ci_load_report.NOT_YET)
        self.assertFalse(ci_load_report.ran(job("x", 0, None, 4, conclusion="cancelled")))

    def test_parse_window(self):
        name, since, until = ci_load_report.parse_window("after=2026-10-28T12:17:00Z..2026-10-29T12:17:00Z")
        self.assertEqual(name, "after")
        self.assertEqual(until - since, timedelta(days=1))
        # A non-UTC offset is converted, so the runs query and the bucketing agree.
        _, since, _ = ci_load_report.parse_window("w=2026-10-28T14:17:00+02:00..2026-10-29T12:17:00Z")
        self.assertEqual(since, datetime(2026, 10, 28, 12, 17, tzinfo=timezone.utc))
        self.assertEqual(since.utcoffset(), timedelta(0))
        for bad in ("after", "after=2026-10-28T12:17:00Z", "=a..b", "w=2026-10-21..2026-10-28T00:00:00Z",
                    "w=2026-10-29T00:00:00Z..2026-10-29T00:00:00Z", "w=2026-10-29T00:00:00Z..2026-10-28T00:00:00Z",
                    "w=yesterday..today"):
            with self.assertRaises(argparse.ArgumentTypeError):
                ci_load_report.parse_window(bad)


class SummarizeRunTest(unittest.TestCase):
    JOBS = [
        job("Gate", 0, 0, 1, runner="ubuntu-latest"),
        job("Test (macOS ARM64) (cli-gated)", 1, 3, 13),
        job("Test (macOS ARM64) (simd)", 1, 2, 6),
        job("Test (x86_64)", 1, 1, 9, runner="ubuntu-latest"),
        job("Test (macOS ARM64)", 13, 13, 14, runner="ubuntu-latest"),
        job("Bench", 1, None, 1, conclusion="skipped", runner="macos-latest"),
    ]

    def test_figures(self):
        summary = ci_load_report.summarize_run(run(1), self.JOBS)
        self.assertEqual(summary["legs"], 2)
        self.assertAlmostEqual(summary["wall"], 14)
        self.assertAlmostEqual(summary["macos_min"], 14)
        self.assertAlmostEqual(summary["runner_min"], 1 + 10 + 4 + 8 + 1)
        self.assertAlmostEqual(summary["worst_macos_queue"], 2)
        # The Linux-hosted macOS aggregate finished last: macOS set the wall-clock.
        self.assertTrue(summary["macos_last"])

    def test_linux_last_is_not_macos_last(self):
        jobs = self.JOBS[:4] + [job("Coverage (ARM64)", 1, 1, 30, runner="ubuntu-24.04-arm")]
        self.assertFalse(ci_load_report.summarize_run(run(1), jobs)["macos_last"])

    def test_excluded_runs(self):
        self.assertIsNone(ci_load_report.summarize_run(run(1, conclusion="cancelled"), self.JOBS))
        self.assertIsNone(ci_load_report.summarize_run(run(1, attempt=2), self.JOBS))
        self.assertIsNone(ci_load_report.summarize_run(run(1), [self.JOBS[0], self.JOBS[3]]))


class ReportTest(unittest.TestCase):
    def test_windows_and_load(self):
        windows = [("before", at(0), at(60)), ("after", at(60), at(120))]
        runs = [run(1, created=0), run(2, created=10), run(3, created=70, event="pull_request"),
                {**run(4, created=65, event="schedule"), "workflow": "scheduled.yml"}]
        jobs = {
            1: [job("Test (macOS ARM64) (simd)", 0, 0, 30)],
            2: [job("Test (macOS ARM64) (simd)", 10, 30, 40)],
            3: [job("Test (macOS ARM64) (simd)", 70, 80, 85)],
            # Not reported on, but it holds a slot when run 3's job is created.
            4: [job("Bench (macOS)", 65, 65, 80)],
        }
        text = ci_load_report.report(windows, runs, jobs)
        # Run 2's job was created while run 1's ran (1 other in flight), and waited 20 min.
        self.assertIn("| 0-1 | n=2, mean 10.0, p90 20.0, 50% over 1 min | n=1, mean 10.0, p90 10.0, 100% over 1 min |",
                      text)
        self.assertIn("| merge_group | before | 2 | 1: 2 | 30.0 / 30.0 |", text)
        self.assertIn("| pull_request | after | 1 | 1: 1 | 15.0 / 15.0 |", text)
        # The scheduled run's own job is not a reported queue-wait sample or leg.
        self.assertNotIn("Bench (macOS)", text)

    def test_overlapping_windows_share_runs(self):
        windows = [("week", at(0), at(60)), ("day", at(20), at(60))]
        runs = [run(1, created=0), run(2, created=30)]
        jobs = {1: [job("Test (macOS ARM64) (simd)", 0, 1, 10)],
                2: [job("Test (macOS ARM64) (simd)", 30, 33, 40)]}
        text = ci_load_report.report(windows, runs, jobs)
        self.assertIn("| merge_group | week | 2 |", text)
        self.assertIn("| merge_group | day | 1 |", text)
        self.assertIn("| 0-1 | n=2, mean 2.0, p90 3.0, 50% over 1 min | n=1, mean 3.0, p90 3.0, 100% over 1 min |", text)
        # By day counts each run once, whichever windows it is in.
        self.assertIn("| 2026-10-28 | 2 |", text)

    def test_rerun_attempts_count_as_load_but_give_no_samples(self):
        windows = [("w", at(0), at(60))]
        runs = [run(1, created=0), run(2, created=5, attempt=2)]
        jobs = {
            1: [job("Test (macOS ARM64) (simd)", 10, 12, 20)],
            # A re-run attempt's job whose `started_at` is from the earlier attempt.
            2: [job("Test (macOS ARM64) (cli-gated)", 5, -50, 30, attempt=2)],
        }
        text = ci_load_report.report(windows, runs, jobs)
        self.assertIn("| 0-1 | n=1, mean 2.0, p90 2.0, 100% over 1 min |", text)
        self.assertNotIn("cli-gated", text)

    def test_a_later_attempts_copy_of_a_job_is_not_counted_twice(self):
        windows = [("w", at(0), at(60))]
        original = job("Test (macOS ARM64) (simd)", 0, 1, 30)
        copy = dict(original, run_attempt=2, created_at=stamp(20))
        runs = [run(1, created=0, attempt=2), run(2, created=25)]
        jobs = {1: [copy, original], 2: [job("Test (macOS ARM64) (simd)", 25, 26, 40)]}
        self.assertIn("| 0-1 | n=2, mean 1.0, p90 1.0, 0% over 1 min |", ci_load_report.report(windows, runs, jobs))
        self.assertEqual(ci_load_report.unique_jobs([copy, original]), [original])
        # Two jobs that never started are not copies of each other.
        queued = job("x", 0, None, None, conclusion=None)
        self.assertEqual(len(ci_load_report.unique_jobs([queued, dict(queued, run_attempt=2)])), 2)

    def test_a_queued_job_counts_as_load(self):
        windows = [("w", at(0), at(60))]
        runs = [run(1, created=0, status="in_progress"), run(2, created=10)]
        jobs = {1: [job("Test (macOS ARM64) (simd)", 0, None, None, conclusion=None)] * 2,
                2: [job("Test (macOS ARM64) (simd)", 10, 12, 20)]}
        self.assertIn("| 2-3 | n=1, mean 2.0,", ci_load_report.report(windows, runs, jobs))


class FetchTest(unittest.TestCase):
    def test_runs_repeated_across_pages_are_kept_once(self):
        page1 = {"workflow_runs": [dict(run(i), status="completed") for i in range(100)]}
        page2 = {"workflow_runs": [dict(run(99), status="completed"), dict(run(100), status="completed")]}
        with mock.patch.object(ci_load_report, "gh_json", side_effect=[page1, page2]):
            runs = ci_load_report.list_runs("ci.yml", at(0), at(60))
        self.assertEqual(sorted(r["id"] for r in runs), list(range(101)))

    def test_jobs_are_cached_per_attempt_once_the_run_completed(self):
        body = {"total_count": 1, "jobs": [dict(job("simd", 0, 1, 2), id=7, extra="x")]}
        with tempfile.TemporaryDirectory() as cache:
            os.makedirs(os.path.join(cache, "jobs"))
            with mock.patch.object(ci_load_report, "gh_json", return_value=body) as gh:
                # A run still in progress: its `needs:`-gated jobs may not exist yet.
                ci_load_report.fetch_jobs(run(5, status="in_progress"), cache)
                self.assertEqual(os.listdir(os.path.join(cache, "jobs")), [])
                self.assertIn("filter=all", gh.call_args.args[0])
                ci_load_report.fetch_jobs(run(5), cache)
                ci_load_report.fetch_jobs(run(5), cache)
                self.assertEqual(gh.call_count, 2)
                # A re-run is another attempt, so another cache entry.
                ci_load_report.fetch_jobs(run(5, attempt=2), cache)
                self.assertEqual(gh.call_count, 3)
            self.assertEqual(sorted(os.listdir(os.path.join(cache, "jobs"))), ["5-1.json", "5-2.json"])
            with open(os.path.join(cache, "jobs", "5-1.json")) as f:
                self.assertEqual(set(json.load(f)[0]), set(ci_load_report.JOB_FIELDS))


if __name__ == "__main__":
    unittest.main()
