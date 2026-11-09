#!/usr/bin/env python3
"""Every `#![cfg(feature = "cli")]` test file in tests/ runs in a required `Test` leg (#6741).

`default = ["std"]`, so a plain `cargo test` compiles a file with that gate to nothing, and the
`cli-gated` legs name their test binaries by hand (`--test a --test b ...`, once per platform).
A new file nobody added to those lists ran only under `Coverage`, which is not a required check
and does not run in the merge queue: #6679's roster test failed on #6673's new row (#6688) and
the queue merged both, because no required leg ran it.

Three things are pinned, for each of the three platform matrices of ci.yml:

- every `cli`-gated file in tests/ is named by a `--test` in some leg of the matrix;
- the `cli-gated` leg names the same files on all three platforms (a list edited on one
  platform only fails here, not silently);
- the check itself reports a missing file, shown against a throwaway tests/ directory.

It is run by the `deep-recursion` legs (ci.yml), which are required checks on every platform,
and, like every scripts/test_*.py, by scripts/pre-push.sh. Run it directly
(`python3 scripts/test_cli_gated_tests.py`) or via `python3 -m unittest`.
"""

import pathlib
import re
import sys
import tempfile
import unittest

_SCRIPTS = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(_SCRIPTS))

import test_deep_recursion_tests as deep  # noqa: E402  (shares the ci.yml leg parser)

TESTS_DIR = _SCRIPTS.parent / "tests"

NAMED_TEST = re.compile(r"--test ([A-Za-z0-9_]+)\b")
INNER_CFG = re.compile(r"^\s*#!\[cfg\(", re.M)


def _predicate(text, at):
    """Parse one cfg predicate of `text` from `at`: (does it require `cli`, index after it)."""
    match = re.compile(r'\s*(all|any|not)\s*\(|\s*([A-Za-z_][A-Za-z0-9_]*)\s*(?:=\s*"([^"]*)")?').match(
        text, at
    )
    if match is None:
        raise ValueError(f"cannot read a cfg predicate at {text[at:at + 40]!r}")
    operator, name, value = match.groups()
    if operator is None:
        return (name == "feature" and value == "cli"), match.end()
    parts, at = [], match.end()
    while True:
        trailing = re.compile(r"\s*\)").match(text, at)  # `all(a, b,)`: rustfmt's wrapped form
        if trailing is not None and parts:
            at = trailing.end()
            break
        part, at = _predicate(text, at)
        parts.append(part)
        closing = re.compile(r"\s*([,)])").match(text, at)
        if closing is None:
            raise ValueError(f"unbalanced cfg near {text[at:at + 40]!r}")
        at = closing.end()
        if closing.group(1) == ")":
            break
    if operator == "all":
        return any(parts), at  # all(...) holds only where one of its terms does
    if operator == "any":
        return all(parts), at  # any(...) can hold without `cli` unless every term needs it
    return False, at  # not(...): `cli` off satisfies it


def requires_cli(source):
    """Whether a file-level `#![cfg(...)]` of `source` compiles the file away without `cli`.

    Parsed rather than matched, so an attribute rustfmt wrapped over several lines, an
    indented one, and `all`/`any`/`not` nesting are read as the compiler reads them. Several
    inner `cfg` attributes all apply, so one that requires `cli` is enough.
    """
    for found in INNER_CFG.finditer(source):
        needs, _ = _predicate(source, found.end())
        if needs:
            return True
    return False


def cli_gated_files(tests_dir=TESTS_DIR):
    """Names of the integration test targets under `tests_dir` that need the `cli` feature.

    A target is `tests/<name>.rs` or `tests/<name>/main.rs`.
    """
    root = pathlib.Path(tests_dir)
    targets = {path.stem: path for path in root.glob("*.rs")}
    targets.update({path.parent.name: path for path in root.glob("*/main.rs")})
    return sorted(name for name, path in targets.items() if requires_cli(path.read_text()))


def named_tests(leg_text):
    """The `--test <name>` binaries one leg's commands run."""
    return set(NAMED_TEST.findall(leg_text))


def missing_from(job_legs, gated):
    """The `cli`-gated files no leg of `job_legs` (a `matrix_legs` result) runs."""
    named = set()
    for leg in job_legs.values():
        named |= named_tests(leg)
    return sorted(set(gated) - named)


class CliGatedFilesRunTests(unittest.TestCase):
    def test_the_gate_is_found_where_it_is_written(self):
        # The roster this checks is empty if the gate regex stops matching: pin a known member.
        files = cli_gated_files()
        self.assertIn("jq_cli_tests", files)
        self.assertIn("jq_extension_roster_tests", files)
        # A bench-runner gate is another leg's business (orchestrate_cli_tests, #1705).
        self.assertNotIn("orchestrate_cli_tests", files)

    def test_every_cli_gated_file_runs_in_a_leg_on_every_platform(self):
        gated = cli_gated_files()
        for job in deep.MATRIX_JOBS:
            with self.subTest(job=job):
                self.assertEqual(
                    missing_from(deep.matrix_legs(job), gated),
                    [],
                    f"a `cli`-gated test file runs in no leg of {job}: add `--test <name>` to its "
                    "`cli-gated` command in .github/workflows/ci.yml (#6741)",
                )

    def test_the_cli_gated_leg_names_the_same_files_on_every_platform(self):
        lists = {
            job: named_tests(deep.matrix_legs(job)["cli-gated"]) for job in deep.MATRIX_JOBS
        }
        first = deep.MATRIX_JOBS[0]
        for job in deep.MATRIX_JOBS[1:]:
            with self.subTest(job=job):
                self.assertEqual(
                    sorted(lists[job] ^ lists[first]),
                    [],
                    f"`cli-gated` names different test files in {job} than in {first}",
                )

    def test_the_deep_recursion_legs_run_this_check(self):
        for job in deep.MATRIX_JOBS:
            with self.subTest(job=job):
                legs = deep.matrix_legs(job)
                self.assertIn("deep-recursion", legs)
                self.assertEqual(
                    legs["deep-recursion"].count("python3 scripts/test_cli_gated_tests.py"), 1
                )


class CheckCatchesAMissingFileTests(unittest.TestCase):
    """The negative test: a new gated file with no leg fails the check."""

    def write(self, directory, name, text):
        (pathlib.Path(directory) / f"{name}.rs").write_text(text)

    def test_a_new_cli_gated_file_with_no_leg_is_reported(self):
        with tempfile.TemporaryDirectory() as tests:
            self.write(tests, "old_cli_tests", '#![cfg(feature = "cli")]\nfn main() {}\n')
            self.write(tests, "new_cli_tests", '#![cfg(feature = "cli")]\nfn main() {}\n')
            legs = {"cli-gated": "cargo test --features cli --test old_cli_tests -- $skip"}
            self.assertEqual(missing_from(legs, cli_gated_files(tests)), ["new_cli_tests"])
            # Naming it in any leg satisfies the check, so a test can move to another leg.
            legs["other"] = "cargo test --features cli --test new_cli_tests"
            self.assertEqual(missing_from(legs, cli_gated_files(tests)), [])

    def test_only_a_file_that_needs_the_cli_feature_counts(self):
        with tempfile.TemporaryDirectory() as tests:
            self.write(tests, "plain", "fn main() {}\n")
            self.write(tests, "bench", '#![cfg(feature = "bench-runner")]\n')
            self.write(tests, "either", '#![cfg(any(feature = "cli", feature = "serde"))]\n')
            self.write(tests, "both_any", '#![cfg(any(feature = "cli", all(feature = "cli", unix)))]\n')
            self.write(tests, "negated", '#![cfg(not(feature = "cli"))]\n')
            self.write(tests, "negated_nested", '#![cfg(all(unix, not(all(feature = "cli", unix))))]\n')
            self.write(tests, "item_only", '#[cfg(feature = "cli")]\nfn f() {}\n')
            self.write(tests, "both", '#![cfg(all(feature = "cli", unix))]\n')
            self.write(
                tests, "wrapped", '#![cfg(all(\n    unix,\n    feature = "cli",\n))]\n'
            )
            self.write(tests, "indented", '  #![cfg(feature = "cli")]\n')
            self.write(tests, "second_attr", '#![cfg(unix)]\n#![cfg(feature = "cli")]\n')
            self.assertEqual(
                cli_gated_files(tests),
                ["both", "both_any", "indented", "second_attr", "wrapped"],
            )

    def test_a_test_target_in_a_directory_is_found(self):
        with tempfile.TemporaryDirectory() as tests:
            target = pathlib.Path(tests) / "dir_target"
            target.mkdir()
            (target / "main.rs").write_text('#![cfg(feature = "cli")]\n')
            (target / "helper.rs").write_text('#![cfg(feature = "cli")]\n')
            self.assertEqual(cli_gated_files(tests), ["dir_target"])


if __name__ == "__main__":
    unittest.main()
