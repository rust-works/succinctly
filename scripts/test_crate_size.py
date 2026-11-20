#!/usr/bin/env python3
"""Tests for scripts/crate_size.py: the verdict, the cargo command, the retry and the wiring.

No `cargo package`: a fake `run` stands in. Run directly (`python3 scripts/test_crate_size.py`).
"""

import os
import re
import subprocess
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import crate_size as c  # noqa: E402

WORKFLOWS = os.path.join(c.ROOT, ".github", "workflows")
COMMAND = "python3 scripts/crate_size.py check --verify --features cli"


def read(*parts):
    with open(os.path.join(*parts), encoding="utf-8") as f:
        return f.read()


class CheckTests(unittest.TestCase):
    def test_the_ceiling_is_inclusive(self):
        self.assertEqual(c.check(c.CEILING)[0], 0)
        self.assertEqual(c.check(c.CEILING + 1)[0], 1)

    def test_a_crate_under_the_ceiling_prints_both_margins(self):
        code, lines = c.check(6_504_486)
        self.assertEqual(code, 0)
        self.assertEqual(len(lines), 1)
        self.assertNotIn("::", lines[0])
        self.assertIn(f"+{c.CEILING - 6_504_486} bytes to this check's ceiling", lines[0])
        self.assertIn(f"+{c.LIMIT - 6_504_486} to crates.io's limit", lines[0])

    def test_the_v0_8_0_crate_with_tests_and_benches_would_have_failed(self):
        # `cargo package --no-verify` at fbf8c4acf, before `tests/` and `benches/` left `include`.
        code, lines = c.check(10_434_790)
        self.assertEqual(code, 1)
        self.assertTrue(lines[0].startswith("::error::"), lines[0])
        self.assertIn("include", lines[2])
        self.assertIn("CEILING", lines[2])

    def test_the_ceiling_leaves_at_least_a_mebibyte_before_the_limit(self):
        self.assertEqual(c.LIMIT, 10_485_760)
        self.assertLessEqual(c.CEILING, c.LIMIT - 1024 * 1024)


class PackageTests(unittest.TestCase):
    def test_package_id_reads_the_real_manifest(self):
        name, version = c.package_id(os.path.join(c.ROOT, "Cargo.toml"))
        self.assertEqual(name, "succinctly")
        self.assertRegex(version, r"^\d+\.\d+\.\d+")

    def test_the_command_verifies_with_the_features_or_skips_verification(self):
        verify = c.package_command("t", True, "cli", False)
        self.assertEqual(verify[:2], ["cargo", "package"])
        self.assertIn("--features", verify)
        self.assertEqual(verify[verify.index("--features") + 1], "cli")
        self.assertNotIn("--no-verify", verify)
        self.assertEqual(verify[verify.index("--target-dir") + 1], "t")
        plain = c.package_command("t", False, "cli", True)
        self.assertIn("--no-verify", plain)
        self.assertNotIn("--features", plain)
        self.assertIn("--offline", plain)

    def test_the_target_dir_follows_cargo_target_dir(self):
        self.assertEqual(c.target_dir("/r", {}), os.path.join("/r", "target", "crate-size"))
        self.assertEqual(c.target_dir("/r", {"CARGO_TARGET_DIR": "/t"}), os.path.join("/t", "crate-size"))

    def fake(self, *results):
        calls = []

        def run(cmd, **kwargs):
            calls.append(cmd)
            code, stderr = results[len(calls) - 1]
            return subprocess.CompletedProcess(cmd, code, "", stderr)

        return calls, run

    def test_an_offline_miss_is_retried_online(self):
        calls, run = self.fake((101, "error: ... you're using offline mode (--offline)"), (0, ""))
        crate = c.package(c.ROOT, False, "cli", run=run)
        self.assertEqual(len(calls), 2)
        self.assertIn("--offline", calls[0])
        self.assertNotIn("--offline", calls[1])
        self.assertTrue(crate.endswith(os.path.join("crate-size", "package", "succinctly-" + c.package_id(os.path.join(c.ROOT, "Cargo.toml"))[1] + ".crate")), crate)

    def test_a_build_failure_is_not_retried(self):
        calls, run = self.fake((101, "error[E0425]: cannot find value"))
        with self.assertRaises(RuntimeError):
            c.package(c.ROOT, True, "cli", run=run)
        self.assertEqual(len(calls), 1)


class WiringTests(unittest.TestCase):
    def include_list(self):
        manifest = read(c.ROOT, "Cargo.toml")
        block = re.search(r"^include = \[\n(.*?)^\]", manifest, re.M | re.S)
        self.assertIsNotNone(block)
        return re.findall(r'"([^"]+)"', block.group(1))

    def test_the_crate_ships_neither_tests_nor_benches(self):
        include = self.include_list()
        self.assertIn("/src/**/*", include)
        for pattern in include:
            with self.subTest(pattern=pattern):
                # Anchored: a bare `README.md` also matched every `docs/**/README.md`.
                self.assertTrue(pattern.startswith("/"), pattern)
                self.assertFalse(re.match(r"/?(tests|benches)\b", pattern), pattern)

    def test_the_checked_features_build_the_binary(self):
        # `[[bin]]` has `required-features = ["cli"]`: a packaged build without `cli` compiles only
        # the library, and most of the `include_str!` sites are under `src/bin/succinctly/`.
        manifest = read(c.ROOT, "Cargo.toml")
        binary = re.search(r'^\[\[bin\]\]\nname = "succinctly"\n(?:.*\n)*?required-features = \[([^\]]*)\]', manifest, re.M)
        self.assertIsNotNone(binary)
        required = set(re.findall(r'"([^"]+)"', binary.group(1)))
        self.assertEqual(required, {"cli"})
        self.assertTrue(required <= set(COMMAND.split("--features ")[1].split(",")))
        self.assertTrue(required <= set(c.DEFAULT_FEATURES.split(",")))

    def test_ci_runs_the_verified_check_on_every_code_change(self):
        ci = read(WORKFLOWS, "ci.yml")
        self.assertIn(f"run: {COMMAND}\n", ci)

    def test_release_checks_the_crate_before_it_creates_the_release(self):
        release = read(WORKFLOWS, "release.yml")
        self.assertIn(f"run: {COMMAND}\n", release)
        create = re.search(r"\n  create-release:\n(?:.*\n)*?(?=  [A-Za-z0-9_-]+:\n)", release)
        self.assertIsNotNone(create)
        self.assertRegex(create.group(0), r"\n    needs: package-check\n")


if __name__ == "__main__":
    unittest.main()
