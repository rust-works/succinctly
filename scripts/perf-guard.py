#!/usr/bin/env python3
"""CI instruction-count regression guard for `succinctly jq`/`yq` (issue #1523).

#1385's two fix commits made `succinctly jq` 2-3x slower on object-heavy input,
landed on `main`, and were found only because someone happened to re-verify an
unrelated, already-deferred perf issue on a pinned bench box a day later --
nothing in CI would have caught it. Wall-clock timing in CI is the wrong
mechanism (shared runners are noisy neighbours; a threshold loose enough not
to flake is loose enough to let a real 2-3x regression through). This script
uses `valgrind --tool=cachegrind` instead: binary instruction-count
instrumentation, deterministic regardless of what else the runner is doing,
so a real 2-3x work increase shows up as a 2-3x instruction-count increase and
a 5% threshold is meaningful rather than aspirational.

Two modes (both require --arch, since instruction counts are architecture-
specific -- different ISA, different codegen -- so the baseline file holds
one set of counts per arch, not one shared set):

    scripts/perf-guard.py --binary target/release/succinctly --arch x86_64 --check
        Measure the fixed query/shape matrix below, compare each against
        that arch's entry in the committed baseline
        (tests/data/perf-guard-baseline.json), and fail (exit 1) if any
        drifts by more than --threshold percent.

    scripts/perf-guard.py --binary target/release/succinctly --arch x86_64 --update-baseline
        Re-measure and overwrite that arch's entry in the baseline file. Run
        this deliberately, with a stated reason in the commit message --
        never automatically -- or the guard silently ratchets and stops
        catching anything (the exact failure mode this issue was filed to
        prevent). `--arch` must match one of ci.yml's `perf-guard` matrix
        names (currently `x86_64`, `ARM64-Linux`) for the CI job to find it.

The query/shape matrix is #1523's own "minimum viable set": #1514's two
regressions were complementary and each was invisible to the other's own
signal (one showed only in a `wide`-shaped `keys_unsorted` query, the other
only in a plain `.` identity query and would have looked like an *improvement*
under `keys_unsorted` alone) -- so this covers both queries against both a
`wide` (many top-level keys, no nesting) and a `users` (typical small-record)
shape, plus an `arrays` shape (no objects, isolates whether a regression is
object-specific) and one `yq`-mode query (the shared evaluator's cost is
otherwise unverified in yq mode at all).

Fixtures are generated fresh each run (`succinctly json generate --seed
<fixed>`) rather than checked in, so instruction counts stay meaningful
without growing the repo -- generation is deterministic per pattern/seed, so
the *content* measured is stable run to run.

Standard library only; no third-party dependencies.
"""

import argparse
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile

# Fixed seed for every generated fixture -- the whole point of a checked-in
# instruction-count baseline is that the input content doesn't change between
# runs, only the code being measured does.
FIXTURE_SEED = 20260101

# (id, generate-pattern, size, mode, filter) -- ids are the baseline file's
# keys, so renaming one here orphans its old baseline entry (caught by
# `--check`'s own "missing baseline entry" error) rather than silently
# comparing against the wrong row.
QUERIES = [
    ("wide_keys_unsorted", "wide", "2mb", "jq", "keys_unsorted"),
    ("wide_identity", "wide", "2mb", "jq", "."),
    ("users_keys_unsorted", "users", "2mb", "jq", "keys_unsorted"),
    ("users_identity", "users", "2mb", "jq", "."),
    ("arrays_identity", "arrays", "2mb", "jq", "."),
    ("users_yq_keys_unsorted", "users", "2mb", "yq", "keys_unsorted"),
]

IR_PATTERN = re.compile(r"I\s+refs:\s+([\d,]+)")

DEFAULT_THRESHOLD = 5.0


def parse_args(argv=None):
    p = argparse.ArgumentParser(
        description="CI instruction-count regression guard for succinctly jq/yq.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__.split("\n\n", 1)[1],
    )
    p.add_argument("--binary", required=True, help="path to the succinctly binary (release build)")
    p.add_argument("--baseline", default="tests/data/perf-guard-baseline.json",
                    help="path to the checked-in baseline file")
    p.add_argument("--arch", required=True,
                    help="baseline key for this run, e.g. x86_64/ARM64-Linux -- instruction "
                         "counts are architecture-specific (different ISA, different codegen), "
                         "so the baseline file holds one set of counts per arch, not one shared "
                         "set")
    p.add_argument("--valgrind-bin", default="valgrind", help="path to valgrind")
    p.add_argument("--threshold", type=float, default=DEFAULT_THRESHOLD,
                    help="max allowed instruction-count drift, in percent")
    p.add_argument("--reps", type=int, default=3, help="measurement repetitions per query (median reported)")
    mode = p.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="compare against the baseline; exit 1 on drift")
    mode.add_argument("--update-baseline", action="store_true", help="overwrite the baseline file")
    return p.parse_args(argv)


def generate_fixture(binary, pattern, size, seed, out_path):
    subprocess.run(
        [binary, "json", "generate", size, "-p", pattern, "-s", str(seed), "-o", out_path],
        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True,
    )


def run_cachegrind_once(valgrind_bin, binary, mode, filter_expr, fixture_path):
    with tempfile.TemporaryDirectory() as tmp:
        log = os.path.join(tmp, "cg.log")
        cg_out = os.path.join(tmp, "cg.out")
        cmd = [
            valgrind_bin, "--tool=cachegrind", f"--log-file={log}",
            f"--cachegrind-out-file={cg_out}",
            binary, mode, filter_expr, fixture_path,
        ]
        result = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        with open(log) as f:
            text = f.read()
        if result.returncode != 0:
            sys.exit(f"'{' '.join(cmd)}' exited {result.returncode}; cachegrind log:\n{text}")
        m = IR_PATTERN.search(text)
        if not m:
            sys.exit(f"could not find instruction count in cachegrind output:\n{text}")
        return int(m.group(1).replace(",", ""))


def measure_query(args, query_id, pattern, size, mode, filter_expr):
    with tempfile.TemporaryDirectory() as tmp:
        # `yq` accepts JSON input directly (auto-detects by content), so every
        # fixture is plain JSON regardless of which CLI mode measures it.
        fixture_path = os.path.join(tmp, f"{query_id}.json")
        generate_fixture(args.binary, pattern, size, FIXTURE_SEED, fixture_path)
        counts = [
            run_cachegrind_once(args.valgrind_bin, args.binary, mode, filter_expr, fixture_path)
            for _ in range(args.reps)
        ]
    return statistics.median(counts)


def load_baseline_file(path):
    """The whole checked-in file: {arch: {query_id: instructions}}."""
    if not os.path.exists(path):
        return {}
    with open(path) as f:
        return json.load(f)


def save_baseline_file(path, data):
    with open(path, "w") as f:
        json.dump(data, f, indent=2, sort_keys=True)
        f.write("\n")


def main(argv=None):
    args = parse_args(argv)

    if shutil.which(args.valgrind_bin) is None:
        sys.exit(
            f"valgrind not found at {args.valgrind_bin!r} -- install it (e.g. `apt-get install "
            f"valgrind` on Linux) or pass --valgrind-bin. This guard has no valgrind on macOS "
            f"(Apple Silicon isn't supported by valgrind at all), so it only runs on the "
            f"Linux CI legs (issue #1523)."
        )
    if not os.path.exists(args.binary):
        sys.exit(f"binary not found: {args.binary}")

    # Fail fast on a missing/incomplete baseline before running any of the
    # (expensive, cachegrind-instrumented) measurements below. Instruction
    # counts are architecture-specific (different ISA, different codegen),
    # so the baseline file is keyed by `--arch` -- x86_64's counts are not a
    # valid baseline for ARM64-Linux's run or vice versa.
    baseline_file = load_baseline_file(args.baseline)
    baseline = {}
    known_ids = {q[0] for q in QUERIES}
    if args.check:
        baseline = baseline_file.get(args.arch, {})
        missing = known_ids - set(baseline)
        if missing:
            sys.exit(
                f"baseline for arch {args.arch!r} is missing entries for: {sorted(missing)} "
                f"-- run with --update-baseline first (and commit the result)."
            )

    measured = {}
    print(f"{'query':<26} {'instructions':>16}")
    print("-" * 44)
    for query_id, pattern, size, mode, filter_expr in QUERIES:
        ir = measure_query(args, query_id, pattern, size, mode, filter_expr)
        measured[query_id] = ir
        print(f"{query_id:<26} {ir:>16,.0f}")
        sys.stdout.flush()
    print()

    if args.update_baseline:
        baseline_file[args.arch] = measured
        save_baseline_file(args.baseline, baseline_file)
        print(f"Wrote {len(measured)} entries for arch {args.arch!r} to {args.baseline}")
        return 0

    stale = set(baseline) - known_ids
    if stale:
        print(f"NOTE: baseline has stale entries no longer measured: {sorted(stale)}")

    failed = []
    print(f"{'query':<26} {'baseline':>14} {'current':>14} {'drift':>8}")
    print("-" * 66)
    for query_id, _, _, _, _ in QUERIES:
        base = baseline[query_id]
        cur = measured[query_id]
        drift = (cur / base - 1) * 100 if base else float("inf")
        flag = "  <-- FAIL" if abs(drift) > args.threshold else ""
        print(f"{query_id:<26} {base:>14,.0f} {cur:>14,.0f} {drift:>+7.1f}%{flag}")
        if abs(drift) > args.threshold:
            failed.append((query_id, drift))

    print()
    if failed:
        print(f"FAILED: {len(failed)} quer{'y' if len(failed) == 1 else 'ies'} exceeded "
              f"the {args.threshold}% threshold:")
        for query_id, drift in failed:
            print(f"  {query_id}: {drift:+.1f}%")
        print()
        print("If this is a real, understood regression (e.g. new correctness work that "
              "genuinely costs more), re-run with --update-baseline and say why in the "
              "commit message. If it's unexpected, that's exactly what this guard exists "
              "to catch -- see docs/guides/benchmarking.md for how to investigate.")
        return 1

    print("OK: all queries within threshold")
    return 0


if __name__ == "__main__":
    sys.exit(main())
