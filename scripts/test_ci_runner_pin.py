#!/usr/bin/env python3
"""Pins the x86_64 release build to ubuntu-24.04 and the x86_64 CI jobs to the image they were verified on (#5878, #5926).

`ubuntu-latest` moved to ubuntu-26.04 on 2026-11-02 and its glibc prints different last digits for
`tgamma`, `lgamma` and friends, so `math_libm_family` and `math_platform_libm_bits` failed on every
x86_64 leg and the merge queue stopped. Those two goldens no longer pin the digits 26.04 prints
differently (#5926), and the x86_64 test matrix, clippy, docs, perf guard (valgrind) and coverage jobs now
run on 26.04. The release build stays on 24.04 on purpose, not for want of a run: the shipped binary
links the glibc of the image that builds it, so a newer image raises the glibc its users need
(docs/guides/release.md). A silent image move is exactly what stopped the queue, so moving the image is a
deliberate edit of this file. Run directly (`python3 scripts/test_ci_runner_pin.py`).
"""

import pathlib
import re
import unittest

WORKFLOWS = pathlib.Path(__file__).resolve().parent.parent / ".github" / "workflows"


def job_block(text, job):
    """The job's own block: from its key to the next top-level job key."""
    match = re.search(rf"\n  {job}:\n(?:.*\n)*?(?=  [A-Za-z0-9_-]+:\n|\Z)", text)
    assert match is not None, job
    return match.group(0)


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
                self.assertEqual(lines, [], "pin to ubuntu-24.04, or decide the release glibc floor first (docs/guides/release.md) (#5926)")

    def test_the_x86_64_test_matrix_clippy_and_docs_run_on_26_04(self):
        text = (WORKFLOWS / "ci.yml").read_text(encoding="utf-8")
        for job in ("test-x86-matrix", "clippy", "docs"):
            with self.subTest(job=job):
                block = job_block(text, job)
                self.assertIn("runs-on: ubuntu-26.04", block)
                self.assertNotIn("ubuntu-24.04\n", block)

    def test_the_perf_guard_x86_64_leg_runs_on_26_04_and_the_arm_leg_does_not_move(self):
        block = job_block((WORKFLOWS / "ci.yml").read_text(encoding="utf-8"), "perf-guard")
        self.assertRegex(block, r"- name: x86_64\n\s+runner: ubuntu-26\.04\n")
        self.assertRegex(block, r"- name: ARM64-Linux\n\s+runner: ubuntu-24\.04-arm\n")

    def test_the_x86_64_coverage_legs_run_on_26_04_in_both_callers(self):
        # The pull-request job and the baseline it is diffed against share one image.
        for name, job in (("ci.yml", "coverage"), ("coverage-baseline.yml", "coverage")):
            with self.subTest(workflow=name):
                block = job_block((WORKFLOWS / name).read_text(encoding="utf-8"), job)
                self.assertRegex(block, r"- name: x86_64\n\s+runner: ubuntu-26\.04\n")
                self.assertRegex(block, r"- name: ARM64\n\s+runner: ubuntu-24\.04-arm\n")

    def test_the_release_build_stays_on_24_04_for_its_glibc_floor(self):
        text = (WORKFLOWS / "release.yml").read_text(encoding="utf-8")
        self.assertRegex(text, r"- build: linux-x64\n(?:\s+#.*\n)*\s+os: ubuntu-24\.04\n")
        self.assertNotIn("ubuntu-26.04", text)


if __name__ == "__main__":
    unittest.main()
