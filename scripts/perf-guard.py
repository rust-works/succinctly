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

`--check` also accepts `--baseline-binary PATH` in place of the default
`--baseline FILE`: instead of comparing against the checked-in JSON, it
measures a second binary (also generated fresh, same as `--binary`) and
compares against that (issue #1582). ci.yml's `perf-guard` job uses this on
`pull_request` runs, building `--baseline-binary` from the PR's own
`git merge-base` -- see "Why a baseline binary, not just the file" below.

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

Known scope limits, deliberate for this minimum-viable-set v1 rather than
oversights -- worth revisiting once this guard has a track record:

- One fixed size (2mb) per query, not a scaling curve across sizes. Catches
  the #1514 shape (a regression visible at a realistic size) but not one
  that only shows up at a different scale.
- The CI job builds with `cargo build --release --features cli`, which does
  not enable the `simd` feature -- so a regression confined to
  `BalancedParens`'s SIMD-accelerated rank/select index build (`src/trees/
  bp.rs`'s NEON/SSE4.1 L1/L2 index builders, gated behind `feature = "simd"`)
  or `src/bits/popcount.rs`'s explicit intrinsics is invisible here, the same
  split `bench`'s own default/simd/portable-popcount 3-way matrix exists to
  separate for `rank_select`.
- The committed `--baseline` file carries a real staleness risk: at this
  project's merge cadence, `Ir` drifts 1-3% across every query within days
  from ordinary accumulated changes alone (measured directly: baseline
  source commit vs. `main` 3 days/147 commits later, 55 of them touching
  `eval.rs`/`json`/`yaml`). This is *not* a `codegen-units=16` artifact --
  pinning `codegen-units=1` on both sides of that same comparison left the
  drift the same size or larger, so #1587's fix for a similar-looking
  wall-clock/icache issue does not apply here. It is why `--baseline-binary`
  exists: a same-run comparison cancels staleness from any cause, instead of
  trying to keep a checked-in file fresh against a fast-moving `main`. `ci.yml`
  now passes `--baseline-binary` on *every* run this job triggers for --
  `pull_request` against the PR's own `git merge-base`, and (#2117) `push`
  against `github.event.before`, the commit `main` pointed to immediately
  before that push -- so the checked-in file only still matters as a
  fallback for the rare case neither can supply a binary (a branch's first
  push, or a `before` SHA this checkout doesn't have), and as a seed for a
  human running this script locally with no comparison binary of their own.

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
# comparing against the wrong row. Several rows share a (pattern, size) --
# `measure_all` below generates each distinct (pattern, size) fixture once
# and reuses it, rather than regenerating an identical file per row.
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

# Per-query threshold overrides for a drift that is real, understood, and
# accepted as the deliberate cost of a correctness fix -- not a regression
# this guard should keep failing on every future run. `length`/`census`
# absorbed a similar ~10% cost for the same reason (#1677's `,`/`:`
# delimiter check) without ever needing an entry here, simply because it
# isn't one of `QUERIES` above; `wide_keys_unsorted` doesn't have that
# option; it's one of the six tracked queries, so its accepted cost needs an
# explicit, narrower threshold instead of silently failing forever.
#
# Measured live on this repo's own CI runners (not a pinned bench box --
# `--baseline-binary`'s same-run merge-base comparison, #1582, is what makes
# this number trustworthy despite that): `wide_keys_unsorted` (159K short
# top-level keys, no nesting) is the one workload with nothing else to
# dilute #1677's two new per-key checks (`key_delimiter_ok`,
# `key_only_value_delimiter_ok`) against -- x86_64 measured +4.8%, already
# under `DEFAULT_THRESHOLD`; ARM64-Linux measured +8.6%, consistently,
# across a redundant-decode fix that halved `key_is_malformed`'s own cost
# without moving this number (both delimiter checks were already on their
# cheapest available scan direction -- see `following_gap_ok`'s own doc
# comment for the +16%-to-noise fix that predates this). 10% leaves headroom
# above the observed ARM64 number while still catching a *further*
# regression on top of this one.
QUERY_THRESHOLDS = {
    "wide_keys_unsorted": 10.0,
}

# argparse wants a plain string for `epilog`; keeping it as a real constant
# (not a slice of `__doc__`) means reflowing the module docstring above can
# never silently truncate or misplace `--help` output.
EPILOG = (
    "The query/shape matrix is #1523's own \"minimum viable set\": #1514's two "
    "regressions were complementary and each was invisible to the other's own "
    "signal (one showed only in a `wide`-shaped `keys_unsorted` query, the "
    "other only in a plain `.` identity query and would have looked like an "
    "*improvement* under `keys_unsorted` alone) -- so this covers both queries "
    "against both a `wide` (many top-level keys, no nesting) and a `users` "
    "(typical small-record) shape, plus an `arrays` shape (no objects, "
    "isolates whether a regression is object-specific) and one `yq`-mode "
    "query (the shared evaluator's cost is otherwise unverified in yq mode "
    "at all)."
)


def positive_int(text):
    value = int(text)
    if value < 1:
        raise argparse.ArgumentTypeError(f"must be >= 1, got {value}")
    return value


def parse_args(argv=None):
    p = argparse.ArgumentParser(
        description="CI instruction-count regression guard for succinctly jq/yq.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=EPILOG,
    )
    p.add_argument("--binary", required=True, help="path to the succinctly binary (release build)")
    baseline_source = p.add_mutually_exclusive_group()
    baseline_source.add_argument("--baseline", default="tests/data/perf-guard-baseline.json",
                    help="path to the checked-in baseline file")
    baseline_source.add_argument("--baseline-binary", default=None,
                    help="path to a second built binary (e.g. the PR's git merge-base) to "
                         "measure and compare against instead of --baseline -- cancels drift "
                         "from any cause shared by both binaries (staleness, codegen shift, "
                         "toolchain version), since both are built the same way in the same "
                         "run. Only valid with --check (issue #1582).")
    p.add_argument("--arch", required=True,
                    help="baseline key for this run, e.g. x86_64/ARM64-Linux -- instruction "
                         "counts are architecture-specific (different ISA, different codegen), "
                         "so the baseline file holds one set of counts per arch, not one shared "
                         "set")
    p.add_argument("--valgrind-bin", default="valgrind", help="path to valgrind")
    p.add_argument("--threshold", type=float, default=DEFAULT_THRESHOLD,
                    help="max allowed instruction-count drift, in percent")
    p.add_argument("--reps", type=positive_int, default=3,
                    help="measurement repetitions per query (median reported)")
    mode = p.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="compare against the baseline; exit 1 on drift")
    mode.add_argument("--update-baseline", action="store_true", help="overwrite the baseline file")
    return p.parse_args(argv)


def generate_fixture(binary, pattern, size, seed, out_path):
    cmd = [binary, "json", "generate", size, "-p", pattern, "-s", str(seed), "-o", out_path]
    result = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    if result.returncode != 0:
        sys.exit(f"'{' '.join(cmd)}' exited {result.returncode}; stderr:\n{result.stderr}")


def run_cachegrind_once(valgrind_bin, binary, mode, filter_expr, fixture_path):
    with tempfile.TemporaryDirectory() as tmp:
        log = os.path.join(tmp, "cg.log")
        cg_out = os.path.join(tmp, "cg.out")
        cmd = [
            valgrind_bin, "--tool=cachegrind", f"--log-file={log}",
            f"--cachegrind-out-file={cg_out}",
            binary, mode, filter_expr, fixture_path,
        ]
        # `--log-file` captures valgrind's own diagnostics; the *traced*
        # binary's stderr is separate and captured too (not discarded) so a
        # failure -- the traced binary erroring on a query it always used to
        # handle, for instance -- reports why, not just an opaque nonzero
        # exit. `LC_ALL=C` pins cachegrind's number formatting so a locale
        # change on the runner can't silently alter how IR_PATTERN below
        # needs to parse it.
        env = {**os.environ, "LC_ALL": "C"}
        result = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
        if result.returncode != 0:
            sys.exit(
                f"'{' '.join(cmd)}' exited {result.returncode}; stderr:\n{result.stderr}"
            )
        # A query that silently starts producing no output (a bug that
        # changes correctness, not just cost) would look exactly like a
        # legitimate speedup -- catch that before the instruction count is
        # ever trusted, rather than only checking the exit code.
        if not result.stdout.strip():
            sys.exit(
                f"'{' '.join(cmd)}' exited 0 but produced no output -- refusing to trust "
                f"its instruction count (a correctness regression can look exactly like a "
                f"speedup)"
            )
        if not os.path.exists(log):
            sys.exit(f"'{' '.join(cmd)}' exited 0 but wrote no cachegrind log at {log}")
        with open(log) as f:
            text = f.read()
        m = IR_PATTERN.search(text)
        if not m:
            sys.exit(f"could not find instruction count in cachegrind output:\n{text}")
        return int(m.group(1).replace(",", ""))


def measure_query(valgrind_bin, binary, mode, filter_expr, fixture_path, reps):
    counts = [
        run_cachegrind_once(valgrind_bin, binary, mode, filter_expr, fixture_path)
        for _ in range(reps)
    ]
    # `statistics.median` returns a float for an even-length input (the
    # average of the two middle values) but an int for odd-length -- round
    # to a plain int either way so the baseline file's schema doesn't flip
    # between int/float purely as a side effect of --reps's parity.
    return round(statistics.median(counts))


def measure_all(binary, valgrind_bin, reps, label="binary"):
    """Generate each distinct (pattern, size) fixture exactly once -- several
    `QUERIES` rows share one, and regenerating an identical file per row
    would be pure waste -- then measure every query against its fixture.
    Returns {query_id: median instruction count}. `label` only affects the
    printed header, distinguishing this run's output in CI logs when
    `measure_all` is called twice (current binary, then --baseline-binary)."""
    with tempfile.TemporaryDirectory() as tmp:
        fixture_paths = {}
        for _, pattern, size, _, _ in QUERIES:
            shape = (pattern, size)
            if shape not in fixture_paths:
                # `succinctly json generate` always writes JSON; the `.json`
                # extension matters for the `yq`-mode query above -- `yq`'s
                # input-format auto-detection is by file extension, not
                # content sniffing, so this is what makes it resolve to JSON
                # rather than falling through to YAML's own default.
                path = os.path.join(tmp, f"{pattern}_{size}.json")
                generate_fixture(binary, pattern, size, FIXTURE_SEED, path)
                fixture_paths[shape] = path

        measured = {}
        print(f"Measuring {label} ({binary}):")
        print(f"{'query':<26} {'instructions':>16}")
        print("-" * 44)
        for query_id, pattern, size, mode, filter_expr in QUERIES:
            ir = measure_query(valgrind_bin, binary, mode, filter_expr, fixture_paths[(pattern, size)], reps)
            measured[query_id] = ir
            print(f"{query_id:<26} {ir:>16,.0f}")
            sys.stdout.flush()
        print()
    return measured


def load_baseline_file(path):
    """The whole checked-in file: {arch: {query_id: instructions}}."""
    if not os.path.exists(path):
        return {}
    with open(path) as f:
        try:
            return json.load(f)
        except json.JSONDecodeError as e:
            sys.exit(f"{path} is not valid JSON ({e}) -- a stale conflict marker from a bad "
                     f"rebase/merge, or a truncated write?")


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
    if not os.access(args.binary, os.X_OK):
        sys.exit(f"binary is not executable: {args.binary} (lost its execute bit in transit?)")

    if args.baseline_binary and args.update_baseline:
        sys.exit("--baseline-binary is only valid with --check -- there's nothing to update "
                 "a checked-in baseline *from* a transient second binary.")
    if args.baseline_binary:
        if not os.path.exists(args.baseline_binary):
            sys.exit(f"baseline binary not found: {args.baseline_binary}")
        if not os.access(args.baseline_binary, os.X_OK):
            sys.exit(f"baseline binary is not executable: {args.baseline_binary} (lost its "
                     f"execute bit in transit?)")

    # Fail fast on a missing/incomplete baseline before running any of the
    # (expensive, cachegrind-instrumented) measurements below. Instruction
    # counts are architecture-specific (different ISA, different codegen),
    # so the baseline file is keyed by `--arch` -- x86_64's counts are not a
    # valid baseline for ARM64-Linux's run or vice versa. None of this
    # applies when `--baseline-binary` is given: `measure_all` below always
    # returns exactly `known_ids`, so there's no file to be stale/incomplete.
    #
    # Loaded whenever `--baseline-binary` isn't in play -- including
    # `--update-baseline` -- because `--update-baseline` merges `measured`
    # into whatever this dict already holds and then writes the whole thing
    # back out. Gating the load on `args.check` left `--update-baseline`
    # starting from `{}` and overwriting the file with only the arch just
    # measured, silently deleting every other arch's entries (#1582).
    baseline_file = {}
    baseline = {}
    known_ids = {q[0] for q in QUERIES}
    if not args.baseline_binary:
        baseline_file = load_baseline_file(args.baseline)
    if args.check and not args.baseline_binary:
        baseline = baseline_file.get(args.arch, {})
        missing = known_ids - set(baseline)
        if missing:
            sys.exit(
                f"baseline for arch {args.arch!r} is missing entries for: {sorted(missing)} "
                f"-- run with --update-baseline first (and commit the result)."
            )

    measured = measure_all(args.binary, args.valgrind_bin, args.reps, label="current binary")

    if args.update_baseline:
        baseline_file[args.arch] = measured
        save_baseline_file(args.baseline, baseline_file)
        print(f"Wrote {len(measured)} entries for arch {args.arch!r} to {args.baseline}")
        return 0

    if args.baseline_binary:
        # A same-run comparison against the PR's own merge-base: whatever
        # staleness the checked-in file would carry (real accumulated drift,
        # codegen shift, toolchain version) is shared by both binaries here
        # and cancels out, leaving only what this binary's diff changed
        # (issue #1582).
        baseline = measure_all(args.baseline_binary, args.valgrind_bin, args.reps,
                                label="baseline binary")

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
        threshold = QUERY_THRESHOLDS.get(query_id, args.threshold)
        flag = "  <-- FAIL" if abs(drift) > threshold else ""
        print(f"{query_id:<26} {base:>14,.0f} {cur:>14,.0f} {drift:>+7.1f}%{flag}")
        if abs(drift) > threshold:
            failed.append((query_id, drift, threshold))

    print()
    if failed:
        print(f"FAILED: {len(failed)} quer{'y' if len(failed) == 1 else 'ies'} exceeded "
              f"its threshold:")
        for query_id, drift, threshold in failed:
            print(f"  {query_id}: {drift:+.1f}% (threshold {threshold}%)")
        print()
        print("A drift in either direction fails: a genuine improvement needs the same "
              "conscious baseline update as a regression, so a stale baseline can't quietly "
              "keep passing. If this is real and understood (new correctness work that "
              "genuinely costs more, or an optimization that genuinely costs less), re-run "
              "with --update-baseline and say why in the commit message. If it's unexpected, "
              "that's exactly what this guard exists to catch -- see "
              "docs/guides/benchmarking.md for how to investigate.")
        return 1

    print("OK: all queries within threshold")
    return 0


if __name__ == "__main__":
    sys.exit(main())
