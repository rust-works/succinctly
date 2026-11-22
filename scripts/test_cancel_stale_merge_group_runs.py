#!/usr/bin/env python3
"""Self-tests for scripts/cancel_stale_merge_group_runs.py (#7552): a fake `gh`, no network.

Run directly (`python3 scripts/test_cancel_stale_merge_group_runs.py`).
"""

import contextlib
import io
import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cancel_stale_merge_group_runs as sweep  # noqa: E402

REPO = "o/r"
LIVE = "gh-readonly-queue/main/pr-2-bbb"
GONE = "gh-readonly-queue/main/pr-1-aaa"


def run_(rid, branch, name="CI", path=".github/workflows/ci.yml", status="queued"):
    return {"id": rid, "name": name, "path": path, "status": status, "head_branch": branch}


class FakeGh:
    """Answers the three API calls from fixed data and records every call, in order."""

    def __init__(self, runs, refs, cancel_error=None, refs_error=None):
        self.runs, self.refs = runs, refs
        self.cancel_error, self.refs_error = cancel_error, refs_error
        self.calls = []

    def __call__(self, args):
        self.calls.append(args)
        endpoint = next(a for a in args if a.startswith("repos/"))
        if "/actions/runs?" in endpoint:
            status = endpoint.split("status=")[1].split("&")[0]
            return "".join(json.dumps(r) + "\n" for r in self.runs if r["status"] == status)
        if "/git/matching-refs/" in endpoint:
            if self.refs_error:
                raise sweep.GhError(args, self.refs_error)
            return "".join(f"refs/heads/{b}\n" for b in self.refs)
        if endpoint.endswith("/cancel"):
            if self.cancel_error:
                raise sweep.GhError(args, self.cancel_error)
            return ""
        raise AssertionError(args)

    def cancelled(self):
        return [int(a[3].split("/")[-2]) for a in self.calls if a[:3] == ["api", "-X", "POST"]]


def main(fake, *argv, run_id="99"):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        code = sweep.main(["--repo", REPO, *argv], run=fake, environ={"GITHUB_RUN_ID": run_id})
    return code, out.getvalue()


class SweepTests(unittest.TestCase):
    def test_a_run_of_a_destroyed_group_is_cancelled_and_a_live_one_is_not(self):
        fake = FakeGh([run_(1, GONE), run_(2, LIVE, status="in_progress")], [LIVE])
        code, out = main(fake)
        self.assertEqual((code, fake.cancelled()), (0, [1]))
        self.assertIn(f"::notice::cancelled run 1 (CI, queued) of destroyed group {GONE}", out)

    def test_every_active_status_is_listed_and_the_runs_before_the_refs(self):
        fake = FakeGh([run_(1, GONE, status=s) for s in sweep.ACTIVE_STATUSES], [])
        main(fake, "--dry-run")
        listed = [a[2] for a in fake.calls if "/actions/runs?" in a[2]]
        self.assertEqual([u.split("status=")[1].split("&")[0] for u in listed], list(sweep.ACTIVE_STATUSES))
        self.assertTrue(all("event=merge_group" in u for u in listed))
        last_list = max(i for i, a in enumerate(fake.calls) if "/actions/runs?" in a[2])
        refs = next(i for i, a in enumerate(fake.calls) if "/git/matching-refs/" in a[2])
        self.assertLess(last_list, refs)

    def test_a_run_seen_under_two_statuses_is_cancelled_once(self):
        fake = FakeGh([run_(1, GONE, status="queued"), run_(1, GONE, status="in_progress")], [])
        main(fake)
        self.assertEqual(fake.cancelled(), [1])

    def test_dry_run_cancels_nothing(self):
        fake = FakeGh([run_(1, GONE)], [LIVE])
        code, out = main(fake, "--dry-run")
        self.assertEqual((code, fake.cancelled()), (0, []))
        self.assertIn(f"would cancel run 1 (CI, queued) of destroyed group {GONE}", out)

    def test_its_own_run_and_workflow_and_non_queue_branches_are_left_alone(self):
        fake = FakeGh(
            [
                run_(99, GONE, name="Cancel stale merge groups"),
                run_(5, GONE, path=sweep.SELF_WORKFLOW),
                run_(6, "main"),
                run_(7, None),
                run_(8, GONE, name="Canary"),
            ],
            [],
        )
        main(fake)
        self.assertEqual(fake.cancelled(), [8])

    def test_an_empty_queue_answered_with_404_cancels_every_active_run(self):
        fake = FakeGh([run_(1, GONE), run_(2, LIVE)], [], refs_error="gh: Not Found (HTTP 404)")
        main(fake)
        self.assertEqual(fake.cancelled(), [1, 2])

    def test_any_other_ref_failure_cancels_nothing(self):
        fake = FakeGh([run_(1, GONE)], [], refs_error="gh: Server Error (HTTP 502)")
        with self.assertRaises(sweep.GhError):
            main(fake)
        self.assertEqual(fake.cancelled(), [])

    def test_a_read_only_token_stops_quietly(self):
        fake = FakeGh([run_(1, GONE), run_(2, GONE)], [], cancel_error="Resource not accessible by integration (HTTP 403)")
        code, out = main(fake)
        self.assertEqual((code, fake.cancelled()), (0, [1]))
        self.assertIn("::notice::this token may not cancel runs", out)

    def test_a_run_that_finished_meanwhile_is_reported_and_the_sweep_goes_on(self):
        fake = FakeGh([run_(1, GONE), run_(2, GONE)], [], cancel_error="Cannot cancel a workflow run that is completed. (HTTP 409)")
        code, out = main(fake)
        self.assertEqual((code, fake.cancelled()), (0, [1, 2]))
        self.assertEqual(out.count("::warning::could not cancel"), 2)


if __name__ == "__main__":
    unittest.main()
