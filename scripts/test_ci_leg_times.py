#!/usr/bin/env python3
"""Tests for scripts/ci_leg_times.py (#5867): the analysis, the report and the issue it files."""

import json
import os
import sys
import unittest
from datetime import datetime, timedelta, timezone
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ci_leg_times as c  # noqa: E402

NOSTD = "Test (x86_64) (no-std)"
AS_OF = datetime(2026, 11, 2, 12, 0, 0, tzinfo=timezone.utc)


def runs(job, seconds):
    """One `{job: seconds}` per run."""
    return [{job: s} for s in seconds]


class AnalyseTests(unittest.TestCase):
    def test_the_5845_regression_is_flagged(self):
        # Eight to nine minutes per leg-run before #5232, fifteen to seventeen after (#5845):
        # the no-std leg's own time went from about four minutes to eleven.
        baseline = runs(NOSTD, [240, 255, 250, 262, 245, 258, 248, 252, 260, 244])
        recent = runs(NOSTD, [650, 700, 640, 720])
        flags = c.analyse(recent, baseline)
        self.assertEqual([f[0] for f in flags], [NOSTD])
        _, was, now = flags[0]
        self.assertAlmostEqual(was, 251.0, delta=5)
        self.assertGreater(now, 600)

    def test_runner_noise_is_not_flagged(self):
        # Adjacent merge-group runs measured the same no-std leg at 4.8 and 6.9 minutes (#5867).
        baseline = runs(NOSTD, [288, 414, 300, 350, 310, 290, 405, 330, 295, 380])
        recent = runs(NOSTD, [288, 414, 300, 410])
        self.assertEqual(c.analyse(recent, baseline), [])

    def test_one_slow_run_among_recent_is_not_enough(self):
        baseline = runs(NOSTD, [250] * 10)
        recent = runs(NOSTD, [250, 900, 250, 260])
        self.assertEqual(c.analyse(recent, baseline), [])

    def test_both_the_ratio_and_the_absolute_gap_are_needed(self):
        # Twice as slow, but a hundred seconds is below MIN_ABS.
        self.assertEqual(c.analyse(runs("a", [200] * 4), runs("a", [100] * 10)), [])
        # Five minutes slower, but only 1.2x.
        self.assertEqual(c.analyse(runs("a", [1800] * 4), runs("a", [1500] * 10)), [])
        # Both.
        self.assertEqual([f[0] for f in c.analyse(runs("a", [400] * 4), runs("a", [200] * 10))], ["a"])

    def test_a_short_job_or_an_aggregator_is_never_flagged(self):
        # `Test (x86_64)` aggregates the legs and takes seconds.
        self.assertEqual(c.analyse(runs("agg", [40] * 4), runs("agg", [3] * 10)), [])

    def test_a_job_with_too_few_samples_is_skipped(self):
        self.assertEqual(c.analyse(runs("a", [600] * 4), runs("a", [200] * (c.MIN_BASELINE - 1))), [])
        self.assertEqual(c.analyse(runs("a", [600] * (c.MIN_RECENT - 1)), runs("a", [200] * 10)), [])

    def test_a_new_job_is_not_flagged(self):
        recent = runs("new", [600] * 4)
        self.assertEqual(c.analyse(recent, [{} for _ in range(10)]), [])

    def test_the_largest_growth_comes_first(self):
        baseline = [{"a": 200, "b": 300} for _ in range(10)]
        recent = [{"a": 400, "b": 900} for _ in range(4)]
        self.assertEqual([f[0] for f in c.analyse(recent, baseline)], ["b", "a"])


class SampleTests(unittest.TestCase):
    def test_fewer_than_asked_are_all_returned(self):
        self.assertEqual(c.sample_evenly([1, 2, 3], 5), [1, 2, 3])

    def test_a_long_list_is_spread_and_in_order(self):
        sampled = c.sample_evenly(list(range(100)), 10)
        self.assertEqual(len(sampled), 10)
        self.assertEqual(sampled, sorted(sampled))
        self.assertEqual(sampled[0], 0)
        self.assertLess(sampled[-1], 100)
        self.assertTrue(all(b - a == 10 for a, b in zip(sampled, sampled[1:])))


class RenderTests(unittest.TestCase):
    def test_no_flags(self):
        text = c.render([], 4, 30, AS_OF)
        self.assertIn("No job is flagged.", text)
        self.assertNotIn("| job |", text)

    def test_a_flag_names_the_job_and_both_times(self):
        text = c.render([(NOSTD, 250.0, 700.0)], 4, 30, AS_OF)
        self.assertIn(f"`{NOSTD}`", text)
        self.assertIn("4.2 min", text)
        self.assertIn("11.7 min", text)
        self.assertIn("2.8x", text)
        self.assertIn("2026-11-02 12:00 UTC", text)


class FetchTests(unittest.TestCase):
    def job(self, name, conclusion, start, end):
        return {"name": name, "conclusion": conclusion, "started_at": start, "completed_at": end}

    def test_only_successful_timed_jobs_count(self):
        body = {
            "jobs": [
                self.job("ok", "success", "2026-11-02T10:00:00Z", "2026-11-02T10:05:00Z"),
                self.job("bad", "failure", "2026-11-02T10:00:00Z", "2026-11-02T10:05:00Z"),
                self.job("skipped", "skipped", "2026-11-02T10:00:00Z", "2026-11-02T10:00:00Z"),
                self.job("unstarted", "success", None, None),
            ]
        }
        with mock.patch.object(c, "api", return_value=body):
            self.assertEqual(c.fetch_jobs(1), {"ok": 300.0})

    def test_jobs_past_the_first_hundred_are_read(self):
        def many(i):
            return self.job(f"j{i}", "success", "2026-11-02T10:00:00Z", "2026-11-02T10:01:00Z")

        pages = {1: {"jobs": [many(i) for i in range(100)]}, 2: {"jobs": [many(100), many(101)]}}
        with mock.patch.object(c, "api", side_effect=lambda path, **p: pages[p["page"]]):
            seconds = c.fetch_jobs(1)
        self.assertEqual(len(seconds), 102)
        self.assertEqual(seconds["j101"], 60.0)

    def test_runs_are_pages_of_the_window_newest_first(self):
        def run(i, when):
            return {"id": i, "created_at": when}

        pages = {
            1: {"workflow_runs": [run(i, "2026-11-03T00:00:00Z") for i in range(5)] + [run(100 + i, "2026-11-02T10:00:00Z") for i in range(95)]},
            2: {"workflow_runs": [run(200, "2026-11-01T10:00:00Z"), run(201, "2026-10-01T10:00:00Z")]},
        }
        calls = []

        def fake(path, **params):
            calls.append(params["page"])
            return pages[params["page"]]

        with mock.patch.object(c, "api", side_effect=fake):
            # the five after `as_of` are skipped, the one before the window ends the walk
            ids = [r["id"] for r in c.fetch_runs(AS_OF)]
        self.assertEqual(ids, [100 + i for i in range(95)] + [200])
        self.assertEqual(calls, [1, 2])

    def test_a_short_page_ends_the_walk(self):
        with mock.patch.object(c, "api", return_value={"workflow_runs": []}) as api:
            self.assertEqual(c.fetch_runs(AS_OF), [])
        self.assertEqual(api.call_count, 1)


class IssueTests(unittest.TestCase):
    def fake_gh(self, issues, comment_times):
        calls = []

        def gh(*args):
            calls.append(args)
            if args[:2] == ("issue", "list"):
                return json.dumps(issues)
            if args[0] == "api":
                return "\n".join(comment_times)
            return ""

        return gh, calls

    def stamp(self, **ago):
        return (AS_OF - timedelta(**ago)).strftime("%Y-%m-%dT%H:%M:%SZ")

    def test_no_open_issue_creates_the_label_and_the_issue(self):
        gh, calls = self.fake_gh([], [])
        with mock.patch.object(c, "gh", gh):
            c.file_issue("body", AS_OF)
        verbs = [a[:2] for a in calls]
        self.assertIn(("label", "create"), verbs)
        self.assertIn(("issue", "create"), verbs)
        create = next(a for a in calls if a[:2] == ("issue", "create"))
        self.assertIn(c.TITLE, create)
        self.assertIn(c.LABEL, create)

    def test_an_open_issue_is_commented_on_at_most_once_a_day(self):
        gh, calls = self.fake_gh([{"number": 7}], [self.stamp(hours=6)])
        with mock.patch.object(c, "gh", gh):
            c.file_issue("body", AS_OF)
        self.assertFalse([a for a in calls if a[:2] in (("issue", "comment"), ("issue", "create"))])

        gh, calls = self.fake_gh([{"number": 7}], [self.stamp(hours=30)])
        with mock.patch.object(c, "gh", gh):
            c.file_issue("body", AS_OF)
        comment = [a for a in calls if a[:2] == ("issue", "comment")]
        self.assertEqual(len(comment), 1)
        self.assertEqual(comment[0][2], "7")

    def test_a_daily_run_that_started_a_few_minutes_early_still_comments(self):
        # Yesterday's run commented at 04:20 and today's starts at 04:17: 23h57m later.
        gh, calls = self.fake_gh([{"number": 7}], [self.stamp(hours=23, minutes=57)])
        with mock.patch.object(c, "gh", gh):
            c.file_issue("body", AS_OF)
        self.assertTrue([a for a in calls if a[:2] == ("issue", "comment")])

    def test_the_newest_of_many_comments_decides(self):
        # More than a page of them: the list is every comment, oldest first, and the last is read.
        old = [self.stamp(days=200 - i) for i in range(120)]  # the last is 81 days old
        gh, calls = self.fake_gh([{"number": 7}], old)
        with mock.patch.object(c, "gh", gh):
            c.file_issue("body", AS_OF)
        self.assertIn("--paginate", next(a for a in calls if a[0] == "api"))
        self.assertTrue([a for a in calls if a[:2] == ("issue", "comment")])

        recent = old[:-1] + [self.stamp(hours=2)]
        gh, calls = self.fake_gh([{"number": 7}], recent)
        with mock.patch.object(c, "gh", gh):
            c.file_issue("body", AS_OF)
        self.assertFalse([a for a in calls if a[:2] == ("issue", "comment")])

    def test_an_open_issue_with_no_comment_yet_gets_one(self):
        gh, calls = self.fake_gh([{"number": 7}], [])
        with mock.patch.object(c, "gh", gh):
            c.file_issue("body", AS_OF)
        self.assertTrue([a for a in calls if a[:2] == ("issue", "comment")])


class ReportTests(unittest.TestCase):
    def setUp(self):
        # `report` appends to the job summary of the workflow that runs these tests.
        patched = mock.patch.dict(os.environ)
        patched.start()
        self.addCleanup(patched.stop)
        os.environ.pop("GITHUB_STEP_SUMMARY", None)

    def test_report_flags_through_the_whole_pipeline_and_files_only_when_asked(self):
        runs_ = [{"id": i, "created_at": "2026-11-02T00:00:00Z"} for i in range(40)]
        jobs = {i: {NOSTD: 700.0 if i < c.RECENT else 250.0} for i in range(40)}
        with mock.patch.object(c, "fetch_runs", return_value=runs_), mock.patch.object(
            c, "fetch_jobs", side_effect=lambda rid: jobs[rid]
        ), mock.patch.object(c, "file_issue") as file_issue, mock.patch("sys.stdout") as out:
            self.assertEqual(c.report(AS_OF, issue=False), 0)
            file_issue.assert_not_called()
            self.assertEqual(c.report(AS_OF, issue=True), 0)
            file_issue.assert_called_once()
        written = "".join(call.args[0] for call in out.write.call_args_list)
        self.assertIn(NOSTD, written)

    def test_nothing_flagged_files_nothing(self):
        runs_ = [{"id": i, "created_at": "2026-11-02T00:00:00Z"} for i in range(40)]
        with mock.patch.object(c, "fetch_runs", return_value=runs_), mock.patch.object(
            c, "fetch_jobs", return_value={NOSTD: 250.0}
        ), mock.patch.object(c, "file_issue") as file_issue, mock.patch("sys.stdout"):
            c.report(AS_OF, issue=True)
        file_issue.assert_not_called()

    def test_an_api_failure_is_a_clean_exit_code(self):
        with mock.patch.object(c, "fetch_runs", side_effect=RuntimeError("gh api: boom")), mock.patch(
            "sys.stderr"
        ):
            self.assertEqual(c.main(["report"]), 2)


if __name__ == "__main__":
    unittest.main()
