#!/usr/bin/env python3
"""Differential sweep for #6181: a repeat operator after a bare option group.

    scripts/jq-regex-option-group-repeat-sweep.py [--jq /usr/bin/jq] [--succinctly target/debug/succinctly]

Runs `test(pattern)` for every generated pattern (nine option groups, fifteen repeat operators, ten
positions: 1,350 patterns) in jq and in `succinctly jq`, and prints how the two verdicts pair up
(`not specified`, `invalid`, true, false, other). jq 1.7.1 refuses every operator but the literal
`{,2}` after a bare option group. `succinctly jq` agrees except on two known sets: a pattern with an
inline `(?x)`, where the repeat checks stand down (#6127), and a possessive operator (`(?i)*+`),
which the possessive refusal words first (#6184). Exits 1 on any other difference.
"""
import argparse
import collections
import concurrent.futures as cf
import subprocess
import sys

OPTIONS = ["(?i)", "(?-i)", "(?m)", "(?i-m)", "(?s)", "(?x)", "(?i)(?m)", "(?i)(?#c)", "(?#c)(?i)"]
OPERATORS = ["*", "+", "?", "{1}", "{2}", "{1,2}", "{,2}", "*?", "+?", "??", "{1}?", "*+", "++", "?+", "{2}+"]
POSITIONS = ["%s%sa", "a%s%s", "a|%s%s", "(?:%s%s)", "(a%s%s)", "(%s%s)", "a%s%sb", "(?:a|%s%s)b", "(?=%s%s)", "(?<=a%s%s)"]


def patterns():
    return sorted({p % (o, q) for p in POSITIONS for o in OPTIONS for q in OPERATORS})


def verdict(cmd, pattern):
    program = '"caab" | try test("%s") catch .' % pattern.replace("\\", "\\\\")
    try:
        r = subprocess.run(cmd + ["-nc", program], capture_output=True, text=True, timeout=3)
    except subprocess.TimeoutExpired:
        return "timeout"
    text = (r.stdout + r.stderr).strip()
    if "not specified" in text:
        return "not-specified"
    if "repeat" in text and "invalid" in text:
        return "invalid"
    return text if text in ("true", "false") else "other"


def known_gap(pattern):
    return "(?x)" in pattern or any(op in pattern for op in ("*+", "++", "?+", "}+"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--jq", default="/usr/bin/jq")
    ap.add_argument("--succinctly", default="target/debug/succinctly")
    args = ap.parse_args()
    pats = patterns()

    def job(p):
        return p, verdict([args.jq], p), verdict([args.succinctly, "jq"], p)

    pairs = collections.Counter()
    unexpected = []
    with cf.ThreadPoolExecutor(8) as pool:
        for p, theirs, ours in pool.map(job, pats):
            pairs[(theirs, ours)] += 1
            if theirs != ours and not known_gap(p):
                unexpected.append((p, theirs, ours))
    print(len(pats), "patterns")
    for (theirs, ours), n in pairs.most_common():
        print("%5d  jq %-14s here %s" % (n, theirs, ours))
    for entry in unexpected[:25]:
        print("DIFFERS", entry)
    return 1 if unexpected else 0


if __name__ == "__main__":
    sys.exit(main())
