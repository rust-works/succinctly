#!/usr/bin/env python3
"""Layout-seeded interleaved A/B for `succinctly jq` binaries (#7960).

A wall-clock A/B between two builds measures the change plus wherever the linker happened to place
the code, and on this codebase placement alone moves a row by ±5%. This harness compares the
two builds over several seeded layouts instead, so a difference that holds over the seeds is the
change's and one that changes sign between them is placement. docs/guides/benchmarking.md
("Layout-seeded A/B") has the recipe and the measurements behind it.

Each binary is named `GROUP` (the unseeded link) or `GROUP.sSEED` (the link under that seed), for
example `A.s3=/x/a-3/succinctly`. Two binaries with the same seed were linked alike
(scripts/link-order/relink.py), so `B.s3 / A.s3` is one paired comparison, and the spread of the
`A.s*` medians is what placement does to a binary whose code is fixed.

    scripts/layout-ab.py --row strings_upcase --input strings-10mb.json --reps 15 \\
        --bin A=a/succinctly --bin B=b/succinctly \\
        --bin A.s1=a-s1/succinctly --bin B.s1=b-s1/succinctly ... --csv out.csv

- Every binary runs once per repetition, in an order shuffled per repetition (rule 1 of the A/B
  method interleaves two binaries; here it is all of them).
- Before timing, every binary must exit 0 and print the same bytes (rule 4); `--no-gate` skips
  the byte comparison, for arms that are meant to answer differently.
- Wall time is the child's elapsed time (`time.perf_counter_ns` around `subprocess.run`), and each
  binary's figure is its median over the repetitions.

Reported for each compared pair of groups, B against A by default (`--pair G/BASE` to choose):

- the plain B/A from the two unseeded links, which is what an ordinary A/B would have said;
- each group's layout spread: the range and standard deviation of its seeded medians;
- the paired B/A: the mean of `B.sK / A.sK - 1` over the seeds both have, with a 95% t interval,
  and whether that interval excludes 0.

`--env GROUP:KEY=VALUE` sets a variable for one group's binaries, for a runtime switch measured as
one binary against itself. Set the switch in every compared group, at equal length (`=1` against
`=0`, not present against absent): the environment's size moves the initial stack, which moved
one row by 2% on its own (#7961). The harness warns when two compared groups differ in that way.

Standard library only.
"""

import argparse
import csv
import hashlib
import math
import os
import random
import re
import resource
import statistics
import subprocess
import sys
import time

# Two-sided 95% quantiles of Student's t, by degrees of freedom; 1.96 past the table.
T975 = [None, 12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179,
        2.160, 2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060,
        2.056, 2.052, 2.048, 2.045, 2.042]

LABEL = re.compile(r"([A-Za-z][A-Za-z0-9_-]*)(?:\.s(\d+))?")


def t975(df):
    if df < 1:
        raise ValueError(f"no t quantile for {df} degrees of freedom")
    return T975[df] if df < len(T975) else 1.96


def parse_label(label):
    """(group, seed or None) of `GROUP` or `GROUP.sSEED`; ValueError for anything else."""
    m = LABEL.fullmatch(label)
    if not m:
        raise ValueError(f"binary label {label!r}: expected GROUP or GROUP.sSEED (e.g. A, A.s3)")
    return m.group(1), (int(m.group(2)) if m.group(2) is not None else None)


def parse_bins(specs):
    """{label: absolute path} from `LABEL=PATH` arguments, in order. ValueError on a bad or repeated
    label or a path that is not an executable file: a bare `succinctly` would otherwise be looked
    up on PATH, not in the current directory."""
    bins = {}
    for spec in specs:
        label, sep, path = spec.partition("=")
        if not sep or not path:
            raise ValueError(f"--bin {spec!r}: expected LABEL=PATH")
        parse_label(label)
        if label in bins:
            raise ValueError(f"--bin {label} given twice")
        if not (os.path.isfile(path) and os.access(path, os.X_OK)):
            raise ValueError(f"--bin {label}={path}: not an executable file")
        bins[label] = os.path.abspath(path)
    return bins


def parse_envs(specs, groups, base=None):
    """{group: environment} from `GROUP:KEY=VALUE` arguments, each a copy of `base` (os.environ)
    with its keys set. ValueError for a group no binary belongs to."""
    base = os.environ if base is None else base
    envs = {}
    for spec in specs:
        group, sep, kv = spec.partition(":")
        key, sep2, value = kv.partition("=")
        if not sep or not sep2 or not key:
            raise ValueError(f"--env {spec!r}: expected GROUP:KEY=VALUE")
        if group not in groups:
            raise ValueError(f"--env {spec!r}: no binary is in group {group!r}")
        envs.setdefault(group, dict(base))[key] = value
    return envs


def parse_pairs(specs, groups):
    """[(group, base)] from `G/BASE` arguments, or every group against the first one."""
    if not specs:
        return [(g, groups[0]) for g in groups[1:]]
    pairs = []
    for spec in specs:
        g, sep, b = spec.partition("/")
        if not sep or g not in groups or b not in groups or g == b:
            raise ValueError(f"--pair {spec!r}: expected G/BASE, two different groups of the binaries")
        pairs.append((g, b))
    return pairs


def switch_warnings(env_specs, pairs):
    """A warning for each compared pair whose `--env` keys differ in presence or value length."""
    keys = {}
    for spec in env_specs:
        group, _, kv = spec.partition(":")
        key, _, value = kv.partition("=")
        keys.setdefault(group, {})[key] = value
    out = []
    for g, b in pairs:
        kg, kb = keys.get(g, {}), keys.get(b, {})
        for key in sorted(set(kg) | set(kb)):
            if key not in kg or key not in kb:
                where = g if key in kg else b
                out.append(f"warning: {key} is set for {where} only in {g}/{b}; set it in both, at equal "
                           f"length (=1 against =0), or the environment's size is part of the difference (#7961)")
            elif len(kg[key]) != len(kb[key]):
                out.append(f"warning: {key} differs in length between {g} and {b}; use values of equal "
                           f"length, or the environment's size is part of the difference (#7961)")
    return out


def row_spec(perf_tsv, row):
    """(flags, filter) of `row` in comparison/jq/queries/perf.tsv."""
    with open(perf_tsv) as f:
        for line in f:
            if line.startswith("#") or not line.strip():
                continue
            parts = line.rstrip("\n").split("\t")
            if parts[0] == row:
                return ([] if parts[1] == "-" else parts[1].split()), parts[2]
    raise ValueError(f"{row}: not in {perf_tsv}")


def no_core():
    resource.setrlimit(resource.RLIMIT_CORE, (1, 1))


def run(binary, flags, filt, inp, env=None):
    t = time.perf_counter_ns()
    r = subprocess.run([binary, "jq"] + flags + [filt, inp], stdout=subprocess.PIPE,
                       stderr=subprocess.PIPE, preexec_fn=no_core, env=env)
    return time.perf_counter_ns() - t, r


def summarize(times, pairs):
    """The report lines for `times` ({label: [ns, ...]}) and the compared `pairs`."""
    med = {label: statistics.median(v) for label, v in times.items()}
    by = {}
    for label, m in med.items():
        g, s = parse_label(label)
        by.setdefault(g, {})[s] = m
    out = []
    for g in dict.fromkeys(x for pair in pairs for x in pair):
        seeded = [m for s, m in by.get(g, {}).items() if s is not None]
        if len(seeded) > 1:
            mean = statistics.mean(seeded)
            out.append(f"layout spread {g}: {len(seeded)} seeds, range {100 * (max(seeded) - min(seeded)) / mean:.2f}%, "
                       f"sd {100 * statistics.stdev(seeded) / mean:.2f}% "
                       f"(medians {min(seeded) / 1e6:.2f}-{max(seeded) / 1e6:.2f} ms)")
    for g, b in pairs:
        mg, mb = by.get(g, {}), by.get(b, {})
        if None in mg and None in mb:
            out.append(f"plain {g}/{b} (unseeded links): {100 * (mg[None] / mb[None] - 1):+.2f}%")
        seeds = sorted(s for s in mg if s is not None and s in mb)
        diffs = [100 * (mg[s] / mb[s] - 1) for s in seeds]
        if len(diffs) > 1:
            m, sd = statistics.mean(diffs), statistics.stdev(diffs)
            h = t975(len(diffs) - 1) * sd / math.sqrt(len(diffs))
            verdict = "excludes 0" if abs(m) > h else "includes 0"
            out.append(f"paired {g}/{b} over {len(diffs)} seeds: {m:+.2f}% +/- {h:.2f}% (95% CI, {verdict}); "
                       f"per seed {', '.join(f'{d:+.1f}' for d in diffs)}")
    return out


def main(argv=None, runner=run):
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0],
                                formatter_class=argparse.RawDescriptionHelpFormatter, epilog=__doc__)
    p.add_argument("--row", required=True, help="an id from comparison/jq/queries/perf.tsv")
    p.add_argument("--perf-tsv", default=os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                                      "..", "comparison", "jq", "queries", "perf.tsv"))
    p.add_argument("--input", required=True, help="the row's input file")
    p.add_argument("--bin", action="append", required=True, metavar="LABEL=PATH",
                   help="a binary: LABEL is GROUP (unseeded link) or GROUP.sSEED")
    p.add_argument("--reps", type=int, default=15, help="repetitions, each running every binary once")
    p.add_argument("--warmup", type=int, default=1, help="untimed rounds, counting the gate's own run")
    p.add_argument("--csv", help="write every timing as rep,label,group,seed,ns")
    p.add_argument("--heartbeat", help="a file touched after every repetition")
    p.add_argument("--order-seed", type=int, default=1, help="seed of the per-repetition run order")
    p.add_argument("--env", action="append", default=[], metavar="GROUP:KEY=VALUE",
                   help="set KEY=VALUE for every binary of GROUP (e.g. P0:SUCCINCTLY_READER_MODEL=0)")
    p.add_argument("--pair", action="append", default=[], metavar="G/BASE",
                   help="a comparison to report (default: every group against the first)")
    p.add_argument("--no-gate", action="store_true", help="time binaries whose outputs differ")
    a = p.parse_args(argv)
    if a.reps < 2:
        p.error("--reps must be at least 2")
    try:
        flags, filt = row_spec(a.perf_tsv, a.row)
        bins = parse_bins(a.bin)
        groups = list(dict.fromkeys(parse_label(label)[0] for label in bins))
        if len(groups) < 2:
            raise ValueError("needs binaries of at least two groups")
        envs = parse_envs(a.env, groups)
        pairs = parse_pairs(a.pair, groups)
    except ValueError as e:
        p.error(str(e))
    env_of = {label: envs.get(parse_label(label)[0]) for label in bins}
    for w in switch_warnings(a.env, pairs):
        print(w, file=sys.stderr)

    digest = {}
    for label, path in bins.items():
        _, r = runner(path, flags, filt, a.input, env_of[label])
        if r.returncode:
            sys.exit(f"{label}: exit {r.returncode}: {r.stderr[:300]!r}")
        digest[label] = hashlib.sha256(r.stdout).hexdigest()
    same = len(set(digest.values())) == 1
    if not same and not a.no_gate:
        sys.exit("output differs between binaries:\n" + "\n".join(f"  {d[:16]} {label}" for label, d in digest.items()))
    for _ in range(a.warmup - 1):
        for label, path in bins.items():
            runner(path, flags, filt, a.input, env_of[label])

    rng = random.Random(a.order_seed)
    times = {label: [] for label in bins}
    rows = []
    labels = list(bins)
    for rep in range(a.reps):
        rng.shuffle(labels)
        for label in labels:
            ns, r = runner(bins[label], flags, filt, a.input, env_of[label])
            if r.returncode:
                sys.exit(f"{label}: exit {r.returncode} at repetition {rep}")
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
    gate = "output identical" if same else "outputs differ (--no-gate)"
    print(f"{a.row} on {os.path.basename(a.input)}: {len(bins)} binaries x {a.reps} reps, {gate}")
    for line in summarize(times, pairs):
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
