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
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

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

    def test_the_static_ledger_check_runs_even_when_the_ledger_file_did_not_change(self):
        # A branch that fixes a to-triage issue and forgets the entry touches only source.
        self.assertIsNone(self.plan(["src/jq/eval.rs"])["ledger-refs"])
        self.assertIsNone(self.plan(["docs/x.md"])["ledger-refs"])

    def test_a_ledger_entry_file_alone_runs_the_static_check_and_the_ledger_gate(self):
        p = self.plan(["comparison/jq/divergences/cli/version.toml"])
        self.assertIsNone(p["ledger-refs"])
        self.assertIsNone(p["ledger-only"])
        self.assertIsNotNone(p["fmt"])

    def test_a_generated_doc_block_runs_the_bench_runner_bin_tests(self):
        # A hand edit to a block the compare tests hold to docs/compare/ fails before the push.
        self.assertIsNone(self.plan(["docs/benchmarks/jq.md"])["bench-runner-bin"])
        self.assertIsNone(self.plan(["docs/compare/jq-linux-x86_64.json"])["bench-runner-bin"])
        self.assertIsNone(self.plan(["src/bin/succinctly/compare/docs.rs"])["bench-runner-bin"])
        self.assertIsNotNone(self.plan(["docs/benchmarks/yq.md"])["bench-runner-bin"])
        self.assertIsNotNone(self.plan(["src/jq/eval.rs"])["bench-runner-bin"])

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


class GateEnvironmentTests(unittest.TestCase):
    """A harness `PATH` without rustup's directory must not fail every cargo gate (#5901)."""

    def cargo_home(self, with_cargo=True):
        home = tempfile.TemporaryDirectory()
        self.addCleanup(home.cleanup)
        bin_dir = os.path.join(home.name, "bin")
        os.makedirs(bin_dir)
        if with_cargo:
            stub = os.path.join(bin_dir, "cargo")
            with open(stub, "w") as f:
                f.write("#!/bin/sh\nexit 0\n")
            os.chmod(stub, 0o755)
        return home.name

    def empty_path(self):
        empty = tempfile.TemporaryDirectory()
        self.addCleanup(empty.cleanup)
        return empty.name

    def test_cargo_home_is_put_on_a_path_that_lacks_cargo(self):
        home = self.cargo_home()
        env = pre_push.gate_environment({}, {"PATH": self.empty_path(), "CARGO_HOME": home})
        self.assertEqual(env["PATH"].split(os.pathsep)[0], os.path.join(home, "bin"))
        self.assertIsNotNone(shutil.which("cargo", path=env["PATH"]))

    def test_dot_cargo_in_home_is_the_fallback_without_cargo_home(self):
        home = tempfile.TemporaryDirectory()
        self.addCleanup(home.cleanup)
        os.makedirs(os.path.join(home.name, ".cargo", "bin"))
        stub = os.path.join(home.name, ".cargo", "bin", "cargo")
        open(stub, "w").close()
        os.chmod(stub, 0o755)
        env = pre_push.gate_environment({}, {"PATH": self.empty_path(), "HOME": home.name})
        self.assertEqual(env["PATH"].split(os.pathsep)[0], os.path.join(home.name, ".cargo", "bin"))

    def test_a_path_that_already_has_cargo_is_left_alone(self):
        on_path = self.cargo_home()
        elsewhere = self.cargo_home()
        path = os.path.join(on_path, "bin")
        env = pre_push.gate_environment({}, {"PATH": path, "CARGO_HOME": elsewhere})
        self.assertEqual(env["PATH"], path)

    def test_a_machine_with_no_cargo_stays_without_it_so_the_gate_fails_loudly(self):
        home = self.cargo_home(with_cargo=False)
        path = self.empty_path()
        env = pre_push.gate_environment({}, {"PATH": path, "CARGO_HOME": home})
        self.assertEqual(env["PATH"], path)
        self.assertIsNone(shutil.which("cargo", path=env["PATH"]))

    def test_the_gates_own_variables_are_kept(self):
        env = pre_push.gate_environment({"RUSTDOCFLAGS": "-D warnings"}, {"PATH": self.empty_path()})
        self.assertEqual(env["RUSTDOCFLAGS"], "-D warnings")

    def test_the_hook_run_reaches_a_gate_that_needs_cargo(self):
        """End to end: a gate step `cargo ...` runs under a PATH without cargo and finds it."""
        home = self.cargo_home()
        env = pre_push.gate_environment({}, {"PATH": "/usr/bin:/bin", "CARGO_HOME": home})
        if shutil.which("cargo", path="/usr/bin:/bin"):
            self.skipTest("a system cargo is on the minimal PATH")
        self.assertEqual(subprocess.run(["bash", "-c", "cargo --version"], env=env).returncode, 0)


class PushDetectionTests(unittest.TestCase):
    def pushes(self, command, cwd="/work"):
        return [d for d, _ in pre_push.pushes(command, cwd)]

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

    def test_text_that_mentions_a_push_is_not_one(self):
        for command in (
            'echo "done; git push origin y"',
            "git commit -m \"$(cat <<'EOF'\nfix it\ngit push origin x\nEOF\n)\"",
            "cat <<EOF\ngit push\nEOF",
            "cat <<-'EOF'\n\tgit push\n\tEOF\necho ok",
            "gh pr create --body 'run: git push -u origin b'",
            "echo a\\\n b",
        ):
            self.assertEqual(self.pushes(command), [], command)

    def test_a_push_after_a_heredoc_is_still_found(self):
        self.assertEqual(self.pushes("cat <<EOF\nnot a push\nEOF\ngit push"), ["/work"])

    def test_a_mixed_delete_and_publish_is_a_push_but_a_pure_delete_is_not(self):
        self.assertEqual(self.pushes("git push origin :old main"), ["/work"])
        self.assertEqual(self.pushes("git push origin :old :older"), [])

    def test_wrappers_do_not_hide_the_push(self):
        self.assertEqual(self.pushes("(cd /x && git push)"), ["/x"])
        self.assertEqual(self.pushes('bash -c "git push origin b"'), ["/work"])
        self.assertEqual(self.pushes("sh -c 'cd /y; git push'"), ["/y"])
        self.assertEqual(self.pushes("git push & wait"), ["/work"])

    def test_a_skip_prefix_travels_with_the_push(self):
        self.assertEqual(pre_push.pushes("PRE_PUSH_SKIP=ledger-only git push", "/w"), [("/w", "ledger-only")])
        self.assertEqual(
            pre_push.pushes('PRE_PUSH_SKIP="fmt ledger-only" git push', "/w"), [("/w", "fmt ledger-only")]
        )
        self.assertEqual(pre_push.pushes("git push", "/w"), [("/w", "")])

    def test_an_unbalanced_quote_still_finds_the_push(self):
        self.assertEqual(self.pushes("echo 'unterminated; git push"), ["/work"])

    def test_a_directory_named_by_an_assignment_in_the_same_command_is_resolved_5850(self):
        for command in (
            'WT=/x/y; git -C "$WT" push -u origin b',
            'WT=/x/y; cd "$WT" && git push',
            'WT=/x/y; cd "$WT" && git -C "$WT" push',
            'export WT=/x/y && git -C "$WT" push',
            'declare -x WT=/x/y; git -C "$WT" push',
            'WT=/x/y bash -c \'git -C "$WT" push\'',
            'export WT=/x/y; bash -c \'git -C "$WT" push\'',
            'WT=/x/y; git -C ${WT} push',
            "bash -c 'WT=/x/y; git -C \"$WT\" push'",
            'true; WT=/x && WT=/x/y && git -C "$WT" push',
            'A=/x; B=$A/y; git -C "$B" push',
        ):
            self.assertEqual(self.pushes(command), ["/x/y"], command)
        self.assertEqual(self.pushes('WT=/x; git -C "$WT/sub" push'), ["/x/sub"])
        self.assertEqual(self.pushes('WT=sub; git -C "$WT" push', "/work"), ["/work/sub"])
        # The shell expands a leading `~` in an assignment.
        home = os.path.expanduser("~")
        self.assertEqual(self.pushes('WT=~/wt/a; git -C "$WT" push'), [os.path.join(home, "wt", "a")])

    def test_a_directory_that_names_an_unknown_variable_is_unresolved_5850(self):
        for command in (
            'git -C "$WT" push',
            'cd "$WT" && git push',
            "git -C '$WT' push",
            'WT=$(git rev-parse --show-toplevel); git -C "$WT" push',
            'WT=`pwd`; git -C "$WT" push',
            'WT=/x; git -C "$OTHER" push',
            'cd "$WT"; git -C sub push',
            # The shell expands a prefix assignment's neighbours first, and it ends with its command.
            'WT=/x/y git -C "$WT" push',
            'env WT=/x/y git -C "$WT" push',
            'WT=/x/y git status; git -C "$WT" push',
            # An assignment that may not have run, or ran in a subshell, leaves the variable unknown.
            '(WT=/x/y); git -C "$WT" push',
            'false && WT=/x/y; git -C "$WT" push',
            'WT=/x/y || WT=/x/z; git -C "$WT" push',
            'WT=/x/y | cat; git -C "$WT" push',
            'cargo build && WT=/x/y; git -C "$WT" push',
            # A child shell sees only what was exported.
            "WT=/x/y; bash -c 'git -C \"$WT\" push'",
        ):
            found = pre_push.pushes(command, "/work")
            self.assertEqual(len(found), 1, command)
            self.assertIsInstance(found[0][0], pre_push.Unresolved, command)
        # `$PWD` is the directory the hook was given, then wherever a resolved `cd` went.
        self.assertEqual(self.pushes('git -C "$PWD" push', "/work"), ["/work"])
        self.assertEqual(self.pushes('cd sub; git -C "$PWD" push', "/work"), ["/work/sub"])
        self.assertEqual(self.pushes('cd "$PWD/sub" && git push', "/work"), ["/work/sub"])
        # An absolute path after an unresolved one is a path again.
        self.assertEqual(self.pushes('cd "$WT"; git -C /abs push'), ["/abs"])
        self.assertNotIsInstance(pre_push.pushes('cd "$WT"; git -C /abs push', "/w")[0][0], pre_push.Unresolved)
        # A variable set to something unreadable stops being known.
        found = pre_push.pushes('WT=/x; WT=$(pwd); git -C "$WT" push', "/work")
        self.assertIsInstance(found[0][0], pre_push.Unresolved)


class HookTests(unittest.TestCase):
    """The hook against a real repository whose `scripts/pre_push.py` is a stub."""

    def setUp(self):
        # `hook()` hands the gate a copy of this process's environment, and the push that runs the
        # suite (`PRE_PUSH_SKIP=fmt git push`, whose `script-tests` gate runs it) sets the variable
        # this class tests: start from a clean one (#5910).
        patched = mock.patch.dict(os.environ)
        patched.start()
        self.addCleanup(patched.stop)
        os.environ.pop("PRE_PUSH_SKIP", None)
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

    def test_the_skip_prefix_reaches_the_gate_through_the_environment(self):
        with open(os.path.join(self.root, "scripts", "pre_push.py"), "w", encoding="utf-8") as f:
            f.write("import os, sys\nsys.exit(0 if 'fmt' in os.environ.get('PRE_PUSH_SKIP', '').split() else 1)\n")
        self.assertEqual(self.call("git push")[0], 2)
        self.assertEqual(self.call("PRE_PUSH_SKIP=fmt git push")[0], 0)

    def test_a_push_through_a_shell_variable_runs_the_gate_5850(self):
        self.stub(1)
        # The spelling the global worktree instructions require, from the main checkout's cwd.
        for command in (
            f'WT={self.root}; git -C "$WT" push -u origin b',
            f'WT={self.root}; cd "$WT" && git -C "$WT" rev-parse --abbrev-ref HEAD && git -C "$WT" push -u origin b',
            f'export WT={self.root}; git -C "$WT" push 2>&1',
        ):
            code, message = self.call(command, cwd="/")
            self.assertEqual(code, 2, command)
            self.assertIn("stub gate output", message, command)
        self.stub(0)
        self.assertEqual(self.call(f'WT={self.root}; git -C "$WT" push', cwd="/")[0], 0)

    def test_a_push_in_a_directory_the_hook_cannot_resolve_is_refused_5850(self):
        self.stub(0)  # a passing gate: the refusal is about not knowing where to run it
        for command in ('git -C "$WT" push', 'cd "$WT" && git push', 'WT=$(pwd); git -C "$WT" push'):
            code, message = self.call(command)
            self.assertEqual(code, 2, command)
            self.assertIn("names a shell variable", message, command)
            self.assertIn("$WT", message, command)

    def test_a_push_in_a_directory_that_is_no_repository_is_refused_not_waved_through_5850(self):
        self.stub(0)
        nowhere = tempfile.TemporaryDirectory()
        self.addCleanup(nowhere.cleanup)
        code, message = self.call(f"git -C {nowhere.name} push")
        self.assertEqual(code, 2)
        self.assertIn("cannot find the repository", message)

    def test_an_inherited_skip_variable_does_not_reach_the_stub_gates_5910(self):
        with open(os.path.join(self.root, "scripts", "pre_push.py"), "w", encoding="utf-8") as f:
            f.write("import os, sys\nsys.exit(0 if 'fmt' in os.environ.get('PRE_PUSH_SKIP', '').split() else 1)\n")
        # `setUp` cleared the variable an enclosing push set; the gate sees only what the command says.
        self.assertEqual(self.call("git push")[0], 2)

    def test_unreadable_input_is_allowed(self):
        self.assertEqual(pre_push.hook(io.StringIO("not json")), 0)


if __name__ == "__main__":
    unittest.main()
