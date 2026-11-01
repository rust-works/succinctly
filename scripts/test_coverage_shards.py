#!/usr/bin/env python3
"""Unit tests for scripts/coverage-shards.py and the way .github/workflows/coverage.yml uses it (#5711).

They pin the property the split rests on: a test runs in exactly one shard, so the combined lcov
covers what the single job's did. Nothing here builds or runs the suite; the one `cargo` call is
`cargo metadata`, which reads manifests. Run directly (`python3 scripts/test_coverage_shards.py`).
"""

import importlib.util
import json
import pathlib
import re
import shutil
import subprocess
import sys
import unittest

_SCRIPTS = pathlib.Path(__file__).resolve().parent
ROOT = _SCRIPTS.parent
WORKFLOWS = ROOT / ".github" / "workflows"

_spec = importlib.util.spec_from_file_location("coverage_shards", _SCRIPTS / "coverage-shards.py")
shards = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(shards)


def selected_tests(args):
    """The `--test NAME` targets of a shard's arguments, before any `--`."""
    head = args[: args.index("--")] if "--" in args else args
    return [head[i + 1] for i, a in enumerate(head) if a == "--test"]


class ShardTests(unittest.TestCase):
    def test_every_shard_has_arguments_and_an_unknown_one_is_refused(self):
        for shard in shards.SHARDS:
            with self.subTest(shard=shard):
                self.assertTrue(shards.shard_args(shard)[: len(shards.COMMON)] == shards.COMMON)
        with self.assertRaises(SystemExit):
            shards.shard_args("bogus")

    def test_every_test_target_runs_in_exactly_one_shard(self):
        seen = {}
        for shard in shards.SHARDS:
            for target in selected_tests(shards.shard_args(shard)):
                seen.setdefault(target, []).append(shard)
        every = set(shards.test_targets())
        # jq_cli_tests is split in two by name, so it is the one target two shards select.
        self.assertEqual(set(seen), every)
        for target, where in seen.items():
            expected = ["jq-cli", "jq-cli-heavy"] if target == shards.JQ_CLI else None
            with self.subTest(target=target):
                if expected:
                    self.assertEqual(sorted(where), sorted(expected))
                else:
                    self.assertEqual(len(where), 1, f"{target} runs in {where}")

    def test_rest_runs_the_lib_and_the_bins(self):
        args = shards.shard_args("rest")
        self.assertIn("--lib", args)
        self.assertIn("--bins", args)
        self.assertNotIn(shards.JQ_CLI, selected_tests(args))
        self.assertNotIn(shards.NESTING, selected_tests(args))

    def test_the_jq_cli_shards_split_the_target_by_name_without_overlap(self):
        skip = shards.shard_args("jq-cli")
        heavy = shards.shard_args("jq-cli-heavy")
        skipped = skip[skip.index("--") + 1 :]
        ran = heavy[heavy.index("--") + 1 :]
        self.assertEqual(skipped[0], "--exact")
        self.assertEqual(ran[0], "--exact")
        skipped_names = skipped[2::2]
        ran_names = ran[1:]
        self.assertEqual(len(ran_names), len(set(ran_names)))
        # What `jq-cli` skips is what `jq-cli-heavy` runs, plus the two stack tests, which are
        # `#[ignore]`d under coverage and so belong to neither run.
        self.assertEqual(skipped_names[2:], ran_names)
        self.assertEqual(len(skipped_names), len(ran_names) + 2)

    @unittest.skipUnless(shutil.which("cargo"), "needs cargo")
    def test_the_glob_finds_the_test_targets_cargo_does(self):
        done = subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
        metadata = json.loads(done.stdout)
        cargo_targets = sorted(
            t["name"]
            for p in metadata["packages"]
            for t in p["targets"]
            if "test" in t["kind"]
        )
        self.assertEqual(shards.test_targets(), cargo_targets)
        for p in metadata["packages"]:
            for t in p["targets"]:
                if "test" in t["kind"]:
                    self.assertFalse(t.get("required-features"), t["name"])

    def test_the_cli_prints_what_the_functions_return(self):
        def run(*argv):
            return subprocess.run(
                [sys.executable, str(_SCRIPTS / "coverage-shards.py"), *argv],
                capture_output=True,
                text=True,
                check=False,
            )

        self.assertEqual(run("shards").stdout.split(), list(shards.SHARDS))
        self.assertEqual(run("args", "jq-nesting").stdout.split(), shards.shard_args("jq-nesting"))
        self.assertNotEqual(run("args", "bogus").returncode, 0)
        self.assertEqual(run().returncode, 2)


class WorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.coverage = (WORKFLOWS / "coverage.yml").read_text(encoding="utf-8")

    def test_the_matrix_lists_the_shards_the_script_defines(self):
        m = re.search(r"^\s*shard: \[([^\]]*)\]", self.coverage, re.M)
        self.assertIsNotNone(m, "coverage.yml has no `shard: [...]` matrix")
        self.assertEqual([s.strip() for s in m.group(1).split(",")], list(shards.SHARDS))

    def test_the_report_job_reads_the_files_the_shards_write(self):
        # The shard writes `shard-<shard>.lcov` into an artifact named per architecture and shard;
        # `report` downloads that architecture's artifacts and globs the files.
        self.assertIn('--output-path "shard-$SHARD.lcov"', self.coverage)
        self.assertIn("name: coverage-shard-${{ inputs.name }}-${{ matrix.shard }}", self.coverage)
        self.assertIn("pattern: coverage-shard-${{ inputs.name }}-*", self.coverage)
        self.assertIn("shard-reports: shards/shard-*.lcov", self.coverage)

    def test_the_shard_runs_the_script_for_its_matrix_entry(self):
        self.assertIn('python3 scripts/coverage-shards.py run "$SHARD"', self.coverage)
        self.assertIn("SHARD: ${{ matrix.shard }}", self.coverage)

    def test_both_callers_use_the_one_definition_per_architecture(self):
        for name in ("ci.yml", "coverage-baseline.yml"):
            text = (WORKFLOWS / name).read_text(encoding="utf-8")
            with self.subTest(workflow=name):
                self.assertIn("uses: ./.github/workflows/coverage.yml", text)
                self.assertIn("name: ${{ matrix.name }}", text)
                self.assertIn("runner: ${{ matrix.runner }}", text)
                self.assertIn("ubuntu-24.04-arm", text)

    def test_the_old_composite_action_is_gone(self):
        self.assertFalse((ROOT / ".github" / "actions" / "coverage").exists())
        for name in ("ci.yml", "coverage-baseline.yml", "coverage.yml"):
            text = (WORKFLOWS / name).read_text(encoding="utf-8")
            self.assertNotIn("uses: ./.github/actions/coverage", text, name)


if __name__ == "__main__":
    unittest.main()
