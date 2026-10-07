#!/usr/bin/env python3
"""Unit tests for scripts/changelog.py (#3920). Need no binary or network; the
`check-pr` tests build a throwaway git repository. Run directly
(`python3 scripts/test_changelog.py`); wired into CI's `Changelog fragments`
job so a regression here fails before a release is the first to find it.

Imported via `importlib` because the script's name is not on `sys.path`.
"""

import contextlib
import importlib.util
import io
import os
import pathlib
import subprocess
import tempfile
import unittest

_SCRIPT_PATH = pathlib.Path(__file__).parent / "changelog.py"
_spec = importlib.util.spec_from_file_location("changelog", _SCRIPT_PATH)
changelog = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(changelog)

# A miniature CHANGELOG.md in the repo's real shape: an Unreleased body with
# repeated `###` headings (as the real one has), a dated release, then links.
LEGACY_BODY = (
    "### Changed\n"
    "\n"
    "- **legacy newest** (#9).\n"
    "  continuation line  with  odd   spacing\n"
    "- legacy older (#8).\n"
    "\n"
    "### Fixed\n"
    "\n"
    "- legacy fix (#7).\n"
    "\n"
    "### Changed\n"
    "\n"
    "- a second Changed heading, as the real file has (#6).\n"
)
CHANGELOG = (
    "# Changelog\n"
    "\n"
    "All notable changes to this project will be documented in this file.\n"
    "\n"
    "## [Unreleased]\n"
    "\n" + LEGACY_BODY + "\n"
    "## [0.7.0] - 2026-04-05\n"
    "\n"
    "### Added\n"
    "\n"
    "- old thing\n"
    "\n"
    "## [0.1.0] - 2026-01-11\n"
    "\n"
    "- first\n"
    "\n"
    "[Unreleased]: https://github.com/rust-works/succinctly/compare/v0.7.0...HEAD\n"
    "[0.7.0]: https://github.com/rust-works/succinctly/compare/v0.6.0...v0.7.0\n"
    "[0.1.0]: https://github.com/rust-works/succinctly/releases/tag/v0.1.0\n"
)

FRAGMENTS = {
    "12.changed.md": "- **twelve** changed (#12).\n  more prose.\n",
    "3.changed.md": "- three changed (#3).\n",
    "3.changed.2.md": "- three changed, second entry (#3).\n",
    "+orphan.changed.md": "- orphan changed.\n",
    "5.added.md": "- five added (#5).\n",
    "6.fixed.md": "- six fixed (#6).\n",
    "7.security.md": "- seven security (#7).\n",
    "8.performance.md": "- eight performance (#8).\n",
    "9.removed.md": "- nine removed (#9).\n",
    "10.deprecated.md": "- ten deprecated (#10).\n",
}

# What the fragments above must render to: Keep a Changelog section order,
# ids ascending (numeric, not lexical: 3 < 12), `.n` suffixes in order, orphans
# after the numbered entries, no blank lines between entries of a section.
GOLDEN_SECTIONS = (
    "### Added\n"
    "\n"
    "- five added (#5).\n"
    "\n"
    "### Changed\n"
    "\n"
    "- three changed (#3).\n"
    "- three changed, second entry (#3).\n"
    "- **twelve** changed (#12).\n"
    "  more prose.\n"
    "- orphan changed.\n"
    "\n"
    "### Deprecated\n"
    "\n"
    "- ten deprecated (#10).\n"
    "\n"
    "### Removed\n"
    "\n"
    "- nine removed (#9).\n"
    "\n"
    "### Performance\n"
    "\n"
    "- eight performance (#8).\n"
    "\n"
    "### Fixed\n"
    "\n"
    "- six fixed (#6).\n"
    "\n"
    "### Security\n"
    "\n"
    "- seven security (#7).\n"
)


def write_fragments(directory, files):
    directory.mkdir(parents=True, exist_ok=True)
    for name, text in files.items():
        (directory / name).write_text(text, encoding="utf-8")


class Workspace(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = pathlib.Path(tmp.name)
        self.frag_dir = self.root / "changelog.d"
        self.changelog = self.root / "CHANGELOG.md"

    def run_cli(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            status = changelog.main(list(argv))
        return status, out.getvalue(), err.getvalue()


class NameValidationTests(unittest.TestCase):
    def test_accepts_every_documented_shape(self):
        self.assertEqual(changelog.validate_name("3767.changed.md"), ("3767", "changed", 0))
        self.assertEqual(changelog.validate_name("3767.changed.2.md"), ("3767", "changed", 2))
        self.assertEqual(changelog.validate_name("+fix-typo.fixed.md"), ("+fix-typo", "fixed", 0))
        for t, _ in changelog.TYPES:
            changelog.validate_name(f"1.{t}.md")

    def test_rejects_a_typo_in_the_type(self):
        # The case the check exists for: `chnaged` must not be dropped silently.
        with self.assertRaisesRegex(changelog.ChangelogError, "unknown type 'chnaged'"):
            changelog.validate_name("3767.chnaged.md")

    def test_rejects_malformed_names(self):
        for name in [
            "3767.md",              # no type
            "changed.md",           # no id
            "3767.changed.txt",     # wrong extension
            "3767.changed.0.md",    # suffix must be positive
            "3767.changed.a.md",    # suffix must be numeric
            "03767.changed.md",     # no leading zero
            "3767.Changed.md",      # type is lower case
            "+.changed.md",         # empty slug
            "+Fix.changed.md",      # slug is lower case
            "3767.changed.2.3.md",  # one suffix only
            "notes.md",
        ]:
            with self.subTest(name=name), self.assertRaises(changelog.ChangelogError):
                changelog.validate_name(name)


class LoadFragmentsTests(Workspace):
    def test_missing_directory_is_no_fragments(self):
        self.assertEqual(changelog.load_fragments(self.frag_dir), ([], []))

    def test_readme_is_not_a_fragment(self):
        write_fragments(self.frag_dir, {"README.md": "# docs\n", "1.added.md": "- x\n"})
        fragments, errors = changelog.load_fragments(self.frag_dir)
        self.assertEqual([f.path.name for f in fragments], ["1.added.md"])
        self.assertEqual(errors, [])

    def test_every_bad_fragment_is_reported_not_just_the_first(self):
        write_fragments(
            self.frag_dir,
            {
                "1.chnaged.md": "- x\n",
                "2.added.md": "",
                "3.added.md": "no bullet\n",
                "4.added.md": "- crlf\r\n",
                "stray.txt": "- x\n",
                "5.added.md": "- fine\n",
            },
        )
        fragments, errors = changelog.load_fragments(self.frag_dir)
        self.assertEqual([f.path.name for f in fragments], ["5.added.md"])
        self.assertEqual(len(errors), 5, errors)

    def test_a_body_line_that_looks_like_structure_is_an_error(self):
        for i, line in enumerate(["## [9.9.9] - x", "### Added", "[Unreleased]: http://x", "# top"]):
            with self.subTest(line=line):
                write_fragments(self.frag_dir, {f"{i + 1}.added.md": f"- entry\n{line}\n"})
                _, errors = changelog.load_fragments(self.frag_dir)
                self.assertTrue(any(f"{i + 1}.added.md" in e for e in errors), errors)

    def test_indented_hash_and_bracket_text_in_a_body_is_fine(self):
        write_fragments(self.frag_dir, {"1.added.md": "- entry\n  # a comment\n  ### not a heading\n  [x]: y\n- [link](u)\n"})
        fragments, errors = changelog.load_fragments(self.frag_dir)
        self.assertEqual((len(fragments), errors), (1, []))

    def test_non_utf8_body_is_an_error(self):
        self.frag_dir.mkdir()
        (self.frag_dir / "1.added.md").write_bytes(b"- \xff\xfe\n")
        _, errors = changelog.load_fragments(self.frag_dir)
        self.assertEqual(len(errors), 1)
        self.assertIn("UTF-8", errors[0])

    def test_a_directory_in_the_fragment_dir_is_an_error(self):
        (self.frag_dir / "1.added.md").mkdir(parents=True)
        _, errors = changelog.load_fragments(self.frag_dir)
        self.assertEqual(len(errors), 1)

    def test_order_is_numeric_then_orphans(self):
        write_fragments(self.frag_dir, FRAGMENTS)
        fragments, errors = changelog.load_fragments(self.frag_dir)
        self.assertEqual(errors, [])
        changed = [f.path.name for f in fragments if f.type == "changed"]
        self.assertEqual(
            changed,
            ["3.changed.md", "3.changed.2.md", "12.changed.md", "+orphan.changed.md"],
        )


class RenderTests(Workspace):
    def fragments(self):
        write_fragments(self.frag_dir, FRAGMENTS)
        fragments, errors = changelog.load_fragments(self.frag_dir)
        self.assertEqual(errors, [])
        return fragments

    def test_sections_match_the_golden(self):
        rendered = "\n\n".join(changelog.render_sections(self.fragments())) + "\n"
        self.assertEqual(rendered, GOLDEN_SECTIONS)

    def test_empty_sections_are_omitted(self):
        write_fragments(self.frag_dir, {"1.fixed.md": "- only fixed\n"})
        fragments, _ = changelog.load_fragments(self.frag_dir)
        self.assertEqual(changelog.render_sections(fragments), ["### Fixed\n\n- only fixed"])

    def test_release_matches_the_golden_and_keeps_legacy_verbatim(self):
        new = changelog.build_release(CHANGELOG, self.fragments(), "0.8.0", "2026-11-01")
        expected = (
            "# Changelog\n"
            "\n"
            "All notable changes to this project will be documented in this file.\n"
            "\n"
            "## [Unreleased]\n"
            "\n"
            "## [0.8.0] - 2026-11-01\n"
            "\n" + GOLDEN_SECTIONS + "\n" + LEGACY_BODY + "\n"
            "## [0.7.0] - 2026-04-05\n"
            "\n"
            "### Added\n"
            "\n"
            "- old thing\n"
            "\n"
            "## [0.1.0] - 2026-01-11\n"
            "\n"
            "- first\n"
            "\n"
            "[Unreleased]: https://github.com/rust-works/succinctly/compare/v0.8.0...HEAD\n"
            "[0.8.0]: https://github.com/rust-works/succinctly/compare/v0.7.0...v0.8.0\n"
            "[0.7.0]: https://github.com/rust-works/succinctly/compare/v0.6.0...v0.7.0\n"
            "[0.1.0]: https://github.com/rust-works/succinctly/releases/tag/v0.1.0\n"
        )
        self.assertEqual(new, expected)
        self.assertIn(LEGACY_BODY, new)  # byte-for-byte, odd spacing included

    def test_release_without_fragments_still_moves_the_legacy_body(self):
        new = changelog.build_release(CHANGELOG, [], "0.8.0", "2026-11-01")
        self.assertIn("## [0.8.0] - 2026-11-01\n\n" + LEGACY_BODY + "\n## [0.7.0]", new)

    def test_second_release_has_only_fragments(self):
        first = changelog.build_release(CHANGELOG, self.fragments(), "0.8.0", "2026-11-01")
        write_fragments(self.frag_dir, {"20.fixed.md": "- later fix\n"})
        later, _ = changelog.load_fragments(self.frag_dir)
        later = [f for f in later if f.id == "20"]
        second = changelog.build_release(first, later, "0.8.1", "2026-12-01")
        self.assertIn(
            "## [Unreleased]\n\n## [0.8.1] - 2026-12-01\n\n### Fixed\n\n- later fix\n\n"
            "## [0.8.0] - 2026-11-01\n",
            second,
        )
        self.assertIn("[0.8.1]: https://github.com/rust-works/succinctly/compare/v0.8.0...v0.8.1", second)
        self.assertIn("[Unreleased]: https://github.com/rust-works/succinctly/compare/v0.8.1...HEAD", second)

    def test_first_ever_release_links_to_the_tag(self):
        text = (
            "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- a\n\n"
            "[Unreleased]: https://github.com/o/r/compare/v0.0.0...HEAD\n"
        )
        new = changelog.build_release(text, [], "0.1.0", "2026-01-01")
        self.assertIn("[0.1.0]: https://github.com/o/r/releases/tag/v0.1.0\n", new)
        self.assertIn("## [0.1.0] - 2026-01-01\n\n### Added\n\n- a\n\n[Unreleased]", new)

    def test_refusals(self):
        frags = self.fragments()
        cases = [
            (CHANGELOG, frags, "0.7.0", "2026-11-01", "already has a section"),
            (CHANGELOG, frags, "v0.8.0", "2026-11-01", "not a semantic version"),
            (CHANGELOG, frags, "0.8", "2026-11-01", "not a semantic version"),
            (CHANGELOG, frags, "0.8.0", "11/01/2026", "not a YYYY-MM-DD"),
            ("# Changelog\n", frags, "0.8.0", "2026-11-01", "no '## \\[Unreleased\\]'"),
            (
                CHANGELOG.replace("[Unreleased]: ", "[Unrel]: "),
                frags, "0.8.0", "2026-11-01", "link to update",
            ),
            (
                "## [Unreleased]\n\n## [0.7.0] - x\n\n[Unreleased]: https://h/compare/v0.7.0...HEAD\n",
                [], "0.8.0", "2026-11-01", "nothing to release",
            ),
        ]
        for text, fr, version, date, pattern in cases:
            with self.subTest(pattern=pattern), self.assertRaisesRegex(
                changelog.ChangelogError, pattern
            ):
                changelog.build_release(text, fr, version, date)

    def test_prerelease_versions_are_accepted(self):
        new = changelog.build_release(CHANGELOG, [], "0.8.0-rc.1", "2026-11-01")
        self.assertIn("## [0.8.0-rc.1] - 2026-11-01", new)


class CollectCommandTests(Workspace):
    def setUp(self):
        super().setUp()
        self.changelog.write_text(CHANGELOG, encoding="utf-8")
        write_fragments(self.frag_dir, {**FRAGMENTS, "README.md": "# docs\n"})

    def collect(self, *extra):
        return self.run_cli(
            "collect", "--version", "0.8.0", "--date", "2026-11-01",
            "--changelog", str(self.changelog), "--fragments-dir", str(self.frag_dir),
            *extra,
        )

    def test_dry_run_prints_the_section_and_touches_nothing(self):
        before = sorted(p.name for p in self.frag_dir.iterdir())
        status, out, err = self.collect("--dry-run")
        self.assertEqual(status, 0, err)
        self.assertEqual(self.changelog.read_text(encoding="utf-8"), CHANGELOG)
        self.assertEqual(sorted(p.name for p in self.frag_dir.iterdir()), before)
        self.assertIn("## [0.8.0] - 2026-11-01", out)
        self.assertIn(GOLDEN_SECTIONS.rstrip("\n"), out)
        self.assertIn("would consume 10 fragment(s)", err)

    def test_collect_rewrites_the_changelog_and_deletes_only_fragments(self):
        status, _, err = self.collect()
        self.assertEqual(status, 0, err)
        new = self.changelog.read_text(encoding="utf-8")
        self.assertIn("## [0.8.0] - 2026-11-01\n\n" + GOLDEN_SECTIONS, new)
        self.assertIn(LEGACY_BODY, new)
        self.assertEqual([p.name for p in self.frag_dir.iterdir()], ["README.md"])

    def test_collect_keeps_the_file_mode(self):
        os.chmod(self.changelog, 0o644)
        self.collect()
        self.assertEqual(self.changelog.stat().st_mode & 0o777, 0o644)

    def test_crlf_changelog_is_refused_not_normalised(self):
        crlf = CHANGELOG.replace("\n", "\r\n").encode()
        self.changelog.write_bytes(crlf)
        status, _, err = self.collect()
        self.assertEqual(status, 1)
        self.assertIn("CRLF", err)
        self.assertEqual(self.changelog.read_bytes(), crlf)

    @unittest.skipIf(os.name == "nt" or (hasattr(os, "geteuid") and os.geteuid() == 0), "needs POSIX perms, non-root")
    def test_unwritable_fragment_dir_is_refused_before_anything_is_written(self):
        os.chmod(self.frag_dir, 0o555)
        self.addCleanup(os.chmod, self.frag_dir, 0o755)
        status, _, err = self.collect()
        self.assertEqual(status, 1)
        self.assertIn("not writable", err)
        self.assertEqual(self.changelog.read_text(encoding="utf-8"), CHANGELOG)

    def test_a_refused_collect_deletes_nothing(self):
        status, _, err = self.run_cli(
            "collect", "--version", "0.7.0", "--changelog", str(self.changelog),
            "--fragments-dir", str(self.frag_dir),
        )
        self.assertEqual(status, 1)
        self.assertIn("already has a section", err)
        self.assertEqual(self.changelog.read_text(encoding="utf-8"), CHANGELOG)
        self.assertEqual(len(list(self.frag_dir.iterdir())), 11)

    def test_an_invalid_fragment_blocks_the_release(self):
        (self.frag_dir / "99.chnaged.md").write_text("- typo\n", encoding="utf-8")
        status, _, err = self.collect()
        self.assertEqual(status, 1)
        self.assertIn("99.chnaged.md", err)
        self.assertEqual(self.changelog.read_text(encoding="utf-8"), CHANGELOG)
        self.assertEqual(len(list(self.frag_dir.iterdir())), 12)


class CheckCommandTests(Workspace):
    def test_valid_directory_passes(self):
        write_fragments(self.frag_dir, FRAGMENTS)
        status, out, _ = self.run_cli("check", "--fragments-dir", str(self.frag_dir))
        self.assertEqual(status, 0)
        self.assertIn("10 fragment(s) valid", out)

    def test_typo_fails(self):
        write_fragments(self.frag_dir, {"3767.chnaged.md": "- x\n"})
        status, _, err = self.run_cli("check", "--fragments-dir", str(self.frag_dir))
        self.assertEqual(status, 1)
        self.assertIn("unknown type 'chnaged'", err)


class WaiverTests(unittest.TestCase):
    def waiver(self, title="feat(jq): x", body="", labels=(), author="alice", touches_changelog=False):
        return changelog.waiver_reason(title, body, set(labels), author, touches_changelog)

    def test_an_ordinary_pr_is_not_waived(self):
        self.assertIsNone(self.waiver())

    def test_label_waives(self):
        self.assertIn("no-changelog", self.waiver(labels=["enhancement", "no-changelog"]))

    def test_body_marker_waives_in_either_spelling(self):
        self.assertIsNotNone(self.waiver(body="Docs only.\n\n[no changelog]\n"))
        self.assertIsNotNone(self.waiver(body="[Skip Changelog]"))

    def test_marker_inside_the_templates_html_comment_does_not_waive(self):
        body = "## Description\n<!-- add [no changelog] to waive the fragment -->\nreal text\n"
        self.assertIsNone(self.waiver(body=body))

    def test_release_pr_is_waived_only_if_it_rewrites_the_changelog(self):
        title = "chore(release): prepare v0.8.0"
        self.assertIn("release", self.waiver(title=title, touches_changelog=True))
        # The title alone must not skip the gate for a feature PR.
        self.assertIsNone(self.waiver(title=title))

    def test_bot_is_waived(self):
        self.assertIn("bot", self.waiver(author="dependabot[bot]"))

    def test_similar_label_does_not_waive(self):
        self.assertIsNone(self.waiver(labels=["no-changelog-needed", "changelog"]))


def git(repo, *args):
    subprocess.run(
        ["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@t",
         "-c", "commit.gpgsign=false", *args],
        check=True, capture_output=True,
    )


class CheckPrTests(Workspace):
    def setUp(self):
        super().setUp()
        git(self.root, "init", "-q", "-b", "main")
        (self.root / "README.md").write_text("x\n")
        write_fragments(self.frag_dir, {"README.md": "# docs\n", "1.added.md": "- already merged\n"})
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", "base")
        git(self.root, "checkout", "-q", "-b", "pr")
        # The git helpers default to this repository; point them at the throwaway one.
        self.addCleanup(setattr, changelog, "REPO", changelog.REPO)
        changelog.REPO = self.root

    def check_pr(self, **kw):
        argv = ["check-pr", "--fragments-dir", str(self.frag_dir), "--base", "main",
                "--head", "pr", "--title", kw.get("title", "feat(jq): x"),
                "--labels", kw.get("labels", ""), "--author", kw.get("author", "alice")]
        if "body" in kw:
            body_file = self.root / "body.txt"
            body_file.write_text(kw["body"])
            argv += ["--body-file", str(body_file)]
        return self.run_cli(*argv)

    def commit(self, files):
        for rel, text in files.items():
            p = self.root / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(text)
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", "change")

    def test_pr_without_a_fragment_fails(self):
        self.commit({"src.rs": "fn main() {}\n"})
        status, _, err = self.check_pr()
        self.assertEqual(status, 1)
        self.assertIn("neither adds a changelog fragment", err)

    def test_pr_adding_a_fragment_passes(self):
        self.commit({"changelog.d/2.fixed.md": "- fix\n", "src.rs": "x\n"})
        status, out, err = self.check_pr()
        self.assertEqual(status, 0, err)
        self.assertIn("2.fixed.md", out)

    def test_pr_editing_an_existing_fragment_passes(self):
        self.commit({"changelog.d/1.added.md": "- already merged, numbers refreshed\n"})
        status, _, err = self.check_pr()
        self.assertEqual(status, 0, err)

    def test_touching_only_the_readme_is_not_a_fragment(self):
        self.commit({"changelog.d/README.md": "# more docs\n"})
        status, _, _ = self.check_pr()
        self.assertEqual(status, 1)

    def test_deleting_a_fragment_is_not_a_fragment(self):
        git(self.root, "rm", "-q", "changelog.d/1.added.md")
        git(self.root, "commit", "-q", "-m", "consume")
        status, _, _ = self.check_pr()
        self.assertEqual(status, 1)

    def test_release_title_waives_only_with_a_changelog_rewrite(self):
        self.commit({"src.rs": "x\n"})
        status, _, _ = self.check_pr(title="chore(release): tweak jq")
        self.assertEqual(status, 1)
        self.commit({"CHANGELOG.md": "# Changelog\n"})  # new file: not a rewrite
        status, _, _ = self.check_pr(title="chore(release): tweak jq")
        self.assertEqual(status, 1)

    def test_release_pr_that_consumes_fragments_passes(self):
        (self.root / "CHANGELOG.md").write_text("# Changelog\n")
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", "add changelog")
        git(self.root, "checkout", "-q", "-B", "main")
        git(self.root, "checkout", "-q", "-b", "release")
        (self.root / "CHANGELOG.md").write_text("# Changelog\n\n## [0.8.0]\n")
        git(self.root, "rm", "-q", "changelog.d/1.added.md")
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", "release")
        argv = ["check-pr", "--fragments-dir", str(self.frag_dir), "--base", "main",
                "--head", "release", "--title", "chore(release): prepare v0.8.0"]
        status, out, err = self.run_cli(*argv)
        self.assertEqual(status, 0, err)
        self.assertIn("release PR", out)

    def test_waived_pr_passes_without_a_fragment(self):
        self.commit({"docs.md": "x\n"})
        status, out, err = self.check_pr(labels="no-changelog")
        self.assertEqual(status, 0, err)
        self.assertIn("waived", out)
        status, _, _ = self.check_pr(body="CI only.\n[no changelog]\n")
        self.assertEqual(status, 0)

    def test_a_malformed_fragment_fails_even_when_waived(self):
        self.commit({"changelog.d/2.chnaged.md": "- typo\n"})
        status, _, err = self.check_pr(labels="no-changelog")
        self.assertEqual(status, 1)
        self.assertIn("unknown type", err)


if __name__ == "__main__":
    unittest.main()
