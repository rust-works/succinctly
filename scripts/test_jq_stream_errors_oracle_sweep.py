#!/usr/bin/env python3
"""Unit tests for the two rules of scripts/jq-stream-errors-oracle-sweep.py that set aside what
`succinctly jq` takes from jq 1.8.2 (#7890). Run directly
(`python3 scripts/test_jq_stream_errors_oracle_sweep.py`) or via
`python3 -m unittest scripts/test_jq_stream_errors_oracle_sweep.py`.

They need no build. The rules are pure functions, and `main()` is driven with two shell scripts in
place of the two binaries. One class asks a real jq 1.7.1 for rule A's probe and is skipped where
none is installed.

The sweep compares against the pinned jq 1.7.1, and two behaviours `succinctly jq` follows are jq
1.8.2's: the array of `--stream-errors -s` holds the event or fault the end of the text leaves
(#7523, #7807), and a NaN with a payload is no number (#7522). Rule A skips a `-s` row only when the
oracle itself runs the program more than once, rule B keeps `nan` and a digit apart in the
generated text, and a ceiling fails the run when either touches too many rows. Skipping more would
hide a real difference, so the tests pin both sides: the #7890 rows are set aside, and a row that
differs for any other reason is still reported.

Imported via `importlib` rather than `import`: the script's filename has hyphens.
"""

import contextlib
import importlib.util
import io
import os
import pathlib
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

_SCRIPT_PATH = pathlib.Path(__file__).parent / "jq-stream-errors-oracle-sweep.py"
_spec = importlib.util.spec_from_file_location("jq_stream_errors_oracle_sweep", _SCRIPT_PATH)
sweep = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(sweep)

# The input of #7890, byte for byte (seed 1, the 184th input): its last byte opens an array that
# the text never closes, after a value (`2.5`) the slurp has taken.
ISSUE_INPUT = b' -{"a"123456781"\\u00e9"":{}\\nulnul{}},"k":[,\n-[x\n2.5['
# The three rows that differed, as (program, on standard input).
ISSUE_ROWS = [
    (["-c", "-s", "[., input_filename, input_line_number]"], False),
    (["-c", "-s", "."], False),
    (["-c", "-s", 'error("y")'], True),
]


def jq_1_7_1():
    """A jq 1.7.1 to ask, or None: `$JQ_ORACLE`, `/usr/bin/jq`, then the one on PATH."""
    for candidate in (os.environ.get("JQ_ORACLE"), "/usr/bin/jq", shutil.which("jq")):
        if not candidate or not os.path.exists(candidate):
            continue
        try:
            version = subprocess.run([candidate, "--version"], capture_output=True, timeout=30).stdout
        except (OSError, subprocess.SubprocessError):
            continue
        if version.startswith(b"jq-1.7.1"):
            return candidate
    return None


class SlurpRerunRuleTests(unittest.TestCase):
    """Rule A reads the oracle's probe output and nothing else."""

    def test_two_runs_or_more_are_set_aside(self):
        # jq 1.7.1's answers to `--stream-errors -c -s 0` over the #7890 input and over `[[1]`.
        self.assertTrue(sweep.oracle_reruns_slurped_program(b"0\n0\n"))
        self.assertTrue(sweep.oracle_reruns_slurped_program(b"0\n0\n0\n"))

    def test_one_run_or_none_is_compared(self):
        # One run is every `-s` input jq 1.7.1 and 1.8.2 read alike; no output is an oracle that failed.
        self.assertFalse(sweep.oracle_reruns_slurped_program(b"0\n"))
        self.assertFalse(sweep.oracle_reruns_slurped_program(b""))

    def test_the_probe_is_a_slurp_of_a_program_that_reads_nothing(self):
        self.assertEqual(sweep.SLURP_PROBE, ["-c", "-s", "0"])


@unittest.skipUnless(jq_1_7_1(), "needs jq 1.7.1 (`$JQ_ORACLE`, /usr/bin/jq or PATH)")
class SlurpRerunAgainstJqTests(unittest.TestCase):
    """The same rule with a real jq 1.7.1 answering the probe, as in a sweep."""

    def probe(self, data, on_stdin):
        oracle = [jq_1_7_1(), "--stream-errors"]
        if on_stdin:
            return sweep.run(oracle, sweep.SLURP_PROBE, data)[0]
        with tempfile.TemporaryDirectory() as scratch:
            path = os.path.join(scratch, "f0.json")
            with open(path, "wb") as handle:
                handle.write(data)
            return sweep.run(oracle, sweep.SLURP_PROBE + [path], None)[0]

    def test_the_three_rows_of_7890_are_skipped(self):
        for program, on_stdin in ISSUE_ROWS:
            self.assertIn("-s", program)
            self.assertTrue(sweep.oracle_reruns_slurped_program(self.probe(ISSUE_INPUT, on_stdin)), program)

    def test_the_shapes_the_limitations_page_names_are_skipped(self):
        # #7523 (a closer ends the text) and #7807 (a fault at the end of the text).
        for data in (b"[1]", b"[1][2]", b'{"a":1}', b"2[", b"[1,", b'1 2"', b"[[1]"):
            for on_stdin in (False, True):
                self.assertTrue(sweep.oracle_reruns_slurped_program(self.probe(data, on_stdin)), data)

    def test_a_slurp_both_releases_read_alike_is_compared(self):
        # A byte after the closer, a fault before the end of the text, and plain values.
        for data in (b"[1]\n", b"[1]]", b"1]", b"]", b"[[1]x", b"2", b"1 2\n", b"", ISSUE_INPUT + b"\n"):
            for on_stdin in (False, True):
                self.assertFalse(sweep.oracle_reruns_slurped_program(self.probe(data, on_stdin)), data)


class NanPayloadRuleTests(unittest.TestCase):
    """Rule B keeps `nan` and a digit apart, and changes nothing else."""

    def test_nan_then_a_digit_is_separated(self):
        self.assertTrue(sweep.has_nan_payload([b"nan1"]))
        self.assertEqual(sweep.separate_nan_payload([b"nan1"]), [b"nan 1"])
        self.assertEqual(sweep.separate_nan_payload([b"[nan12345678,-nan2.5]"]), [b"[nan 12345678,-nan 2.5]"])
        self.assertEqual(sweep.separate_nan_payload([b"nannan1nan1"]), [b"nannan 1nan 1"])

    def test_a_join_across_two_files_is_separated(self):
        # jq reads its files as one text, so `nan` ending one and `1` starting the next is `nan1`.
        self.assertTrue(sweep.has_nan_payload([b"[nan", b"1]"]))
        self.assertEqual(sweep.separate_nan_payload([b"[nan", b"1]"]), [b"[nan", b" 1]"])
        self.assertEqual(sweep.separate_nan_payload([b"nan1", b"2", b"nan", b"12345678"]), [b"nan 1", b"2", b"nan", b" 12345678"])

    def test_an_input_without_the_join_is_untouched(self):
        for datas in ([b"nan"], [b"nan,1"], [b"nan 1"], [b"nanx1"], [b"nan-1"], [b"1nan"], [b"nan\n", b"1"], [ISSUE_INPUT]):
            self.assertFalse(sweep.has_nan_payload(datas), datas)
            self.assertEqual(sweep.separate_nan_payload(datas), datas)

    def test_the_generator_still_spells_the_join(self):
        # The rule is a rewrite of what `generate` made, not a change to TOKENS: the tokens that
        # spell a payload are all still there, so `--no-adopted-rules` can compare them.
        for token in ("nan", "1", "2.5", "12345678"):
            self.assertIn(token, sweep.TOKENS)


class CeilingTests(unittest.TestCase):
    def test_a_rule_within_its_share_passes(self):
        self.assertEqual(sweep.over_ceiling({"slurp-rerun": 6, "nan-payload": 234}, 2600), [])
        self.assertEqual(sweep.over_ceiling({"slurp-rerun": 19, "nan-payload": 702}, 2600), [])

    def test_a_rule_over_its_share_is_named(self):
        self.assertEqual(sweep.over_ceiling({"slurp-rerun": 20, "nan-payload": 0}, 2600), ["slurp-rerun"])
        self.assertEqual(sweep.over_ceiling({"slurp-rerun": 0, "nan-payload": 703}, 2600), ["nan-payload"])
        self.assertEqual(sweep.over_ceiling({"slurp-rerun": 600, "nan-payload": 2600}, 2600), ["nan-payload", "slurp-rerun"])

    def test_a_short_run_allows_two_inputs_of_slurp_rows(self):
        self.assertEqual(sweep.over_ceiling({"slurp-rerun": 6, "nan-payload": 0}, 13), [])
        self.assertEqual(sweep.over_ceiling({"slurp-rerun": 7, "nan-payload": 0}, 13), ["slurp-rerun"])

    def test_the_ceilings_are_small(self):
        # Rule A is the one that skips rows: it may never be allowed a tenth of the `-s` rows.
        self.assertLess(sweep.ADOPTED_CEILINGS["slurp-rerun"], 0.1 * 3 / len(sweep.PROGRAMS))
        self.assertEqual(sum("-s" in program for program in sweep.PROGRAMS), 3)


class MainWithStandInBinariesTests(unittest.TestCase):
    """`main()` over two shell scripts: what a rule sets aside, and what it must not."""

    def setUp(self):
        self.scratch = tempfile.mkdtemp(prefix="jq-stream-errors-sweep-test-")
        self.addCleanup(shutil.rmtree, self.scratch, ignore_errors=True)

    def stand_in(self, name, answer, probe_runs=1):
        """A script that prints `0` `probe_runs` times for rule A's probe (a last-but-files argument
        of `0`) and `answer` for every other program."""
        path = os.path.join(self.scratch, name)
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(
                "#!/bin/sh\n"
                'for a in "$@"; do if [ "$a" = 0 ]; then ' + "echo 0; " * probe_runs + "exit 0; fi; done\n"
                f"echo {answer}\n"
            )
        os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)
        return path

    def sweep_main(self, subject, oracle, *flags):
        argv = ["sweep", subject, "--oracle", oracle, "--count", "10", *flags]
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            saved, sys.argv = sys.argv, argv
            try:
                status = sweep.main()
            finally:
                sys.argv = saved
        return status, out.getvalue()

    def test_equal_binaries_pass_and_report_what_the_rules_touched(self):
        status, out = self.sweep_main(self.stand_in("subject", "same"), self.stand_in("oracle", "same"))
        self.assertEqual(status, 0, out)
        self.assertIn("130 runs, 0 differences", out)
        self.assertIn("slurp-rerun skipped 0 rows", out)

    def test_a_genuine_difference_is_not_skipped(self):
        # The oracle runs every `-s` program once, so no row is set aside: all 130 differ and fail.
        status, out = self.sweep_main(self.stand_in("subject", "ours"), self.stand_in("oracle", "theirs"))
        self.assertEqual(status, 1, out)
        self.assertIn("130 runs, 130 differences", out)
        self.assertIn("slurp-rerun skipped 0 rows", out)

    def test_a_difference_in_a_row_whose_nan_was_separated_is_still_reported(self):
        # Rule B rewrites the input and skips nothing: with the rule on, as many rows differ as run.
        status, out = self.sweep_main(self.stand_in("subject", "ours"), self.stand_in("oracle", "theirs"), "--seed", "23")
        self.assertEqual(status, 1, out)
        self.assertIn("130 runs, 130 differences", out)
        # Seed 23 joins `nan` to a digit in two of its first ten inputs, 13 rows each.
        self.assertIn("nan-payload separated `nan` from a digit in the input of 26 rows", out)

    def test_a_rule_that_swallows_the_slurp_rows_fails_the_run(self):
        # An oracle that always runs a `-s` program twice: rule A skips all 30 `-s` rows of 130, far
        # past its ceiling, so the run fails though no compared row differs: exit 2, which
        # `dev compare jq` reads as an error that no ledger entry can list.
        status, out = self.sweep_main(self.stand_in("subject", "same"), self.stand_in("oracle", "same", probe_runs=2))
        self.assertEqual(status, 2, out)
        self.assertIn("130 runs, 0 differences", out)
        self.assertIn("slurp-rerun skipped 30 rows", out)
        self.assertIn("FAIL: over its ceiling share of the rows: slurp-rerun", out)

    def test_only_slurp_rows_are_ever_skipped(self):
        # The same oracle against a subject that differs: the 100 rows without `-s` are all reported.
        status, out = self.sweep_main(self.stand_in("subject", "ours"), self.stand_in("oracle", "theirs", probe_runs=2))
        self.assertEqual(status, 2, out)
        self.assertIn("130 runs, 100 differences", out)

    def test_no_adopted_rules_compares_every_row_and_classes_the_differences(self):
        status, out = self.sweep_main(
            self.stand_in("subject", "ours"), self.stand_in("oracle", "theirs", probe_runs=2), "--no-adopted-rules"
        )
        self.assertEqual(status, 1, out)
        self.assertIn("130 runs, 130 differences", out)
        self.assertIn("slurp-rerun flags 30 rows, 30 of them differ", out)


if __name__ == "__main__":
    unittest.main()
