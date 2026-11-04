#!/usr/bin/env python3
"""Differential sweep for #6217: `\\p{NAME}` property names against jq 1.7.1.

    scripts/jq-regex-property-name-sweep.py [--jq /usr/bin/jq] [--succinctly target/debug/succinctly]

Takes every name in the property table of the `regex-syntax` version `Cargo.lock` pins (the names
the `regex` crate reads), adds a few hand-picked spellings, and runs each in nine forms
(`\\p{N}`, `\\P{N}`, `\\p{^N}`, `\\P{^N}`, `[\\p{N}]`, `[^\\P{N}]`, `[a\\p{N}b]`, `[\\P{N}]`, `[\\p{^N}]`) over a string of
varied characters in jq and in `succinctly jq`. A case differs when one tool refuses and the other
answers, or when both answer differently; two refusals are not compared by wording. A new
`regex-syntax` release can add names Oniguruma (jq 1.7.1) lacks: they show up here, and belong in
`property_name_kind` in `src/jq/eval.rs`. The `In_*` block names (#6228) are not in the crate's table:
every block in `src/jq/regex_blocks.rs` is run in three spellings, the nine forms, with and without
the `i` flag, over the edges of every block (the code point before and after each range and each
range's ends). Exits 1 on a difference.
"""
import argparse
import concurrent.futures as cf
import glob
import json
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
    "\\\\p{%s}", "\\\\P{%s}", "\\\\p{^%s}", "\\\\P{^%s}", "[\\\\p{%s}]", "[^\\\\P{%s}]", "[a\\\\p{%s}b]", "[\\\\P{%s}]", "[\\\\p{^%s}]",
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


def block_cases():
    """Each block name of `src/jq/regex_blocks.rs`, the string of its edge code points, and its ranges."""
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    text = open(os.path.join(root, "src/jq/regex_blocks.rs")).read()
    cases = []
    for name, body in re.findall(r'\(\s*"([a-z0-9]+)",\s*&\[([^\]]*)\]\s*,?\s*\)', text):
        edges = set()
        for lo, hi in re.findall(r"\(0x([0-9A-F]+), 0x([0-9A-F]+)\)", body):
            for cp in (int(lo, 16) - 1, int(lo, 16), int(hi, 16), int(hi, 16) + 1):
                if 0 <= cp <= 0x10FFFF and not 0xD800 <= cp <= 0xDFFF:
                    edges.add(cp)
        if not edges:
            edges = {0x41, 0x3BB}
        cases.append((name, sorted(edges)))
    return cases


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

    blocks = block_cases()
    # `In_` plus the rest, as the `In_Basic_Latin` spelling; and `IN-` with the rest upper-cased.
    spellings = [lambda n: n, lambda n: "In_" + n[2:], lambda n: "IN-" + n[2:].upper()]
    block_jobs = [
        (spell(n), f, flag, edges)
        for n, edges in blocks
        for spell in spellings
        for f in FORMS
        for flag in ("", "i")
    ]

    def block_job(case):
        name, form, flag, edges = case
        text = "".join(chr(c) for c in edges)
        program = '[%s|explode[]|[.]|implode]|map(try test("%s"; "%s") catch .)' % (
            json.dumps(text), form % name, flag
        )
        return case, run([args.jq], program), run([args.succinctly, "jq"], program)

    bad = []
    with cf.ThreadPoolExecutor(8) as pool:
        for case, theirs, ours in pool.map(block_job, block_jobs):
            if theirs != ours:
                bad.append(((case[0], case[1], case[2]), theirs[:70], ours[:70]))
    print(len(blocks), "blocks,", len(block_jobs), "cases;", len(bad), "differ")
    block_bad = bad
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
    for case, theirs, ours in block_bad[:20]:
        print(repr(case), "jq=", theirs, "us=", ours)
    return 1 if bad or block_bad else 0


if __name__ == "__main__":
    sys.exit(main())
