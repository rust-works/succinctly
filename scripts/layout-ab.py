#!/usr/bin/env python3
"""Layout-randomized interleaved A/B for `succinctly jq` binaries (spike).

Each binary is named `GROUP.sSEED` (or `GROUP` alone for the unseeded link), for example
`A.s3=/x/a-3/succinctly`. Binaries with the same seed were linked the same way
(scripts/link-order/relink.py), so `B.s3 / A.s3` is one paired comparison, and the spread of
`A.s*` alone is what placement does to a binary whose code is fixed.

    scripts/layout-ab.py --row strings_upcase --input strings-10mb.json --reps 15 \\
        --bin A=... --bin B=... --bin A.s1=... --bin B.s1=... ... --csv out.csv

Per repetition every binary runs once, in an order shuffled per repetition (rule 1 of
docs/guides/benchmarking.md interleaves two binaries; here it is all of them). Output identity
and exit status are gated first: every binary must print the same bytes and exit 0. Wall time
is the child's elapsed time (`time.perf_counter_ns` around `subprocess.run`).

Reported per pair of groups, say A and B:
- naive: B/A from the two unseeded links, which is what a plain A/B would have said;
- layout spread: the range and standard deviation of the `A.s*` medians, relative to their mean;
- paired: the mean of `B.sK/A.sK - 1` over the seeds, with a 95% t interval.
Standard library only.
"""
import argparse
import csv
import hashlib
import math
import os
import random
import resource
import statistics
import subprocess
import sys
import time

T975 = {1: 12.71, 2: 4.30, 3: 3.18, 4: 2.78, 5: 2.57, 6: 2.45, 7: 2.36, 8: 2.31, 9: 2.26,
        10: 2.23, 11: 2.20, 12: 2.18, 13: 2.16, 14: 2.14, 15: 2.13, 19: 2.09, 24: 2.06, 29: 2.05}


def t975(df):
    return T975.get(df) or T975[max(k for k in T975 if k <= df)] if df >= 1 else float("nan")


def row_spec(perf_tsv, row):
    with open(perf_tsv) as f:
        for line in f:
            if line.startswith("#") or not line.strip():
                continue
            parts = line.rstrip("\n").split("\t")
            if parts[0] == row:
                flags = [] if parts[1] == "-" else parts[1].split()
                return flags, parts[2]
    sys.exit(f"{row}: not in {perf_tsv}")


def no_core():
    resource.setrlimit(resource.RLIMIT_CORE, (1, 1))


def run(binary, flags, filt, inp):
    t = time.perf_counter_ns()
    r = subprocess.run([binary, "jq"] + flags + [filt, inp], stdout=subprocess.PIPE,
                       stderr=subprocess.PIPE, preexec_fn=no_core)
    return time.perf_counter_ns() - t, r


def parse_label(label):
    g, _, s = label.partition(".s")
    return g, (int(s) if s else None)


def summarize(times, groups):
    med = {k: statistics.median(v) for k, v in times.items()}
    out = []
    for g in groups:
        seeded = {s: med[l] for l, (gg, s) in ((l, parse_label(l)) for l in med) if gg == g and s is not None}
        if len(seeded) > 1:
            vals = list(seeded.values())
            m = statistics.mean(vals)
            out.append(f"layout spread {g}: {len(vals)} seeds, range {100 * (max(vals) - min(vals)) / m:.2f}%, "
                       f"sd {100 * statistics.stdev(vals) / m:.2f}% (medians {min(vals) / 1e6:.2f}-{max(vals) / 1e6:.2f} ms)")
    base = groups[0]
    for g in groups[1:]:
        if base in med and g in med:
            out.append(f"naive {g}/{base} (unseeded links): {100 * (med[g] / med[base] - 1):+.2f}%")
        diffs = []
        for l in med:
            gg, s = parse_label(l)
            if gg == g and s is not None and f"{base}.s{s}" in med:
                diffs.append(100 * (med[l] / med[f"{base}.s{s}"] - 1))
        if len(diffs) > 1:
            m, sd = statistics.mean(diffs), statistics.stdev(diffs)
            h = t975(len(diffs) - 1) * sd / math.sqrt(len(diffs))
            sign = "excludes 0" if abs(m) > h else "includes 0"
            out.append(f"paired {g}/{base} over {len(diffs)} seeds: {m:+.2f}% +/- {h:.2f}% (95% CI, {sign}); "
                       f"per seed {', '.join(f'{d:+.1f}' for d in diffs)}")
    return out


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0],
                                formatter_class=argparse.RawDescriptionHelpFormatter, epilog=__doc__)
    p.add_argument("--row", required=True, help="an id from comparison/jq/queries/perf.tsv")
    p.add_argument("--perf-tsv", default=os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                                      "..", "comparison", "jq", "queries", "perf.tsv"))
    p.add_argument("--input", required=True)
    p.add_argument("--bin", action="append", required=True, metavar="LABEL=PATH")
    p.add_argument("--reps", type=int, default=15)
    p.add_argument("--warmup", type=int, default=1)
    p.add_argument("--csv")
    p.add_argument("--heartbeat", help="file touched after every repetition")
    p.add_argument("--order-seed", type=int, default=1)
    a = p.parse_args(argv)
    flags, filt = row_spec(a.perf_tsv, a.row)
    bins = dict(b.split("=", 1) for b in a.bin)
    groups = list(dict.fromkeys(parse_label(l)[0] for l in bins))

    digest = {}
    for label, path in bins.items():
        _, r = run(path, flags, filt, a.input)
        if r.returncode:
            sys.exit(f"{label}: exit {r.returncode}: {r.stderr[:300]!r}")
        digest[label] = hashlib.sha256(r.stdout).hexdigest()
    if len(set(digest.values())) != 1:
        sys.exit(f"output differs between binaries: {digest}")
    for _ in range(a.warmup - 1):
        for path in bins.values():
            run(path, flags, filt, a.input)

    rng = random.Random(a.order_seed)
    times = {l: [] for l in bins}
    rows = []
    labels = list(bins)
    for rep in range(a.reps):
        rng.shuffle(labels)
        for label in labels:
            ns, r = run(bins[label], flags, filt, a.input)
            if r.returncode:
                sys.exit(f"{label}: exit {r.returncode} at rep {rep}")
            times[label].append(ns)
            g, s = parse_label(label)
            rows.append((rep, label, g, "" if s is None else s, ns))
        if a.heartbeat:
            with open(a.heartbeat, "a"):
                os.utime(a.heartbeat)
    if a.csv:
        with open(a.csv, "w", newline="") as f:
            w = csv.writer(f)
            w.writerow(["rep", "label", "group", "seed", "ns"])
            w.writerows(rows)
    print(f"{a.row} on {os.path.basename(a.input)}: {len(bins)} binaries x {a.reps} reps, output identical")
    for line in summarize(times, groups):
        print(line)


if __name__ == "__main__":
    main()
