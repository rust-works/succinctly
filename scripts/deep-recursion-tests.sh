#!/bin/bash
# The jq_cli_tests whose cost is the native stack being filled, kept out of the
# `cli-gated` CI legs and run in their own `deep-recursion` leg instead (#3698).
#
# Each of the two takes ~100-150 s in a debug build and pins a core; together they
# were a third of the test time of the ~4,500-test `cli-gated` leg on every
# platform. They are intrinsic to the check (ADR-0025's guard refuses once the
# registered 2 GB stack is half spent), so the cost is moved, not reduced.
#
# This file is the single source of truth for the list, for both sides:
#
#   deep-recursion-tests.sh skip-args   prints `--exact --skip <name> ...`, which
#                                       ci.yml appends after `--` on `cli-gated`
#   deep-recursion-tests.sh run         the whole `deep-recursion` leg
#
# Why `run` checks more than `cargo test -- --exact <names>` does: libtest runs
# zero tests and exits 0 when a filter matches nothing (`running 0 tests`,
# `test result: ok. 0 passed`), so a renamed test would leave the new leg green
# and the test running nowhere. `run` therefore (1) fails before running anything
# if a listed name is not a test in jq_cli_tests, and (2) fails afterwards unless
# exactly as many tests passed as are listed, which also catches an `#[ignore]`.
# Because the skip list is generated from the same array, that is enough to keep
# the two legs in step: there is no second list to go stale.
#
# To move another slow test here, add its name to DEEP_RECURSION_TESTS. It must be a
# test in tests/jq_cli_tests.rs: `run` lists and runs that one target only.
#
# Calls `cargo` from PATH; scripts/test_deep_recursion_tests.py puts a fake one first
# on PATH, so its cases can never launch the real suite. Runs on macOS runners, whose
# /bin/bash is 3.2: no mapfile, no associative arrays.

set -euo pipefail

TEST_TARGET=jq_cli_tests

DEEP_RECURSION_TESTS=(
  test_recursion_refuses_before_the_native_stack_runs_out_3262
  test_composed_recursion_across_two_defs_errors_not_aborts_1371
)

usage() {
  echo "usage: $0 skip-args|run" >&2
}

skip_args() {
  local args="--exact" name
  for name in "${DEEP_RECURSION_TESTS[@]}"; do
    args="$args --skip $name"
  done
  echo "$args"
}

run() {
  local expected="${#DEEP_RECURSION_TESTS[@]}"
  local listing output name missing=0 passed
  # Not `local`: the EXIT trap fires after this function has returned, when a
  # local would be unset (and `set -u` would turn that into a failure).
  workdir="$(mktemp -d)"
  trap 'rm -rf "$workdir"' EXIT
  listing="$workdir/listing"
  output="$workdir/output"

  cargo test --features cli --test "$TEST_TARGET" -- --list > "$listing"
  for name in "${DEEP_RECURSION_TESTS[@]}"; do
    # grep reads the file, not a pipe: with `pipefail`, `grep -q` exiting on its
    # first match could SIGPIPE a producer and read as "not found".
    if ! grep -Fxq "$name: test" "$listing"; then
      echo "::error::$name is not a test in tests/$TEST_TARGET.rs; rename it in scripts/deep-recursion-tests.sh too (#3698)"
      missing=1
    fi
  done
  if [ "$missing" -ne 0 ]; then
    exit 1
  fi

  # `pipefail` makes this pipeline's status cargo's, so a failing test fails the leg.
  # `--color never`: libtest wraps `ok` in escape codes when told to colour (and
  # ci.yml sets CARGO_TERM_COLOR=always), which would hide the result line below.
  cargo test --features cli --test "$TEST_TARGET" -- --color never --exact "${DEEP_RECURSION_TESTS[@]}" 2>&1 | tee "$output"

  passed="$(sed -n 's/^test result: ok\. \([0-9][0-9]*\) passed.*/\1/p' "$output")"
  if [ "$passed" != "$expected" ]; then
    echo "::error::expected $expected deep-recursion tests to pass, the harness reported ${passed:-none}; is one #[ignore]d or cfg'd out? (#3698)"
    exit 1
  fi
}

case "${1:-}" in
  skip-args) skip_args ;;
  run) run ;;
  *) usage; exit 2 ;;
esac
