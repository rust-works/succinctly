#!/usr/bin/env python3
"""Unit tests for scripts/jq-path-register-sweep.py's process runner that need no
build and no jq: the children are `cat`, `yes`, `sleep` and a Python one-liner. Run
directly (`python3 scripts/test_jq_path_register_sweep.py`) or via
`python3 -m unittest scripts/test_jq_path_register_sweep.py`.

They pin #3893: an operand that is infinite on some input made the runner buffer an
unbounded stdout (`recurse(.a)?` on `null`) or let jq itself grow to tens of GB with
nothing printed (`[path(recurse(.a)?)]`), so a run was killed by its memory cap rather
than classed `ORACLE_TIMEOUT`. Every cap test also asserts the run came back well inside
its `--timeout`: a cap that only fired at the deadline would be the old behaviour.

Imported via `importlib` rather than `import`: the script's filename has hyphens.
"""

import importlib.util
import pathlib
import sys
import time
import unittest

_SCRIPT_PATH = pathlib.Path(__file__).parent / "jq-path-register-sweep.py"
_spec = importlib.util.spec_from_file_location("jq_path_register_sweep", _SCRIPT_PATH)
sweep = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(sweep)

# Long enough that a cap which never fires would run out this clock, short enough
# that a passing test is not slow.
DEADLINE = 20.0
PROMPT = 5.0  # a cap must fire well before the deadline


class RunOneTests(unittest.TestCase):
    def timed(self, argv, doc="", **patches):
        saved = {k: getattr(sweep, k) for k in patches}
        for k, v in patches.items():
            setattr(sweep, k, v)
        try:
            started = time.monotonic()
            result = sweep.run_one(argv, doc, DEADLINE)
            return result, time.monotonic() - started
        finally:
            for k, v in saved.items():
                setattr(sweep, k, v)

    def test_ordinary_run_returns_stdout_and_exit_code(self):
        self.assertEqual(self.timed(["cat"], "[1,2]")[0], ("[1,2]", 0))
        self.assertEqual(self.timed(["sh", "-c", "echo out; exit 3"])[0], ("out\n", 3))

    def test_a_run_that_never_reads_its_input_is_not_an_error(self):
        # `true` exits before reading stdin: the write's BrokenPipeError is not a failure.
        self.assertEqual(self.timed(["true"], "x" * 1000)[0], ("", 0))

    def test_the_deadline_is_a_timeout(self):
        started = time.monotonic()
        result = sweep.run_one(["sleep", "30"], "", 0.3)
        self.assertEqual(result, ("", None))
        self.assertLess(time.monotonic() - started, PROMPT)

    def test_unbounded_stdout_is_killed_at_the_cap_and_is_a_timeout(self):
        (stdout, code), elapsed = self.timed(["yes"], OUTPUT_CAP=1 << 20)
        self.assertIsNone(code)
        self.assertIn("output exceeded", stdout)
        self.assertLess(elapsed, PROMPT)

    def test_output_just_under_the_cap_is_kept_whole(self):
        size = 1 << 16
        (stdout, code), _ = self.timed(
            [sys.executable, "-c", f"print('a' * {size - 1})"], OUTPUT_CAP=size
        )
        self.assertEqual((len(stdout), code), (size, 0))

    def test_unbounded_memory_without_output_is_killed_at_the_cap_and_is_a_timeout(self):
        # What jq does on `[path(recurse(.a)?)]`: grow forever, print nothing. The bytes
        # are written, not just reserved, so the resident size really grows.
        grow = (
            "import time\n"
            "held = []\n"
            "while True:\n"
            "    held.append(b'x' * (8 << 20))\n"
            "    time.sleep(0.01)\n"
        )
        (stdout, code), elapsed = self.timed(
            [sys.executable, "-c", grow], RSS_CAP=96 << 20
        )
        self.assertIsNone(code)
        self.assertIn("resident size exceeded", stdout)
        self.assertLess(elapsed, PROMPT)

    def test_a_modest_run_is_not_mistaken_for_a_memory_hog(self):
        # Still running at the first poll, so the watchdog samples it, and stays small.
        (stdout, code), _ = self.timed(
            [sys.executable, "-c", "import time; time.sleep(0.3); print('ok')"],
            RSS_CAP=256 << 20,
        )
        self.assertEqual((stdout, code), ("ok\n", 0))


class ClassifyTests(unittest.TestCase):
    def test_a_capped_oracle_is_skipped_and_a_capped_build_is_a_timeout(self):
        capped = ("<output exceeded 4194304 bytes>", None)
        self.assertEqual(sweep.classify(capped, ("[]\n", 0)), "ORACLE_TIMEOUT")
        self.assertEqual(sweep.classify(("[]\n", 0), capped), "TIMEOUT")


if __name__ == "__main__":
    unittest.main()
