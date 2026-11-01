#!/usr/bin/env python3
"""Unit tests for scripts/jq-grammar-fuzz.py that need no build and no jq: the generator is
deterministic per seed, a rendered program is balanced, the stderr normalizer keeps only the
fault's class, `classify` sorts the outcomes the way the docstring says, and the minimizer
reduces a tree to the part a predicate needs and always terminates (#5695: replacing a subtree
by an equal one made the first version loop forever). Run directly or via
`python3 -m unittest scripts/test_jq_grammar_fuzz.py`.
"""

import importlib.util
import json
import os
import pathlib
import random
import sys
import unittest

_PATH = pathlib.Path(__file__).parent / "jq-grammar-fuzz.py"
_spec = importlib.util.spec_from_file_location("jq_grammar_fuzz", _PATH)
fuzz = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fuzz)


def programs(seed, n, depth=3):
    rng = random.Random(seed)
    return [fuzz.gen(rng, depth, set()).render() for _ in range(n)]


class GeneratorTests(unittest.TestCase):
    def test_a_seed_gives_the_same_programs(self):
        self.assertEqual(programs(5, 50), programs(5, 50))
        self.assertNotEqual(programs(5, 50), programs(6, 50))

    def test_every_program_renders_with_balanced_brackets(self):
        for text in programs(1, 400, depth=4):
            for open_, close in ("()", "[]", "{}"):
                # Quoted text can hold a bracket, so count outside strings only.
                outside = "".join(part for i, part in enumerate(text.split('"')) if i % 2 == 0)
                self.assertEqual(outside.count(open_), outside.count(close), text)

    def test_a_variable_is_only_used_where_it_is_bound(self):
        for text in programs(2, 400, depth=4):
            for name in set(__import__("re").findall(r"\$(\w+)", text)):
                if name in ("ENV", "__loc__"):
                    continue
                self.assertRegex(text, r"(as|reduce|foreach)[^|]*\$" + name, text)

    def test_json_documents_are_deterministic(self):
        a = [fuzz.gen_json(random.Random(3), 3) for _ in range(5)]
        b = [fuzz.gen_json(random.Random(3), 3) for _ in range(5)]
        self.assertEqual(a, b)


class ComparisonTests(unittest.TestCase):
    def test_normalize_err_keeps_the_fault_and_drops_position_and_value(self):
        n = fuzz.normalize_err
        self.assertEqual(n('jq: error (at <stdin>:3): Cannot index number with "a"'), "jq: error: Cannot index number with")
        self.assertEqual(n("jq: error (at <unknown>) (not a string): [1,2]"), "jq: error")
        self.assertEqual(n(""), "")
        self.assertEqual(n("jq: error (at <stdin>:0): x cannot be negated"), "jq: error: x cannot be negated")

    def test_classify(self):
        ok, fail = (0, "1\n", ""), (5, "", "jq: error: x")
        self.assertEqual(fuzz.classify(ok, ok), "agree")
        self.assertEqual(fuzz.classify(fail, fail), "agree")
        self.assertEqual(fuzz.classify((0, "2\n", ""), ok), "mismatch")
        self.assertEqual(fuzz.classify(ok, fail), "ours-only")
        self.assertEqual(fuzz.classify(fail, ok), "theirs-only")
        self.assertEqual(fuzz.classify(None, ok), "skip")
        self.assertEqual(fuzz.classify(ok, None), "skip")
        self.assertEqual(fuzz.classify(ok, (3, "", "jq: error: compile")), "skip")

    def test_a_signature_ignores_the_program(self):
        ours, theirs = (0, "1\n", ""), (5, "", "jq: error: x")
        self.assertEqual(fuzz.signature("ours-only", ours, theirs), fuzz.signature("ours-only", ours, theirs))
        self.assertNotEqual(fuzz.signature("ours-only", ours, theirs), fuzz.signature("mismatch", ours, theirs))


class ReportTests(unittest.TestCase):
    def test_the_tools_run_in_the_fixed_environment(self):
        saved = dict(os.environ)
        try:
            os.environ.update(LANG="de_DE.UTF-8", LC_TIME="de_DE.UTF-8", LC_ALL="tr_TR", TZ="Asia/Tokyo",
                              JQ_COLORS="1;31", NO_COLOR="1", LANGUAGE="de", KEEP_ME="1")
            env = fuzz.tool_env()
        finally:
            os.environ.clear()
            os.environ.update(saved)
        self.assertEqual((env["LC_ALL"], env["TZ"], env["KEEP_ME"]), ("C", "UTC", "1"))
        for gone in ("LANG", "LC_TIME", "JQ_COLORS", "NO_COLOR", "LANGUAGE"):
            self.assertNotIn(gone, env)

    def test_the_json_report_carries_each_finding_with_both_outcomes(self):
        args = type("A", (), {"seed": 7, "n": 9, "depth": 3})()
        ours, theirs = (0, "1\n", ""), (5, "", "jq: error: x")
        rep = fuzz.report(args, {"agree": 8, "ours-only": 1}, 1, [(4, "ours-only", ".a", "{}", ours, theirs)])
        self.assertEqual((rep["seed"], rep["programs"], rep["signatures"]), (7, 9, 1))
        self.assertEqual(
            rep["findings"],
            [{"kind": "ours-only", "members": 4, "program": ".a", "doc": "{}",
              "reference": {"exit": 5, "stdout": "", "error": "jq: error: x"},
              "subject": {"exit": 0, "stdout": "1\n", "error": ""}}],
        )
        json.dumps(rep)  # serializable as it stands


class ShrinkTests(unittest.TestCase):
    def tree(self):
        n = fuzz.Node
        return n("{} | {}", [n("{} + {}", [fuzz.atom(".a"), fuzz.atom("MARK")]), n("[{}]", [fuzz.atom(".b")])])

    def test_it_keeps_only_what_the_predicate_needs(self):
        small = fuzz.shrink(self.tree(), lambda root: "MARK" in root.render())
        self.assertEqual(small.render(), "MARK")

    def test_it_terminates_when_every_candidate_still_holds(self):
        small = fuzz.shrink(self.tree(), lambda root: True)
        self.assertLessEqual(len(small.render()), 4)

    def test_it_leaves_a_tree_the_predicate_rejects_after_any_change(self):
        tree = self.tree()
        before = tree.render()
        small = fuzz.shrink(tree, lambda root: root.render() == before)
        self.assertEqual(small.render(), before)


if __name__ == "__main__":
    unittest.main()
