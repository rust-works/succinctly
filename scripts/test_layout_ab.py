#!/usr/bin/env python3
"""Self-tests for scripts/layout-ab.py (#7960) that need no `succinctly` binary.

The "binaries" are small Python scripts in a temporary directory, so these check the harness's
parsing, its output and exit-status gate, its per-group environment and the arithmetic of its
report, and nothing about the tool it times. Run directly (`python3 scripts/test_layout_ab.py`);
CI runs it in the `perf-guard` job beside `test_ab_cli.py`.

Imported via `importlib`: the script's filename has a hyphen.
"""

import contextlib
import csv
import importlib.util
import io
import os
import pathlib
import stat
import sys
import tempfile
import unittest

_spec = importlib.util.spec_from_file_location("layout_ab", pathlib.Path(__file__).parent / "layout-ab.py")
layout_ab = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(layout_ab)

ROW = "row_x\t-c\t.\tusers\t10mb\n"


def write_bin(directory, name, body):
    path = os.path.join(directory, name)
    with open(path, "w") as f:
        f.write(f"#!{sys.executable}\n{body}\n")
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)
    return path


class Parsing(unittest.TestCase):
    def test_labels_are_a_group_and_an_optional_seed(self):
        self.assertEqual(layout_ab.parse_label("A"), ("A", None))
        self.assertEqual(layout_ab.parse_label("P0.s12"), ("P0", 12))
        self.assertEqual(layout_ab.parse_label("main-x.s0"), ("main-x", 0))
        # The spike split on the first `.s`, which read `A.shuf.s3` as group `A`, seed `huf.s3`.
        for bad in ("A.shuf.s3", "A.s", "A.sx", ".s3", "3A", "A B"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                layout_ab.parse_label(bad)

    def test_bins_keep_their_order_and_refuse_repeats_and_bad_specs(self):
        with tempfile.TemporaryDirectory() as d:
            x = write_bin(d, "x", "")
            bins = layout_ab.parse_bins([f"B={x}", f"A={x}", f"A.s1={x}"])
            self.assertEqual(list(bins), ["B", "A", "A.s1"])
            for bad in ([f"A={x}", f"A={x}"], ["A"], ["A="], [f"A.x={x}"], [f"A={d}/missing"], [f"A={d}"]):
                with self.subTest(bad=bad), self.assertRaises(ValueError):
                    layout_ab.parse_bins(bad)

    def test_a_relative_path_is_made_absolute_not_looked_up_on_path(self):
        # #7960's validation passed `B=cargo-succinctly` from the binary's directory, which
        # subprocess looked up on PATH and did not find.
        with tempfile.TemporaryDirectory() as d:
            write_bin(d, "cargo-succinctly", "")
            old = os.getcwd()
            os.chdir(d)
            try:
                path = layout_ab.parse_bins(["B=cargo-succinctly"])["B"]
            finally:
                os.chdir(old)
            self.assertTrue(os.path.isabs(path))
            self.assertTrue(os.path.samefile(path, os.path.join(d, "cargo-succinctly")))

    def test_an_environment_is_a_copy_of_the_base_per_group(self):
        base = {"PATH": "/bin"}
        envs = layout_ab.parse_envs(["P:K=1", "P0:K=0", "P:J=x"], ["M", "P", "P0"], base)
        self.assertEqual(envs["P"], {"PATH": "/bin", "K": "1", "J": "x"})
        self.assertEqual(envs["P0"], {"PATH": "/bin", "K": "0"})
        self.assertNotIn("M", envs)
        self.assertEqual(base, {"PATH": "/bin"})
        for bad in (["Q:K=1"], ["P"], ["P:K"], ["P:=1"]):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                layout_ab.parse_envs(bad, ["M", "P", "P0"], base)

    def test_pairs_default_to_every_group_against_the_first(self):
        self.assertEqual(layout_ab.parse_pairs([], ["A", "B", "C"]), [("B", "A"), ("C", "A")])
        self.assertEqual(layout_ab.parse_pairs(["P/P0"], ["M", "P", "P0"]), [("P", "P0")])
        for bad in (["P/Q"], ["P"], ["P/P"]):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                layout_ab.parse_pairs(bad, ["M", "P", "P0"])

    def test_a_switch_set_in_one_arm_or_at_unequal_length_is_warned_7961(self):
        self.assertEqual(layout_ab.switch_warnings(["P:K=1", "P0:K=0"], [("P", "P0")]), [])
        one_arm = layout_ab.switch_warnings(["P0:K=0"], [("P", "P0")])
        self.assertEqual(len(one_arm), 1)
        self.assertIn("set for P0 only", one_arm[0])
        uneven = layout_ab.switch_warnings(["P:K=on", "P0:K=0"], [("P", "P0")])
        self.assertIn("differs in length", uneven[0])
        # Only compared pairs count: M is no arm of P/P0.
        self.assertEqual(layout_ab.switch_warnings(["P:K=1", "P0:K=0"], [("P", "P0"), ("P", "M")])[0].count("only"), 1)

    def test_the_t_quantiles(self):
        self.assertAlmostEqual(layout_ab.t975(1), 12.706)
        self.assertAlmostEqual(layout_ab.t975(7), 2.365)
        self.assertAlmostEqual(layout_ab.t975(30), 2.042)
        self.assertAlmostEqual(layout_ab.t975(100), 1.96)
        with self.assertRaises(ValueError):
            layout_ab.t975(0)


class Summary(unittest.TestCase):
    def test_a_constant_ratio_is_reported_exactly_and_excludes_zero(self):
        times = {"A": [100, 100], "B": [110, 110]}
        for s, base in ((1, 100), (2, 120), (3, 90)):
            times[f"A.s{s}"] = [base, base]
            times[f"B.s{s}"] = [base * 1.05, base * 1.05]
        lines = layout_ab.summarize(times, [("B", "A")])
        self.assertIn("plain B/A (unseeded links): +10.00%", lines)
        paired = [line for line in lines if line.startswith("paired")][0]
        self.assertTrue(paired.startswith("paired B/A over 3 seeds: +5.00% +/- 0.00% (95% CI, excludes 0)"), paired)
        spread = [line for line in lines if line.startswith("layout spread A")][0]
        self.assertIn("3 seeds, range 29.03%", spread)

    def test_a_sign_that_flips_between_seeds_includes_zero(self):
        times = {}
        for s, d in ((1, 1.04), (2, 0.96), (3, 1.03), (4, 0.97)):
            times[f"A.s{s}"] = [100]
            times[f"B.s{s}"] = [100 * d]
        paired = [line for line in layout_ab.summarize(times, [("B", "A")]) if line.startswith("paired")][0]
        self.assertIn("includes 0", paired)
        self.assertIn("+0.00%", paired)

    def test_only_seeds_both_groups_have_are_paired(self):
        times = {"A.s1": [100], "A.s2": [100], "A.s3": [100], "B.s2": [101], "B.s3": [101], "B.s4": [500]}
        paired = [line for line in layout_ab.summarize(times, [("B", "A")]) if line.startswith("paired")][0]
        self.assertIn("over 2 seeds: +1.00%", paired)


class EndToEnd(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        d = self.dir.name
        self.tsv = os.path.join(d, "perf.tsv")
        with open(self.tsv, "w") as f:
            f.write("# id\tflags\tfilter\tshapes\ttier\n" + ROW)
        self.input = os.path.join(d, "in.json")
        with open(self.input, "w") as f:
            f.write("{}\n")
        self.ok = write_bin(d, "ok", 'import sys; assert sys.argv[1:] == ["jq", "-c", ".", sys.argv[-1]], sys.argv; print("same")')
        self.other = write_bin(d, "other", 'print("different")')
        self.fails = write_bin(d, "fails", "import sys; sys.exit(3)")
        self.env = write_bin(d, "env", 'import os; print(os.environ.get("SW", "unset"))')

    def main(self, *extra):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = layout_ab.main(["--row", "row_x", "--perf-tsv", self.tsv, "--input", self.input, "--reps", "3", *extra])
        return code, out.getvalue(), err.getvalue()

    def test_every_binary_runs_once_per_repetition_and_the_csv_has_every_run(self):
        csv_path = os.path.join(self.dir.name, "out.csv")
        bins = ["--bin", f"A={self.ok}", "--bin", f"B={self.ok}", "--bin", f"A.s1={self.ok}",
                "--bin", f"B.s1={self.ok}", "--bin", f"A.s2={self.ok}", "--bin", f"B.s2={self.ok}"]
        code, out, _ = self.main(*bins, "--csv", csv_path)
        self.assertEqual(code, 0)
        self.assertIn("6 binaries x 3 reps, output identical", out)
        self.assertIn("plain B/A", out)
        self.assertIn("paired B/A over 2 seeds", out)
        with open(csv_path) as f:
            rows = list(csv.DictReader(f))
        self.assertEqual(len(rows), 18)
        self.assertEqual({r["rep"] for r in rows}, {"0", "1", "2"})
        self.assertEqual({(r["label"], r["group"], r["seed"]) for r in rows if r["label"] == "B.s2"}, {("B.s2", "B", "2")})

    def test_differing_output_is_refused_unless_ungated(self):
        with self.assertRaises(SystemExit) as e:
            self.main("--bin", f"A={self.ok}", "--bin", f"B={self.other}")
        self.assertIn("output differs", str(e.exception.code))
        code, out, _ = self.main("--bin", f"A={self.ok}", "--bin", f"B={self.other}", "--no-gate")
        self.assertEqual(code, 0)
        self.assertIn("outputs differ (--no-gate)", out)

    def test_a_failing_binary_is_refused_not_timed(self):
        with self.assertRaises(SystemExit) as e:
            self.main("--bin", f"A={self.ok}", "--bin", f"B={self.fails}", "--no-gate")
        self.assertIn("B: exit 3", str(e.exception.code))

    def test_a_groups_environment_reaches_its_binaries_only(self):
        # P and P0 are one binary; only the switch differs, so their outputs differ by design.
        code, out, err = self.main("--bin", f"P={self.env}", "--bin", f"P0={self.env}",
                                   "--env", "P:SW=1", "--env", "P0:SW=0", "--no-gate", "--pair", "P/P0")
        self.assertEqual(code, 0)
        self.assertEqual(err, "")
        with self.assertRaises(SystemExit) as e:
            self.main("--bin", f"P={self.env}", "--bin", f"P0={self.env}", "--env", "P:SW=1", "--env", "P0:SW=0")
        self.assertIn("output differs", str(e.exception.code))
        _, _, err = self.main("--bin", f"P={self.env}", "--bin", f"P0={self.env}", "--env", "P0:SW=0", "--no-gate")
        self.assertIn("set for P0 only", err)

    def test_bad_arguments_are_usage_errors(self):
        for extra in (["--bin", f"A={self.ok}"],
                      ["--bin", f"A={self.ok}", "--bin", f"A.s1={self.ok}"],
                      ["--bin", f"A={self.ok}", "--bin", f"B={self.ok}", "--env", "C:K=1"],
                      ["--bin", f"A={self.ok}", "--bin", f"B={self.ok}", "--pair", "B/C"]):
            with self.subTest(extra=extra), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as e:
                    self.main(*extra)
                self.assertEqual(e.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
