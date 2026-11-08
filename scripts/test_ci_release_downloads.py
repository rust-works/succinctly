#!/usr/bin/env python3
"""Pins that CI fetches pinned release assets through the API endpoint, with retries (#6652).

On 2026-11-08 `https://github.com/<owner>/<repo>/releases/download/<tag>/<asset>` answered 504 to
every request for hours (about 70 ms from the edge, no redirect) while the API asset endpoint served
the same bytes, so `yq golden drift check`, `jq golden drift check` and `Check Links` failed on every
PR. A retry of the same URL cannot recover from that. `gh release download` uses the API endpoint, and
`scripts/gh-release-download.sh` is the one copy of the download-and-retry loop that every workflow
step calls.

Three kinds of check, none of which touches the network (a fake `gh` stands in):
  - no workflow line fetches `/releases/download/` (or `/releases/latest/download/`), however it
    spells the fetch (curl, wget, an `-o` on the next line), and the detector can fail;
  - each pinned-tool install step goes through the helper and passes it a token;
  - the helper retries, then gives up with an error, and hands `gh` the arguments it was given.

It reads the workflow text, so it cannot see inside a third-party action. The two this repo used that
download from that URL in their own code (`lycheeverse/lychee-action`, `EmbarkStudios/cargo-deny-action`,
whose Docker build pipes the 504 page into `tar`) were replaced, and are listed in
`ACTIONS_THAT_CURL_THE_DOWNLOAD_URL` so they stay out; any other `uses:` that installs a tool is a
review question, not something this test answers.

Run directly (`python3 scripts/test_ci_release_downloads.py`).
"""

import os
import pathlib
import re
import stat
import subprocess
import tempfile
import unittest

SCRIPTS = pathlib.Path(__file__).resolve().parent
WORKFLOWS = SCRIPTS.parent / ".github" / "workflows"
HELPER = SCRIPTS / "gh-release-download.sh"

# The URL shape of the direct download, whoever fetches it. A `gh release download` never spells it.
DIRECT_DOWNLOAD = re.compile(r"/releases/(?:latest/)?download/")


# Actions whose own code fetches a release asset from the direct URL, so using one is the same fault.
ACTIONS_THAT_CURL_THE_DOWNLOAD_URL = ("lycheeverse/lychee-action", "EmbarkStudios/cargo-deny-action")


def used_actions(text, names):
    """`(line number, line)` for each non-comment `uses:` line that names one of `names`."""
    return [
        (i, line.strip())
        for i, line in enumerate(text.splitlines(), 1)
        if "uses:" in line and not line.lstrip().startswith("#") and any(n in line for n in names)
    ]


def direct_download_lines(text):
    """`(line number, line)` for each non-comment line that names a direct release-download URL."""
    return [
        (i, line.strip())
        for i, line in enumerate(text.splitlines(), 1)
        if DIRECT_DOWNLOAD.search(line) and not line.lstrip().startswith("#")
    ]


def step_block(text, name):
    """The step named `name`: from its `- name:` line to the next step or job key."""
    match = re.search(rf"\n( +)- name: {re.escape(name)}\n(?:.*\n)*?(?=\1- |\Z|  [A-Za-z0-9_-]+:\n)", text)
    assert match is not None, name
    return match.group(0)


class DirectDownloadGuardTests(unittest.TestCase):
    def test_no_workflow_fetches_a_release_asset_from_the_download_url(self):
        workflows = sorted(WORKFLOWS.glob("*.yml"))
        self.assertTrue(workflows)
        for path in workflows:
            with self.subTest(workflow=path.name):
                found = [
                    f"line {i}: {line}" for i, line in direct_download_lines(path.read_text(encoding="utf-8"))
                ]
                self.assertEqual(
                    found,
                    [],
                    "fetch a pinned release asset with scripts/gh-release-download.sh (the API endpoint), "
                    "not from /releases/download/, which answered 504 for hours on 2026-11-08 (#6652)",
                )

    def test_no_workflow_uses_an_action_that_curls_the_direct_download_url(self):
        for path in sorted(WORKFLOWS.glob("*.yml")):
            with self.subTest(workflow=path.name):
                found = [
                    f"line {i}: {line}"
                    for i, line in used_actions(path.read_text(encoding="utf-8"), ACTIONS_THAT_CURL_THE_DOWNLOAD_URL)
                ]
                self.assertEqual(found, [], "install the pinned release with scripts/gh-release-download.sh (#6652)")

    def test_the_action_detector_flags_a_use_and_ignores_a_comment(self):
        self.assertTrue(used_actions("      - uses: EmbarkStudios/cargo-deny-action@v2.1.1", ACTIONS_THAT_CURL_THE_DOWNLOAD_URL))
        self.assertTrue(used_actions("        uses: lycheeverse/lychee-action@v2", ACTIONS_THAT_CURL_THE_DOWNLOAD_URL))
        self.assertEqual(used_actions("      # uses: lycheeverse/lychee-action@v2", ACTIONS_THAT_CURL_THE_DOWNLOAD_URL), [])

    def test_the_detector_flags_every_spelling_of_the_direct_download(self):
        for label, text in (
            ("curl", 'curl -fsSL "https://github.com/o/r/releases/download/v1/a" -o a'),
            ("wget", "wget https://github.com/o/r/releases/download/v1/a"),
            ("latest", "curl -sL https://github.com/o/r/releases/latest/download/a"),
            ("continued", "curl -fsSL \\\n  https://github.com/o/r/releases/download/${V}/a \\\n  -o a"),
            ("interpolated", 'curl -fsSL "https://github.com/${REPO}/releases/download/${V}/${ASSET}"'),
        ):
            with self.subTest(spelling=label):
                self.assertTrue(direct_download_lines(text), text)

    def test_the_detector_passes_what_it_should(self):
        for label, text in (
            ("a comment naming the URL", "# the releases/download/ URL answered 504"),
            ("the API route", "gh release download v1 -R o/r -p a -O a --clobber"),
            ("the release page", "echo https://github.com/o/r/releases/tag/v1"),
            ("the helper", "./scripts/gh-release-download.sh o/r v1 a /usr/local/bin/a"),
        ):
            with self.subTest(case=label):
                self.assertEqual(direct_download_lines(text), [], text)


class InstallStepTests(unittest.TestCase):
    CASES = (
        ("ci.yml", "Install pinned yq", "mikefarah/yq", "yq_linux_amd64"),
        ("ci.yml", "Install pinned jq", "jqlang/jq", "jq-linux-amd64"),
        ("links.yml", "Install lychee", "lycheeverse/lychee", "lychee-x86_64-unknown-linux-gnu.tar.gz"),
        ("security.yml", "Install cargo-deny", "EmbarkStudios/cargo-deny", "cargo-deny-${CARGO_DENY_VERSION}-x86_64-unknown-linux-musl.tar.gz"),
    )

    def test_each_pinned_tool_install_goes_through_the_helper_with_a_token(self):
        for workflow, step, repo, asset in self.CASES:
            with self.subTest(step=step):
                block = step_block((WORKFLOWS / workflow).read_text(encoding="utf-8"), step)
                self.assertIn("GH_TOKEN: ${{ github.token }}", block)
                self.assertRegex(block, rf"\./scripts/gh-release-download\.sh {re.escape(repo)} ")
                self.assertIn(asset, block)
                self.assertNotRegex(block, r"\b(curl|wget)\b", "the install is the helper's, not a second loop")

    def test_the_helper_is_executable(self):
        self.assertTrue(HELPER.stat().st_mode & stat.S_IXUSR)


class HelperTests(unittest.TestCase):
    """The helper against a fake `gh` that logs its arguments and fails a set number of times."""

    def run_helper(self, failures, *args, attempts=None, helper_args=("o/r", "v1", "asset.bin", "OUT")):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            log = tmp / "calls.log"
            fake = tmp / "bin" / "gh"
            fake.parent.mkdir()
            fake.write_text(
                "#!/bin/sh\n"
                f'echo "$@" >> "{log}"\n'
                f'n=$(wc -l < "{log}")\n'
                f'[ "$n" -gt {failures} ]\n',
                encoding="utf-8",
            )
            fake.chmod(0o755)
            env = dict(os.environ, PATH=f"{fake.parent}:{os.environ['PATH']}", GH_RELEASE_DOWNLOAD_SLEEP="0")
            if attempts is not None:
                env["GH_RELEASE_DOWNLOAD_ATTEMPTS"] = str(attempts)
            done = subprocess.run(
                ["bash", str(HELPER), *helper_args], env=env, capture_output=True, text=True, timeout=60
            )
            calls = log.read_text(encoding="utf-8").splitlines() if log.exists() else []
            return done, calls

    def test_a_first_try_success_calls_gh_once_with_the_given_arguments(self):
        done, calls = self.run_helper(0)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(calls, ["release download v1 -R o/r -p asset.bin -O OUT --clobber"])

    def test_it_retries_a_failing_download_until_one_succeeds(self):
        done, calls = self.run_helper(3)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(len(calls), 4)
        self.assertEqual(done.stdout.count("::warning::"), 3)

    def test_it_gives_up_after_the_attempt_limit_with_an_error_annotation(self):
        done, calls = self.run_helper(99)
        self.assertEqual(done.returncode, 1)
        self.assertEqual(len(calls), 5, "five attempts by default")
        self.assertIn("::error::could not download asset.bin of o/r v1 after 5 attempts", done.stdout)

    def test_the_attempt_limit_can_be_set(self):
        done, calls = self.run_helper(99, attempts=2)
        self.assertEqual(done.returncode, 1)
        self.assertEqual(len(calls), 2)

    def test_a_single_attempt_does_not_announce_a_retry(self):
        done, _ = self.run_helper(99, attempts=1)
        self.assertEqual(done.returncode, 1)
        self.assertNotIn("retrying", done.stdout)

    def test_a_wrong_argument_count_is_a_usage_error_and_never_calls_gh(self):
        done, calls = self.run_helper(0, helper_args=("o/r", "v1"))
        self.assertEqual(done.returncode, 2)
        self.assertIn("usage:", done.stderr)
        self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
