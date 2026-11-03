#!/usr/bin/env python3
"""Tests for scripts/skill_lint.py (#5971): each check fails on a skill built to break it."""

import contextlib
import io
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import skill_lint  # noqa: E402

GOOD = "---\nname: {name}\ndescription: Does a thing. Use when the user asks for the thing.\n---\n\n# Title\n\nBody.\n"


class SkillCase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = self._tmp.name

    def skill(self, name="demo", manifest=None, extra=None):
        """Write `.../<name>/SKILL.md` (a good one unless `manifest` is given) and return its dir."""
        directory = os.path.join(self.root, name)
        os.makedirs(directory, exist_ok=True)
        if manifest is not False:
            with open(os.path.join(directory, "SKILL.md"), "w", encoding="utf-8") as f:
                f.write(GOOD.format(name=name) if manifest is None else manifest)
        for relative, content in (extra or {}).items():
            path = os.path.join(directory, relative)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8") as f:
                f.write(content)
        return directory

    def lint(self, directory):
        return skill_lint.lint_skill(directory)

    def messages(self, directory, level="error"):
        return [message for lvl, _, message in self.lint(directory) if lvl == level]


class ParseFrontmatterTests(unittest.TestCase):
    def test_plain_quoted_folded_and_continued_values(self):
        text = (
            "---\nname: a\ndescription: \"quoted: value\"\nargument-hint: '[x]'\n"
            "long: >-\n  one\n  two\nlines: |\n  a\n  b\nwrapped: first\n  second\n---\nbody\n"
        )
        fields, body, start = skill_lint.parse_frontmatter(text)
        self.assertEqual(fields["name"], "a")
        self.assertEqual(fields["description"], "quoted: value")
        self.assertEqual(fields["argument-hint"], "[x]")
        self.assertEqual(fields["long"], "one two")
        self.assertEqual(fields["lines"], "a\nb")
        self.assertEqual(fields["wrapped"], "first second")
        self.assertEqual(body, "body\n")
        self.assertEqual(start, 14)

    def test_no_frontmatter_is_none(self):
        self.assertIsNone(skill_lint.parse_frontmatter("# Release Skill\n"))
        self.assertIsNone(skill_lint.parse_frontmatter("---\nname: a\nno closing fence\n"))


class LintSkillTests(SkillCase):
    def test_a_good_skill_has_no_findings(self):
        self.assertEqual(self.lint(self.skill()), [])

    def test_missing_manifest(self):
        self.assertEqual(self.messages(self.skill(manifest=False)), ["SKILL.md not found"])

    def test_missing_frontmatter_is_the_release_skill_defect(self):
        # `.claude/skills/release/SKILL.md` had no frontmatter until #5956.
        self.assertIn("no YAML frontmatter", self.messages(self.skill(manifest="# Release Skill\n\nText.\n"))[0])

    def test_name_rules(self):
        cases = {
            "---\ndescription: Use when x.\n---\n": "frontmatter has no name",
            "---\nname: Demo\ndescription: Use when x.\n---\n": "must be lowercase letters",
            "---\nname: de--mo\ndescription: Use when x.\n---\n": "single hyphens",
            "---\nname: other\ndescription: Use when x.\n---\n": 'must equal its directory name "demo"',
            "---\nname: " + "a" * 65 + "\ndescription: Use when x.\n---\n": "(limit 64)",
        }
        for manifest, expected in cases.items():
            with self.subTest(expected=expected):
                self.assertTrue(
                    any(expected in m for m in self.messages(self.skill(manifest=manifest))),
                    manifest,
                )

    def test_reserved_words_in_the_name(self):
        for word in ("claude", "anthropic"):
            directory = self.skill(name=f"my-{word}-tool")
            self.assertTrue(any(word in m for m in self.messages(directory)))

    def test_description_rules(self):
        self.assertIn(
            "frontmatter has no description",
            self.messages(self.skill(manifest="---\nname: demo\n---\n")),
        )
        long = "---\nname: demo\ndescription: " + "x" * 1025 + "\n---\n"
        self.assertTrue(any("(limit 1024)" in m for m in self.messages(self.skill(manifest=long))))
        exact = "---\nname: demo\ndescription: Use when " + "x" * (1024 - len("Use when ")) + "\n---\n"
        self.assertEqual(self.messages(self.skill(manifest=exact)), [])

    def test_a_description_without_a_trigger_is_a_warning_not_an_error(self):
        directory = self.skill(manifest="---\nname: demo\ndescription: A skill.\n---\n")
        self.assertEqual(self.messages(directory), [])
        self.assertTrue(any("when to use" in m for m in self.messages(directory, "warning")))

    def test_an_unknown_key_is_an_error_and_a_claude_only_key_is_not(self):
        directory = self.skill(
            manifest="---\nname: demo\ndescription: Use when x.\nuser-invocable: true\nargument-hint: '[n]'\nallowed-tools: Bash\n---\n"
        )
        self.assertEqual(self.lint(directory), [])
        directory = self.skill(manifest="---\nname: demo\ndescription: Use when x.\ndescripton: typo\n---\n")
        self.assertEqual(self.messages(directory), ['unknown frontmatter key "descripton": no host reads it'])

    def test_the_body_limit_is_500_lines(self):
        head = "---\nname: demo\ndescription: Use when x.\n---\n"
        self.assertEqual(self.messages(self.skill(manifest=head + "line\n" * 500)), [])
        over = self.messages(self.skill(manifest=head + "line\n" * 501))
        self.assertEqual(len(over), 1)
        self.assertIn("501 lines", over[0])

    def test_dead_links_are_errors_wherever_the_skill_keeps_markdown(self):
        directory = self.skill(
            manifest=GOOD.format(name="demo") + "[ok](ok.md) [dead](missing.md#part) [up](../../nope/x.md)\n",
            extra={"ok.md": "x\n", "references/more.md": "[gone](gone.md)\n"},
        )
        found = sorted(self.messages(directory))
        self.assertEqual(
            found,
            ["link to a missing file: ../../nope/x.md", "link to a missing file: gone.md", "link to a missing file: missing.md"],
        )

    def test_a_link_to_a_directory_resolves(self):
        directory = self.skill(
            manifest=GOOD.format(name="demo") + "[dir](sub/)\n", extra={"sub/a.md": "x\n"}
        )
        self.assertEqual(self.lint(directory), [])

    def test_urls_anchors_code_and_placeholders_are_not_links(self):
        body = (
            "[web](https://example.com/x) [mail](mailto:a@b.c) [anchor](#top) [empty]() [dots](...)\n"
            "`[inline](nope.md)`\n\n```\n[fenced](nope2.md)\n```\n\n~~~\n[tilde](nope3.md)\n~~~\n"
        )
        self.assertEqual(self.lint(self.skill(manifest=GOOD.format(name="demo") + body)), [])


class MainTests(SkillCase):
    def run_main(self, *args):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = skill_lint.main(list(args))
        return code, out.getvalue()

    def test_exit_codes_and_summary(self):
        good = self.skill(name="good")
        bad = self.skill(name="bad", manifest="---\nname: bad\n---\n")
        code, out = self.run_main(good)
        self.assertEqual((code, out.strip()), (0, "1 skill(s), 0 error(s), 0 warning(s)"))
        code, out = self.run_main(good, bad)
        self.assertEqual(code, 1)
        self.assertIn("ERROR: ", out)
        self.assertIn("2 skill(s), 1 error(s)", out)

    def test_warnings_fail_only_under_strict(self):
        warned = self.skill(name="warned", manifest="---\nname: warned\ndescription: A skill.\n---\n")
        self.assertEqual(self.run_main(warned)[0], 0)
        code, out = self.run_main("--strict", warned)
        self.assertEqual(code, 1)
        self.assertIn("warning: ", out)


class RepositoryTests(unittest.TestCase):
    def test_every_skill_in_the_repository_passes(self):
        # The point of the check: a skill that cannot load fails CI, not a session.
        directories = skill_lint.skill_directories()
        self.assertGreaterEqual(len(directories), 13)
        found = [f for d in directories for f in skill_lint.lint_skill(d) if f[0] == "error"]
        self.assertEqual(found, [])

    def test_codex_reads_the_same_directory(self):
        # `.agents/skills` is a symlink (#5956), which is why Claude-only keys are accepted.
        link = os.path.join(skill_lint.ROOT, ".agents", "skills")
        self.assertEqual(os.readlink(link), "../.claude/skills")


if __name__ == "__main__":
    unittest.main()
