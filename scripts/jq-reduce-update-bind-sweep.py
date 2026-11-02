#!/usr/bin/env python3
"""Differential grid for a `reduce` UPDATE that destructures an outer `$x`, under `try`/`?` (#4934).

`jq-path-register-sweep.py` has no operand with a bind chain in the UPDATE of an outermost `reduce`,
and `jq-fold-body-register-grid-sweep.py` always nests a fold in the UPDATE (which the resolver reads
differently: `update_reads_foreign_marker` excludes it), so neither gates the shape #4901/#4932/#4934/#4953
changed. The grid here is INIT x SOURCE x BIND x UPDATE shape x document x context, six x six x six x
twenty-eight x six x three = 108,864 rows. The UPDATE shapes are a bind chain bare, under `try`/`?` (with and
without a handler), under `limit`/`first`/`label`/`if`/a comma/`//`, and ending in `.`, a navigation,
a literal, `$a` or `$x`; the INITs are `.`, the literal accumulators `0`/`{}`/`[]`/`null` and a navigated
`.a`; the documents include `null`, which a `null` `$x` is the register of by kind.

Rows are classified against jq (`/usr/bin/jq`) per build, as the other sweeps do: MATCH,
MSGDIFF (same stdout and exit code, different error wording), ACCEPT_WRONG (jq refused, the build
answered: the dangerous one, `del`/`=`/`|=` write through it), REFUSE_WRONG (the safe direction) and
DIFF. With `--base`, exit status 1 when a row that matched jq on the base no longer does (a lost match:
a refuse-only fallback leaked under `try` before, #3186/#3267) or a row that was not an ACCEPT_WRONG on
the base is one on `--bin`. A wrong accept both builds share is the documented residual and does not
fail. Without `--base` the run only reports (a timeout of `--bin` still fails it): the residual has no
reference to be told apart from. A timeout is its own class and never a match, whichever side hung.
`--jq-gate` fails the run on any ACCEPT_WRONG or DIFF of `--bin` as well, with or without `--base`: no
residual is documented today (both grids are all MATCH), and it is how `succinctly dev compare jq
--dimension sweeps` runs this script (#5552). A residual that becomes accepted has to be listed here first.

Usage: scripts/jq-reduce-update-bind-sweep.py --bin BIN [--base BIN] [--jq JQ] [--jobs N] [--show N]
                                              [--sample K] [--alt] [--dump FILE] [--jq-gate]
"""
import argparse
import collections
import concurrent.futures as cf
import itertools
import json
import re
import subprocess
import sys

INITS = [".", "0", "{}", ".a", "null", "[]"]
SRCS = ["1", "(1,2)", ".a", ".a?", "..", "empty"]
BINDS = [
    "$x as {a:$a}",
    "$x as [$a]",
    "$x as $q",
    "$x as {a:{b:$a}}",
    "$x as {a:$a} ?// $a",
    ". as {a:$a}",
]
# (#4964) `--alt`: the same grid over `?//` chains, whose alternatives fail by value on different documents
# (an object pattern on an array, an array pattern on an object) and so retry onto the next one.
ALT_BINDS = [
    "$x as {a:$a} ?// $a",
    "$x as [$a] ?// $a",
    "$x as {a:$a} ?// [$a]",
    "$x as [$a] ?// {a:$a}",
    "$x as {a:{b:$a}} ?// $a",
    "$x as {a:$a} ?// {b:$a}",
    ". as {a:$a} ?// $a",
    "$x as {a:$a}",
]
DOCS = ["null", '{"a":1}', '{"a":null}', "[1]", '{"a":{"b":1}}', "true"]
CONTEXTS = ["path({E})", "del({E})", "({E}) |= 9"]
PREFIX = ". as $x | {R}"


def shapes(b):
    """The UPDATE shapes built around the bind chain `b`."""
    return [
        f"{b} | .",
        f"try ({b} | .)",
        f"({b} | .)?",
        f"try ({b} | .) catch .",
        f"try ({b} | .a)",
        f"try ({b} | 5)",
        f"first(try ({b} | .))",
        f"try ({b} | .), .",
        f"(try ({b} | .)) | .",
        f"if true then try ({b} | .) else . end",
        f"{b} | try .",
        f"try {b} | .",
        f"try ($x as {{a:$a}} | {b} | .)",
        f"try ({b} | $x)",
        f"try ({b} | $a)",
        f"label $l | try ({b} | .)",
        f"limit(1; try ({b} | .))",
        f"try (try ({b} | .))",
        f"try ({b} | .) // 7",
        f"select(true) | try ({b} | .)",
        # (#4953) the wrappers around a chain that ends in a computed value, which hands no accumulator on
        f"limit(1; try ({b} | 5))",
        f"label $l | try ({b} | 5)",
        f"try ({b} | 5), .",
        f"if true then try ({b} | 5) else . end",
        f"first(try ({b} | $a))",
        f"nth(0; try ({b} | 5))",
        f"({b} | 5)?",
        f"label $l | ({b} | break $l)",
    ]


def grid(binds=None):
    for init, src, b in itertools.product(INITS, SRCS, binds or BINDS):
        for upd in shapes(b):
            reduce = f"reduce {src} as $i ({init}; {upd})"
            for ctx in CONTEXTS:
                expr = ctx.replace("{E}", PREFIX.replace("{R}", reduce))
                for doc in DOCS:
                    yield doc, expr


def run(cmd, doc, expr):
    try:
        p = subprocess.run(cmd + ["-c", expr], input=doc.encode(), capture_output=True, timeout=20)
    except subprocess.TimeoutExpired:
        return ("TIMEOUT", -1, "")
    err = re.sub(r"\(at <[^>]*>\)", "", p.stderr.decode()).strip()
    err = re.sub(r"^jq: error[^:]*: ", "E: ", err)
    return (p.stdout.decode().strip(), p.returncode, err.splitlines()[0] if err else "")


def classify(jq, build):
    if jq[0] == "TIMEOUT":
        return "ORACLE_TIMEOUT"
    if build[0] == "TIMEOUT":
        return "TIMEOUT"
    if (jq[0], jq[1]) == (build[0], build[1]):
        return "MATCH" if (jq[2] == build[2] or jq[1] == 0) else "MSGDIFF"
    if jq[1] != 0 and build[1] == 0:
        return "ACCEPT_WRONG"
    if jq[1] == 0 and build[1] != 0:
        return "REFUSE_WRONG"
    return "DIFF"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--bin", required=True, help="the build under test")
    ap.add_argument("--base", help="a reference build (a `main` binary)")
    ap.add_argument("--jq", default="/usr/bin/jq")
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--show", type=int, default=10, help="rows printed per transition")
    ap.add_argument("--alt", action="store_true", help="the grid over `?//` chains (#4964)")
    ap.add_argument("--sample", type=int, default=1, help="run every Kth row (a quick, deterministic subset)")
    ap.add_argument("--dump", help="write every row of a transition worth reading, as JSON")
    ap.add_argument(
        "--jq-gate",
        action="store_true",
        help="also fail on any ACCEPT_WRONG or DIFF row of --bin against jq (the gate without --base, #5552)",
    )
    a = ap.parse_args()

    rows = list(grid(ALT_BINDS if a.alt else None))[:: max(a.sample, 1)]
    print(f"{len(rows)} rows", file=sys.stderr)

    def work(row):
        doc, expr = row
        jq = run([a.jq], doc, expr)
        base = run([a.base, "jq"], doc, expr) if a.base else None
        return row, jq, base, run([a.bin, "jq"], doc, expr)

    counts = collections.Counter()
    shown = collections.defaultdict(list)
    failed = False
    with cf.ThreadPoolExecutor(a.jobs) as pool:
        for row, jq, base, cand in pool.map(work, rows, chunksize=64):
            kc = classify(jq, cand)
            kb = classify(jq, base) if base else kc
            counts[(kb, kc)] += 1
            if (a.base and kb != kc) or kc in ("ACCEPT_WRONG", "DIFF"):
                shown[(kb, kc)].append((row, jq, base, cand))
            # A hang is never the documented residual, with or without a base to compare against.
            if kc == "TIMEOUT" and kb != "TIMEOUT":
                failed = True
            if a.base and (
                (kb == "MATCH" and kc != "MATCH") or (kb != "ACCEPT_WRONG" and kc == "ACCEPT_WRONG")
            ):
                failed = True
            # No residual is documented, so against jq alone a wrong answer is never expected.
            if a.jq_gate and kc in ("ACCEPT_WRONG", "DIFF"):
                failed = True
    print(("base -> candidate" if a.base else "candidate") + " (vs jq):")
    for (kb, kc), n in sorted(counts.items(), key=lambda kv: -kv[1]):
        print(f"  {kb:13} -> {kc:13} {n}" if a.base else f"  {kc:13} {n}")
    for (kb, kc), items in shown.items():
        print(f"\n== {kb} -> {kc} ({len(items)})")
        for (doc, expr), jq, base, cand in items[: a.show]:
            print(f"  doc={doc}  {expr}\n     jq={jq}" + (f"\n     base={base}" if base else "") + f"\n     bin={cand}")
    if a.dump:
        with open(a.dump, "w") as out:
            json.dump(
                {f"{kb}->{kc}": [list(item[0]) for item in items] for (kb, kc), items in shown.items()},
                out,
            )
    if failed:
        print("\nFAIL: a timeout, a lost match, a new wrong accept or (--jq-gate) a wrong answer (see above)", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
