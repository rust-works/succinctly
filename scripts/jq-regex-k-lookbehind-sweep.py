#!/usr/bin/env python3
"""Differential sweep for #6169: `\K` inside look-behinds and groups under a global search.

    scripts/jq-regex-k-lookbehind-sweep.py [--jq /usr/bin/jq] [--succinctly target/debug/succinctly]

Runs `match(pattern; "g")` for every generated pattern (a `\K` among `a`, `b`, `.`, `(?:a|b)`,
`\b`, `$` in 1-3 items, wrapped as a look-behind, negative look-behind, look-ahead or plain group)
on three strings, in jq and in `succinctly jq`, each under a 1.5 s kill. jq 1.7.1 loops on the
patterns whose `\K` ends a look-behind match; `succinctly jq` must not, and must give jq's
answer wherever jq gives one. Exits 1 on a difference or a loop here.
"""
import argparse
import concurrent.futures as cf
import itertools
import subprocess
import sys

ATOMS = ["a", "b", r"\K", ".", "(?:a|b)", r"\b", "$"]
INPUTS = ["cab", "abab", "ba"]


def patterns():
    out = set()
    for n in (1, 2, 3):
        for seq in itertools.product(ATOMS, repeat=n):
            body = "".join(seq)
            if r"\K" not in body:
                continue
            for wrap in ("(?<=%s)", "(?<!%s)", "(?=%s)", "(?:%s)"):
                out.add(wrap % body)
            out.add("(?<=%s)b" % body)
            out.add("a(?<=%s)" % body)
    return sorted(out)


def run(cmd, pattern, text):
    program = '"%s" | [match("%s";"g")|[.offset,.length,.string]]' % (
        text,
        pattern.replace("\\", "\\\\"),
    )
    try:
        r = subprocess.run(cmd + ["-nc", program], capture_output=True, text=True, timeout=1.5)
        return (r.stdout + r.stderr).strip()[:200]
    except subprocess.TimeoutExpired:
        return "TIMEOUT"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--jq", default="/usr/bin/jq")
    ap.add_argument("--succinctly", default="target/debug/succinctly")
    args = ap.parse_args()
    jobs = [(p, t) for p in patterns() for t in INPUTS]

    def job(case):
        pattern, text = case
        return case, run([args.jq], pattern, text), run([args.succinctly, "jq"], pattern, text)

    stats = {"agree": 0, "jq-refuses": 0, "jq-loops": 0, "differ": 0, "loops-here": 0}
    bad = []
    with cf.ThreadPoolExecutor(8) as pool:
        for case, theirs, ours in pool.map(job, jobs):
            if ours == "TIMEOUT":
                stats["loops-here"] += 1
                bad.append((case, theirs, ours))
            elif theirs == "TIMEOUT":
                stats["jq-loops"] += 1
            elif theirs.startswith("jq: error"):
                stats["jq-refuses"] += 1
            elif theirs == ours:
                stats["agree"] += 1
            else:
                stats["differ"] += 1
                bad.append((case, theirs, ours))
    print(len(jobs), "cases", stats)
    for entry in bad[:25]:
        print(entry)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
