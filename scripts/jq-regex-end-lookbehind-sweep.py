#!/usr/bin/env python3
"""Differential sweep for #6510: look-behinds and lookaheads beside `\\z`, `$` and `\\Z`.

    scripts/jq-regex-end-lookbehind-sweep.py SUBJECT [BASELINE] [--seed N] [--count M]

Draws `--count` patterns of one to four items: look-behinds and negative look-behinds (whose
bodies consume or not, and may hold another look-behind), lookaheads holding a look-behind or an
end anchor, the anchors `\\z`, `$`, `\\Z`, `\\b`, `^`, a literal now and then, in groups, in
alternations and under `(?m)` or `(?i)`. Each runs `[match(p; "g") | [.offset, .length,
(.captures | map(.offset))]]`, `test(p)` and `sub(p; "X")` on texts with and without a final
newline, a one-character text and a multibyte one, under jq 1.7.1 (`/usr/bin/jq`), SUBJECT and
BASELINE. A pattern counts as fixed when SUBJECT agrees with jq on every text and BASELINE does
not, broken the other way round, and still differing when neither does. Exits 1 on a broken
pattern.
"""
import argparse
import concurrent.futures as cf
import json
import random
import subprocess
import sys

TEXTS = ["ab", "ab\n", "b", "bb", "a\nb", "a\n", "éb", "abb", "\n", ""]
BODIES = ["b", "a", r"\w", ".", "[ab]", r"\n", "ab", "a?b", "b+", "(b)", r"\b", "b?",
          "(?<=a)b", "b(?<=b)", "(?:a|b)", "^", r"\W",
          # A way that matches nothing (#6516 review).
          "b|", "|b", "b|c?", "(?:b|)", "b?|c", "(b)|"]
ANCHORS = [r"\z", "$", r"\Z", r"\b", "^", r"\B"]

PROGRAM = (
    "$texts[] as $s | [$s | try ([match($p; \"g\") | [.offset, .length, "
    "(.captures | map(.offset))]], test($p), sub($p; \"X\")) catch \"error\"]"
)


def item(rng, depth):
    roll = rng.random()
    if roll < 0.35:
        sign = rng.choice(["=", "!"])
        return "(?<%s%s)" % (sign, rng.choice(BODIES))
    if roll < 0.60:
        return rng.choice(ANCHORS)
    if roll < 0.70:
        inner = rng.choice([r"(?<=b)", r"(?<!b)", r"\z", "$", r"(?<=\w)"])
        return "(?%s%s)" % (rng.choice("=!"), inner)
    if roll < 0.80 and depth < 2:
        return "(?:%s)" % sequence(rng, depth + 1)
    if roll < 0.87 and depth < 2:
        return "(%s)" % sequence(rng, depth + 1)
    if roll < 0.94 and depth < 2:
        return "(?:%s|%s)" % (sequence(rng, depth + 1), sequence(rng, depth + 1))
    return rng.choice(["b", "a", "(?m)", "(?i)"])


def sequence(rng, depth):
    return "".join(item(rng, depth) for _ in range(rng.randint(1, 3)))


def pattern(rng):
    body = "".join(item(rng, 0) for _ in range(rng.randint(1, 4)))
    if rng.random() < 0.15:
        body = rng.choice(["(?m)", "(?i)"]) + body
    return body


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
    ap.add_argument("baseline", nargs="?")
    ap.add_argument("--jq", default="/usr/bin/jq")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--count", type=int, default=2000)
    args = ap.parse_args()
    rng = random.Random(args.seed)
    pats = sorted({pattern(rng) for _ in range(args.count)})

    def job(pat):
        want = run(args.jq, pat)
        got = run(args.subject, pat)
        base = run(args.baseline, pat) if args.baseline else None
        return pat, want, got, base

    fixed, broke, differ, refused = [], [], [], 0
    with cf.ThreadPoolExecutor(8) as pool:
        for pat, want, got, base in pool.map(job, pats):
            if want and want[0].startswith("exit"):
                refused += 1
                continue
            ok = got == want
            was = base == want if base is not None else ok
            if ok and not was:
                fixed.append(pat)
            elif was and not ok:
                broke.append((pat, want, got))
            elif not ok:
                differ.append((pat, want, got))
    print("%d patterns (%d jq refuses): fixed %d, broke %d, still differ %d"
          % (len(pats), refused, len(fixed), len(broke), len(differ)))
    for pat, want, got in broke[:10]:
        print("BROKE", pat, "\n  jq:", want, "\n  here:", got)
    for pat, want, got in differ[:5]:
        print("DIFFER", pat, "\n  jq:", want, "\n  here:", got)
    return 1 if broke else 0


if __name__ == "__main__":
    sys.exit(main())
