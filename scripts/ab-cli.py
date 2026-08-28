#!/usr/bin/env python3
"""Interleaved A/B timing for two `succinctly` binaries.

Implements the method in docs/guides/benchmarking.md § A/B Benchmarking Method:
the two binaries alternate *within* each repetition (rule 1), output identity is
gated before any timing is believed (rule 4), inputs below 1 MB are called out
(rule 2), and the machine is checked for idleness and mains power (rule 6).

    scripts/ab-cli.py --before /path/succ-base --after /path/succ-head \\
        --corpus ~/wrk/bench-scratch/mycorpus

`--control` replaces the "after" side with a copy of the "before" binary. Run it
once per machine before trusting any result: it measures what this harness
reports for a change that does not exist, which is the bar a real delta has to
clear. On an idle M4 Pro it reads +0.09% median with a per-row range of
-1.8%..+1.5%, so a single row at +1% means nothing and twenty rows at +1% mean a
regression.

Standard library only; no third-party dependencies.
"""

import argparse
import glob
import hashlib
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

SPAWN_FLOOR_BYTES = 1_000_000  # rule 2: startup is ~4-6 ms, the whole runtime below this


def parse_args(argv=None):
    p = argparse.ArgumentParser(
        description="Interleaved A/B timing for two succinctly binaries.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__.split("\n\n", 1)[1],
    )
    p.add_argument("--before", required=True, help="baseline binary")
    p.add_argument("--after", help="binary under test (omit with --control)")
    p.add_argument("--before-profile",
                   help="build profile --before was built with, e.g. 'codegen-units=1' "
                        "(recorded only, not verified — rule 9)")
    p.add_argument("--after-profile",
                   help="build profile --after was built with; a mismatch against "
                        "--before-profile is warned about (rule 9)")
    p.add_argument("--corpus", help="directory searched recursively for inputs")
    p.add_argument("--files", nargs="*", default=[], help="explicit input files or globs")
    p.add_argument("--tool", default="yq", help="succinctly subcommand (default: yq)")
    p.add_argument("--queries", nargs="*", default=[".", ".[]"], help="queries to time")
    p.add_argument("--output", default="json", help="-o format passed to the tool")
    p.add_argument("--reps", type=int, default=7, help="repetitions per configuration")
    p.add_argument("--control", action="store_true",
                   help="time --before against a copy of itself (noise floor)")
    p.add_argument("--no-identity", action="store_true",
                   help="skip the output identity gate (rule 4) — for control runs")
    p.add_argument("--force", action="store_true",
                   help="run even if the machine looks CPU-busy or has build/benchmark "
                        "processes running (does NOT waive the battery check)")
    p.add_argument("--force-battery", action="store_true",
                   help="run even on battery power — separate from --force (#1640) so "
                        "the one check that's never a false positive can't be waived by "
                        "the same flag that waives the noisy heuristics")
    return p.parse_args(argv)


def collect_inputs(args):
    paths = []
    for pattern in args.files:
        paths.extend(sorted(glob.glob(pattern)))
    if args.corpus:
        for root, _dirs, names in os.walk(args.corpus):
            paths.extend(sorted(os.path.join(root, n) for n in names
                                if n.endswith((".yaml", ".yml", ".json"))))
    seen, unique = set(), []
    for p in paths:
        if p not in seen and os.path.isfile(p):
            seen.add(p)
            unique.append(p)
    return unique


def profile_status(args):
    """Rule 9: codegen-units/LTO placement differences between two binaries can add
    noise unrelated to any code change (issue #1587), and a binary can't be
    introspected after the fact to check — this only reports what the caller asserts."""
    if args.control:
        return None  # a copy of --before always shares its own layout
    before, after = args.before_profile, args.after_profile
    if before and after:
        if before == after:
            return f"profile: {before} (matched)"
        return (f"WARNING: binaries were built with different profiles "
                f"(before={before!r}, after={after!r}) — a measured delta may include a "
                f"codegen-placement artifact, not just the code change. See "
                f"docs/guides/benchmarking.md § A/B Benchmarking Method, rule 9.")
    if before or after:
        return (f"WARNING: build profile given for only one binary "
                f"(before={before!r}, after={after!r}) — pass both or neither. See "
                f"docs/guides/benchmarking.md § A/B Benchmarking Method, rule 9.")
    return ("WARNING: build profile not specified for either binary — codegen-units/LTO "
            "differences between --before and --after can add noise unrelated to any code "
            "change (issue #1587). Pass --before-profile/--after-profile once confirmed. See "
            "docs/guides/benchmarking.md § A/B Benchmarking Method, rule 9.")


def battery_warning():
    """Rule 6: never benchmark a laptop on battery — the one check here that's never a
    false positive, so it gets its own --force-battery flag (#1640) rather than sharing
    --force with the noisier heuristics below."""
    if sys.platform != "darwin":
        return None
    batt = run_text(["pmset", "-g", "batt"])
    if "Battery Power" in batt:
        return "running on battery — never benchmark a laptop unplugged"
    return None


def machine_warnings():
    """Rule 6: read the signals that actually mean 'busy right now', not a proxy that
    only correlates with it. Two prior proxies were tried and rejected here (#1640):
    macOS load average counts uninterruptible-wait threads, and `ps -o pcpu` reports
    each process's *lifetime-average* CPU, not its instantaneous usage — summing that
    across a long-uptime machine gives a number that only ever grows, unrelated to
    current load (a 26-day-uptime idle Mac read 30% "busy" from iTerm2/WindowServer/
    DriverKit's accumulated CPU-seconds alone)."""
    warnings = []
    if sys.platform == "darwin":
        # Discard `top`'s first sample (itself a lifetime average since boot); the
        # second is the instantaneous delta between the two samples.
        top = run_text(["top", "-l", "2", "-n", "0"])
        idle_samples = re.findall(r"([\d.]+)%\s*idle", top)
        if len(idle_samples) >= 2:
            idle = float(idle_samples[-1])
            if idle < 75.0:
                warnings.append(f"{100.0 - idle:.0f}% CPU busy (instantaneous, `top -l 2`)")
    else:
        load1 = os.getloadavg()[0]
        if load1 > 1.0:
            warnings.append(f"1-minute load average is {load1:.2f}")
    # `claude` deliberately excluded: Claude Code's own background helper
    # (`--chrome-native-host`) matches this pattern, sleeping, on any machine with the
    # CLI installed — that fired on every run regardless of load (#1640). Real build
    # tools are a reliable signal on their own; a concurrent agent that's actually
    # compiling shows up as `cargo`/`rustc` here too.
    busy = run_text(["pgrep", "-fl", "cargo|rustc|criterion"])
    if busy.strip():
        warnings.append("build or benchmark processes are running:\n    "
                        + "\n    ".join(busy.strip().splitlines()[:5]))
    return warnings


def run_text(cmd):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, check=False).stdout
    except (OSError, ValueError):
        return ""


def command(binary, args, query, path):
    if args.tool == "yq":
        return [binary, args.tool, "-o", args.output, "-I0", query, path]
    return [binary, args.tool, query, path]


def digest(binary, args, query, path):
    """Hash stdout+stderr in a pipe; capturing 10 MB into memory costs more than the run."""
    h = hashlib.blake2b(digest_size=16)
    proc = subprocess.Popen(command(binary, args, query, path),
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    for chunk in iter(lambda: proc.stdout.read(1 << 16), b""):
        h.update(chunk)
    proc.stdout.close()
    proc.wait()
    return h.hexdigest()


def gate_identity(args, inputs):
    """Rule 4: a faster binary that changed behaviour is not a win."""
    checked = differences = 0
    for path in inputs:
        for query in args.queries:
            checked += 1
            if digest(args.before, args, query, path) != digest(args.after, args, query, path):
                differences += 1
                print(f"  DIFF  {path}  query={query!r}")
    print(f"  identity: {checked} configurations, {differences} differences")
    return differences == 0


def time_once(binary, args, query, path):
    t0 = time.perf_counter()
    subprocess.run(command(binary, args, query, path),
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    return (time.perf_counter() - t0) * 1000.0


def measure(args, query, path):
    """Rule 1: alternate the binaries within each repetition, never in halves."""
    time_once(args.before, args, query, path)  # warm the page cache, discarded
    time_once(args.after, args, query, path)
    before, after = [], []
    for i in range(args.reps):
        if i % 2 == 0:
            before.append(time_once(args.before, args, query, path))
            after.append(time_once(args.after, args, query, path))
        else:
            after.append(time_once(args.after, args, query, path))
            before.append(time_once(args.before, args, query, path))
    return before, after


def main(argv=None):
    args = parse_args(argv)
    control_copy = None
    if args.control:
        if args.after:
            sys.exit("--control replaces --after with a copy of --before; pass only one")
        fd, control_copy = tempfile.mkstemp(prefix="ab-control-")
        os.close(fd)
        shutil.copy2(args.before, control_copy)
        os.chmod(control_copy, 0o755)
        args.after = control_copy
        args.no_identity = True
    elif not args.after:
        sys.exit("--after is required (or use --control)")

    inputs = collect_inputs(args)
    if not inputs:
        sys.exit("no inputs: pass --corpus and/or --files")

    battery = battery_warning()
    if battery:
        print(f"WARNING: {battery}")
        if not args.force_battery:
            sys.exit("machine is on battery; plug in or pass --force-battery")

    warnings = machine_warnings()
    for w in warnings:
        print(f"WARNING: {w}")
    if warnings and not args.force:
        sys.exit("machine does not look idle; fix the above or pass --force")

    small = [p for p in inputs if os.path.getsize(p) < SPAWN_FLOOR_BYTES]
    if small:
        print(f"WARNING: {len(small)} input(s) below 1 MB — process spawn is ~4-6 ms, "
              f"which is most of the runtime at that size (rule 2)")

    print(f"before={args.before}")
    print(f"after ={args.after}" + ("  (copy of --before: noise floor)" if args.control else ""))
    status = profile_status(args)
    if status:
        print(status)
    print(f"inputs={len(inputs)} queries={args.queries} reps={args.reps}")
    print()

    if not args.no_identity:
        print("output identity gate:")
        if not gate_identity(args, inputs):
            sys.exit("outputs differ — fix that before believing any timing (rule 4)")
        print()

    header = (f"{'workload':<44} {'before min':>11} {'after min':>11} {'min d':>7} "
              f"{'before med':>11} {'after med':>11} {'med d':>7}")
    print(header)
    print("-" * len(header))
    deltas = []
    try:
        for path in inputs:
            for query in args.queries:
                before, after = measure(args, query, path)
                bmin, amin = min(before), min(after)
                bmed, amed = statistics.median(before), statistics.median(after)
                dmin = (amin / bmin - 1) * 100
                dmed = (amed / bmed - 1) * 100
                deltas.append(dmed)
                label = f"{os.path.basename(path)} {query}"
                print(f"{label:<44} {bmin:>10.1f}m {amin:>10.1f}m {dmin:>+6.1f}% "
                      f"{bmed:>10.1f}m {amed:>10.1f}m {dmed:>+6.1f}%")
                sys.stdout.flush()
    finally:
        if control_copy:
            os.unlink(control_copy)

    print()
    print(f"median of medians: {statistics.median(deltas):+.2f}%   "
          f"mean: {statistics.mean(deltas):+.2f}%   "
          f"range: {min(deltas):+.1f}%..{max(deltas):+.1f}%   "
          f"({sum(1 for d in deltas if d > 0)}/{len(deltas)} rows slower)")
    print("A single row inside the control's range is noise; a consistent sign across "
          "every row is not.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
