#!/usr/bin/env python3
"""Unit tests for scripts/jq-regex-corpus.py that need no build and no jq (#7965).

- The default corpus is pinned by digest: it is the gate of ADR-0031's migration, and stored answers
  are only comparable while its rows stay the same.
- The batched driver records a row that ends its process and resumes after it.
- The standalone control's program and its agreement rule hold.
- The analysis sorts rows into the families and transitions its docstring names.

Run directly or via `python3 -m unittest scripts/test_jq_regex_corpus.py`.
"""

import hashlib
import importlib.util
import json
import os
import pathlib
import stat
import sys
import tempfile
import unittest

_PATH = pathlib.Path(__file__).parent / "jq-regex-corpus.py"
_spec = importlib.util.spec_from_file_location("jq_regex_corpus", _PATH)
corpus = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(corpus)

# sha256 of MANIFEST followed by every chunk, for `gen` with its defaults. The corpus the ADR-0031
# measurements were taken on (459,678 rows, 330,133 patterns) has this digest.
DEFAULT_CORPUS_SHA256 = "7fefc97e9f823f477434ba03d69dcf2d450c414c167d40ec073087b9e6ba28f1"


def line(rid, *answers):
    return (json.dumps([rid] + list(answers), separators=(",", ":")) + "\n").encode()


class GeneratorTests(unittest.TestCase):
    def test_the_default_corpus_is_the_one_the_measurements_used(self):
        with tempfile.TemporaryDirectory() as d:
            with open(os.devnull, "w") as null:
                saved, sys.stdout = sys.stdout, null
                try:
                    self.assertEqual(corpus.cmd_gen([d], {}), 0)
                finally:
                    sys.stdout = saved
            h = hashlib.sha256()
            manifest = pathlib.Path(d, "MANIFEST").read_bytes()
            names = [l.split()[0] for l in manifest.decode().splitlines()]
            h.update(manifest)
            for n in names:
                h.update(pathlib.Path(d, n).read_bytes())
            self.assertEqual(len(names), 92)
            self.assertEqual(h.hexdigest(), DEFAULT_CORPUS_SHA256,
                             "the corpus changed: every stored answer has to be regenerated with it")

    def test_a_seed_gives_the_same_rows_and_another_seed_does_not(self):
        a, _ = corpus.generate(500, 7)
        b, _ = corpus.generate(500, 7)
        c, _ = corpus.generate(500, 8)
        self.assertEqual(a, b)
        self.assertNotEqual(a, c)

    def test_rows_have_the_documented_shape(self):
        rows, npatterns = corpus.generate(300, 1)
        self.assertEqual([r[0] for r in rows], list(range(len(rows))))
        self.assertGreater(npatterns, len(corpus.PROBES))
        for rid, gen, op, pat, fl, subjects in rows:
            self.assertIn(op, corpus.OPEXPR)
            self.assertIsInstance(pat, str)
            self.assertLessEqual(len(pat), 200)
            self.assertTrue(fl is None or isinstance(fl, str))
            self.assertEqual(len(subjects), 3)
        self.assertEqual({r[1] for r in rows if r[0] < len(corpus.PROBES) * 32}, {"probe"})


FAKE_JQ = r'''#!{python}
# Stands in for jq: answers every row of the input file, and dies on row {crash} without a newline.
import json, sys
for raw in open(sys.argv[-1]):
    rid = json.loads(raw)[0]
    if rid == {crash}:
        sys.stdout.write("[%d" % rid)
        sys.stdout.flush()
        sys.stderr.write("boom\n")
        sys.exit(3)
    sys.stdout.write(json.dumps([rid, ["ok", rid], ["ok", rid], ["ok", rid]]) + "\n")
    sys.stdout.flush()
'''


class DriverTests(unittest.TestCase):
    def test_a_row_that_ends_the_process_is_recorded_and_the_batch_resumes(self):
        with tempfile.TemporaryDirectory() as d:
            fake = d + "/fake-jq"
            with open(fake, "w") as f:
                f.write(FAKE_JQ.format(python=sys.executable, crash=2))
            os.chmod(fake, os.stat(fake).st_mode | stat.S_IEXEC)
            rows = [(json.dumps([i, "g", "test", "a", None, ["", "", ""]]) + "\n").encode() for i in range(5)]
            out = [json.loads(l) for l in corpus.run_batch(fake, rows, d)]
        self.assertEqual([r[0] for r in out], [0, 1, 2, 3, 4])
        self.assertEqual(out[2][1], "CRASH")
        self.assertIn("rc=3 boom", out[2][2])
        self.assertEqual(out[3], [3, ["ok", 3], ["ok", 3], ["ok", 3]])

    def test_the_standalone_program_holds_the_pattern_and_flags_as_literals(self):
        row = [9, "g", "sub", "a\"b", "gx", ["s\n", "", ""]]
        self.assertEqual(corpus.standalone_program(row), '"s\\n" | sub("a\\"b"; "<\\(tojson)>"; "gx")')
        row = [9, "g", "split", "a", None, ["s", "", ""]]
        self.assertEqual(corpus.standalone_program(row), '"s" | split("a"; null)')
        row = [9, "g", "matchg", "a", None, ["s", "", ""]]
        self.assertEqual(corpus.standalone_program(row), '"s" | [match("a"; "g")]')

    def test_a_standalone_run_agrees_only_with_the_same_value_or_error(self):
        agrees = corpus.standalone_agrees
        self.assertTrue(agrees(["ok", [1]], "[1]\n", "", 0))
        self.assertFalse(agrees(["ok", [1]], "[2]\n", "", 0))
        self.assertFalse(agrees(["ok", [1]], "[1]\n", "warning\n", 0))
        msg = "Regex failure: x"
        self.assertTrue(agrees(["err", msg], "", "jq: error (at <unknown>): " + msg + "\n", 5))
        self.assertFalse(agrees(["err", msg], "", "jq: error (at <unknown>): " + msg + "\n", 1))
        self.assertFalse(agrees(["err", {"a": 1}], "", "", 5))


class AnalysisTests(unittest.TestCase):
    ROW = [0, "classes", "test", "[a", "x", ["a", "b", "c"]]

    def fam(self, jq, here, row=None):
        return corpus.family(row or self.ROW, [0] + jq, [0] + here)

    def test_families_name_who_refused(self):
        ok, err = ["ok", True], ["err", "Regex failure: premature end of char-class"]
        self.assertEqual(self.fam([ok, ok, ok], [err, ok, ok]), ("refused, jq answers", "here: premature end of char-class"))
        self.assertEqual(self.fam([err, ok, ok], [ok, ok, ok]), ("answered, jq refuses", "jq: premature end of char-class"))
        self.assertEqual(self.fam([ok, ok, ok], [ok, ["ok", False], ok]), ("different answer", "extended mode"))
        other = ["err", "Regex failure: invalid backref number/name"]
        k, f = self.fam([err, ok, ok], [other, ok, ok])
        self.assertEqual(k, "error text")
        self.assertEqual(corpus.coarse(self.ROW, k, f), "E4 different refusal, a class error involved (order)")

    def test_messages_are_normalised_before_they_are_compared(self):
        self.assertEqual(corpus.norm_msg("Regex failure: invalid group name <n1>"), "invalid group name <N>")
        self.assertEqual(corpus.norm_msg("too big number for repeat range 100001"), "too big number for repeat range #")
        self.assertEqual(corpus.norm_msg("regex parse error: whatever"), "(regex crate parse error)")

    def test_a_killed_row_is_a_resource_family(self):
        k, f = corpus.family(self.ROW, [0, ["ok", 1]], [0, "TIMEOUT", ""])
        self.assertEqual(k, "candidate TIMEOUT")
        self.assertEqual(corpus.coarse(self.ROW, k, f), "resource: killed by the driver (RSS / timeout / crash)")

    def test_totals_and_transitions_against_a_base(self):
        rows = {i: [i, "plain", "test", "a", None, ["a", "b", "c"]] for i in range(4)}
        ok, no = ["ok", True], ["ok", False]
        err = ["err", "Regex failure: x"]
        j1 = {0: line(0, ok, ok, ok), 1: line(1, ok, ok, ok), 2: line(2, ok, ok, ok), 3: line(3, ok, ok, ok)}
        j2 = dict(j1)
        j2[3] = line(3, no, ok, ok)
        base = {0: line(0, err, ok, ok), 1: line(1, ok, ok, ok), 2: line(2, err, ok, ok), 3: line(3, ok, ok, ok)}
        cand = {0: line(0, ok, ok, ok), 1: line(1, err, ok, ok), 2: line(2, err, ok, ok), 3: line(3, no, ok, ok)}
        out = corpus.analyze(rows, j1, j2, cand, base)
        self.assertEqual(out["totals"]["rows"], 4)
        self.assertEqual(out["totals"]["same171"], 1)
        self.assertEqual(out["totals"]["follows182"], 1)
        self.assertEqual(out["totals"]["differs"], 2)
        self.assertEqual(out["totals"]["moved same171 -> follows182"], 1)
        tr = out["coarse_transitions"]["R5 refused in Oniguruma's words, jq answers"]
        self.assertEqual((tr.get("resolved"), tr.get("unchanged")), (1, 1))
        self.assertEqual(sum(r["rows"] for r in out["regressed"]), 1)
        self.assertEqual(out["regressed_rows"][0]["id"], 1)


class CommandLineTests(unittest.TestCase):
    def test_bad_usage_prints_the_help_and_exits_2(self):
        with open(os.devnull, "w") as null:
            saved, sys.stdout = sys.stdout, null
            try:
                self.assertEqual(corpus.main(["x"]), 2)
                self.assertEqual(corpus.main(["x", "nope"]), 2)
                self.assertEqual(corpus.main(["x", "run", "label"]), 2)
                self.assertEqual(corpus.main(["x", "gen", "dir", "--seed"]), 2)
            finally:
                sys.stdout = saved


if __name__ == "__main__":
    unittest.main()
