#!/usr/bin/env python3
"""Differential sweep for #6517: `$` beside `(?m)`, `(?-m)` and `(?m:...)` switches.

    scripts/jq-regex-dollar-multiline-sweep.py SUBJECT BASELINE [--seed N] [--count M]

Draws `--count` patterns of one to five items: `$`, `^`, `\\z`, `\\n`, `a`, `b`, `.`, `b*`, `\\s*`,
flag switches (`(?m)`, `(?-m)`, `(?i)`, `(?im)`), and groups, flag groups (`(?m:...)`,
`(?-m:...)`) and alternations of them. Each runs `[match(p; "g") | [.offset, .length]]`,
`test(p)` and `sub(p; "X"; "g")` on texts with zero, one and two final newlines, under jq 1.7.1
(`/usr/bin/jq`), SUBJECT and BASELINE. A pattern counts as fixed when SUBJECT agrees with jq on
every text and BASELINE does not, broken the other way round. Exits 1 on a broken pattern.
"""
import argparse
import concurrent.futures as cf
import json
import random
import subprocess
import sys

TEXTS = ["ab", "ab\n", "ab\n\n", "b\nb\n", "a\nb", "\n", "", "b \n"]
ATOMS = ["$", "$", "$", "^", r"\z", r"\n", "a", "b", ".", "b*", r"\s*", "(?m)", "(?-m)",
         "(?i)", "(?im)"]
PROGRAM = (
    "$texts[] as $s | [$s | try ([match($p; \"g\") | [.offset, .length]], test($p), "
    "sub($p; \"X\"; \"g\")) catch \"error\"]"
)


def item(rng, depth):
    roll = rng.random()
    if depth < 2 and roll < 0.12:
        return "(?m:%s)" % sequence(rng, depth + 1)
    if depth < 2 and roll < 0.18:
        return "(?-m:%s)" % sequence(rng, depth + 1)
    if depth < 2 and roll < 0.26:
        return "(%s)" % sequence(rng, depth + 1)
    if depth < 2 and roll < 0.32:
        return "(?:%s|%s)" % (sequence(rng, depth + 1), sequence(rng, depth + 1))
    return rng.choice(ATOMS)


def sequence(rng, depth):
    return "".join(item(rng, depth) for _ in range(rng.randint(1, 3)))


def pattern(rng):
    return "".join(item(rng, 0) for _ in range(rng.randint(1, 5)))


def run(binary, pat):
    cmd = [binary] + (["jq"] if not binary.endswith("/jq") else [])
    cmd += ["-nc", "--arg", "p", pat, "--argjson", "texts", json.dumps(TEXTS), PROGRAM]
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
        return out.stdout.splitlines() if out.returncode == 0 else ["exit %d" % out.returncode]
    except subprocess.TimeoutExpired:
        return ["timeout"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("subject")
    ap.add_argument("baseline")
    ap.add_argument("--jq", default="/usr/bin/jq")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--count", type=int, default=2000)
    args = ap.parse_args()
    rng = random.Random(args.seed)
    pats = sorted({pattern(rng) for _ in range(args.count)})

    def job(pat):
        return pat, run(args.jq, pat), run(args.subject, pat), run(args.baseline, pat)

    fixed, broke, differ = [], [], []
    with cf.ThreadPoolExecutor(8) as pool:
        for pat, want, got, base in pool.map(job, pats):
            if got == want and base != want:
                fixed.append(pat)
            elif base == want and got != want:
                broke.append((pat, want, got))
            elif got != want:
                differ.append((pat, want, got))
    print("%d patterns: fixed %d, broke %d, still differ %d"
          % (len(pats), len(fixed), len(broke), len(differ)))
    for pat, want, got in broke[:10]:
        print("BROKE", pat, "\n  jq:", want, "\n  here:", got)
    for pat, want, got in differ[:8]:
        print("DIFFER", pat, "\n  jq:", want, "\n  here:", got)
    return 1 if broke else 0


if __name__ == "__main__":
    sys.exit(main())
