#!/usr/bin/env python3
"""Differential sweep for #6310 and #6332: a capture group inside a loop's body.

    scripts/jq-regex-capture-loop-sweep.py SUBJECT BASELINE [--count 5000] [--seed 2] [--nest | --ways | --assert | --bounded | --fancy] [--newlines [--multiline]] [--blanks] [--flags g] [--jq /usr/bin/jq]

Oniguruma ends a greedy loop at the first iteration that matches nothing (#6106), and that holds when the
body has a capture group: after its empty way (`(?:b*|(a))+`, #6310), or before it, in it, or with its own
empty way first (`(?:(a)|b*|c)+`, #6332). This draws `--count` loops
at random from a pool of atoms (capture groups, nullable and consuming, among them), each with a body of
two or three ways, one of 13 quantifiers, a plain or capturing group around it, and sometimes a prefix or
a suffix, under `--flags`; with `--nest` half of them also stand inside an outer repeat or hold a repeated group of their own,
which re-runs the copies of a group the rewrite wrote (#6332). `--ways` (#6358) draws the other shape that
`--nest` undersamples: an outer repeat, of any of ten quantifiers, over two or three ways that are each one to three items,
six in ten of them a loop with a group (or a plain atom otherwise), so that a loop stands in a way beside other items and
the ways share a prefix; it is the draw that found the alternation `regex-syntax` factors (#6134) and the lazy `*?` and `+?`
that keep their text. `--assert` (#6358) draws the loops whose ways hold a zero-width anchor (`\\b`, `\\B`, `^`, `$`, `\\A`, `\\z`)
beside a capture group or a consuming item, under the counted and lazy quantifiers, on subjects with a word boundary
in the middle; `--newlines` runs it on subjects that hold a newline instead, and lets the ways consume one, which is what says `^` and
`$` may be written as `\\A` and `\\z`; `--multiline` (#6425) puts `(?m)` before every pattern, with `^`, `$` and a newline drawn more often, which is what makes
them line anchors. `--fancy` (#6358) draws the plain loops with a lookaround or an atomic group beside them, outside the repeat, so
that they run on `fancy-regex`. `--bounded` (#6428, #6427) draws a lazy repeat that may run once or not at all (`{0,n}?`, `{1,n}?`, `*?`, `+?`) over anchors, flag switches and loops of its own, beside another loop or
inside a repeat. Each runs on ten strings under `g`, in one process per binary, and the match offsets, strings and
captures are compared. It prints how many patterns SUBJECT fixed (BASELINE differed from jq, SUBJECT does
not), broke (the reverse) and still differs on, and exits 1 when it broke one.

`--jq` is jq 1.7.1: pass it where `/usr/bin/jq` is an older jq (Ubuntu's is 1.6, whose `match` differs on empty matches, and
thousands of patterns then read as differing). A pattern that makes jq abort (an assertion in its UTF-8 reader on some inputs) is
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
NESTED = ["(?:b*|(a))+", "(?:(a)|b?){0,2}", "(?:(a)|b)*", "(b|)*", "(?:a|(b))+?"]
OUTER = ["*", "+", "{2}", "{0,3}", "{2,}?", "{2}?", "{1,2}"]
LOOPS = ["(?:b*|(a))+", "(?:(a)|b?)*", "(?:(a)|b)*", "(b|)*", "(?:a|(b))+?", "(?:(b)*|a)+", "(?:(a*)|b)+",
         "(?:b*|a(\\w*))*", "(?:(b?)|(a))*?", "(?:a*|(b))*"]
SIMPLE = ["a", "b", "b*", "a?", "(a)", "(a|b)", "(b*)", "()", "c?", "(?:a|b)", r"\w"]
WAYS_QUANTS = ["{2}", "{2,}", "{0,3}", "{1,2}", "+", "*", "{2,3}?", "{2}?", "{0,2}?", "+?"]
SUBJECTS = ["ba", "bab", "abab", "aab", "abba", "baab", "aaa", "bbb", "cba", "abc"]
# `--assert`: the anchors, and subjects that put a word boundary inside the string.
ANCHORS = [r"\b", r"\B", "^", "$", r"\A", r"\z"]
ASSERT_SUBJECTS = SUBJECTS[:6] + ["a b", " ab", "ba b", "b a"]
NEWLINE_SUBJECTS = ["a\nb", "\nab", "ab\n", "b\n\na", "a\n", "\n", "ba\nb a"]
# `--fancy`: what stands beside the loop, on one side or both.
BEFORE = ["(?=b)", "(?=[ab])", "(?!c)", "(?<=b)", "(?<!c)", "(?>b)", "(?>a|b)", "(?#c)"]
AFTER = ["(?=b)", "(?!a)", "(?!c)", "(?<=a)", "(?<!b)", "(?>b)", "(?#c)"]


# `--blanks` (#6442): subjects with a blank in them, for the patterns `blanked` writes.
BLANK_SUBJECTS = SUBJECTS[:6] + ["a b", "b a", "ab "]


def blanked(plist):
    """Half the patterns with blanks between their tokens, and a literal `\\ ` for `c`: read under the `x` flag
    they are the pattern they were, and without it a blank is a character, so both engines still agree."""
    out = []
    for i, p in enumerate(plist):
        if i % 2:
            p = p.replace("|", " | ").replace("(?:", "(?: ").replace(")", " )").replace("c", "\\ ")
        out.append(p)
    return sorted(set(out))


def program(subjects):
    return ('.[] as $p | %s[] as $s | [$p,$s,(try ([$s|match($p;$flags)|[.offset,.string,'
            '(.captures|map(.string))]]) catch "ERR")]' % json.dumps(subjects))


PROGRAM = program(SUBJECTS)


def patterns(count, rng, nest=False):
    pool = ATOMS + (NESTED if nest else [])

    def way():
        return "".join(rng.choice(pool) for _ in range(rng.choice([1, 1, 1, 2])))

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
        if nest and rng.random() < 0.5:
            pattern = rng.choice(["(?:%s)%s", "(%s)%s"]) % (pattern, rng.choice(OUTER))
        out.add(pattern)
    return sorted(out)


def ways_patterns(count, rng):
    """`--ways`: an outer repeat over ways that hold loops with groups beside other items."""
    def way():
        return "".join(rng.choice(LOOPS if rng.random() < 0.6 else SIMPLE)
                       for _ in range(rng.choice([1, 2, 2, 3])))

    out = set()
    while len(out) < count:
        body = "|".join(way() for _ in range(rng.choice([2, 2, 3])))
        pattern = rng.choice(["(?:%s)", "(%s)"]) % body + rng.choice(WAYS_QUANTS)
        r = rng.random()
        if r < 0.2:
            pattern += "c"
        elif r < 0.3:
            pattern = "(?:" + pattern + ")(b)"
        if rng.random() < 0.3:
            pattern = rng.choice(["(?:%s)%s", "(%s)%s"]) % (pattern, rng.choice(["*", "+", "{2}", "{0,2}"]))
        out.add(pattern)
    return sorted(out)


def assert_patterns(count, rng, multiline=False):
    """`--assert`: ways that hold a zero-width anchor beside groups and consuming items, under a repeat.
    `multiline` (#6425) draws `^`, `$` and a newline more often: they are what a `(?m)` loop turns on."""
    pool = ATOMS + ANCHORS * 3 + (["^", "$"] * 6 + [r"\n"] * 6 if multiline else [])

    def way():
        return "".join(rng.choice(pool) for _ in range(rng.choice([1, 1, 2, 2])))

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


BOUNDED = ["{0,2}?", "{1,2}?", "{0,3}?", "{1,3}?", "{0,4}?", "{1,4}?", "{0,12}?", "{2,3}?", "{2,5}?", "*?", "+?"]


def bounded_patterns(count, rng):
    """`--bounded`: a lazy repeat, most with a minimum under two, over anchors, flag switches and loops, beside a loop or in a repeat."""
    pool = ATOMS + NESTED + ANCHORS + ["(?i)", r"(\b)", r"(\z)", "(.*?)"]

    def way():
        return "".join(rng.choice(pool) for _ in range(rng.choice([1, 1, 2, 2])))

    out = set()
    while len(out) < count:
        body = "|".join(way() for _ in range(rng.choice([2, 2, 3])))
        pattern = rng.choice(["(?:%s)", "(?:%s)", "(%s)"]) % body + rng.choice(BOUNDED)
        pattern += rng.choice(["", "b", "c", "$"])
        r = rng.random()
        if r < 0.2:
            pattern = "(?:(a)|b*|c)+" + pattern
        elif r < 0.4:
            pattern = "(?:%s|c)%s" % (pattern, rng.choice(OUTER + ["*?", "+?"]))
        out.add(pattern)
    return sorted(out)


def fancy_patterns(count, rng):
    """`--fancy`: a plain draw with a construct only `fancy-regex` reads beside each loop."""
    out = set()
    for pattern in patterns(count * 2, rng):
        side = rng.random()
        before = rng.choice(BEFORE) if side < 0.7 else ""
        after = rng.choice(AFTER) if side > 0.4 else ""
        out.add(before + pattern + after)
    return sorted(out)[:count]


def run(cmd, plist, flags="g", prog=PROGRAM):
    r = subprocess.run(cmd + ["-c", "--arg", "flags", flags, prog], input=json.dumps(plist),
                       capture_output=True, text=True)
    return r.stdout.splitlines() if r.returncode == 0 else None


def answers(cmd, plist, flags="g", prog=PROGRAM):
    out = {}
    for i in range(0, len(plist), 200):
        chunk = plist[i:i + 200]
        lines = run(cmd, chunk, flags, prog)
        if lines is None:
            lines = []
            for p in chunk:
                lines += run(cmd, [p], flags, prog) or []
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
    shape = ap.add_mutually_exclusive_group()
    shape.add_argument("--nest", action="store_true", help="also nest loops inside repeats")
    shape.add_argument("--ways", action="store_true", help="loops beside other items in the ways of an outer repeat")
    shape.add_argument("--assert", dest="anchors", action="store_true", help="ways that hold a zero-width anchor")
    shape.add_argument("--bounded", action="store_true", help="a lazy repeat, most with a minimum under two, over anchors, switches and loops")
    shape.add_argument("--fancy", action="store_true", help="a lookaround or atomic group beside the loop")
    ap.add_argument("--newlines", action="store_true", help="with --assert: subjects that hold a newline")
    ap.add_argument("--multiline", action="store_true",
                    help="with --assert --newlines: `(?m)` before every pattern, under which `^` and `$` hold at every line (#6425)")
    ap.add_argument("--blanks", action="store_true",
                    help="blanks between the tokens of half the patterns, for `--flags gx` (#6442)")
    ap.add_argument("--flags", default="g", help="the match flags (`gi`: case folding)")
    ap.add_argument("--jq", default="/usr/bin/jq")
    args = ap.parse_args()
    rng = random.Random(args.seed)
    if args.multiline and not (args.anchors and args.newlines):
        ap.error("--multiline applies to --assert --newlines")
    if args.anchors:
        plist, prog = assert_patterns(args.count, rng, args.multiline), program(ASSERT_SUBJECTS)
        if args.newlines:
            plist = [p.replace("a|b", "a|\\n") if i % 2 else p for i, p in enumerate(plist)]
            prog = program(NEWLINE_SUBJECTS)
            if args.multiline:
                plist = sorted({"(?m)" + p for p in plist})
    elif args.bounded:
        plist, prog = bounded_patterns(args.count, rng), PROGRAM
    elif args.fancy:
        plist, prog = fancy_patterns(args.count, rng), PROGRAM
    else:
        plist = ways_patterns(args.count, rng) if args.ways else patterns(args.count, rng, args.nest)
        prog = PROGRAM
    if args.blanks:
        if args.anchors or args.fancy:
            ap.error("--blanks applies to the plain, --nest and --ways draws")
        plist, prog = blanked(plist), program(BLANK_SUBJECTS)
    ref = answers([args.jq], plist, args.flags, prog)
    base = answers([args.baseline, "jq"], plist, args.flags, prog)
    subj = answers([args.subject, "jq"], plist, args.flags, prog)
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
