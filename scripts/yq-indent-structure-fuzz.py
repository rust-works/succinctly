#!/usr/bin/env python3
"""Randomised differential for YAML block structure and indentation -- the companion of
`scripts/yq-comment-oracle-fuzz.py` (#5086, #6370, #6385).

Draws 2-4 lines from a pool of block-structure fragments (`- a`, `- k: a`, `- - a`, `k:`, `? a`,
`?`, `: v`, `- ? a`, ...), indents each by 0-4 columns, and runs the document through the pinned oracle
(`yq`, v4.53.3) and through `succinctly yq` (and, with `--baseline`, an older build). It compares
accept/reject and, for accepted documents, the `-o json -I0 .` text.

    scripts/yq-indent-structure-fuzz.py [--count 1500] [--seed 1] [--wide] \\
        [--bin target/release/succinctly] [--baseline OLD/succinctly] [--yq yq] [--show 12]

With `--baseline` it reports how many documents BASELINE differed from yq on and `--bin` fixed, and
**regressions**: documents BASELINE answered as yq did and `--bin` does not (exit status 1 when
there are any). Without it, it only lists where `--bin` still differs, classified by direction:

    yq-refuses-we-read   accepts YAML yq refuses (the usual residual: a tolerance, or a stray line dropped)
    we-refuse-yq-reads   refuses YAML yq reads (the direction users meet)
    values-differ        both read it and print different values
    timeout              a run took over 20 s (reported apart from a refusal, which it is not)

`--wide` adds flow collections, quoted scalars, block scalars, comments, anchors, tags and document
markers to the pool; many of its residual differences are an *indented* `---` or `%YAML` line, which
yq reads and succinctly does not (nothing real writes either).

A random sweep is how #5086's missed rows were found: the tolerances for an out-dented `-` (#325/#485/
#900) and for a `key: value` line between a sequence item and its compact mapping (#885) accounted for
about 60 of every 75 documents where yq refused and `main` answered. Pair it with a real-file sweep
(every `*.yaml` on disk through the old and new binary, expecting no output difference) before
tightening anything in `src/yaml/parser.rs`.
"""
import argparse
import random
import subprocess
import sys

BASE = ["- a", "- k: a", "- - a", "- ", "k:", "k: v", "? a", "?", "b: 1", ": v", "- k:", "  j: b", "x", "- ? a"]
WIDE = BASE + [
    "a: [1, 2]", "- {k: v}", "k: |", "  text", 'k: "q"', "k: 'q'", "# c", "---", "&a x: 1", "- &a b", "*a",
    "k: &a", "!!str x", "k: !!str v", "", "k: >", "  more", "k: [", "  1]", '- "q"', "... ", "%YAML 1.2",
    "k: v # c", "- a # c", "? |", "k: - x",
]


def document(rng, pool, indents):
    count = rng.choice([2, 3, 3, 4])
    return "".join(" " * rng.choice(indents) + rng.choice(pool) + "\n" for _ in range(count))


def run(cmd, text):
    try:
        done = subprocess.run(cmd + ["-o", "json", "-I0", "."], input=text.encode(), capture_output=True, timeout=20)
    except subprocess.TimeoutExpired:
        return ("timeout", None)
    return (done.returncode != 0, None if done.returncode != 0 else done.stdout.decode())


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--count", type=int, default=1500)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--wide", action="store_true")
    ap.add_argument("--bin", default="target/release/succinctly", help="the build under test")
    ap.add_argument("--baseline", help="an older build, to report fixes and regressions")
    ap.add_argument("--yq", default="yq", help="the pinned oracle (Homebrew yq v4.53.3)")
    ap.add_argument("--show", type=int, default=12, help="documents to print per class")
    args = ap.parse_args()

    rng = random.Random(args.seed)
    pool = WIDE if args.wide else BASE
    indents = [0, 0, 0, 1, 2, 2, 4] if args.wide else [0, 0, 1, 1, 2, 2, 3, 4]
    seen, regressions, fixed, still = set(), [], 0, []
    for _ in range(args.count):
        text = document(rng, pool, indents)
        if text in seen:
            continue
        seen.add(text)
        oracle = run([args.yq], text)
        ours = run([args.bin, "yq"], text)
        if args.baseline:
            old = run([args.baseline, "yq"], text)
            if old == oracle and ours != oracle:
                regressions.append(text)
                continue
            if old != oracle and ours == oracle:
                fixed += 1
                continue
        if ours != oracle:
            still.append((text, oracle, ours))

    print(f"documents {len(seen)}  fixed {fixed}  regressions {len(regressions)}  still differ {len(still)}")
    for text in regressions[: args.show]:
        print("REGRESSION", repr(text))
    classes = {"yq-refuses-we-read": [], "we-refuse-yq-reads": [], "values-differ": [], "timeout": []}
    for text, oracle, ours in still:
        if "timeout" in (oracle[0], ours[0]):
            key = "timeout"
        else:
            key = "yq-refuses-we-read" if oracle[0] else "we-refuse-yq-reads" if ours[0] else "values-differ"
        classes[key].append((text, oracle, ours))
    for key, rows in classes.items():
        print(f"{key}: {len(rows)}")
        for text, oracle, ours in rows[: args.show]:
            print(f"  {text!r}  yq={oracle[1]!r}  ours={ours[1]!r}")
    return 1 if regressions else 0


if __name__ == "__main__":
    sys.exit(main())
