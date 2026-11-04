#!/usr/bin/env python3
"""Differential sweep for #6128 (item 3): a repeat operator after a comment, inside a loop body.

    scripts/jq-regex-comment-quantifier-sweep.py SUBJECT BASELINE [--jq /usr/bin/jq]

Oniguruma applies a repeat operator after `(?#...)` to the item before the comment, so `b(?#)*` is
`b*`. For each generated loop `(?:BODY|ALT)` (BODY an atom, a comment and a quantifier, or the
comment-free twin) on four strings, with and without `g`, this runs jq 1.7.1, the `succinctly`
binary under test (SUBJECT) and an older one (BASELINE), and prints how many cases SUBJECT fixed,
broke and still differs on. A pattern with a comment right after a quantifier (`b*(?#)*`) keeps its
comment on purpose and is expected to differ; one with a capture group differs as `(b*|a)+` does.
Exits 1 when SUBJECT breaks a case BASELINE answered as jq does.
"""
import argparse
import collections
import concurrent.futures as cf
import subprocess
import sys

ATOMS = ["a", "b", "b*", "b?", "(?:b|c)", "c", r"\*", r"\?", r"\w", "[bc]", "(b)"]
QUANTS = ["*", "+", "?", "{2}", "{1,2}", "*?", "{2,}"]
ALTS = ["a", "b", "c", "b*", ""]
INPUTS = ["bab", "ba", "cbb", "abc", "*a?"]


def patterns():
    bodies = set()
    for atom in ATOMS:
        for quant in QUANTS:
            bodies.add("%s(?#)%s" % (atom, quant))
            bodies.add("%s(?#c)%s" % (atom, quant))
            bodies.add("%s%s" % (atom, quant))
    out = set()
    for body in bodies:
        for alt in ALTS:
            for text in ("%s|%s" % (body, alt), "%s|%s" % (alt, body)):
                for loop in ("+", "*", "{2,}"):
                    out.add("(?:%s)%s" % (text, loop))
    return sorted(out)


def run(cmd, pattern, text, flags):
    escaped = pattern.replace("\\", "\\\\").replace('"', '\\"')
    program = '"%s" | [match("%s";"%s")|[.offset,.length]]' % (text, escaped, flags)
    try:
        r = subprocess.run(cmd + ["-nc", program], capture_output=True, text=True, timeout=4)
    except subprocess.TimeoutExpired:
        return "TIMEOUT"
    lines = (r.stdout + r.stderr).strip().splitlines()
    return lines[0][:100] if lines else ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("subject")
    ap.add_argument("baseline")
    ap.add_argument("--jq", default="/usr/bin/jq")
    args = ap.parse_args()
    jobs = [(p, t, f) for p in patterns() for t in INPUTS for f in ("g", "")]

    def job(case):
        p, t, f = case
        return (
            case,
            run([args.jq], p, t, f),
            run([args.subject, "jq"], p, t, f),
            run([args.baseline, "jq"], p, t, f),
        )

    stats = collections.Counter()
    broken, still = [], collections.Counter()
    with cf.ThreadPoolExecutor(8) as pool:
        for case, theirs, new, old in pool.map(job, jobs):
            if theirs.startswith("jq: error"):
                stats["jq-error"] += 1
            elif new == theirs and old == theirs:
                stats["agree-both"] += 1
            elif new == theirs:
                stats["fixed"] += 1
            elif old == theirs:
                stats["broken"] += 1
                broken.append((case, theirs, new, old))
            else:
                stats["still-differ"] += 1
                still[case[0]] += 1
    print(len(jobs), "cases", dict(stats))
    print("still-differ patterns:", len(still))
    for entry in broken[:15]:
        print("BROKEN", entry)
    return 1 if broken else 0


if __name__ == "__main__":
    sys.exit(main())
