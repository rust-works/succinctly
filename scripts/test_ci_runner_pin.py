#!/usr/bin/env python3
"""Pins every x86_64 Linux runner except the test matrix to ubuntu-24.04 until each is verified on 26.04 (#5878, #5926).

`ubuntu-latest` moved to ubuntu-26.04 on 2026-11-02 and its glibc prints different last digits for
`tgamma`, `lgamma` and friends, so `math_libm_family` and `math_platform_libm_bits` failed on every
x86_64 leg and the merge queue stopped. Those two goldens no longer pin the digits 26.04 prints
differently (#5926), and the x86_64 test matrix now runs on 26.04. The pin stays on the other jobs
because the perf guard (valgrind), coverage and release jobs have not been run there; a silent image
move is exactly what stopped the queue, so moving the image is a deliberate edit of this file. Run
directly (`python3 scripts/test_ci_runner_pin.py`).
"""

import pathlib
import re
import unittest

WORKFLOWS = pathlib.Path(__file__).resolve().parent.parent / ".github" / "workflows"


class RunnerPinTests(unittest.TestCase):
    def test_no_workflow_uses_the_floating_ubuntu_image(self):
        workflows = sorted(WORKFLOWS.glob("*.yml"))
        self.assertTrue(workflows)
        for path in workflows:
            with self.subTest(workflow=path.name):
                lines = [
                    f"line {i}: {line.strip()}"
                    for i, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1)
                    if "ubuntu-latest" in line and not line.lstrip().startswith("#")
                ]
                self.assertEqual(lines, [], "pin to ubuntu-24.04, or verify the perf guard, coverage and release jobs on the new image first (#5926)")

    def test_the_x86_64_test_matrix_runs_on_26_04(self):
        text = (WORKFLOWS / "ci.yml").read_text(encoding="utf-8")
        self.assertRegex(text, r"test-x86-matrix:\n(?:.*\n)*?    runs-on: ubuntu-26\.04\n")

    def test_the_jobs_not_yet_verified_on_26_04_stay_pinned(self):
        text = (WORKFLOWS / "ci.yml").read_text(encoding="utf-8")
        for job in ("clippy", "docs", "perf-guard", "coverage"):
            with self.subTest(job=job):
                # The job's own block: from its key to the next top-level job key.
                match = re.search(rf"\n  {job}:\n(?:.*\n)*?(?=  [A-Za-z0-9_-]+:\n|\Z)", text)
                self.assertIsNotNone(match)
                block = match.group(0)
                self.assertIn("ubuntu-24.04", block)
                self.assertNotIn("ubuntu-26.04", block)


if __name__ == "__main__":
    unittest.main()
