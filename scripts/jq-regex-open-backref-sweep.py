#!/usr/bin/env python3
"""Differential sweep for #6262: a backreference to a group that is still open.

    scripts/jq-regex-open-backref-sweep.py SUBJECT [--jq /usr/bin/jq]

Oniguruma reads `\\1` inside group 1 as the group's last completed value (unset on the first pass,
so the reference fails). `fancy-regex` saves a group's start when it opens, so the same reference
reads from the new start to the old end, which is empty whenever iterations are contiguous.

For each generated pattern (a group that holds a reference to itself, under a repeat or not, with
the group nested in other groups, lookarounds and alternations) on four strings, with and without
`g`, this runs jq 1.7.1 and the `succinctly` binary under test (SUBJECT) and prints how many cases
agree, differ, or are refused here (jq answers, SUBJECT says `invalid regex`). It lists, per
class of pattern, whether the group sits under a repeat that can run twice (the shape
`fancy_has_open_group_backref` refuses) or not (the shape it must leave alone).
Exits 1 when a pattern of the refused shape is answered on some text (the guard is gone or has a
hole), or one outside it is refused, differs from jq or fails otherwise.
"""
import argparse
import collections
import concurrent.futures as cf
import subprocess
import sys

# What the group holds: the reference itself, in the first or the later position, beside text.
BODIES = [
    r"\1|\w",
    r"a|\1",
    r"\w|\1",
    r"\1a|\w",
    r"a\1?",
    r"(?!\1)",
    r"b?\1?a?",
    r"(?:\1|a)b?",
    r"\w\1?",
    r"\1?\w",
    r"a(?=\1)|b",
    r"a(?!\1)",
]
# The repeat the group sits under; "" is a group outside any repeat.
REPEATS = ["", "+", "*", "{2}", "{1,2}", "{2,}", "?", "+?", "*?", "{0}", "{1}"]
INPUTS = ["ab", "aab", "abab", "ba", "abc", "aaaa", "abababab", "aaabbb"]


def patterns():
    out = {}
    for body in BODIES:
        for rep in REPEATS:
            group = "(%s)%s" % (body, rep)
            runs_twice = rep in ("+", "*", "{2}", "{1,2}", "{2,}", "+?", "*?")
            out[group] = runs_twice
            # the group inside an outer repeat or alternative
            for outer, outer_twice in (("(?:%s)+", True), ("(?:%s|x)", False), ("(?:%s)?", False)):
                out[outer % group] = runs_twice or outer_twice
            out["%sb" % group] = runs_twice
    return out


def run(cmd, pattern, text, flags):
    escaped = pattern.replace("\\", "\\\\").replace('"', '\\"')
    program = '"%s" | [match("%s";"%s")|[.offset,.length,(.captures[]|[.offset,.string])]]' % (
        text,
        escaped,
        flags,
    )
    try:
        r = subprocess.run(cmd + ["-nc", program], capture_output=True, text=True, timeout=6)
    except subprocess.TimeoutExpired:
        return "TIMEOUT"
    lines = (r.stdout + r.stderr).strip().splitlines()
    return lines[0][:140] if lines else ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("subject")
    ap.add_argument("--jq", default="/usr/bin/jq")
    ap.add_argument("--show", type=int, default=12)
    ap.add_argument("--list", action="store_true")
    args = ap.parse_args()
    pats = patterns()
    jobs = [(p, t, f) for p in sorted(pats) for t in INPUTS for f in ("g", "")]

    def job(case):
        p, t, f = case
        return case, run([args.jq], p, t, f), run([args.subject, "jq"], p, t, f)

    stats = collections.Counter()
    per_pattern = collections.defaultdict(lambda: collections.Counter())
    examples = collections.defaultdict(list)
    with cf.ThreadPoolExecutor(8) as pool:
        for case, theirs, ours in pool.map(job, jobs):
            p = case[0]
            kind = "twice" if pats[p] else "once"
            if theirs.startswith("jq: error"):
                key = "jq-refuses"
            elif "invalid regex" in ours:
                key = "refused-here"
            elif ours.startswith("jq: error") or ours == "TIMEOUT":
                key = "other-failure"
            elif ours == theirs:
                key = "agree"
            else:
                key = "differ"
                examples[kind].append((case, theirs, ours))
            stats[(kind, key)] += 1
            per_pattern[p][key] += 1
    for kind in ("twice", "once"):
        row = {k: v for (kd, k), v in sorted(stats.items()) if kd == kind}
        print(kind, row)
    bad = 0
    for p, c in sorted(per_pattern.items()):
        refused_shape = pats[p]
        # the refused shape must be refused on every text jq answers, the others never
        if refused_shape and (c["agree"] or c["differ"] or c["other-failure"]):
            bad += 1
            print("NOT REFUSED", p, dict(c))
        if not refused_shape and (c["differ"] or c["refused-here"] or c["other-failure"]):
            bad += 1
            print("ONCE-SHAPE CHANGED", p, dict(c))
    twice = [c for q, c in per_pattern.items() if pats[q]]
    print(
        "twice-shape patterns:",
        len(twice),
        "| differ on some text:",
        sum(1 for c in twice if c["differ"]),
        "| agree on every text:",
        sum(1 for c in twice if not c["differ"]),
    )
    if args.list:
        for q, c in sorted(per_pattern.items()):
            if pats[q]:
                print("LIST", "DIFFER" if c["differ"] else "agree ", q)
    for kind in ("twice", "once"):
        for entry in examples[kind][: args.show]:
            print("DIFFER", kind, entry)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
