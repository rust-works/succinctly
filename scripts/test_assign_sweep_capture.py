#!/usr/bin/env python3
"""Tests for `capture_case` in scripts/jq-assign-path-oracle-sweep.sh (#5997).

macOS's /bin/bash 3.2 sometimes reads a failed child's exit status as 0 after `x="$(cmd)"`. The
sweep reruns a case that exits 0 with text on stderr. That path runs once in about 140,000 real
captures, so it is driven here with a child that loses its status on purpose: `capture_case` is
cut out of the script and run under the system bash (3.2 on macOS, where the bug lives).
"""

import os
import re
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "jq-assign-path-oracle-sweep.sh")


def capture_case_source():
    with open(SCRIPT) as f:
        text = f.read()
    match = re.search(r"^lost_status=0\n.*?^capture_case\(\) \{\n.*?^\}\n", text, re.S | re.M)
    if not match:
        raise AssertionError("capture_case not found in %s" % SCRIPT)
    return match.group(0)


class CaptureCase(unittest.TestCase):
    def run_capture(self, child_script):
        """Run capture_case on `child_script` (a bash body); returns (out, code, err, lost, stderr)."""
        with tempfile.TemporaryDirectory() as tmp:
            child = os.path.join(tmp, "child.sh")
            with open(child, "w") as f:
                f.write("#!/bin/bash\nSTATE=%s/state\n%s\n" % (tmp, child_script))
            os.chmod(child, 0o755)
            driver = (
                "set -euo pipefail\n%s\n"
                'capture_case out code "%s/err" err "%s"\n'
                'printf "%%s\\n%%s\\n%%s\\n%%s\\n" "$out" "$code" "$err" "$lost_status"\n'
            ) % (capture_case_source(), tmp, child)
            result = subprocess.run(["/bin/bash", "-c", driver], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            out, code, err, lost = result.stdout.rstrip("\n").split("\n")
            return out, code, err, int(lost), result.stderr

    def test_a_failure_is_recorded_as_it_is(self):
        out, code, err, lost, _ = self.run_capture('echo partial; echo "boom" >&2; exit 5')
        self.assertEqual((out, code, err, lost), ("partial", "5", "boom", 0))

    def test_a_success_with_no_stderr_is_not_rerun(self):
        out, code, err, lost, _ = self.run_capture("echo fine; exit 0")
        self.assertEqual((out, code, err, lost), ("fine", "0", "", 0))

    def test_a_status_read_as_0_with_stderr_is_rerun_until_it_reads_the_failure(self):
        # The first run exits 0 as a lost status does; later runs exit 5.
        body = (
            'echo "boom" >&2\n'
            'if [ ! -e "$STATE" ]; then : > "$STATE"; exit 0; fi\n'
            "exit 5"
        )
        out, code, err, lost, stderr = self.run_capture(body)
        self.assertEqual((code, err, lost), ("5", "boom", 1))
        self.assertIn("lost-status: exit 0 with stderr", stderr)
        self.assertIn("run 1 of 3", stderr)

    def test_two_lost_statuses_in_a_row_are_both_rerun(self):
        body = (
            'echo "boom" >&2\n'
            'n=$(cat "$STATE" 2>/dev/null || echo 0); echo $((n + 1)) > "$STATE"\n'
            'if [ "$n" -lt 2 ]; then exit 0; fi\n'
            "exit 5"
        )
        _, code, _, lost, _ = self.run_capture(body)
        self.assertEqual((code, lost), ("5", 2))

    def test_a_case_that_always_reads_0_with_stderr_is_recorded_as_0_and_says_so(self):
        out, code, err, lost, stderr = self.run_capture('echo "warn" >&2; exit 0')
        self.assertEqual((code, err, lost), ("0", "warn", 2))
        self.assertIn("on all 3 runs, recorded as exit 0", stderr)

    def test_the_callers_variable_names_do_not_collide_with_the_locals(self):
        # `out`, `code`, `err` and `attempt` are the names a caller would pick.
        with tempfile.TemporaryDirectory() as tmp:
            driver = (
                "set -euo pipefail\n%s\n"
                'capture_case attempt out "%s/e" err /bin/sh -c "echo hi; exit 3"\n'
                'printf "%%s %%s\\n" "$attempt" "$out"\n'
            ) % (capture_case_source(), tmp)
            result = subprocess.run(["/bin/bash", "-c", driver], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), "hi 3")


if __name__ == "__main__":
    sys.exit(unittest.main())
