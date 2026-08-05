#!/usr/bin/env python3
"""Interleaved instructions/cache-miss A/B for two `block_scalars_harness` binaries.

Companion to scripts/ab-cli.py, built for issue #595: distinguish "same work, worse
code layout" from "a real logic regression" for the yaml/block_scalars build-time
delta between pre- and post-#64 (CS-Poppy) `main`. Wall-clock A/B can't tell those
apart; this drives instruction/cache-miss counters instead, which don't move for a
pure layout shift the way cycles do.

Default tool is `cachegrind` (valgrind's binary-instrumentation instruction/cache
simulator): it needs no hardware perf-counter access, which matters because the
pinned x86 bench host (terminus, WSL2) cannot use `perf stat` counters. Pass
`--tool perf` on a host where `perf stat -e instructions,cycles` works instead, or
`--tool wallclock` (min/median ms, no extra tool needed — e.g. on the ARM cross-check
box, which has neither valgrind nor perf) to reproduce the original criterion-based
signal with this same harness/interleaving.

Build the two binaries first (see docs/guides/benchmarking.md § "Building both
halves on a remote box" for the reverse-patch/tarball recipe, or just check out
and build the two commits directly):

    cargo build --release --example block_scalars_harness   # in each checkout

Then:

    scripts/perf-ab.py --before ./succ-before/target/release/examples/block_scalars_harness \\
        --after ./succ-after/target/release/examples/block_scalars_harness

Standard library only; no third-party dependencies.
"""

import argparse
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

# All seven yaml/block_scalars ids from benches/yaml_bench.rs.
ALL_SHAPES = [
    "10x10lines",
    "50x50lines",
    "100x100lines",
    "10x1000lines",
    "long_10x100lines",
    "long_50x100lines",
    "long_100x100lines",
]

CACHEGRIND_PATTERNS = {
    "Ir": re.compile(r"I\s+refs:\s+([\d,]+)"),
    "I1mr": re.compile(r"I1\s+misses:\s+([\d,]+)"),
    "ILmr": re.compile(r"LLi misses:\s+([\d,]+)"),
}


def parse_args(argv=None):
    p = argparse.ArgumentParser(
        description="Interleaved instructions/cache-miss A/B for block_scalars_harness.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__.split("\n\n", 1)[1],
    )
    p.add_argument("--before", required=True, help="baseline harness binary")
    p.add_argument("--after", help="binary under test (omit with --control)")
    p.add_argument("--tool", choices=["cachegrind", "perf", "wallclock"], default="cachegrind")
    p.add_argument("--valgrind-bin", default="valgrind",
                   help="path to valgrind (e.g. a rootless ~/local/bin/valgrind build)")
    p.add_argument("--perf-bin", default="perf", help="path to perf")
    p.add_argument("--shapes", nargs="*", default=ALL_SHAPES,
                   help="block_scalars shape ids to run (default: all seven)")
    p.add_argument("--iterations", type=int, default=50,
                   help="YamlIndex::build iterations per harness invocation "
                        "(amortizes process-startup and instrumentation overhead)")
    p.add_argument("--reps", type=int, default=5, help="repetitions per shape")
    p.add_argument("--control", action="store_true",
                   help="measure --before against a copy of itself (noise floor)")
    p.add_argument("--no-identity", action="store_true",
                   help="skip the checksum identity gate — for control runs")
    p.add_argument("--force", action="store_true",
                   help="run even if the machine looks busy")
    return p.parse_args(argv)


def machine_warnings():
    """Rule 6 (docs/guides/benchmarking.md): on Linux the load average is trustworthy."""
    warnings = []
    if sys.platform != "darwin":
        load1 = os.getloadavg()[0]
        if load1 > 1.0:
            warnings.append(f"1-minute load average is {load1:.2f}")
    # "perf stat"/"perf record", not bare "perf" — a bare pattern self-matches this
    # script's own name (perf-ab.py) via `pgrep -f`.
    raw = subprocess.run(["pgrep", "-fl", "cargo|rustc|criterion|claude|valgrind|perf (stat|record)"],
                         capture_output=True, text=True, check=False).stdout
    # `-f` matches the full cmdline (which, over Tailscale SSH, is a "tailscaled be-child
    # ssh ... --cmd=<this remote command>" wrapper whose own argv contains the full command
    # being run — including this search pattern itself, so it always self-matches) but `-l`
    # only *prints* the short comm name, which is just "tailscaled" — never a genuine
    # competing benchmark process, so it's always safe to drop from this check. Also drop
    # our own PID: --valgrind-bin/--perf-bin's path literally contains "valgrind"/"perf".
    own_pid = str(os.getpid())
    busy = "\n".join(line for line in raw.splitlines()
                     if line.split(None, 1)[1:2] != ["tailscaled"]
                     and line.split(None, 1)[:1] != [own_pid])
    if busy.strip():
        warnings.append("build/agent/profiler processes are running:\n    "
                        + "\n    ".join(busy.strip().splitlines()[:5]))
    return warnings


def run_checksum(binary, shape, iterations):
    """Run the harness once, plainly, to read its stdout summary line."""
    out = subprocess.run([binary, shape, str(iterations)],
                         capture_output=True, text=True, check=True).stdout.strip()
    m = re.search(r"checksum=(\d+)", out)
    if not m:
        sys.exit(f"could not parse harness output: {out!r}")
    return out, int(m.group(1))


def gate_identity(args, shapes):
    """A faster binary that built a different index is not a win."""
    checked = differences = 0
    for shape in shapes:
        checked += 1
        _, before_cksum = run_checksum(args.before, shape, args.iterations)
        _, after_cksum = run_checksum(args.after, shape, args.iterations)
        if before_cksum != after_cksum:
            differences += 1
            print(f"  DIFF  {shape}  before={before_cksum} after={after_cksum}")
    print(f"  identity: {checked} shapes, {differences} differences")
    return differences == 0


def run_cachegrind(valgrind_bin, binary, shape, iterations):
    with tempfile.TemporaryDirectory() as tmp:
        log = os.path.join(tmp, "cg.log")
        cmd = [valgrind_bin, "--tool=cachegrind", "--cache-sim=yes", f"--log-file={log}",
               "--cachegrind-out-file=" + os.path.join(tmp, "cg.out"),
               binary, shape, str(iterations)]
        subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
        with open(log) as f:
            text = f.read()
    result = {}
    for key, pattern in CACHEGRIND_PATTERNS.items():
        m = pattern.search(text)
        if not m:
            sys.exit(f"could not find {key} in cachegrind output:\n{text}")
        result[key] = int(m.group(1).replace(",", ""))
    return result


def run_perf(perf_bin, binary, shape, iterations):
    with tempfile.TemporaryDirectory() as tmp:
        out = os.path.join(tmp, "perf.csv")
        cmd = [perf_bin, "stat", "-x", ",", "-e", "instructions,cycles", "-o", out,
               "--", binary, shape, str(iterations)]
        subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
        with open(out) as f:
            rows = [line.strip().split(",") for line in f if line.strip() and not line.startswith("#")]
    result = {}
    for row in rows:
        value, event = row[0], row[2]
        if event in ("instructions", "cycles"):
            result[event] = int(value)
    if "instructions" not in result or "cycles" not in result:
        sys.exit(f"could not parse perf stat output:\n{rows}")
    return {"Ir": result["instructions"], "cycles": result["cycles"]}


def run_wallclock(binary, shape, iterations):
    t0 = time.perf_counter()
    subprocess.run([binary, shape, str(iterations)],
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    return {"ms": (time.perf_counter() - t0) * 1000.0}


def measure_once(args, binary, shape):
    if args.tool == "cachegrind":
        return run_cachegrind(args.valgrind_bin, binary, shape, args.iterations)
    if args.tool == "perf":
        return run_perf(args.perf_bin, binary, shape, args.iterations)
    return run_wallclock(binary, shape, args.iterations)


def measure(args, shape):
    """Alternate the binaries within each repetition (never in halves)."""
    measure_once(args, args.before, shape)  # warm up, discarded
    measure_once(args, args.after, shape)
    before, after = [], []
    for i in range(args.reps):
        if i % 2 == 0:
            before.append(measure_once(args, args.before, shape))
            after.append(measure_once(args, args.after, shape))
        else:
            after.append(measure_once(args, args.after, shape))
            before.append(measure_once(args, args.before, shape))
    return before, after


def agg_min(runs, key):
    return min(r[key] for r in runs)


def agg_median(runs, key):
    return statistics.median(r[key] for r in runs)


def metric_specs(tool):
    """(column label, dict key, aggregator) triples — one per report column. cachegrind
    reports medians of three counters (instructions, L1 and LL icache misses — L1 is the
    one a hot-loop layout/alignment shift actually stresses; LL is included too since it's
    free from the same run). perf reports instructions and cycles. wallclock has only one
    counter (ms), so it reports min and median of the same field instead, matching
    scripts/ab-cli.py's convention."""
    if tool == "cachegrind":
        return [("Ir", "Ir", agg_median), ("I1mr", "I1mr", agg_median), ("ILmr", "ILmr", agg_median)]
    if tool == "perf":
        return [("Ir", "Ir", agg_median), ("cycles", "cycles", agg_median)]
    return [("min ms", "ms", agg_min), ("median ms", "ms", agg_median)]


def main(argv=None):
    args = parse_args(argv)
    control_copy = None
    if args.control:
        if args.after:
            sys.exit("--control replaces --after with a copy of --before; pass only one")
        fd, control_copy = tempfile.mkstemp(prefix="perf-ab-control-")
        os.close(fd)
        shutil.copy2(args.before, control_copy)
        os.chmod(control_copy, 0o755)
        args.after = control_copy
        args.no_identity = True
    elif not args.after:
        sys.exit("--after is required (or use --control)")

    if args.tool == "cachegrind" and shutil.which(args.valgrind_bin) is None:
        sys.exit(f"valgrind not found at {args.valgrind_bin!r} — install it, pass --valgrind-bin, "
                 f"or use --tool perf")
    if args.tool == "perf" and shutil.which(args.perf_bin) is None:
        sys.exit(f"perf not found at {args.perf_bin!r} — pass --perf-bin or use --tool cachegrind")

    warnings = machine_warnings()
    for w in warnings:
        print(f"WARNING: {w}")
    if warnings and not args.force:
        sys.exit("machine does not look idle; fix the above or pass --force")

    print(f"before={args.before}")
    print(f"after ={args.after}" + ("  (copy of --before: noise floor)" if args.control else ""))
    print(f"tool={args.tool} shapes={args.shapes} iterations={args.iterations} reps={args.reps}")
    print()

    if not args.no_identity:
        print("checksum identity gate:")
        if not gate_identity(args, args.shapes):
            sys.exit("checksums differ — fix that before believing any instruction-count delta")
        print()

    specs = metric_specs(args.tool)
    col = " ".join(f"{'before ' + lbl:>14} {'after ' + lbl:>14} {lbl + ' d':>7}" for lbl, _, _ in specs)
    header = f"{'shape':<20} {col}"
    print(header)
    print("-" * len(header))
    deltas = [[] for _ in specs]
    try:
        for shape in args.shapes:
            before, after = measure(args, shape)
            row = [shape]
            for i, (lbl, key, agg) in enumerate(specs):
                b, a = agg(before, key), agg(after, key)
                d = (a / b - 1) * 100 if b else float("nan")
                deltas[i].append(d)
                row.append(f"{b:>14,.2f} {a:>14,.2f} {d:>+6.1f}%")
            print(f"{row[0]:<20} " + " ".join(row[1:]))
            sys.stdout.flush()
    finally:
        if control_copy:
            os.unlink(control_copy)

    print()
    for (lbl, _, _), ds in zip(specs, deltas):
        print(f"{lbl} median delta: {statistics.median(ds):+.2f}%   "
              f"range: {min(ds):+.1f}%..{max(ds):+.1f}%")
    print()
    if args.tool == "wallclock":
        print("Wall-clock only — cannot distinguish layout artifact from real extra work.")
        print("Use --tool cachegrind/perf on x86 to tell those apart (issue #595).")
    else:
        label1 = specs[0][0]
        rest = "/".join(s[0] for s in specs[1:])
        print(f"{label1} ~flat + {rest} moving = layout/cache artifact (issue #595's theory).")
        print(f"{label1} moving with {rest} = real extra work — reopen #64's Step-B decision.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
