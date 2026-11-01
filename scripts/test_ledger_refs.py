#!/usr/bin/env python3
"""Unit tests for scripts/ledger_refs.py (#5585)."""

import os
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import ledger_refs  # noqa: E402

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "ledger_refs.py")


def rel_of(id_):
    """`features:strftime/1` is `features/strftime.1.toml`."""
    suite, probe = id_.split(":", 1)
    return f"{suite}/{probe.replace('/', '.')}.toml"


def ledger(*entries):
    """The ledger directory as `{path: text}`; each entry is `(id, category, issue-or-None)`."""
    files = {}
    for id_, category, issue in entries:
        body = f'category = "{category}"\nreason = "r"\n'
        if issue:
            body += f'issue = "{issue}"\n'
        files[rel_of(id_)] = body
    return files


def old_ledger(*entries):
    """The single file of before #5769: `[[divergence]]` blocks."""
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
        return ledger_refs.stale_looking(
            ledger_refs.to_triage_files(base),
            ledger_refs.to_triage_files(head),
            branch,
            list(subjects),
            list(messages),
        )

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
        self.assertEqual(self.check({}, ledger(FILES_MISSING), branch="issue-5472-x"), [])

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


class EntryPathTests(unittest.TestCase):
    def test_an_id_is_the_path(self):
        for rel, id_ in [
            ("cli/version.toml", "cli:version"),
            ("features/strftime.1.toml", "features:strftime/1"),
            ("features/get_jq_origin.0.toml", "features:get_jq_origin/0"),
            ("since-secondary/add.1.toml", "since-secondary:add/1"),
        ]:
            self.assertEqual(ledger_refs.entry_id(rel), id_)
            self.assertEqual(rel_of(id_), rel)

    def test_a_path_no_id_maps_to_has_none(self):
        for rel in ["README.md", "cli/a.txt", "a.toml", "cli/sub/a.toml", "cli/a b.toml", "cli/.toml"]:
            self.assertIsNone(ledger_refs.entry_id(rel), rel)

    def test_the_single_file_shape_is_still_read_for_a_base_before_the_split(self):
        text = old_ledger(FILES_MISSING, ("cli:version", "by-design", "#5472"))
        self.assertEqual(
            ledger_refs.to_triage(text), {5472: ["cli:files-missing-continues"]}
        )

    def test_it_reads_the_split_checked_in_ledger(self):
        root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        files = ledger_refs.read_dir(root)
        self.assertGreaterEqual(len(files), 10)
        for rel in files:
            self.assertTrue(rel == "README.md" or ledger_refs.entry_id(rel), rel)


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
        self.write(ledger(FILES_MISSING, ("cli:other", "by-design", None)))
        self.commit("chore(ci): seed")

    def git(self, *args):
        subprocess.run(["git", *args], cwd=self.dir.name, check=True, capture_output=True)

    def write(self, files):
        """Make the ledger directory exactly `files`."""
        root = os.path.join(self.dir.name, ledger_refs.LEDGER)
        shutil.rmtree(root, ignore_errors=True)
        for rel, text in files.items():
            path = os.path.join(root, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8") as f:
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
        files = ledger(FILES_MISSING, ("cli:other", "by-design", None))
        files[rel_of("cli:files-missing-continues")] += "# touched\n"
        self.write(files)
        self.commit("fix(jq): a missing input file is skipped (#5472)")

        failed = self.run_check()
        self.assertEqual(failed.returncode, 1, failed.stderr)
        self.assertIn("cli:files-missing-continues", failed.stderr)

        self.write(ledger(("cli:other", "by-design", None)))
        self.commit("fix(compare): drop the entry (#5472)")
        passed = self.run_check()
        self.assertEqual(passed.returncode, 0, passed.stderr)

    def test_a_base_from_before_the_split_is_read_from_the_single_file(self):
        # The branch that splits the ledger: its base has `divergences.toml`, its head the directory.
        self.git("checkout", "-q", "--orphan", "old-main")
        self.git("rm", "-rq", "--cached", ".")
        shutil.rmtree(os.path.join(self.dir.name, ledger_refs.LEDGER))
        with open(os.path.join(self.dir.name, ledger_refs.OLD_LEDGER), "w", encoding="utf-8") as f:
            f.write(old_ledger(FILES_MISSING))
        self.commit("chore(ci): single file")
        self.git("checkout", "-q", "-b", "issue-5472-split")
        os.remove(os.path.join(self.dir.name, ledger_refs.OLD_LEDGER))
        self.write(ledger(FILES_MISSING))
        self.commit("refactor(compare): split the ledger (#5472)")
        failed = subprocess.run(
            [sys.executable, SCRIPT, "check", "--base", "old-main"],
            cwd=self.dir.name, capture_output=True, text=True, check=False,
        )
        self.assertEqual(failed.returncode, 1, failed.stderr)
        self.assertIn("cli:files-missing-continues", failed.stderr)

    def test_two_branches_that_add_and_delete_different_entries_merge_cleanly(self):
        # #5769: one add and one delete, in neighbouring files, no conflict.
        self.git("checkout", "-q", "-b", "adds")
        self.write(ledger(FILES_MISSING, ("cli:other", "by-design", None), ("cli:a", "to-triage", None)))
        self.commit("chore(compare): add a")
        self.git("checkout", "-q", "main")
        self.git("checkout", "-q", "-b", "deletes")
        self.write(ledger(FILES_MISSING))
        self.commit("fix(compare): delete other")
        self.git("merge", "-q", "--no-edit", "adds")
        merged = sorted(ledger_refs.read_dir(self.dir.name))
        self.assertEqual(
            merged, sorted([rel_of("cli:files-missing-continues"), rel_of("cli:a")])
        )

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
