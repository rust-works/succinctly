#!/usr/bin/env python3
"""Differential sweep for #6217: `\\p{NAME}` property names against jq 1.7.1.

    scripts/jq-regex-property-name-sweep.py [--jq /usr/bin/jq] [--succinctly target/debug/succinctly]

Takes every name in the property table of the `regex-syntax` version `Cargo.lock` pins (the names
the `regex` crate reads), adds a few hand-picked spellings, and runs each in seven forms
(`\\p{N}`, `\\P{N}`, `\\p{^N}`, `\\P{^N}`, `[\\p{N}]`, `[^\\P{N}]`, `[a\\p{N}b]`) over a string of
varied characters in jq and in `succinctly jq`. A case differs when one tool refuses and the other
answers, or when both answer differently; two refusals are not compared by wording. A new
`regex-syntax` release can add names Oniguruma (jq 1.7.1) lacks: they show up here, and belong in
`property_name_kind` in `src/jq/eval.rs`. The `In_*` block names are not in the crate's table and
are not covered. Exits 1 on a difference.
"""
import argparse
import concurrent.futures as cf
import glob
import os
import re
import subprocess
import sys

EXTRA = [
    "XDigit", "xdigit", "X_Digit", "x digit", " XDigit ", "x-digit", "XDIGIT", "Age=1.1",
    "Script=Latin", "sc=Greek", "gc=L", "gc:L", "Script_Extensions=Latin", "L=", "=", "Foo", "",
    "IDSU", "Modifier-Combining Mark", "mcm", "Bidi_Mirrored", "bidi mirrored", "ASCII_Hex_Digit",
]
FORMS = [
    "\\\\p{%s}", "\\\\P{%s}", "\\\\p{^%s}", "\\\\P{^%s}", "[\\\\p{%s}]", "[^\\\\P{%s}]", "[a\\\\p{%s}b]",
]


def crate_names():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    lock = open(os.path.join(root, "Cargo.lock")).read()
    version = re.search(r'name = "regex-syntax"\nversion = "([^"]+)"', lock).group(1)
    table = glob.glob(
        os.path.expanduser(
            "~/.cargo/registry/src/*/regex-syntax-%s/src/unicode_tables/property_names.rs" % version
        )
    )
    if not table:
        sys.exit("regex-syntax %s is not in the cargo registry: run `cargo fetch`" % version)
    pairs = re.findall(r'\("([^"]+)",\s*"([^"]+)"\)', open(table[0]).read())
    return sorted({a for a, _ in pairs} | {b for _, b in pairs})


def run(cmd, program):
    try:
        r = subprocess.run(cmd + ["-nc", program], capture_output=True, text=True, timeout=10)
    except subprocess.TimeoutExpired:
        return "TIMEOUT"
    out = (r.stdout + r.stderr).strip()
    refused = "invalid" in out or "parse error" in out or "error" in out.lower()
    return "REFUSED" if refused else out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--jq", default="/usr/bin/jq")
    ap.add_argument("--succinctly", default="target/debug/succinctly")
    args = ap.parse_args()
    names = crate_names() + EXTRA
    jobs = [(n, f) for n in names for f in FORMS]

    def job(case):
        name, form = case
        program = '["aA5fG_ -\\u00e9\\u0663"|explode[]|[.]|implode]|map(try test("%s") catch .)' % (
            form % name
        )
        return case, run([args.jq], program), run([args.succinctly, "jq"], program)

    bad = []
    with cf.ThreadPoolExecutor(8) as pool:
        for case, theirs, ours in pool.map(job, jobs):
            if theirs != ours:
                bad.append((case, theirs[:70], ours[:70]))
    print(len(names), "names,", len(jobs), "cases;", len(bad), "differ")
    seen = set()
    for case, theirs, ours in bad:
        if case[0] not in seen:
            seen.add(case[0])
            print(repr(case), "jq=", theirs, "us=", ours)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
