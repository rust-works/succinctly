#!/usr/bin/env python3
"""Unit tests for scripts/perf-guard.py that need no binary, valgrind, or
built fixtures -- pure checks on the script's own logic. Run directly
(`python3 scripts/test_perf_guard.py`) or via `python3 -m unittest
scripts/test_perf_guard.py`; wired into CI's `perf-guard` job (#3169) so a
regression here fails fast, before either matrix leg's expensive
valgrind-based measurement even starts.

Imported via `importlib` rather than `import perf_guard`: the script's
filename has a hyphen, which is not a valid Python module identifier.
"""

import importlib.util
import pathlib
import unittest

_SCRIPT_PATH = pathlib.Path(__file__).parent / "perf-guard.py"
_spec = importlib.util.spec_from_file_location("perf_guard", _SCRIPT_PATH)
perf_guard = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(perf_guard)


class FailureAdviceModeTests(unittest.TestCase):
    """#3169: the closing failure advice must depend on whether this run
    compared against `--baseline-binary` (CI's mode on every PR/push run,
    where `--update-baseline` is both rejected outright and would not
    consult the checked-in baseline it can't be run against anyway) or the
    checked-in baseline file (a local `--check`, where `--update-baseline`
    is the correct and only advice)."""

    def test_baseline_binary_mode_never_recommends_update_baseline(self):
        # The advice may still *mention* --update-baseline to say it can't
        # help (which it does, deliberately) -- what must never appear is
        # the recommending phrase itself.
        advice = perf_guard.failure_advice(baseline_binary=True)
        self.assertNotIn("re-run with --update-baseline", advice)

    def test_baseline_binary_mode_points_at_query_thresholds(self):
        advice = perf_guard.failure_advice(baseline_binary=True)
        self.assertIn("QUERY_THRESHOLDS", advice)

    def test_checked_in_baseline_mode_advises_update_baseline(self):
        advice = perf_guard.failure_advice(baseline_binary=False)
        self.assertIn("--update-baseline", advice)

    def test_checked_in_baseline_mode_does_not_mention_query_thresholds(self):
        # Not because an override is wrong in this mode too, but because
        # --update-baseline is the direct, correct fix here -- unlike
        # --baseline-binary mode, where it's rejected outright (#3169).
        advice = perf_guard.failure_advice(baseline_binary=False)
        self.assertNotIn("QUERY_THRESHOLDS", advice)


if __name__ == "__main__":
    unittest.main()
