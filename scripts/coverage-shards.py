#!/usr/bin/env python3
"""Which tests each `Coverage` shard runs (#5711).

.github/workflows/coverage.yml splits the instrumented test run across concurrent jobs, one per
shard below, and combines their lcov reports. This file is the single definition of the split:

    coverage-shards.py shards        the shard names, one per line
    coverage-shards.py args SHARD    the `cargo test` arguments of a shard, space separated
    coverage-shards.py run SHARD     `cargo test` with those arguments (the caller has already
                                     loaded cargo-llvm-cov's `show-env`)

The shards, and why they are cut where they are (times are the #5711 measurement, 4 CPUs):

    jq-cli        `jq_cli_tests` without the tests in scripts/deep-recursion-tests.sh
    jq-cli-heavy  exactly those tests (the 328 s `recurse` cap test is the floor)
    jq-nesting    `jq_library_eval_nesting_depth_tests`: one test, 224 s, alone in its binary
    rest          the lib, the bins and every other test target

`jq-cli` and `jq-cli-heavy` both come from deep-recursion-tests.sh, so a test cannot be in both or
in neither, and a heavy name that goes stale falls out of the skip list and runs in `jq-cli`: no
test is lost to a rename. `rest` is every test target under tests/ that no other shard names, so
a new test file lands in it by construction. Doctests are in no shard: stable `cargo llvm-cov`
does not instrument them (#3778 measured the 52 of them adding no covered line).

To regroup, change SHARDS and the `shard` matrix in coverage.yml together;
scripts/test_coverage_shards.py fails when they differ.
"""

import pathlib
import subprocess
import sys

SCRIPTS = pathlib.Path(__file__).resolve().parent
ROOT = SCRIPTS.parent
DEEP_RECURSION = SCRIPTS / "deep-recursion-tests.sh"

# What the old composite action passed: `test-args: --features cli,simd,regex,serde --workspace`.
COMMON = ["--features", "cli,simd,regex,serde", "--workspace"]

JQ_CLI = "jq_cli_tests"
NESTING = "jq_library_eval_nesting_depth_tests"

SHARDS = ("jq-cli", "jq-cli-heavy", "jq-nesting", "rest")


def test_targets():
    """Every integration-test target cargo auto-discovers: tests/*.rs and tests/*/main.rs.

    Cargo.toml has no `[[test]]` table and does not set `autotests`, which is what makes a
    glob enough; scripts/test_coverage_shards.py compares this with `cargo metadata`.
    """
    tests = ROOT / "tests"
    names = {p.stem for p in tests.glob("*.rs")}
    names |= {p.parent.name for p in tests.glob("*/main.rs")}
    return sorted(names)


def deep_recursion_args(subcommand):
    out = subprocess.run(
        [str(DEEP_RECURSION), subcommand], capture_output=True, text=True, check=True
    )
    return out.stdout.split()


def shard_args(shard):
    if shard == "jq-cli":
        return COMMON + ["--test", JQ_CLI, "--"] + deep_recursion_args("skip-args")
    if shard == "jq-cli-heavy":
        return COMMON + ["--test", JQ_CLI, "--"] + deep_recursion_args("heavy-args")
    if shard == "jq-nesting":
        return COMMON + ["--test", NESTING]
    if shard == "rest":
        others = [t for t in test_targets() if t not in (JQ_CLI, NESTING)]
        return COMMON + ["--lib", "--bins"] + [a for t in others for a in ("--test", t)]
    raise SystemExit(f"unknown shard '{shard}'; expected one of: {', '.join(SHARDS)}")


def main(argv):
    if len(argv) == 2 and argv[1] == "shards":
        print("\n".join(SHARDS))
        return 0
    if len(argv) == 3 and argv[1] == "args":
        print(" ".join(shard_args(argv[2])))
        return 0
    if len(argv) == 3 and argv[1] == "run":
        return subprocess.run(["cargo", "test", *shard_args(argv[2])], check=False).returncode
    print(f"usage: {argv[0]} shards | args SHARD | run SHARD", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
