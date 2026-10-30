#!/usr/bin/env python3
"""Run the CI gates a diff can fail before it is pushed (#5585).

    scripts/pre-push.sh [run] [--full] [--base REF] [--only GATE ...]
    scripts/pre-push.sh list
    scripts/pre-push.sh hook          # Claude Code PreToolUse hook: reads the tool call on stdin

`run` maps the paths changed since the merge base with origin/main to the gates in GATES and runs
the ones that apply, cheapest first. A failing gate stops the more expensive ones: fix it first.
The default `fast` tier is what the hook runs on every agent push; `--full` adds the lints CI runs
(three clippy invocations, rustdoc with warnings denied, the `--no-default-features` build), which
take minutes. The CI test matrices are `scripts/test.sh`'s job and are not mirrored.

Every command in `.github/workflows/*.yml` that looks like a gate is either mirrored by a gate here
(`Gate.mirrors`) or listed in NOT_MIRRORED with the reason. `scripts/test_pre_push.py` fails when
one is neither, so a new CI gate cannot be added without deciding what the push gate does about it.

`PRE_PUSH_SKIP="ledger-only fmt"` skips the named gates; a skip is always printed.

`hook` blocks a `git push` (exit 2, the reason on stderr) when `run` fails. It looks at the command
itself, so it also sees `git -C DIR push`, a push inside `a && b`, and a push with `--no-verify`
(which only skips git's own hooks). It lets through a delete, a dry run and any repository without
this script.
"""

import fnmatch
import json
import os
import platform
import re
import shlex
import subprocess
import sys
from dataclasses import dataclass, field

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_BASE = "origin/main"

# The paths `.github/workflows/compare-jq-ledger.yml` triggers on.
LEDGER_PATHS = (
    "comparison/jq/*",
    "src/*",
    "Cargo.toml",
    "Cargo.lock",
    "scripts/compare-jq-build.sh",
)
RUST_PATHS = ("*.rs", "Cargo.toml", "Cargo.lock", "rust-toolchain*", "clippy.toml", ".cargo/*")

RELEASE_FEATURES = "cli,simd,regex,serde,bench-runner"  # comparison/jq/manifest.toml


@dataclass(frozen=True)
class Gate:
    name: str
    tier: str  # "fast" or "full"
    steps: tuple  # shell command strings, run in order with `bash -c`
    mirrors: tuple  # regexes for the workflow commands this gate stands in for
    paths: tuple = ()  # fnmatch globs of changed files that make it apply; empty: always
    cost: int = 0  # ordering: a failure stops every gate with a higher cost
    arch: tuple = ()  # machine architectures it applies to; empty: all
    env: dict = field(default_factory=dict)


GATES = (
    Gate(
        "ledger-refs",
        "fast",
        ("python3 scripts/ledger_refs.py check --base {base}",),
        (r"^python3 scripts/ledger_refs\.py check",),
        paths=("comparison/jq/*",),
        cost=0,
    ),
    Gate(
        "script-tests",
        "fast",
        ("for t in scripts/test_*.py; do python3 \"$t\" >/dev/null 2>&1 || {{ python3 \"$t\"; exit 1; }}; done",),
        (r"^python3 scripts/test_\w+\.py",),
        paths=("scripts/*.py", ".github/workflows/*"),
        cost=0,
    ),
    Gate(
        "changelog",
        "fast",
        ("python3 scripts/changelog.py check",),
        (r"^python3 scripts/changelog\.py check$",),
        paths=("changelog.d/*", "scripts/changelog.py"),
        cost=0,
    ),
    Gate(
        "doc-lines",
        "fast",
        ("python3 scripts/doc_lines.py check",),
        (r"^python3 scripts/doc_lines\.py check$",),
        paths=("docs/compliance/*",),
        cost=0,
    ),
    Gate(
        "fmt",
        "fast",
        ("cargo fmt --all -- --check",),
        (r"^cargo fmt --all -- --check$",),
        paths=RUST_PATHS,
        cost=1,
    ),
    Gate(
        "ledger-only",
        "fast",
        (
            f"cargo build --release --features {RELEASE_FEATURES}",
            "{target}/release/succinctly dev compare jq --ledger-only --out .ai/scratch/pre-push-ledger",
        ),
        (
            rf"^cargo build --release --features {re.escape(RELEASE_FEATURES)}$",
            r"^\./target/release/succinctly dev compare jq --quick",
        ),
        paths=LEDGER_PATHS,
        cost=3,
    ),
    Gate(
        "clippy",
        "full",
        ("cargo clippy --all-targets --all-features -- -D warnings",),
        (r"^cargo clippy --all-targets --all-features -- -D warnings$",),
        paths=RUST_PATHS,
        cost=4,
    ),
    Gate(
        "clippy-simd-backends",
        "full",
        (
            'source scripts/lib/clippy-variants.sh && cargo clippy --all-targets --features "$CLIPPY_CORE_FEATURES" -- -D warnings',
        ),
        (r'^cargo clippy --all-targets --features "\$CLIPPY_CORE_FEATURES" -- -D warnings$',),
        paths=RUST_PATHS,
        cost=4,
    ),
    Gate(
        "clippy-arm-broadword",
        "full",
        (
            'source scripts/lib/clippy-variants.sh && cargo clippy --all-targets --features "$CLIPPY_ARM_BROADWORD_FEATURES" -- -D warnings',
        ),
        (r'^cargo clippy --all-targets --features "\$CLIPPY_ARM_BROADWORD_FEATURES" -- -D warnings$',),
        paths=RUST_PATHS,
        cost=4,
        arch=("aarch64", "arm64"),
    ),
    Gate(
        "rustdoc",
        "full",
        ("cargo doc --no-deps --all-features",),
        (r"^cargo doc --no-deps --all-features$",),
        paths=RUST_PATHS,
        cost=4,
        env={"RUSTDOCFLAGS": "-D warnings"},
    ),
    Gate(
        "no-default-features",
        "full",
        ("cargo test --no-default-features",),
        (r"^cargo test --no-default-features --verbose$",),
        paths=RUST_PATHS,
        cost=5,
    ),
)

# Workflow commands the push gate deliberately does not run, each with why.
NOT_MIRRORED = (
    (r"^cargo test\b", "the test matrices (per architecture and feature set) are scripts/test.sh's job"),
    (r"^cargo build\b", "builds a binary for a later step; a gate that needs one builds it itself"),
    (r"^cargo bench\b", "scheduled benchmarks"),
    (r"^cargo check --manifest-path fuzz/", "compiles the separate fuzz crate; CI-only"),
    (r"^\./scripts/sync-[\w-]+\.sh", "golden and corpus drift checks need the pinned yq/jq/corpus tooling CI installs"),
    (r"^python3 scripts/perf-guard\.py", "needs valgrind and a release binary on Linux"),
    (r"^python3 scripts/changelog\.py check-pr", "needs the PR's title, body and labels"),
    (r"^\./target/release/succinctly dev compare jq", "the full report (compare-jq.yml); the ledger-fed subset is the ledger-only gate"),
)

PREFIXES = re.compile(
    r"^(cargo (fmt|clippy|doc|check|test|build|bench)\b|python3 scripts/|\./?scripts/|\./target/release/succinctly)"
)


def git(*args, cwd=ROOT):
    out = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, check=False)
    if out.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)}: {out.stderr.strip()}")
    return out.stdout


def changed_paths(base, cwd=ROOT):
    return [p for p in git("diff", "--name-only", f"{base}..HEAD", cwd=cwd).splitlines() if p]


def applies(gate, paths, arch):
    if gate.arch and arch not in gate.arch:
        return False
    return not gate.paths or any(fnmatch.fnmatch(p, g) for p in paths for g in gate.paths)


def target_dir(root):
    return os.environ.get("CARGO_TARGET_DIR") or os.path.join(root, "target")


def plan(paths, tier, arch, skip):
    """`[(gate, reason or None)]`: reason is why a gate of the tier will not run."""
    out = []
    for g in sorted(GATES, key=lambda g: g.cost):
        if tier == "fast" and g.tier != "fast":
            continue
        if g.name in skip:
            out.append((g, "skipped by PRE_PUSH_SKIP"))
        elif not applies(g, paths, arch):
            out.append((g, "no changed path needs it"))
        else:
            out.append((g, None))
    return out


def run_gates(root, base, tier):
    arch = platform.machine().lower()
    skip = set(os.environ.get("PRE_PUSH_SKIP", "").split())
    paths = changed_paths(base, root)
    dirty = git("status", "--porcelain", cwd=root).strip()
    print(f"pre-push: {len(paths)} file(s) changed since {base}, tier {tier}" + (" (working tree is dirty: gates see it)" if dirty else ""))
    failed = []
    for gate, reason in plan(paths, tier, arch, skip):
        if reason:
            print(f"  - {gate.name}: not run ({reason})")
            continue
        if failed and gate.cost > min(f.cost for f in failed):
            print(f"  - {gate.name}: not run (a cheaper gate failed: {', '.join(f.name for f in failed)})")
            continue
        print(f"  * {gate.name}", flush=True)
        env = dict(os.environ, **gate.env)
        for step in gate.steps:
            cmd = step.format(base=base, target=target_dir(root))
            print(f"    $ {cmd}", flush=True)
            if subprocess.run(["bash", "-c", cmd], cwd=root, env=env).returncode != 0:
                failed.append(gate)
                print(f"  ! {gate.name} FAILED", flush=True)
                break
    if failed:
        print(f"pre-push: FAILED: {', '.join(g.name for g in failed)}")
        print('pre-push: to skip a gate on purpose, set PRE_PUSH_SKIP="<gate> ..." (see scripts/pre_push.py)')
        return 1
    print("pre-push: ok")
    return 0


# --- the hook ---------------------------------------------------------------

_ENV_ASSIGN = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")
_GLOBAL_WITH_ARG = {"-C", "-c", "--git-dir", "--work-tree", "--namespace", "--exec-path"}


def _segments(command):
    return [s for s in re.split(r"&&|\|\||[;|\n]", command) if s.strip()]


def pushes(command, cwd):
    """`[directory]` for each `git push` in `command` that publishes refs; a segment that only
    deletes, or only simulates, is not one. Best effort: quotes the shell would expand are not."""
    found = []
    for segment in _segments(command):
        try:
            tokens = shlex.split(segment)
        except ValueError:
            tokens = segment.split()
        while tokens and (_ENV_ASSIGN.match(tokens[0]) or tokens[0] in ("env", "command", "time", "exec")):
            tokens = tokens[1:]
        if tokens[:1] == ["cd"] and len(tokens) > 1:
            cwd = os.path.normpath(os.path.join(cwd, os.path.expanduser(tokens[1])))
            continue
        if tokens[:1] != ["git"]:
            continue
        directory, i = cwd, 1
        while i < len(tokens) and tokens[i].startswith("-"):
            opt = tokens[i]
            if opt == "-C" and i + 1 < len(tokens):
                directory = os.path.normpath(os.path.join(directory, os.path.expanduser(tokens[i + 1])))
            i += 2 if opt in _GLOBAL_WITH_ARG else 1
        if i >= len(tokens) or tokens[i] != "push":
            continue
        rest = tokens[i + 1 :]
        if any(t in ("--delete", "-d", "--dry-run", "-n") or t.startswith(":") for t in rest):
            continue
        found.append(directory)
    return found


def hook(stdin):
    try:
        call = json.load(stdin)
    except ValueError:
        return 0
    if call.get("tool_name") != "Bash":
        return 0
    command = (call.get("tool_input") or {}).get("command") or ""
    cwd = call.get("cwd") or os.getcwd()
    for directory in dict.fromkeys(pushes(command, cwd)):
        try:
            root = git("rev-parse", "--show-toplevel", cwd=directory).strip()
        except (RuntimeError, OSError):
            continue
        script = os.path.join(root, "scripts", "pre_push.py")
        if not os.path.exists(script):
            continue
        result = subprocess.run(
            [sys.executable, script, "run"], cwd=root, capture_output=True, text=True, check=False
        )
        if result.returncode != 0:
            sys.stderr.write(
                f"git push blocked: scripts/pre-push.sh fails in {root}. Fix it and push again.\n\n"
                + (result.stdout + result.stderr)[-6000:]
            )
            return 2
    return 0


# --- the command line -------------------------------------------------------


def main(argv):
    cmd = argv[0] if argv and not argv[0].startswith("-") else "run"
    rest = argv[1:] if argv and not argv[0].startswith("-") else argv
    if cmd == "hook":
        return hook(sys.stdin)
    if cmd == "list":
        for g in sorted(GATES, key=lambda g: g.cost):
            where = ", ".join(g.paths) if g.paths else "always"
            print(f"{g.name:22} {g.tier:5} {where}")
        return 0
    if cmd != "run":
        print(__doc__, file=sys.stderr)
        return 2
    tier, base, i = "fast", None, 0
    while i < len(rest):
        if rest[i] == "--full":
            tier = "full"
        elif rest[i] == "--base" and i + 1 < len(rest):
            i += 1
            base = rest[i]
        else:
            print(f"pre-push: unknown argument {rest[i]!r}", file=sys.stderr)
            return 2
        i += 1
    try:
        if base is None:
            base = git("merge-base", "HEAD", DEFAULT_BASE).strip()
        return run_gates(ROOT, base, tier)
    except RuntimeError as e:
        print(f"pre-push: {e}\npre-push: `git fetch origin main` or pass --base", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
