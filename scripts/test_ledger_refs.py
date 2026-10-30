#!/usr/bin/env python3
"""Unit tests for scripts/ledger_refs.py (#5585)."""

import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import ledger_refs  # noqa: E402

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "ledger_refs.py")


def ledger(*entries):
    """A divergences.toml body; each entry is `(id, category, issue-or-None)`."""
    out = []
    for id_, category, issue in entries:
        block = f'[[divergence]]\nid = "{id_}"\ncategory = "{category}"\nreason = "r"\n'
        if issue:
            block += f'issue = "{issue}"\n'
        out.append(block)
    return "\n".join(out)


FILES_MISSING = ("cli:files-missing-continues", "to-triage", "#5472")


class StaleLookingTests(unittest.TestCase):
    def check(self, base, head, branch="main", subjects=(), messages=()):
        return ledger_refs.stale_looking(base, head, branch, list(subjects), list(messages))

    def test_a_branch_that_names_the_issue_and_keeps_the_entry_is_flagged(self):
        text = ledger(FILES_MISSING)
        self.assertEqual(
            self.check(text, text, branch="issue-5472-jq-missing-input-file-continues"),
            [(5472, "cli:files-missing-continues")],
        )

    def test_a_commit_subject_reference_is_enough(self):
        text = ledger(FILES_MISSING)
        self.assertEqual(
            self.check(text, text, subjects=["fix(jq): a missing file is skipped (#5472)"]),
            [(5472, "cli:files-missing-continues")],
        )

    def test_deleting_the_entry_clears_it(self):
        self.assertEqual(
            self.check(ledger(FILES_MISSING), ledger(), branch="issue-5472-x"), []
        )

    def test_an_entry_the_branch_adds_is_not_a_leftover(self):
        self.assertEqual(
            self.check(ledger(), ledger(FILES_MISSING), branch="issue-5472-x"), []
        )

    def test_a_missing_base_ledger_adds_everything(self):
        self.assertEqual(self.check("", ledger(FILES_MISSING), branch="issue-5472-x"), [])

    def test_only_to_triage_entries_are_checked(self):
        text = ledger(("cli:version", "by-design", "#5472"))
        self.assertEqual(self.check(text, text, branch="issue-5472-x"), [])

    def test_an_unrelated_issue_is_not_flagged(self):
        text = ledger(FILES_MISSING)
        self.assertEqual(
            self.check(text, text, branch="issue-5585-gate", subjects=["ci: gate (#5585)"]), []
        )

    def test_a_longer_number_is_not_a_prefix_match(self):
        text = ledger(FILES_MISSING)
        self.assertEqual(
            self.check(text, text, branch="issue-54721-x", subjects=["fix: y (#54721)"]), []
        )

    def test_the_waiver_line_keeps_the_entry(self):
        text = ledger(FILES_MISSING)
        message = "fix(jq): half of it (#5472)\n\nbody\n[ledger keeps #5472]\n"
        self.assertEqual(
            self.check(text, text, branch="issue-5472-x", messages=[message]), []
        )

    def test_a_waiver_for_another_issue_does_not_help(self):
        text = ledger(FILES_MISSING)
        self.assertEqual(
            self.check(text, text, branch="issue-5472-x", messages=["[ledger keeps #1]\n"]),
            [(5472, "cli:files-missing-continues")],
        )

    def test_an_entry_without_an_issue_is_never_flagged(self):
        text = ledger(("cli:x", "to-triage", None))
        self.assertEqual(self.check(text, text, branch="issue-5472-x"), [])

    def test_two_entries_for_one_issue_are_both_named(self):
        text = ledger(("cli:a", "to-triage", "#7"), ("cli:b", "to-triage", "#7"))
        self.assertEqual(self.check(text, text, branch="issue-7-x"), [(7, "cli:a"), (7, "cli:b")])


class ReferencedIssuesTests(unittest.TestCase):
    def test_branch_and_subject_forms(self):
        self.assertEqual(
            ledger_refs.referenced_issues("issue-12-slug", ["fix: a (#34)", "b #56, and #7"]),
            {12, 34, 56, 7},
        )

    def test_a_branch_without_the_issue_form_has_no_reference(self):
        self.assertEqual(ledger_refs.referenced_issues("feature-12", []), set())
        self.assertEqual(ledger_refs.referenced_issues(None, []), set())


class GitTests(unittest.TestCase):
    """The command line against a real repository, which is what the pre-push gate runs."""

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.email", "t@example.com")
        self.git("config", "user.name", "t")
        self.git("config", "commit.gpgsign", "false")
        os.makedirs(os.path.join(self.dir.name, "comparison/jq"))
        self.write(ledger(FILES_MISSING))
        self.commit("chore(ci): seed")

    def git(self, *args):
        subprocess.run(["git", *args], cwd=self.dir.name, check=True, capture_output=True)

    def write(self, text):
        with open(os.path.join(self.dir.name, ledger_refs.LEDGER), "w", encoding="utf-8") as f:
            f.write(text)

    def commit(self, message):
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)

    def run_check(self, *extra):
        return subprocess.run(
            [sys.executable, SCRIPT, "check", "--base", "main", *extra],
            cwd=self.dir.name,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_fails_naming_the_entry_then_passes_once_it_is_deleted(self):
        self.git("checkout", "-q", "-b", "issue-5472-fix")
        self.write(ledger(FILES_MISSING) + "\n# touched\n")
        self.commit("fix(jq): a missing input file is skipped (#5472)")

        failed = self.run_check()
        self.assertEqual(failed.returncode, 1, failed.stderr)
        self.assertIn("cli:files-missing-continues", failed.stderr)

        self.write(ledger())
        self.commit("fix(compare): drop the entry (#5472)")
        passed = self.run_check()
        self.assertEqual(passed.returncode, 0, passed.stderr)

    def test_an_unreadable_base_is_a_usage_error_not_a_pass(self):
        result = subprocess.run(
            [sys.executable, SCRIPT, "check", "--base", "no-such-ref"],
            cwd=self.dir.name,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 2, result.stderr)


if __name__ == "__main__":
    unittest.main()
