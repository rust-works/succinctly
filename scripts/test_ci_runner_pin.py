#!/usr/bin/env python3
"""Pins every x86_64 Linux runner to ubuntu-24.04 until the jq libm goldens are re-captured (#5878).

`ubuntu-latest` moved to ubuntu-26.04 on 2026-11-02 and its glibc prints different last digits for
`tgamma`, `lgamma` and friends, so `math_libm_family` and `math_platform_libm_bits` failed on every
x86_64 leg and the merge queue stopped. A silent image move is exactly what that was; this makes the
next one a deliberate edit of this file. Run directly (`python3 scripts/test_ci_runner_pin.py`).
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
                self.assertEqual(lines, [], "pin to ubuntu-24.04, or re-capture the libm goldens first (#5878)")

    def test_the_x86_64_test_matrix_is_pinned(self):
        text = (WORKFLOWS / "ci.yml").read_text(encoding="utf-8")
        self.assertRegex(text, r"test-x86-matrix:\n(?:.*\n)*?    runs-on: ubuntu-24\.04\n")


if __name__ == "__main__":
    unittest.main()
