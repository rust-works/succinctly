#!/usr/bin/env python3
"""Tests for scripts/binary_size.py (#7390): the verdict, its messages, the strip and the committed budget.

No build and no real `strip`: a fake strip program stands in. Run directly
(`python3 scripts/test_binary_size.py`).
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


def config(bytes_, tolerance=0.05, target="t"):
    return {"tolerance": tolerance, "targets": {target: {"bytes": bytes_, "measured": "test"}}}


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
        self.assertIn("python3 scripts/binary_size.py check --binary target/release/succinctly\n", ci)
        self.assertIn("python3 scripts/test_binary_size.py -v\n", ci)


if __name__ == "__main__":
    unittest.main()
