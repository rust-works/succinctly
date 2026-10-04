#!/usr/bin/env python3
"""Unit tests for scripts/deep-recursion-tests.sh that need no cargo build: a fake `cargo`,
a small shell script first on PATH, prints canned `--list` and run output, so these check
the script's own guards and nothing about the tests it names (and can never launch the
real suite). Run directly (`python3 scripts/test_deep_recursion_tests.py`) or via
`python3 -m unittest scripts/test_deep_recursion_tests.py`.

The guards matter because libtest exits 0 with `running 0 tests` when `--exact` matches
nothing (#3698): without them a renamed test would leave the `deep-recursion` leg green
and the test running nowhere. `CiWiringTests` pins the other half: that on every platform
the tests the `cli-gated` leg skips are the ones a `deep-recursion` leg runs.

It is run by the `deep-recursion` legs themselves (ci.yml), so it sits on the required
path: weakening a guard fails `Test (...)`, not just a side job.
"""

import os
import pathlib
import re
import stat
import subprocess
import tempfile
import unittest

_SCRIPTS = pathlib.Path(__file__).resolve().parent
SCRIPT = _SCRIPTS / "deep-recursion-tests.sh"
CI_YML = _SCRIPTS.parent / ".github" / "workflows" / "ci.yml"
MATRIX_JOBS = ("test-x86-matrix", "test-arm-matrix", "test-macos-arm-matrix")

# Records each invocation, then answers `--list` or a run from the environment.
FAKE_CARGO = """#!/bin/sh
printf '%s\\n' "$*" >> "$FAKE_LOG"
case " $* " in
*" --list "*) printf '%s\\n' "$FAKE_LISTING"; exit 0 ;;
esac
echo "running tests"
printf '%s\\n' "$FAKE_RESULT"
exit "${FAKE_EXIT:-0}"
"""

PASSED_2 = "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 2321 filtered out; finished in 101.00s"


def skip_arg_tokens():
    done = subprocess.run(
        [str(SCRIPT), "skip-args"], capture_output=True, text=True, check=True
    )
    return done.stdout.split()


def listed_names():
    tokens = skip_arg_tokens()
    assert tokens[0] == "--exact", tokens
    return tokens[2::2]


class SkipArgsTests(unittest.TestCase):
    def test_is_exact_and_skips_every_name_once(self):
        tokens = skip_arg_tokens()
        names = listed_names()
        self.assertGreater(len(names), 0)
        self.assertEqual(tokens, ["--exact"] + [t for n in names for t in ("--skip", n)])
        self.assertEqual(len(names), len(set(names)))

    def test_unknown_or_missing_subcommand_is_a_usage_error(self):
        for args in ([], ["bogus"]):
            with self.subTest(args=args):
                done = subprocess.run(
                    [str(SCRIPT), *args], capture_output=True, text=True
                )
                self.assertEqual(done.returncode, 2)


class RunTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.dir = pathlib.Path(self._tmp.name)
        self.cargo = self.dir / "cargo"
        self.cargo.write_text(FAKE_CARGO)
        self.cargo.chmod(self.cargo.stat().st_mode | stat.S_IXUSR)
        self.log = self.dir / "calls"
        self.names = listed_names()

    def run_script(self, listing=None, result=PASSED_2, exit_code=0):
        """Returns (CompletedProcess, [one string per fake-cargo invocation])."""
        if listing is None:
            listing = [f"{n}: test" for n in ["unrelated_test", *self.names]]
        env = dict(
            os.environ,
            PATH=f"{self.dir}{os.pathsep}{os.environ['PATH']}",
            FAKE_LOG=str(self.log),
            FAKE_LISTING="\n".join(listing),
            FAKE_RESULT=result,
            FAKE_EXIT=str(exit_code),
        )
        done = subprocess.run(
            [str(SCRIPT), "run"], capture_output=True, text=True, env=env
        )
        calls = self.log.read_text().splitlines() if self.log.exists() else []
        return done, calls

    def test_passes_and_runs_exactly_the_names_skip_args_skips(self):
        done, calls = self.run_script()
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertEqual(len(calls), 2, calls)
        self.assertIn("--list", calls[0])
        self.assertEqual(
            calls[1],
            "test --features cli --test jq_cli_tests -- --color never --exact "
            + " ".join(self.names),
        )

    def test_a_listed_name_that_is_not_a_test_fails_before_running_anything(self):
        gone = self.names[-1]
        listing = [f"{n}: test" for n in self.names[:-1]]
        done, calls = self.run_script(listing=listing)
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        self.assertIn(gone, done.stdout)
        self.assertIn("::error::", done.stdout)
        self.assertEqual(len(calls), 1, calls)  # the --list probe only

    def test_a_name_is_matched_whole_not_as_a_substring(self):
        # A test moved into a module is listed as `module::name`, which `--exact name`
        # no longer matches (running zero tests, exit 0); it must not read as present.
        listing = [f"some_mod::{n}: test" for n in self.names]
        done, calls = self.run_script(listing=listing)
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        self.assertIn("::error::", done.stdout)
        self.assertEqual(len(calls), 1, calls)

    def test_a_listed_name_that_is_not_a_test_entry_fails(self):
        # `--list` also prints `name: benchmark`; only `: test` entries count.
        listing = [f"{n}: benchmark" for n in self.names]
        done, calls = self.run_script(listing=listing)
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        self.assertEqual(len(calls), 1, calls)

    def test_fewer_passes_than_names_fails(self):
        result = "test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 2321 filtered out; finished in 1.00s"
        done, _ = self.run_script(result=result)
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        self.assertIn("::error::expected 2 deep-recursion tests to pass", done.stdout)
        self.assertIn("reported 1", done.stdout)

    def test_zero_tests_run_fails(self):
        # What libtest prints, and exits 0 on, for an `--exact` filter matching nothing.
        result = "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2323 filtered out; finished in 0.00s"
        done, _ = self.run_script(result=result)
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        self.assertIn("::error::expected 2 deep-recursion tests to pass", done.stdout)
        self.assertIn("reported 0", done.stdout)

    def test_no_result_line_fails(self):
        done, _ = self.run_script(result="")
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        self.assertIn("reported none", done.stdout)

    def test_a_failing_test_fails_the_leg_with_cargo_s_status(self):
        # Failing through the pipeline, not the count check: no `::error::` line.
        result = "test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 2321 filtered out; finished in 9.00s"
        done, _ = self.run_script(result=result, exit_code=101)
        self.assertEqual(done.returncode, 101, done.stdout + done.stderr)
        self.assertNotIn("::error::", done.stdout)

    def test_cargo_failing_on_a_clean_looking_result_still_fails_the_leg(self):
        done, _ = self.run_script(result=PASSED_2, exit_code=101)
        self.assertEqual(done.returncode, 101, done.stdout + done.stderr)
        self.assertNotIn("::error::", done.stdout)


def matrix_legs(job):
    """{leg name: its non-comment text} for one matrix job of ci.yml.

    Plain text, not YAML: PyYAML is not guaranteed on every runner. The job and leg
    indentation are the file's own, so a restructure fails here loudly rather than
    passing vacuously.
    """
    text = CI_YML.read_text()
    found = re.search(
        rf"^  {re.escape(job)}:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)", text, re.M | re.S
    )
    if not found:
        raise AssertionError(f"{job} is not a job in {CI_YML}")
    body = "\n".join(
        line for line in found.group(1).split("\n") if not line.lstrip().startswith("#")
    )
    parts = re.split(r"^          - name: ", body, flags=re.M)[1:]
    return {part.split("\n", 1)[0].strip(): part for part in parts}


class CiWiringTests(unittest.TestCase):
    """The names are single-sourced; this pins the platforms (#3698)."""

    def test_every_platform_skips_in_cli_gated_and_runs_in_deep_recursion(self):
        for job in MATRIX_JOBS:
            with self.subTest(job=job):
                legs = matrix_legs(job)
                self.assertIn("cli-gated", legs)
                self.assertIn("deep-recursion", legs)

                gated = legs["cli-gated"]
                self.assertEqual(
                    gated.count("skip=$(scripts/deep-recursion-tests.sh skip-args)"), 1
                )
                # The skip is only worth anything on the line that runs jq_cli_tests.
                commands = [
                    line.strip()
                    for line in gated.split("\n")
                    if line.strip().startswith("cargo test ")
                ]
                self.assertEqual(len(commands), 1, commands)
                self.assertIn(" --test jq_cli_tests ", commands[0])
                self.assertTrue(commands[0].endswith(" -- $skip"), commands[0])

                deep = legs["deep-recursion"]
                self.assertEqual(deep.count("scripts/deep-recursion-tests.sh run"), 1)
                # The guard's own self-test rides in the same (required) leg.
                self.assertEqual(
                    deep.count("python3 scripts/test_deep_recursion_tests.py"), 1
                )
                self.assertIn("buildCli: true", deep)

    def test_no_other_leg_runs_jq_cli_tests_with_the_two_tests_twice(self):
        # A third place running `--test jq_cli_tests` unskipped would run them twice.
        for job in MATRIX_JOBS:
            with self.subTest(job=job):
                for name, leg in matrix_legs(job).items():
                    if name != "cli-gated":
                        self.assertNotIn("--test jq_cli_tests", leg, name)


if __name__ == "__main__":
    unittest.main()
