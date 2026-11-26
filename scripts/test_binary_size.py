#!/usr/bin/env python3
"""Tests for scripts/binary_size.py (#7390, #7959): the verdict, its messages, the growth gate, the
tracking issue, the strip, the committed budget and the workflows that run it.

No build, no real `strip` and no real `gh`: a fake strip program and a recording `gh` stand in. Run
directly (`python3 scripts/test_binary_size.py`).
"""

import json
import os
import re
import stat
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import binary_size as b  # noqa: E402

WORKFLOWS = os.path.join(b.ROOT, ".github", "workflows")
LINUX_CI_TARGETS = ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu")


def config(bytes_, tolerance=0.05, target="t", max_pr_growth=0.01):
    return {
        "tolerance": tolerance,
        "max_pr_growth": max_pr_growth,
        "targets": {target: {"bytes": bytes_, "measured": "test"}},
    }


class VerdictTests(unittest.TestCase):
    def test_the_tolerance_is_inclusive_on_both_sides(self):
        self.assertEqual(b.verdict(1050, 1000, 0.05), "ok")
        self.assertEqual(b.verdict(950, 1000, 0.05), "ok")
        self.assertEqual(b.verdict(1051, 1000, 0.05), "over")
        self.assertEqual(b.verdict(949, 1000, 0.05), "under")

    def test_the_7384_growth_would_have_failed(self):
        # #7384: 15.6 MB stripped on macOS arm64 from 19.9 MB before it; the old size against the
        # new budget is the growth this check exists to stop.
        self.assertEqual(b.verdict(19_900_000, 15_610_000, 0.05), "over")


class CheckTests(unittest.TestCase):
    def test_growth_past_the_tolerance_fails_and_prints_the_edit(self):
        code, lines = b.check(1_100_000, "t", config(1_000_000), b.BUDGET)
        self.assertEqual(code, 1)
        text = "\n".join(lines)
        self.assertTrue(lines[0].startswith("::error::"), lines[0])
        self.assertIn("+10.00%", text)
        self.assertIn('set "t" -> "bytes" to 1100000', text)
        self.assertIn(os.path.join("tests", "data", "binary-size-budget.json"), text)
        # Enforced only by release.yml and the scheduled job, so the edit is never "in this PR".
        self.assertIn("in a PR of its own", text)
        self.assertNotIn("this PR", text)

    def test_growth_within_the_tolerance_passes_quietly(self):
        code, lines = b.check(1_040_000, "t", config(1_000_000), b.BUDGET)
        self.assertEqual(code, 0)
        self.assertEqual(len(lines), 1)
        self.assertNotIn("::", lines[0])
        self.assertIn("+4.00%", lines[0])

    def test_a_shrink_past_the_tolerance_passes_with_a_warning(self):
        code, lines = b.check(900_000, "t", config(1_000_000), b.BUDGET)
        self.assertEqual(code, 0)
        self.assertTrue(lines[0].startswith("::warning::"), lines[0])
        self.assertIn('set "t" -> "bytes" to 900000', "\n".join(lines))

    def test_a_target_without_a_budget_fails_with_the_entry_to_add(self):
        code, lines = b.check(1_234, "other", config(1_000_000), b.BUDGET)
        self.assertEqual(code, 1)
        self.assertIn('"other": {"bytes": 1234', "\n".join(lines))


class ReportModeTests(unittest.TestCase):
    def test_a_budget_overrun_is_a_warning_that_passes(self):
        code, lines = b.check(1_100_000, "t", config(1_000_000), b.BUDGET, report=True)
        self.assertEqual(code, 0)
        self.assertTrue(lines[0].startswith("::warning::"), lines[0])
        self.assertIn("+10.00%", lines[0])
        text = "\n".join(lines)
        self.assertNotIn("::error::", text)
        self.assertIn("PR of its own", text)
        # It names where the budget is enforced, so a reader of a green run knows it still matters.
        self.assertIn("`binary-size` issue", text)
        self.assertIn("release.yml fails", text)

    def test_report_mode_leaves_the_other_verdicts_alone(self):
        self.assertEqual(b.check(1_040_000, "t", config(1_000_000), b.BUDGET, report=True)[0], 0)
        self.assertEqual(b.check(1_234, "other", config(1_000_000), b.BUDGET, report=True)[0], 1)


class GrowthTests(unittest.TestCase):
    def test_growth_past_the_limit_fails_and_names_the_waiver(self):
        code, lines = b.check_growth(1_020_000, 1_000_000, 0.01)
        self.assertEqual(code, 1)
        text = "\n".join(lines)
        self.assertTrue(lines[0].startswith("::error::"), lines[0])
        self.assertIn("+2.00%", text)
        self.assertIn("[size-growth: ok #N]", text)

    def test_the_limit_itself_passes(self):
        self.assertEqual(b.check_growth(1_010_000, 1_000_000, 0.01)[0], 0)
        self.assertEqual(b.check_growth(1_010_001, 1_000_000, 0.01)[0], 1)

    def test_a_shrink_or_no_change_passes_quietly(self):
        for measured in (900_000, 1_000_000):
            code, lines = b.check_growth(measured, 1_000_000, 0.01)
            self.assertEqual((code, len(lines)), (0, 1))
            self.assertNotIn("::", lines[0])

    def test_a_waiver_accepts_the_growth_with_a_notice(self):
        code, lines = b.check_growth(1_300_000, 1_000_000, 0.01, waiver="7960")
        self.assertEqual(code, 0)
        self.assertTrue(lines[0].startswith("::notice::"), lines[0])
        self.assertIn("#7960", lines[0])


class WaiverTests(unittest.TestCase):
    def test_a_line_of_its_own_names_the_issue(self):
        self.assertEqual(b.find_waiver(["feat(regex): x\n\n[size-growth: ok #7960]\n"]), "7960")
        self.assertEqual(b.find_waiver(["title", "body\n  [size-growth: ok #12]  \n"]), "12")

    def test_a_mention_inside_a_sentence_does_not_count(self):
        self.assertIsNone(b.find_waiver(["add [size-growth: ok #7] to waive it"]))
        self.assertIsNone(b.find_waiver(["[size-growth: ok]"]))  # no issue number
        self.assertIsNone(b.find_waiver(["[size-growth: ok #x]"]))

    def test_a_template_comment_does_not_count(self):
        body = "<!--\nwaive with\n[size-growth: ok #N]\n[size-growth: ok #1]\n-->\nreal text"
        self.assertIsNone(b.find_waiver([body]))
        self.assertIsNone(b.find_waiver(["", None]))

    def test_commit_messages_reads_a_git_range(self):
        with mock.patch("subprocess.run") as run:
            run.return_value = mock.Mock(returncode=0, stdout="one\n\x00two\n[size-growth: ok #5]\n\x00")
            msgs = b.commit_messages("origin/main..HEAD")
        self.assertEqual(b.find_waiver(msgs), "5")
        with mock.patch("subprocess.run") as run:
            run.return_value = mock.Mock(returncode=128, stdout="")
            self.assertEqual(b.commit_messages("nope..HEAD"), [])


class FakeGh:
    """Records `gh` calls; `issue list` answers with `open_issues`, a failing verb raises."""

    def __init__(self, open_issues=(), fail=None):
        self.open_issues = list(open_issues)
        self.fail = fail
        self.calls = []

    def __call__(self, *args):
        self.calls.append(args)
        if self.fail and args[:2] == self.fail:
            raise RuntimeError(f"gh {' '.join(args[:2])}: HTTP 502")
        if args[:2] == ("issue", "list"):
            return json.dumps(self.open_issues)
        if args[:2] == ("issue", "create"):
            return "https://github.com/rust-works/succinctly/issues/9999\n"
        return ""

    def verbs(self):
        return [a[:2] for a in self.calls]


class TrackTests(unittest.TestCase):
    TARGET = "x86_64-unknown-linux-gnu"

    def track(self, measured, fake, issue=True, measured_at="main at abc123"):
        with mock.patch.object(b, "gh", fake):
            return b.track(measured, self.TARGET, config(1_000_000, target=self.TARGET), b.BUDGET, measured_at, issue)

    def test_within_the_budget_files_nothing(self):
        fake = FakeGh()
        code, lines = self.track(1_040_000, fake)
        self.assertEqual(code, 0)
        self.assertEqual(fake.calls, [])
        self.assertIn("Measured: main at abc123.", lines)

    def test_a_shrink_past_the_tolerance_files_nothing(self):
        # A warning to tighten the budget, but not a breach for a tracking issue.
        fake = FakeGh()
        self.assertEqual(self.track(900_000, fake)[0], 0)
        self.assertEqual(fake.calls, [])

    def test_over_the_budget_without_issue_fails_like_check(self):
        fake = FakeGh()
        code, lines = self.track(1_100_000, fake, issue=False)
        self.assertEqual(code, 1)
        self.assertTrue(lines[0].startswith("::error::"), lines[0])
        self.assertEqual(fake.calls, [])

    def test_over_the_budget_opens_a_labelled_issue_and_passes(self):
        fake = FakeGh()
        code, lines = self.track(1_100_000, fake)
        self.assertEqual(code, 0)
        self.assertEqual(fake.verbs(), [("issue", "list"), ("label", "create"), ("issue", "create")])
        listed = fake.calls[0]
        self.assertIn("--label", listed)
        self.assertEqual(listed[listed.index("--label") + 1], b.LABEL)
        self.assertEqual(listed[listed.index("--state") + 1], "open")
        create = fake.calls[2]
        self.assertEqual(create[create.index("--title") + 1], b.issue_title(self.TARGET))
        self.assertEqual(create[create.index("--label") + 1], b.LABEL)
        body = create[create.index("--body") + 1]
        self.assertIn("+10.00%", body)
        self.assertIn("Measured: main at abc123.", body)
        self.assertIn('set "x86_64-unknown-linux-gnu" -> "bytes" to 1100000', body)
        self.assertNotIn("::error::", body)  # a workflow command reads as noise in an issue
        self.assertIn("opened tracking issue https://github.com/rust-works/succinctly/issues/9999", lines[-1])

    def test_a_target_with_no_budget_is_a_breach_too(self):
        fake = FakeGh()
        with mock.patch.object(b, "gh", fake):
            code, _ = b.track(1_000, "riscv64gc-unknown-linux-gnu", config(1_000_000), b.BUDGET, "x", True)
        self.assertEqual(code, 0)
        self.assertIn(("issue", "create"), fake.verbs())

    def test_the_open_issue_is_rewritten_not_duplicated(self):
        fake = FakeGh([{"number": 42, "title": b.issue_title(self.TARGET), "body": "yesterday's numbers"}])
        code, lines = self.track(1_100_000, fake)
        self.assertEqual(code, 0)
        self.assertEqual(fake.verbs(), [("issue", "list"), ("issue", "edit")])
        edit = fake.calls[1]
        self.assertEqual(edit[2], "42")
        self.assertIn("+10.00%", edit[edit.index("--body") + 1])
        self.assertEqual(lines[-1], "rewrote tracking issue #42")

    def test_an_unchanged_body_is_left_alone(self):
        body = b.tracking_body(self.TARGET, b.check(1_100_000, self.TARGET, config(1_000_000, target=self.TARGET), b.BUDGET)[1], "main at abc123")
        fake = FakeGh([{"number": 42, "title": b.issue_title(self.TARGET), "body": body}])
        code, lines = self.track(1_100_000, fake)
        self.assertEqual(code, 0)
        self.assertEqual(fake.verbs(), [("issue", "list")])
        self.assertEqual(lines[-1], "tracking issue #42 already says this")

    def test_each_target_has_its_own_issue(self):
        other = b.issue_title("aarch64-unknown-linux-gnu")
        self.assertNotEqual(other, b.issue_title(self.TARGET))
        fake = FakeGh([{"number": 7, "title": other, "body": "arm"}])
        self.track(1_100_000, fake)
        self.assertEqual(fake.verbs(), [("issue", "list"), ("label", "create"), ("issue", "create")])

    def test_gh_raises_on_a_failed_command(self):
        with mock.patch("subprocess.run") as run:
            run.return_value = mock.Mock(returncode=1, stdout="", stderr="HTTP 502")
            with self.assertRaisesRegex(RuntimeError, "HTTP 502"):
                b.gh("issue", "list")
        with mock.patch("subprocess.run") as run:
            run.return_value = mock.Mock(returncode=0, stdout="[]", stderr="")
            self.assertEqual(b.gh("issue", "list"), "[]")


class StripTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.binary = os.path.join(self.tmp.name, "succinctly")
        with open(self.binary, "wb") as f:
            f.write(b"x" * 1000)
        # Keeps the first 600 bytes of the file it is given, in place, as `strip FILE` does.
        self.strip = os.path.join(self.tmp.name, "fake-strip")
        with open(self.strip, "w") as f:
            f.write('#!/bin/sh\nhead -c 600 "$1" > "$1.t" && mv "$1.t" "$1"\n')
        os.chmod(self.strip, os.stat(self.strip).st_mode | stat.S_IXUSR)

    def test_a_copy_is_stripped_and_the_binary_is_left_alone(self):
        self.assertEqual(b.stripped_size(self.binary, self.strip), 600)
        self.assertEqual(os.path.getsize(self.binary), 1000)

    def test_main_reports_to_stdout_and_the_step_summary(self):
        budget = os.path.join(self.tmp.name, "budget.json")
        with open(budget, "w") as f:
            json.dump(config(500, target="x86_64-unknown-linux-gnu"), f)
        summary = os.path.join(self.tmp.name, "summary.md")
        argv = ["check", "--binary", self.binary, "--target", "x86_64-unknown-linux-gnu"]
        argv += ["--budget", budget, "--strip", self.strip]
        with mock.patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": summary}), mock.patch("builtins.print") as out:
            self.assertEqual(b.main(argv), 1)
        printed = "\n".join(c.args[0] for c in out.call_args_list)
        self.assertIn("stripped binary 600 bytes", printed)
        with open(summary) as f:
            self.assertIn("stripped binary 600 bytes", f.read())

    def _run(self, base_size, env=None, extra=(), budget_bytes=600, budget_extra=None):
        """`main` over the 600-byte binary against a base of `base_size` bytes (a fake-strip result)."""
        base = os.path.join(self.tmp.name, "base")
        with open(base, "wb") as f:
            f.write(b"x" * base_size)
        # The fake strip keeps 600 bytes at most, so make the base smaller than that by hand instead.
        cfg = config(budget_bytes, target="x86_64-unknown-linux-gnu")
        cfg.update(budget_extra or {})
        budget = os.path.join(self.tmp.name, "budget.json")
        with open(budget, "w") as f:
            json.dump(cfg, f)
        argv = ["check", "--binary", self.binary, "--target", "x86_64-unknown-linux-gnu", "--budget", budget]
        argv += ["--strip", self.strip, "--baseline-binary", base, *extra]
        with mock.patch.dict(os.environ, env or {}, clear=False), mock.patch("builtins.print") as out:
            code = b.main(argv)
        return code, "\n".join(c.args[0] for c in out.call_args_list)

    def test_main_fails_a_pr_that_outgrew_its_base(self):
        code, printed = self._run(300, env={"PR_BODY": ""})  # base 300, head 600: +100%
        self.assertEqual(code, 1)
        self.assertIn("against its base 300 bytes", printed)

    def test_main_passes_a_pr_within_the_limit_of_its_base(self):
        code, printed = self._run(598, env={"PR_BODY": ""})  # +0.33%
        self.assertEqual(code, 0)

    def test_main_reads_the_waiver_from_the_pr_body(self):
        code, printed = self._run(300, env={"PR_BODY": "why\n\n[size-growth: ok #7960]\n"})
        self.assertEqual(code, 0)
        self.assertIn("accepted by [size-growth: ok #7960]", printed)

    def test_main_reads_the_waiver_from_commit_messages(self):
        with mock.patch.object(b, "commit_messages", return_value=["feat: x\n\n[size-growth: ok #9]\n"]) as cm:
            code, printed = self._run(300, env={"PR_BODY": ""}, extra=["--commit-range", "origin/main..HEAD"])
        cm.assert_called_once_with("origin/main..HEAD")
        self.assertEqual(code, 0)

    def test_report_mode_leaves_the_budget_to_the_growth_check(self):
        # 600 bytes against a 500-byte budget (+20%, past 5%) but equal to its base: passes in report mode.
        code, printed = self._run(600, env={"PR_BODY": ""}, extra=["--budget-mode", "report"], budget_bytes=500)
        self.assertEqual(code, 0)
        self.assertIn("::warning::", printed)
        self.assertNotIn("::error::", printed)

    def test_enforce_mode_still_fails_the_budget_when_the_growth_is_fine(self):
        code, printed = self._run(600, env={"PR_BODY": ""}, budget_bytes=500)
        self.assertEqual(code, 1)
        self.assertIn("past its budget", printed)

    def test_a_missing_baseline_falls_back_to_enforcing_the_budget(self):
        budget = os.path.join(self.tmp.name, "budget.json")
        with open(budget, "w") as f:
            json.dump(config(500, target="x86_64-unknown-linux-gnu"), f)
        argv = ["check", "--binary", self.binary, "--target", "x86_64-unknown-linux-gnu", "--budget", budget]
        argv += ["--strip", self.strip, "--baseline-binary", "/nonexistent/succinctly", "--budget-mode", "report"]
        with mock.patch("builtins.print") as out:
            code = b.main(argv)
        printed = "\n".join(c.args[0] for c in out.call_args_list)
        self.assertEqual(code, 1)
        self.assertIn("no baseline binary", printed)

    def _track_main(self, budget_bytes, fake, issue=True):
        budget = os.path.join(self.tmp.name, "budget.json")
        with open(budget, "w") as f:
            json.dump(config(budget_bytes, target="x86_64-unknown-linux-gnu"), f)
        argv = ["track", "--binary", self.binary, "--target", "x86_64-unknown-linux-gnu", "--budget", budget]
        argv += ["--strip", self.strip, "--measured-at", "main at abc"] + (["--issue"] if issue else [])
        with mock.patch.object(b, "gh", fake), mock.patch("builtins.print") as out:
            code = b.main(argv)
        return code, "\n".join(str(c.args[0]) for c in out.call_args_list)

    def test_main_track_files_an_issue_for_main_over_its_budget(self):
        fake = FakeGh()
        code, printed = self._track_main(500, fake)  # 600 bytes against 500: +20%
        self.assertEqual(code, 0)
        self.assertIn(("issue", "create"), fake.verbs())
        self.assertIn("stripped binary 600 bytes", printed)

    def test_main_track_within_its_budget_touches_no_issue(self):
        fake = FakeGh()
        self.assertEqual(self._track_main(600, fake)[0], 0)
        self.assertEqual(fake.calls, [])

    def test_main_track_exits_2_when_gh_fails(self):
        code, printed = self._track_main(500, FakeGh(fail=("issue", "list")))
        self.assertEqual(code, 2)
        self.assertIn("HTTP 502", printed)

    def test_a_failing_strip_is_an_error_not_a_size(self):
        with open(self.strip, "w") as f:
            f.write("#!/bin/sh\nexit 1\n")
        with self.assertRaises(Exception):
            b.stripped_size(self.binary, self.strip)


class CommittedBudgetTests(unittest.TestCase):
    def setUp(self):
        with open(b.BUDGET, encoding="utf-8") as f:
            self.config = json.load(f)

    def test_tolerance_is_five_percent(self):
        # The issue's figure; docs in binary_size.py and the PR explain why a toolchain bump stays inside it.
        self.assertEqual(self.config["tolerance"], 0.05)

    def test_the_per_pr_growth_limit_is_a_fraction_inside_the_tolerance(self):
        self.assertGreater(self.config["max_pr_growth"], 0)
        self.assertLess(self.config["max_pr_growth"], self.config["tolerance"])

    def test_every_linux_ci_leg_has_a_budget(self):
        for target in LINUX_CI_TARGETS:
            with self.subTest(target=target):
                entry = self.config["targets"][target]
                self.assertIsInstance(entry["bytes"], int)
                self.assertGreater(entry["bytes"], 1_000_000)
                self.assertTrue(entry["measured"])

    def test_ci_checks_the_binary_release_yml_ships(self):
        # The budget is of the shipped shape: the perf-guard build must use release.yml's features.
        with open(os.path.join(WORKFLOWS, "release.yml"), encoding="utf-8") as f:
            release = f.read()
        with open(os.path.join(WORKFLOWS, "ci.yml"), encoding="utf-8") as f:
            ci = f.read()
        shipped = re.search(r"run: cargo build --verbose --release (--features \S+) --target", release)
        self.assertIsNotNone(shipped)
        self.assertIn(f"run: cargo build --release {shipped.group(1)}\n", ci)
        self.assertIn("python3 scripts/binary_size.py check --binary target/release/succinctly", ci)
        # A PR gates its growth against the base the perf guard compares with.
        self.assertIn("--baseline-binary", ci.split("Check stripped binary size", 1)[1])
        self.assertIn("python3 scripts/test_binary_size.py -v\n", ci)


def read_workflow(name):
    with open(os.path.join(WORKFLOWS, name), encoding="utf-8") as f:
        return f.read()


def commands(text, needle):
    """Each command line of `text` naming `needle`, with `\\` continuations joined."""
    joined = re.sub(r"\\\n\s*", " ", text)
    return [line.strip() for line in joined.split("\n") if needle in line and not line.strip().startswith("#")]


def job_block(text, job):
    """A job's own block: from its key to the next job key at the same indent."""
    m = re.search(rf"\n  {re.escape(job)}:\n(?:.*\n)*?(?=  [A-Za-z0-9_-]+:\n|\Z)", text)
    assert m is not None, job
    return m.group(0)


class WhereTheBudgetIsEnforcedTests(unittest.TestCase):
    """#7959: ci.yml reports the budget; the scheduled job tracks it and release.yml enforces it."""

    def test_no_ci_run_fails_on_the_budget(self):
        # On a PR and on a push to main alike: failing main's push would leave main red with no owner.
        checks = commands(read_workflow("ci.yml"), "scripts/binary_size.py check")
        self.assertEqual(len(checks), 2, checks)
        for line in checks:
            with self.subTest(line=line):
                self.assertIn("--budget-mode report", line)

    def test_release_yml_enforces_the_budget_on_both_linux_builds(self):
        release = read_workflow("release.yml")
        checks = commands(release, "scripts/binary_size.py check")
        self.assertEqual(checks, ['run: python3 scripts/binary_size.py check --binary "target/${{ matrix.target }}/release/succinctly" --target "${{ matrix.target }}"'])
        step = release.split("- name: Check the stripped binary against its size budget", 1)[1].split("- name:", 1)[0]
        self.assertIn("matrix.build == 'linux-x64' || matrix.build == 'linux-arm64'", step)
        # After the strip, so the size is the archive's, and before the archive is uploaded.
        self.assertLess(release.index("- name: Strip release binary"), release.index("- name: Check the stripped binary"))
        self.assertLess(release.index("- name: Check the stripped binary"), release.index("- name: Upload release archive"))
        for target in LINUX_CI_TARGETS:
            self.assertIn(f"target: {target}\n", release)

    def test_the_scheduled_job_tracks_each_perf_guard_leg_from_its_artifact(self):
        ci = read_workflow("ci.yml")
        job = job_block(read_workflow("scheduled.yml"), "binary-size")
        guard = job_block(ci, "perf-guard")
        # The arch names, runners (so the same strip) and targets of ci.yml's perf-guard legs.
        for name, runner, target in (
            ("x86_64", "ubuntu-26.04", "x86_64-unknown-linux-gnu"),
            ("ARM64-Linux", "ubuntu-24.04-arm", "aarch64-unknown-linux-gnu"),
        ):
            with self.subTest(leg=name):
                self.assertRegex(guard, rf"- name: {re.escape(name)}\n\s+runner: {re.escape(runner)}\n")
                self.assertRegex(job, rf"- name: {re.escape(name)}\n\s+runner: {re.escape(runner)}\n\s+target: {re.escape(target)}\n")
        # It looks up the name ci.yml uploads, from main's push runs only.
        self.assertIn("name: perf-guard-bin-${{ matrix.name }}-${{ github.sha }}-${{ env.RUSTC_KEY }}", ci)
        self.assertIn('startswith(\\"perf-guard-bin-$ARCH-$sha-\\")', job)
        self.assertIn("runs?event=push&branch=main", job)
        self.assertIn('select(.head_branch == "main")', job)
        tracks = commands(job, "scripts/binary_size.py track")
        self.assertEqual(len(tracks), 1, tracks)
        self.assertIn("--issue", tracks[0])
        self.assertIn('--target "$TARGET"', tracks[0])
        for permission in ("contents: read", "actions: read", "issues: write"):
            self.assertIn(permission, job)
        self.assertIn("if: github.repository == 'rust-works/succinctly'", job)


if __name__ == "__main__":
    unittest.main()
