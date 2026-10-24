#!/usr/bin/env python3
"""Differential grid for an `if`/`elif` in a `foreach` UPDATE that has a caught foreign-`$x` fold in one arm (#5115).

The #4800 slices (#5084, #5093, #5099, #5104) changed how the arms of an `if` in a `foreach` UPDATE
are resolved when one arm is a literal and the other a `foreach` whose source destructures an outer
`$x` under `try`. Each justified the change with a grid against `/usr/bin/jq` that was never committed;
this is that grid. `jq-fold-body-register-grid-sweep.py` has no `if` UPDATE, so it cannot gate this one.

Grid: UPDATE (`if C then A else B end`, `C` in `.a`, `.`, `.b`, a literal in one arm (`1`, `null`, `true`,
`false`) and the fold FE in the other, both orders, plus `elif` chains) x FE (a `try`-caught `$x` destructure, a
`?//` chain, four folds that do not hand their input on or whose source is not that destructure (#5113)) x wrapper (none, `try .. catch`, comma siblings, `?`, `| .`, `label`) x loop source (short, ten items long, unbounded) x EXTRACT x INIT (one,
two and three values, and two behind a pipe, which succinctly's static bound cannot count) x the loop's pattern (`$i`,
`?//` chains) x write context x document. The full product is about 23.9 million rows: `--n` (40,000 by default) samples
it deterministically (`--seed`), decoding each sampled index into its row, and `--only` narrows it to one axis value
(`--only null-then`, `--only alt-obj`) to sample that slice densely.

Rows are classified against jq (`/usr/bin/jq`) per build, as `jq-fold-body-register-grid-sweep.py`
does: MATCH, ACCEPT_WRONG (jq refused, the build answered: the dangerous one, `del`/`|=` write through
it), REFUSE_WRONG (the safe direction) and DIFF. Exit status 1 when, against `--base`, a row matched
jq on the base and no longer does (a lost match: a refuse-only fallback has leaked under `try` before,
#3186/#3267), the candidate is ACCEPT_WRONG where the base was not, or a row the base refused (REFUSE_WRONG) now
answers wrongly (DIFF: under a `?` a refusal can become a truncated answer, #5113, #5132) or prints a different partial
output before it refuses, or on a candidate timeout. A wrong answer that became another wrong answer is listed, not
failed (it may be a partial fix).
`--fixed` rows (wrong on the base, MATCH on the candidate) are counted and listed, so a change can
show what it moved. Run it against a build from before the change as `--base`.

`--control` puts the fold in the `then` arm only, the shape a change to the literal-`else` code must
not touch: with `--base` it exits 1 on any row whose output differs between the two builds.

Usage: scripts/jq-foreach-update-if-arm-sweep.py --bin BIN [--base BIN] [--jq JQ] [--n N] [--seed S]
                                                  [--show N] [--only SUBSTR[,SUBSTR...]] [--jobs N]
                                                  [--control] [--dump FILE]
"""
import argparse
import collections
import concurrent.futures
import itertools
import json
import random
import subprocess
import sys

# The caught foreign-`$x` fold: `$x` is the context's variable, not the loop's, so its destructure runs
# on a node that is not jq's path register.
FOLDS = [
    ("fe", "foreach (try ($x as {a:$a} | .)) as $y (.; .; .)"),
    ("fe-chain", "foreach (try ($x as {a:$a} ?// [$a] | .)) as $y (.; .; .)"),
    # #5113: folds that do not hand their input on (INIT, UPDATE or EXTRACT stores another value), and one whose source
    # is not a caught destructure: the state a step stores after them is not the `null` they were entered with.
    ("fe-init", "foreach (try ($x as {a:$a} | .)) as $y (5; .; .)"),
    ("fe-upd", "foreach (try ($x as {a:$a} | .)) as $y (.; 5; .)"),
    ("fe-ext", "foreach (try ($x as {a:$a} | .)) as $y (.; .; 5)"),
    ("fe-plain", "foreach (try (.a | .)) as $y (.; .; .)"),
]
# arm pairs: (name, then, else); {F} is the fold, `1` the literal.
ARMS = [
    ("lit-then", "1", "{F}"),
    ("lit-else", "{F}", "1"),
    # #5132: `null` and the booleans are `jv_identical` to any equal value by kind, so a stored one is the register
    # wherever it came from; a `1` cannot show the truncation that reading such a literal on a non-last step caused
    # (#5113).
    ("null-then", "null", "{F}"),
    ("false-then", "false", "{F}"),
    ("true-then", "true", "{F}"),
    ("null-else", "{F}", "null"),
    ("false-else", "{F}", "false"),
    ("true-else", "{F}", "true"),
]
CONDS = [("ca", ".a"), ("cdot", "."), ("cb", ".b")]
# `elif` chains: the fold behind the second condition, or the last arm.
ELIFS = [
    ("elif-lit-fe-lit", "if .a then 1 elif . then {F} else 2 end"),
    ("elif-fe-lit-lit", "if .a then {F} elif . then 1 else 2 end"),
    ("elif-lit-lit-fe", "if .a then 1 elif . then 2 else {F} end"),
    ("elif-lit-fe-nothing", "if .a then 1 elif .b then {F} end"),
    # #5132: the same chains with a literal `jv_identical` by kind.
    ("elif-null-fe-lit", "if .a then null elif . then {F} else 2 end"),
    ("elif-fe-null-lit", "if .a then {F} elif . then null else 2 end"),
    ("elif-null-lit-fe", "if .a then null elif . then false else {F} end"),
]
CONTROL_ARMS = [("fe-then", "{F}", "1")]
WRAPPERS = [
    ("none", "{U}"),
    ("try-1", "try ({U}) catch 1"),
    ("try-dot", "try ({U}) catch ."),
    ("comma-after", "({U}, 7)"),
    ("comma-before", "(7, {U})"),
    ("opt", "({U})?"),
    ("pipe-dot", "({U}) | ."),
    ("label", "label $l | {U}"),
    ("comma-empty", "({U}, empty)"),
]
# #5113: ...and sources whose steps the lookahead cannot count one by one: one longer than its cap (ten items) and the
# unbounded `.[]`, `.[]?` and `range(3)`, whose bound no static shape says.
SOURCES = [
    ".a",
    "(.a,.)",
    "(.,.a)",
    ".",
    "(.a,.a)",
    "(.a,.,.a)",
    "(.a,.b)",
    "(.,.a,.,.a,.,.a,.,.a,.,.a)",
    "(.a,.a,.a,.a,.a,.a,.a,.a,.a,.a)",
    ".[]",
    ".[]?",
    "range(3)",
]
EXTRACTS = [".a", "$i", "."]
INITS = [("init1", "."), ("initN", "(., .a)"), ("init3", "(., null, .a)"), ("initU", "(.,.) | .")]
# #5132: the loop's own pattern, a `?//` chain included: a retry runs UPDATE again on the state a step stored, and a
# change that reads an earlier INIT fork's step wrote through one where jq raises (#5113 review).
PATS = [("pat-var", "$i"), ("alt-obj", "{a:$i} ?// $i"), ("alt-arr", "[$i] ?// $i")]
CONTEXTS = [
    ("paths", "[path(. as $x | {O})?]"),
    ("path", "path(. as $x | {O})"),
    ("del", "del(. as $x | {O})"),
    ("update", "(. as $x | {O}) |= 9"),
]
DOCS = [
    "null",
    '{"a":null}',
    '{"a":{"a":null}}',
    '{"b":true}',
    "true",
    "[1]",
    '{"a":[true]}',
    '{"a":{"b":1},"b":2}',
    '{"a":false,"b":null}',
]


def build_axes(control):
    """The grid's axes, in `itertools.product` order: a row is one pick from each (#5132: the product is about 7.9
    million rows, so rows are decoded from an index rather than all built)."""
    updates = []
    for (cond_name, cond), (an, then, els) in itertools.product(CONDS, CONTROL_ARMS if control else ARMS):
        updates.append((f"{cond_name}-{an}", f"if {cond} then {then} else {els} end"))
    if not control:
        updates.extend(ELIFS)
    return [updates, FOLDS, WRAPPERS, SOURCES, EXTRACTS, INITS, CONTEXTS, DOCS, PATS]


def picks(axes, index):
    """The pick from each axis that row `index` of the product is (the last axis varies fastest)."""
    out = []
    for axis in reversed(axes):
        index, slot = divmod(index, len(axis))
        out.append(axis[slot])
    return out[::-1]


def row_name(parts):
    (un, _), (fn, _), (wn, _), src, ex, (inn, _), (cn, _), _, (pn, _) = parts
    return f"{un}/{fn}/{wn}/{src}/{ex}/{inn}/{cn}/{pn}"


def make_row(parts):
    (_, u), (_, f), (_, w), src, ex, (_, init), (_, c), doc, (_, pat) = parts
    update = w.replace("{U}", u.replace("{F}", f))
    outer = f"foreach {src} as {pat} ({init}; {update}; {ex})"
    return (row_name(parts), c.replace("{O}", outer), doc)


def run(prefix, prog, doc, timeout=10):
    try:
        p = subprocess.run([*prefix, "-c", prog], input=doc, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout
    except subprocess.TimeoutExpired:
        return "TIMEOUT", ""


def classify(oracle, got):
    (oc, oo), (gc, go) = oracle, got
    if gc == "TIMEOUT":
        return "TIMEOUT"
    if oc == gc and oo == go:
        return "MATCH"
    if oc != 0 and gc == 0:
        return "ACCEPT_WRONG"
    if oc == 0 and gc != 0:
        return "REFUSE_WRONG"
    return "DIFF"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--bin", required=True, help="succinctly build under test")
    ap.add_argument(
        "--base",
        help="reference succinctly build; a lost match, a new ACCEPT_WRONG or a refusal now answered wrongly fails",
    )
    ap.add_argument("--jq", default="/usr/bin/jq", help="the pinned oracle (jq 1.7.1)")
    ap.add_argument("--n", type=int, default=40000, help="rows to sample (0 = the whole grid)")
    ap.add_argument("--seed", type=int, default=0, help="sampling seed: the same seed and --n pick the same rows")
    ap.add_argument("--show", type=int, default=20, help="max rows printed per section")
    ap.add_argument("--only", default="", help="restrict to rows whose name contains this (comma-separated: any of)")
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--control", action="store_true", help="fold in the `then` arm only; any output change against --base fails")
    ap.add_argument("--dump", help="write every row's classes as JSON lines to this file")
    args = ap.parse_args()
    if args.control and not args.base:
        ap.error("--control compares against --base: pass a reference build")

    axes = build_axes(args.control)
    size = 1
    for axis in axes:
        size *= len(axis)
    indexes = range(size)
    if args.only:
        wanted = args.only.split(",")
        indexes = [i for i in indexes if any(part in row_name(picks(axes, i)) for part in wanted)]
    total = len(indexes)
    if args.n and args.n < total:
        indexes = random.Random(args.seed).sample(indexes, args.n)
    rows = [make_row(picks(axes, i)) for i in indexes]

    def work(row):
        name, prog, doc = row
        oracle = run([args.jq], prog, doc)
        if oracle[0] == "TIMEOUT":
            return None
        cand = run([args.bin, "jq"], prog, doc)
        base = run([args.base, "jq"], prog, doc) if args.base else None
        return (name, prog, doc, oracle, cand, base)

    with concurrent.futures.ThreadPoolExecutor(args.jobs) as ex:
        results = [r for r in ex.map(work, rows) if r]

    out = []
    for name, prog, doc, oracle, cand, base in results:
        out.append(
            {
                "name": name,
                "prog": prog,
                "doc": doc,
                "oracle": oracle,
                "cand": cand,
                "cls": classify(oracle, cand),
                "base": base,
                "bcls": classify(oracle, base) if base else None,
            }
        )
    if args.dump:
        with open(args.dump, "w") as f:
            for r in out:
                f.write(json.dumps({k: r[k] for k in ("name", "doc", "prog", "cls", "bcls")}) + "\n")

    print(f"rows={len(out)} sampled of {total} (the pinned oracle timed out on {len(rows) - len(out)})")
    print(f"candidate={dict(sorted(collections.Counter(r['cls'] for r in out).items()))}")
    if args.base:
        print(f"     base={dict(sorted(collections.Counter(r['bcls'] for r in out).items()))}")

    hung = [r for r in out if r["cls"] == "TIMEOUT"]
    sections = [("TIMEOUT", hung)]
    failed = bool(hung)
    if args.base and args.control:
        changed = [r for r in out if r["cand"] != r["base"] and r["base"][0] != "TIMEOUT"]
        print(f"control rows whose output changed against --base: {len(changed)}")
        sections.append(("CHANGED", changed))
        failed = failed or bool(changed)
    elif args.base:
        lost = [r for r in out if r["bcls"] == "MATCH" and r["cls"] != "MATCH"]
        newly = [r for r in out if r["cls"] == "ACCEPT_WRONG" and r["bcls"] != "ACCEPT_WRONG"]
        # #5132: a refusal the base made that is now another answer (under `?` a truncated one): neither a lost match
        # nor an ACCEPT_WRONG, and the hazard #5113 named.
        # A refusal whose partial output changed (jq answers, both refuse, but the candidate now prints more or less
        # first) is the same hazard one step later; a wrong answer that became another wrong answer is listed only.
        silenced = [
            r
            for r in out
            if r["bcls"] == "REFUSE_WRONG"
            and (r["cls"] == "DIFF" or (r["cls"] == "REFUSE_WRONG" and r["cand"][1] != r["base"][1]))
        ]
        rewritten = [r for r in out if r["bcls"] == "DIFF" and r["cls"] == "DIFF" and r["cand"] != r["base"]]
        fixed = [r for r in out if r["bcls"] not in (None, "MATCH") and r["cls"] == "MATCH"]
        print(
            f"fixed against --base: {len(fixed)}; lost matches: {len(lost)}; new ACCEPT_WRONG: {len(newly)}; "
            f"refusals now answered wrongly: {len(silenced)}; wrong answers changed (not failed): {len(rewritten)}"
        )
        sections += [
            ("LOST MATCH", lost),
            ("NEW ACCEPT_WRONG", newly),
            ("REFUSAL NOW WRONG", silenced),
            ("WRONG ANSWER CHANGED", rewritten),
        ]
        failed = failed or bool(lost or newly or silenced)
    else:
        print(f"ACCEPT_WRONG: {sum(r['cls'] == 'ACCEPT_WRONG' for r in out)} (pass --base to judge a change)")
        sections.append(("ACCEPT_WRONG", [r for r in out if r["cls"] == "ACCEPT_WRONG"]))
    for title, section in sections:
        for r in section[: args.show]:
            print(f"{title} {r['name']} [{r['cls']}]\n   {r['prog']}\n   in={r['doc']} jq={r['oracle']} build={r['cand']} base={r['base']}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
