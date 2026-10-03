#!/usr/bin/env python3
"""Unit tests for scripts/ab-cli.py that need no `succinctly` binary: the "binaries"
are small Python scripts written to a temporary directory, so these check the harness's
own gate and nothing about the tool it times. Run directly
(`python3 scripts/test_ab_cli.py`) or via `python3 -m unittest scripts/test_ab_cli.py`;
wired into CI's `perf-guard` job beside `test_perf_guard.py`.

Imported via `importlib` rather than `import ab_cli`: the script's filename has a
hyphen, which is not a valid Python module identifier.
"""

import contextlib
import importlib.util
import io
import os
import pathlib
import stat
import sys
import tempfile
import unittest
from unittest import mock

_SCRIPT_PATH = pathlib.Path(__file__).parent / "ab-cli.py"
_spec = importlib.util.spec_from_file_location("ab_cli", _SCRIPT_PATH)
ab_cli = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ab_cli)

# The command line the harness builds for `--tool yq` ends `QUERY PATH`, so a fake binary
# reads its query from `sys.argv[-2]`.
GOOD = 'import sys; sys.stdout.write("ok\\n")'
BAD_QUERY_FAILS = (
    "import sys\n"
    'if sys.argv[-2] == "BAD":\n'
    '    sys.stderr.write("error: parse error near BAD\\n"); sys.exit(3)\n'
    'sys.stdout.write("ok\\n")\n'
)
# Same output as GOOD, a different exit status: the case a digest alone cannot see.
OK_BUT_EXITS_3 = 'import sys; sys.stdout.write("ok\\n"); sys.exit(3)'
DIFFERENT_OUTPUT = 'import sys; sys.stdout.write("different\\n")'


def flaky(counter_file):
    """Exits 0 the first time it is run (the gate's run) and 7 every time after."""
    return (
        "import os, sys\n"
        f"p = {str(counter_file)!r}\n"
        "n = int(open(p).read()) if os.path.exists(p) else 0\n"
        "open(p, 'w').write(str(n + 1))\n"
        "sys.stdout.write('ok\\n')\n"
        "sys.exit(0 if n == 0 else 7)\n"
    )


class GateTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.dir = pathlib.Path(self._tmp.name)
        self.input = self.dir / "input.json"
        self.input.write_text("{}\n")
        # The idleness and battery checks read `top`/`pmset` and take seconds; they are
        # not what is under test.
        for name, value in (("battery_warning", None), ("machine_warnings", [])):
            patcher = mock.patch.object(ab_cli, name, return_value=value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def binary(self, name, body):
        path = self.dir / name
        path.write_text(f"#!{sys.executable}\n{body}")
        path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
        return str(path)

    def run_harness(self, *extra, queries=("q",)):
        """Run `main` and return `(exit, stdout)`: `exit` is the return value, or the
        message `sys.exit` was given."""
        argv = [*extra, "--files", str(self.input), "--queries", *queries, "--reps", "1",
                "--before-profile", "x", "--after-profile", "x"]
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            try:
                code = ab_cli.main(argv)
            except SystemExit as exit_:
                code = exit_.code
        return code, out.getvalue()

    def test_two_binaries_failing_the_same_way_are_refused_not_timed(self):
        # The #2626/#3479 shape: identical stderr, so identical digests, so the identity
        # gate alone passes and the table prints a delta for a parse error.
        before = self.binary("before", BAD_QUERY_FAILS)
        after = self.binary("after", BAD_QUERY_FAILS)
        code, out = self.run_harness("--before", before, "--after", after, queries=("BAD",))
        self.assertIsInstance(code, str, "a refusal is a sys.exit message, not a return")
        self.assertIn("exited non-zero", code)
        self.assertIn("FAIL", out)
        self.assertIn("input.json", out, "the failing input is named")
        self.assertIn("query='BAD'", out, "the failing query is named")
        self.assertIn("parse error near BAD", out, "the first line of the error is shown")
        self.assertIn("identity: 1 configurations, 0 differences", out)
        self.assertIn("exit status: 0 of 1 configurations exited 0", out)
        self.assertNotIn("median of medians", out, "no timing table may follow a refusal")

    def test_a_clean_run_reports_how_many_configurations_exited_zero(self):
        before = self.binary("before", GOOD)
        after = self.binary("after", GOOD)
        code, out = self.run_harness("--before", before, "--after", after, queries=("a", "b"))
        self.assertEqual(code, 0)
        self.assertIn("identity: 2 configurations, 0 differences", out)
        self.assertIn("exit status: 2 of 2 configurations exited 0", out)
        self.assertIn("median of medians", out)

    def test_one_failing_configuration_refuses_the_whole_run(self):
        # Timing the other rows alone would print a table that looks complete.
        before = self.binary("before", BAD_QUERY_FAILS)
        after = self.binary("after", BAD_QUERY_FAILS)
        code, out = self.run_harness("--before", before, "--after", after,
                                     queries=("good", "BAD"))
        self.assertIn("1 configuration(s) exited non-zero", code)
        self.assertIn("exit status: 1 of 2 configurations exited 0", out)
        self.assertNotIn("median of medians", out)

    def test_differing_exit_status_with_identical_output_is_refused(self):
        before = self.binary("before", GOOD)
        after = self.binary("after", OK_BUT_EXITS_3)
        code, out = self.run_harness("--before", before, "--after", after)
        self.assertIn("differ", code)
        self.assertIn("exit status differs: before 0, after 3", out)
        self.assertNotIn("median of medians", out)

    def test_differing_output_is_still_refused(self):
        before = self.binary("before", GOOD)
        after = self.binary("after", DIFFERENT_OUTPUT)
        code, out = self.run_harness("--before", before, "--after", after)
        self.assertIn("differ", code)
        self.assertIn("DIFF", out)
        self.assertIn("identity: 1 configurations, 1 differences", out)

    def test_allow_nonzero_times_an_error_path_and_says_so(self):
        before = self.binary("before", BAD_QUERY_FAILS)
        after = self.binary("after", BAD_QUERY_FAILS)
        code, out = self.run_harness("--before", before, "--after", after,
                                     "--allow-nonzero", queries=("BAD",))
        self.assertEqual(code, 0)
        self.assertIn("NONZERO (allowed)", out)
        self.assertIn("exit status: 0 of 1 configurations exited 0 (non-zero allowed)", out)
        self.assertIn("median of medians", out)

    def test_allow_nonzero_does_not_waive_a_difference_in_exit_status(self):
        # Two binaries that fail differently are not comparable even on purpose.
        before = self.binary("before", GOOD)
        after = self.binary("after", OK_BUT_EXITS_3)
        code, _out = self.run_harness("--before", before, "--after", after, "--allow-nonzero")
        self.assertIn("differ", code)

    def test_a_control_run_refuses_a_failing_command(self):
        # `--control` skips the identity gate and runs one binary against a copy of itself,
        # so before this the exit status was never looked at on that path.
        before = self.binary("before", BAD_QUERY_FAILS)
        code, out = self.run_harness("--before", before, "--control", queries=("BAD",))
        self.assertIn("exited non-zero", code)
        self.assertIn("exit status gate (identity skipped)", out)
        self.assertNotIn("median of medians", out)

    def test_no_identity_still_gates_the_exit_status(self):
        before = self.binary("before", BAD_QUERY_FAILS)
        after = self.binary("after", BAD_QUERY_FAILS)
        code, out = self.run_harness("--before", before, "--after", after, "--no-identity",
                                     queries=("BAD",))
        self.assertIn("exited non-zero", code)
        self.assertNotIn("identity:", out)

    def test_a_run_that_fails_while_being_timed_is_not_a_sample(self):
        # The gate's run passes; every later run exits 7 (an OOM kill, a flaky input). The
        # old harness discarded the status and put the fast failure into min-of-N.
        before = self.binary("before", flaky(self.dir / "before-count"))
        after = self.binary("after", GOOD)
        code, out = self.run_harness("--before", before, "--after", after)
        self.assertIn("exited 7 while being timed", code)
        self.assertNotIn("median of medians", out)


if __name__ == "__main__":
    unittest.main()
