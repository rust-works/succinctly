#!/usr/bin/env python3
"""Both Stop hooks that check skill manifest casing must agree (#5956).

`.claude/hooks/check-skill-casing.sh` and `.codex/hooks/check-skill-casing.sh` are two
files because each reads its own harness's input, but they do one job: block the turn
when a skill manifest is not named exactly `SKILL.md` (#189). `.agents/skills` is a
symlink to `.claude/skills`, so the Codex hook has to look through it.
"""

import os
import shutil
import subprocess
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HOOKS = {
    "claude": os.path.join(ROOT, ".claude", "hooks", "check-skill-casing.sh"),
    "codex": os.path.join(ROOT, ".codex", "hooks", "check-skill-casing.sh"),
}
BLOCKED = 2


class SkillCasingHooks(unittest.TestCase):
    def setUp(self):
        self.repo = tempfile.mkdtemp(prefix="skill-hooks-")
        self.addCleanup(shutil.rmtree, self.repo, ignore_errors=True)
        subprocess.run(["git", "init", "-q", self.repo], check=True)
        os.makedirs(os.path.join(self.repo, ".claude", "skills", "demo"))
        os.makedirs(os.path.join(self.repo, ".agents"))
        os.symlink("../.claude/skills", os.path.join(self.repo, ".agents", "skills"))

    def run_hook(self, name):
        env = dict(os.environ, CLAUDE_PROJECT_DIR=self.repo)
        return subprocess.run(
            ["bash", HOOKS[name]],
            cwd=self.repo,
            env=env,
            input="{}",
            capture_output=True,
            text=True,
        )

    def write_manifest(self, filename):
        path = os.path.join(self.repo, ".claude", "skills", "demo", filename)
        with open(path, "w") as f:
            f.write("---\nname: demo\ndescription: Demo.\n---\n")

    def test_both_hooks_block_a_lowercase_manifest(self):
        self.write_manifest("skill.md")
        for name in HOOKS:
            with self.subTest(hook=name):
                result = self.run_hook(name)
                self.assertEqual(result.returncode, BLOCKED, result.stderr)
                self.assertIn("skill.md", result.stderr)

    def test_both_hooks_pass_a_correctly_named_manifest(self):
        self.write_manifest("SKILL.md")
        for name in HOOKS:
            with self.subTest(hook=name):
                result = self.run_hook(name)
                self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
