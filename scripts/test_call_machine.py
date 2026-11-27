#!/usr/bin/env python3
"""Self-tests for the call machine's harness, `scripts/call_machine/` (#7977). No build, no jq.

    python3 scripts/test_call_machine.py -v

- The golden file `tests/data/call-machine-corpus-7977.json`, which `tests/jq_cli/i7977.rs` runs
  in CI, holds exactly the cases of `cases.py`, in order: a case edited without `check.py capture`
  fails here instead of leaving CI checking the old program.
- `ladders.py --bin LABEL=PATH@VAR=VALUE` gives that binary its own environment, which the A/B of
  a runtime switch needs (#7961).
"""
import json
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "call_machine"))

import cases  # noqa: E402
import check  # noqa: E402

try:  # ladders.py sets resource limits on its children, which needs a POSIX `resource`
    import ladders  # noqa: E402
except ImportError:
    ladders = None


class GoldenTests(unittest.TestCase):
    def test_the_golden_file_holds_the_cases_in_order(self):
        with open(check.GOLDEN) as f:
            golden = json.load(f)
        self.assertEqual(
            [(r["id"], r["args"], r["filter"], r["route"], r["stdin"]) for r in golden],
            [(c["id"], c["args"], c["filter"], c["route"], c["stdin"]) for c in cases.CORRECT],
        )
        for row in golden:
            self.assertEqual(
                set(row), {"id", "args", "filter", "route", "stdin", "stdout", "stderr", "exit"}
            )

    def test_case_ids_are_unique(self):
        ids = [c["id"] for c in cases.CORRECT]
        self.assertEqual(len(ids), len(set(ids)))

    def test_every_taken_case_holds_a_def(self):
        for c in cases.CORRECT:
            if c["route"] == "take":
                self.assertIn("def ", c["filter"], c["id"])


@unittest.skipIf(ladders is None, "no POSIX `resource` module")
class LadderTests(unittest.TestCase):
    def test_a_bin_without_assignments_inherits_the_environment(self):
        self.assertEqual(ladders.parse_bin("main=bin/main"), ("main", "bin/main", None))

    def test_a_bin_sets_each_assignment(self):
        label, path, env = ladders.parse_bin("on=bin/s@SUCCINCTLY_CALL_MACHINE=1@X=a=b")
        self.assertEqual((label, path), ("on", "bin/s"))
        self.assertEqual(env["SUCCINCTLY_CALL_MACHINE"], "1")
        self.assertEqual(env["X"], "a=b")
        self.assertEqual(env.get("PATH"), os.environ.get("PATH"))

    def test_a_path_may_hold_an_at_sign(self):
        self.assertEqual(ladders.parse_bin("a=/x@y/bin"), ("a", "/x@y/bin", None))
        label, path, env = ladders.parse_bin("a=/x@y/bin@V=1")
        self.assertEqual((label, path, env["V"]), ("a", "/x@y/bin", "1"))

    def test_every_ladder_has_a_size_placeholder_and_sizes(self):
        for lad in cases.LADDERS:
            self.assertTrue(lad["sizes"], lad["id"])
            if lad["sizes"] != [0]:
                self.assertIn("{N}", lad["filter"], lad["id"])


if __name__ == "__main__":
    unittest.main()
