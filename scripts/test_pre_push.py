#!/usr/bin/env python3
"""Unit tests for scripts/pre_push.py (#5585), including the check that it cannot drift from CI.

`WorkflowDriftTests` is the one that matters: it reads every command out of
`.github/workflows/*.yml` and fails when a gate-looking one is neither mirrored by a gate in
`pre_push.GATES` nor listed in `pre_push.NOT_MIRRORED`.
"""

import glob
import io
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import pre_push  # noqa: E402

ROOT = pre_push.ROOT
BLOCK_SCALARS = {"|", ">", "|-", ">-", "|+", ">+", ""}


def workflow_commands():
    """`{command: workflow file}` for every gate-looking command of every workflow.

    A line scan, not a YAML parse (PyYAML is not guaranteed on a runner): a `run:` or `testCmd:`
    value is either inline or an indented block scalar, whose lines are joined across `\\`
    continuations and split into one command per line.
    """
    found = {}
    for path in sorted(glob.glob(os.path.join(ROOT, ".github", "workflows", "*.yml"))):
        with open(path, encoding="utf-8") as f:
            lines = f.read().split("\n")
        i = 0
        while i < len(lines):
            m = re.match(r"^(\s*)(?:- )?(?:run|testCmd):\s*(.*)$", lines[i])
            i += 1
            if not m:
                continue
            indent, value = len(m.group(1)), m.group(2).strip()
            if value in BLOCK_SCALARS:
                body = []
                while i < len(lines) and (
                    not lines[i].strip() or len(lines[i]) - len(lines[i].lstrip()) > indent
                ):
                    body.append(lines[i].strip())
                    i += 1
                value = "\n".join(body)
            value = re.sub(r"\\\n\s*", " ", value)
            for line in value.split("\n"):
                line = re.sub(r"\s+", " ", line.strip())
                if pre_push.PREFIXES.match(line):
                    found.setdefault(line, os.path.basename(path))
    return found


class WorkflowDriftTests(unittest.TestCase):
    def test_the_scan_finds_the_gates_it_is_meant_to_police(self):
        commands = workflow_commands()
        for expected in (
            "cargo fmt --all -- --check",
            "cargo clippy --all-targets --all-features -- -D warnings",
            "python3 scripts/changelog.py check",
        ):
            self.assertIn(expected, commands)
        # A backslash continuation is one command, not two.
        self.assertTrue(
            any(c.startswith("./target/release/succinctly dev compare jq --quick --dimension") for c in commands)
        )

    def test_every_workflow_command_is_mirrored_or_deliberately_not(self):
        mirrors = [re.compile(m) for g in pre_push.GATES for m in g.mirrors]
        excluded = [re.compile(rx) for rx, _ in pre_push.NOT_MIRRORED]
        unaccounted = {
            c: wf
            for c, wf in workflow_commands().items()
            if not any(r.search(c) for r in mirrors + excluded)
        }
        self.assertEqual(
            unaccounted,
            {},
            "a CI command is neither run by scripts/pre_push.py nor listed in its NOT_MIRRORED "
            "(with a reason): add a Gate that mirrors it, or an exclusion",
        )

    def test_every_mirror_and_exclusion_still_matches_a_workflow_command(self):
        commands = list(workflow_commands())
        for gate in pre_push.GATES:
            for rx in gate.mirrors:
                self.assertTrue(
                    any(re.search(rx, c) for c in commands),
                    f"gate {gate.name} mirrors /{rx}/, which no workflow runs any more",
                )
        for rx, why in pre_push.NOT_MIRRORED:
            self.assertTrue(why.strip())
            self.assertTrue(
                any(re.search(rx, c) for c in commands),
                f"NOT_MIRRORED /{rx}/ matches no workflow command any more",
            )


class GateTableTests(unittest.TestCase):
    def test_names_are_unique_and_tiers_known(self):
        names = [g.name for g in pre_push.GATES]
        self.assertEqual(len(names), len(set(names)))
        self.assertTrue(all(g.tier in ("fast", "full") for g in pre_push.GATES))

    def test_every_step_formats_and_every_script_it_runs_exists(self):
        for gate in pre_push.GATES:
            for step in gate.steps:
                cmd = step.format(base="BASE", target="TARGET")
                for script in re.findall(r"\b(scripts/[\w./-]+\.(?:py|sh))", cmd):
                    self.assertTrue(os.path.exists(os.path.join(ROOT, script)), f"{gate.name}: {script}")

    def test_the_ledger_gate_runs_on_the_paths_the_ledger_workflow_triggers_on(self):
        with open(os.path.join(ROOT, ".github/workflows/compare-jq-ledger.yml"), encoding="utf-8") as f:
            workflow = f.read()
        triggers = re.search(r"pull_request:\n(?:.*\n)*?\s+paths:\n((?:\s+- '.*'\n)+)", workflow).group(1)
        # `comparison/jq/**` in the workflow is `comparison/jq/*` here (fnmatch's `*` crosses `/`).
        listed = {re.sub(r"\*\*", "*", p) for p in re.findall(r"- '(.*)'", triggers)}
        gate = next(g for g in pre_push.GATES if g.name == "ledger-only")
        for path in gate.paths:
            self.assertIn(path, listed)


class HookRegistrationTests(unittest.TestCase):
    def test_the_checked_in_settings_run_the_hook_on_bash_calls_with_room_to_build(self):
        with open(os.path.join(ROOT, ".claude", "settings.json"), encoding="utf-8") as f:
            settings = json.load(f)
        entries = [
            h
            for group in settings["hooks"]["PreToolUse"]
            if group.get("matcher") == "Bash"
            for h in group["hooks"]
            if h["command"].endswith("/scripts/pre-push.sh hook")
        ]
        self.assertEqual(len(entries), 1)
        # The ledger gate builds a release binary.
        self.assertGreaterEqual(entries[0]["timeout"], 900)


class PlanTests(unittest.TestCase):
    def plan(self, paths, tier="fast", arch="x86_64", skip=()):
        return {g.name: why for g, why in pre_push.plan(paths, tier, arch, set(skip))}

    def test_a_docs_only_diff_needs_no_build_gate(self):
        p = self.plan(["docs/guides/x.md"])
        self.assertIsNotNone(p["fmt"])
        self.assertIsNotNone(p["ledger-only"])

    def test_a_source_change_runs_fmt_and_the_ledger_gate(self):
        p = self.plan(["src/jq/eval.rs"])
        self.assertIsNone(p["fmt"])
        self.assertIsNone(p["ledger-only"])
        self.assertNotIn("clippy", p)

    def test_the_ledger_file_alone_runs_the_static_check_and_the_ledger_gate(self):
        p = self.plan(["comparison/jq/divergences.toml"])
        self.assertIsNone(p["ledger-refs"])
        self.assertIsNone(p["ledger-only"])
        self.assertIsNotNone(p["fmt"])

    def test_full_adds_the_lints_and_arm_only_where_it_applies(self):
        self.assertIsNone(self.plan(["src/lib.rs"], "full")["clippy"])
        self.assertIsNotNone(self.plan(["src/lib.rs"], "full", "x86_64")["clippy-arm-broadword"])
        self.assertIsNone(self.plan(["src/lib.rs"], "full", "arm64")["clippy-arm-broadword"])

    def test_skip_is_reported_not_silent(self):
        self.assertEqual(
            self.plan(["src/lib.rs"], skip=["ledger-only"])["ledger-only"],
            "skipped by PRE_PUSH_SKIP",
        )

    def test_gates_are_ordered_cheapest_first(self):
        order = [g.cost for g, _ in pre_push.plan([], "full", "x86_64", set())]
        self.assertEqual(order, sorted(order))


class PushDetectionTests(unittest.TestCase):
    def pushes(self, command, cwd="/work"):
        return pre_push.pushes(command, cwd)

    def test_plain_and_compound_pushes(self):
        self.assertEqual(self.pushes("git push"), ["/work"])
        self.assertEqual(self.pushes("git push -u origin b"), ["/work"])
        self.assertEqual(self.pushes("cargo test && git push origin b"), ["/work"])
        self.assertEqual(self.pushes("git add -A; git commit -m x; git push --force-with-lease"), ["/work"])

    def test_no_verify_does_not_skip_it(self):
        self.assertEqual(self.pushes("git push --no-verify origin b"), ["/work"])

    def test_dash_c_and_cd_choose_the_repository(self):
        self.assertEqual(self.pushes("git -C /elsewhere push"), ["/elsewhere"])
        self.assertEqual(self.pushes("git -C sub -C deeper push", "/work"), ["/work/sub/deeper"])
        self.assertEqual(self.pushes("git -c user.name=x push"), ["/work"])
        self.assertEqual(self.pushes("cd /other && git push"), ["/other"])

    def test_environment_prefixes_are_skipped(self):
        self.assertEqual(self.pushes("FOO=1 git push"), ["/work"])
        self.assertEqual(self.pushes("env FOO=1 git push"), ["/work"])

    def test_things_that_are_not_a_publishing_push(self):
        for command in (
            "git status",
            "git commit -m 'run git push later'",
            'echo "git push"',
            "git push --delete origin b",
            "git push origin :b",
            "git push -d origin b",
            "git push --dry-run",
            "git stash push -m x",
            "gh pr create",
        ):
            self.assertEqual(self.pushes(command), [], command)

    def test_an_unbalanced_quote_still_finds_the_push(self):
        self.assertEqual(self.pushes("echo 'unterminated; git push"), ["/work"])


class HookTests(unittest.TestCase):
    """The hook against a real repository whose `scripts/pre_push.py` is a stub."""

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.root = os.path.realpath(self.dir.name)
        subprocess.run(["git", "init", "-q", self.root], check=True)
        os.makedirs(os.path.join(self.root, "scripts"))

    def stub(self, exit_code):
        with open(os.path.join(self.root, "scripts", "pre_push.py"), "w", encoding="utf-8") as f:
            f.write(f"import sys\nprint('stub gate output')\nsys.exit({exit_code})\n")

    def call(self, command, tool="Bash", cwd=None):
        payload = json.dumps({"tool_name": tool, "tool_input": {"command": command}, "cwd": cwd or self.root})
        err = io.StringIO()
        old = sys.stderr
        sys.stderr = err
        try:
            code = pre_push.hook(io.StringIO(payload))
        finally:
            sys.stderr = old
        return code, err.getvalue()

    def test_a_failing_gate_blocks_a_push_even_with_no_verify(self):
        self.stub(1)
        for command, cwd in (
            ("git push", self.root),
            ("git push --no-verify", self.root),
            ("true && git push", self.root),
            (f"git -C {self.root} push", "/"),
        ):
            code, message = self.call(command, cwd=cwd)
            self.assertEqual(code, 2, command)
            self.assertIn("stub gate output", message)

    def test_a_passing_gate_lets_the_push_through(self):
        self.stub(0)
        self.assertEqual(self.call("git push")[0], 0)

    def test_commands_that_are_not_pushes_never_run_the_gate(self):
        self.stub(1)
        for command in ("git status", "cargo test", "git push --delete origin b"):
            self.assertEqual(self.call(command)[0], 0, command)
        self.assertEqual(self.call("git push", tool="Edit")[0], 0)

    def test_a_repository_without_the_script_is_not_gated(self):
        other = tempfile.TemporaryDirectory()
        self.addCleanup(other.cleanup)
        subprocess.run(["git", "init", "-q", other.name], check=True)
        self.assertEqual(self.call("git push", cwd=other.name)[0], 0)

    def test_unreadable_input_is_allowed(self):
        self.assertEqual(pre_push.hook(io.StringIO("not json")), 0)


if __name__ == "__main__":
    unittest.main()
