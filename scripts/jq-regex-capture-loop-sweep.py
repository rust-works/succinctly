#!/usr/bin/env python3
"""Differential sweep for #6310: a capture group inside a greedy loop's body.

    scripts/jq-regex-capture-loop-sweep.py SUBJECT BASELINE [--count 5000] [--seed 2] [--jq /usr/bin/jq]

Oniguruma ends a greedy loop at the first iteration that matches nothing (#6106), and that holds when the
body has a capture group in a way after its empty one (`(?:b*|(a))+`, #6310). This draws `--count` loops
at random from a pool of atoms (capture groups, nullable and consuming, among them), each with a body of
two or three ways, one of 13 quantifiers, a plain or capturing group around it, and sometimes a prefix or
a suffix. Each runs on ten strings under `g`, in one process per binary, and the match offsets, strings and
captures are compared. It prints how many patterns SUBJECT fixed (BASELINE differed from jq, SUBJECT does
not), broke (the reverse) and still differs on, and exits 1 when it broke one.

`--jq` is jq 1.7.1. A pattern that makes jq abort (an assertion in its UTF-8 reader on some inputs) is
run alone and left out when it aborts again.
"""
import argparse
import json
import random
import subprocess
import sys

ATOMS = ["a", "b", "b*", "a*", "b?", "(a)", "(b*)", "(a|b)", "()", "(?:a|b*)", "(?<n>a)", "(b)*",
         "(a)?", "(?:(a)|b)", r"\w", r"(\w*)", "(a*)", "(?:|a)", "(?:a|)"]
QUANTS = ["+", "*", "{2}", "{2,}", "{0,3}", "{1,2}", "{2,3}", "+?", "*?", "{2}?", "{2,}?", "{2,3}?",
          "{0,2}?"]
SUBJECTS = ["ba", "bab", "abab", "aab", "abba", "baab", "aaa", "bbb", "cba", "abc"]
PROGRAM = ('.[] as $p | %s[] as $s | [$p,$s,(try ([$s|match($p;"g")|[.offset,.string,'
           '(.captures|map(.string))]]) catch "ERR")]' % json.dumps(SUBJECTS))


def patterns(count, rng):
    def way():
        return "".join(rng.choice(ATOMS) for _ in range(rng.choice([1, 1, 1, 2])))

    out = set()
    while len(out) < count:
        body = "|".join(way() for _ in range(rng.choice([2, 2, 3])))
        pattern = rng.choice(["(?:%s)", "(?:%s)", "(%s)"]) % body + rng.choice(QUANTS)
        r = rng.random()
        if r < 0.15:
            pattern += "c"
        elif r < 0.3:
            pattern = "x?" + pattern
        elif r < 0.4:
            pattern = "(?:" + pattern + ")(b)"
        out.add(pattern)
    return sorted(out)


def run(cmd, plist):
    r = subprocess.run(cmd + ["-c", PROGRAM], input=json.dumps(plist), capture_output=True, text=True)
    return r.stdout.splitlines() if r.returncode == 0 else None


def answers(cmd, plist):
    out = {}
    for i in range(0, len(plist), 200):
        chunk = plist[i:i + 200]
        lines = run(cmd, chunk)
        if lines is None:
            lines = []
            for p in chunk:
                lines += run(cmd, [p]) or []
        for line in lines:
            row = json.loads(line)
            out[(row[0], row[1])] = row[2]
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("subject")
    ap.add_argument("baseline")
    ap.add_argument("--count", type=int, default=5000)
    ap.add_argument("--seed", type=int, default=2)
    ap.add_argument("--jq", default="/usr/bin/jq")
    args = ap.parse_args()
    plist = patterns(args.count, random.Random(args.seed))
    ref = answers([args.jq], plist)
    base = answers([args.baseline, "jq"], plist)
    subj = answers([args.subject, "jq"], plist)
    keys = [k for k in ref if k in base and k in subj]
    fixed = {k[0] for k in keys if base[k] != ref[k] and subj[k] == ref[k]}
    broke = {k[0] for k in keys if base[k] == ref[k] and subj[k] != ref[k]}
    left = {k[0] for k in keys if subj[k] != ref[k]}
    print("%d patterns, %d cases: fixed %d patterns, broke %d, still differ %d"
          % (len(plist), len(keys), len(fixed), len(broke), len(left)))
    for p in sorted(broke)[:20]:
        print("  broke:", p)
    return 1 if broke else 0


if __name__ == "__main__":
    sys.exit(main())
