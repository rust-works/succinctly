#!/usr/bin/env python3
"""Sweep of repeated groups that hold an assertion, against jq's Oniguruma (#6127).

    scripts/jq-regex-repeat-target-sweep.py SUBJECT [ORACLE]

SUBJECT is a `succinctly` binary, ORACLE a jq 1.7.1 (default `$JQ_ORACLE`, then `/usr/bin/jq`).
Every group of up to three alternatives drawn from `ATOMS`, under each quantifier spelling in
`QUANTIFIERS`, is compiled by both through `test` (75,780 patterns), and the outcomes are compared:
`true`, `false` or the error's text. Exit 0 when they agree, 1 with the differences listed.

The rule it pins is Oniguruma's `is_invalid_quantifier_target`: an assertion is an invalid repeat
target, a sequence of several nodes is not, an alternation is when any alternative is, a plain
`(?:...)` group is its body, and a capturing, named, atomic or option group may repeat anything.
"""
import itertools
import json
import os
import subprocess
import sys

ATOMS = ["a", "[ab]\\w", "", "\\b", "\\B", "^", "$", "\\A", "\\z", "\\Z", "(?=a)", "(?!a)",
         "(?<=a)", "(?<!a)", "\\b\\b", "\\G", "(?:)", "()", "(?:\\b)", "(?:a|\\b)"]
QUANTIFIERS = ["*", "+", "?", "{1,2}", "{2}", "{0,}", "*?", "+?", "??"]
PROGRAM = (
    'def t($r): try (("ab" | test($r)) | tostring) catch '
    '(if (.|type)=="string" then (.|sub("^.*: ";"")|sub("\\\\(.*";"")) else "E" end); '
    ".[] | [., t(.)] | @tsv"
)


def outcomes(argv, patterns):
    done = subprocess.run(argv + ["-r", PROGRAM], input=json.dumps(patterns).encode(),
                          capture_output=True, timeout=1200)
    result = {}
    for line in done.stdout.decode().split("\n"):
        if "\t" in line:
            pattern, outcome = line.rsplit("\t", 1)
            result[pattern.replace("\\\\", "\\")] = outcome
    return result


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    subject = sys.argv[1]
    oracle = sys.argv[2] if len(sys.argv) > 2 else os.environ.get("JQ_ORACLE", "/usr/bin/jq")
    patterns = ["(?:" + "|".join(alts) + ")" + q
                for n in (1, 2, 3)
                for alts in itertools.product(ATOMS, repeat=n)
                for q in QUANTIFIERS]
    want = outcomes([oracle], patterns)
    got = outcomes([subject, "jq"], patterns)
    # A refused pattern reads `target of repeat operator is invalid` in jq and here (the subject
    # adds its `invalid regex:` prefix, which the sub() above strips with the rest of the prefix).
    bad = [(p, want[p], got.get(p)) for p in want if want[p] != got.get(p)]
    print(f"{len(want)} patterns, {len(bad)} differ")
    for pattern, w, g in bad[:40]:
        print(f"  {pattern}  jq: {w}  subject: {g}")
    sys.exit(1 if bad else 0)


main()
