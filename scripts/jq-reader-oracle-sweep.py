#!/usr/bin/env python3
"""Input-reader sweep: a fixed-seed subset of `jq-reader-oracle diff` (#7970, #7986).

`jq-reader-oracle/` runs jq 1.7.1's and 1.8.2's own C input readers in-process and judges
`succinctly jq` against them by the ADR-0018 amendment (#7963): each case is `same`,
`split=1.7.1`, `split=1.8.2`, `E1`, `E2`, `E2-pending`, `recorded`, `BUG(agree)` or `BUG(split)`.
Only the last two are bugs. `split=1.8.2` rows are #7971's to sort out, not this sweep's.

This builds the package (it compiles jq's vendored C, so Linux and macOS only), runs the subset
and fails when

- a bug case falls outside RESIDUALS: a new divergence in the reader layer; or
- a RESIDUALS entry no longer shows: its issue was fixed, so delete the entry in the same change.

A residual is keyed by the oracle's signature (class, family, flags), the first three columns of
`summary.txt`'s signature table, so a second bug with the same signature is not caught until the
first is fixed. RESIDUALS holds for the default subset only: another `--seed` or `--cases` reaches
other cases, so read its NEW and GONE lines as a report, not a verdict.

Exit 0 agrees, 1 diverges, 2 is a setup error (the build failed, or the oracle disagreed with
itself).

Usage: scripts/jq-reader-oracle-sweep.py SUCCINCTLY [--seed N] [--cases N] [--out DIR] [--oracle BIN]
"""
import argparse
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# The subset. Seed 3 reaches the lazy-prefix line count of #7948/#7980 at case 14434, so reverting
# #7974 fails this sweep; seed 1's first 40,000 cases do not reach it. Against a release build on an
# Apple silicon laptop, with the oracle's default 8 jobs: 17 s for the cases, and about 65 s in all
# with the minimised repros it writes for each bug signature.
SEED = 3
CASES = 15000

# (class, family, flags) -> the issue that tracks it. Captured at seed 3, cases 0..15000, on
# main 1b01f0911 (macOS arm64).
RESIDUALS = {
    ("BUG(split)", "nul-in-string", "--seq"): "#7989",
    ("BUG(split)", "nul-in-string", "--seq -n"): "#7989",
    ("BUG(split)", "mix", "--seq"): "#7989",
    ("BUG(split)", "nul-in-string", "-n"): "#7991",
    ("BUG(split)", "nul-in-string", "-n -s"): "#7991",
    ("BUG(split)", "mix", "-n"): "#7991",
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("subject", help="the succinctly binary")
    ap.add_argument("--seed", type=int, default=SEED)
    ap.add_argument("--cases", type=int, default=CASES)
    ap.add_argument("--out", default=os.path.join(ROOT, "target", "compare", "reader-oracle"))
    ap.add_argument("--oracle", help="a built jq-reader-oracle; default: build it")
    a = ap.parse_args()

    oracle = a.oracle
    if oracle is None:
        manifest = os.path.join(ROOT, "jq-reader-oracle", "Cargo.toml")
        build = ["cargo", "build", "--quiet", "--release", "--locked", "--manifest-path", manifest]
        if subprocess.run(build).returncode != 0:
            print("jq-reader-oracle did not build", file=sys.stderr)
            return 2
        oracle = os.path.join(ROOT, "jq-reader-oracle", "target", "release", "jq-reader-oracle")

    # Absolute: the oracle runs the subject from its own staging directory.
    subject = os.path.abspath(a.subject)
    cmd = [oracle, "diff", "--subject", subject, "--seed", str(a.seed),
           "--cases", str(a.cases), "--out", a.out]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode not in (0, 1):
        sys.stdout.write(r.stdout)
        sys.stderr.write(r.stderr)
        print(f"jq-reader-oracle diff exited {r.returncode}", file=sys.stderr)
        return 2

    seen = {}
    with open(os.path.join(a.out, "cases.jsonl")) as f:
        for line in f:
            c = json.loads(line)
            if c["class"].startswith("BUG("):
                seen.setdefault((c["class"], c["family"], c["flags"]), []).append(c["index"])

    new = {k: v for k, v in seen.items() if k not in RESIDUALS}
    gone = [k for k in RESIDUALS if k not in seen]
    for k, v in sorted(seen.items()):
        tag = f"residual {RESIDUALS[k]}" if k in RESIDUALS else "NEW"
        print(f"{tag}: {' / '.join(k)}: {len(v)} case(s), first {v[:5]}")
    for k in gone:
        print(f"GONE: {' / '.join(k)} ({RESIDUALS[k]}) no longer shows; delete its RESIDUALS entry")
    bugs = sum(len(v) for v in seen.values())
    print(f"reader oracle, seed {a.seed}, {a.cases} cases: {bugs} bug case(s), "
          f"{len(new)} new signature(s), {len(gone)} gone; report in {a.out}/summary.txt")
    return 1 if new or gone else 0


if __name__ == "__main__":
    sys.exit(main())
