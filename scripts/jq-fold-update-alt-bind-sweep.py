#!/usr/bin/env python3
"""Differential grid for a `foreach`/`reduce` UPDATE that is a `//` over a destructuring bind of an outer `$x` (#5157).

`jq-reduce-update-bind-sweep.py` has one `// 7` shape (and only around a `try`), and no `foreach` UPDATE
with a bind, so neither gates the shape #5157 changed: `(B | .) // 5` as the UPDATE, where `B` destructures
a `$x` bound outside the fold. jq's `a // b` does not restore its path register when `a` emits a truthy
output, so a bind that stepped a `null` `$x` against a `null` register has moved the register for every
later step, while a falsy output backtracks to the entry register and runs `b` from there. The grid here is
document x INIT x SOURCE x BIND x TAIL x WRAPPER x EXTRACT x write context (`--fold foreach2` is the
two-argument `foreach`, whose UPDATE output is emitted and path-checked, and `--fold reduce` drops EXTRACT).

Rows are classified against jq (`/usr/bin/jq`) per build, as `jq-reduce-update-bind-sweep.py` does:
MATCH, MSGDIFF (same stdout and exit code, different error wording), ACCEPT_WRONG (jq refused, the build
answered: the dangerous one, `del`/`=`/`|=` write through it), REFUSE_WRONG (the safe direction) and DIFF.
With `--base`, exit status 1 when a row that matched jq on the base no longer does (a lost match: a
refuse-only fallback leaked under `try` before, #3186/#3267), a row that was not an ACCEPT_WRONG on the
base is one on `--bin`, or a row the base refused now answers wrongly (DIFF). A wrong answer both builds
share is listed, not failed. A timeout of `--bin` that `--base` did not have fails. Without `--base` only that
timeout fails, unless `--jq-gate` is given: it fails the run on any ACCEPT_WRONG or DIFF of `--bin` against jq as
well, with or without `--base` (MSGDIFF and REFUSE_WRONG stay listed only). No residual is documented today, and it
is how `succinctly dev compare jq --dimension sweeps` runs this script (#5552); a residual that becomes accepted has
to be listed here first.

The `foreach` product is 6.9M rows, so `--sample K` keeps about one row in K, chosen by a hash of the row's index
and `--seed` while the grid is generated (a list of the whole grid passed a 4 GB cap before #5552), and `--only`
narrows to rows containing a substring.

Usage: scripts/jq-fold-update-alt-bind-sweep.py --bin BIN [--base BIN] [--jq JQ] [--fold foreach|foreach2|reduce]
                                                [--sample K] [--seed S] [--only SUBSTR[,SUBSTR...]]
                                                [--jobs N] [--show N] [--dump FILE] [--jq-gate]
"""
import argparse
import collections
import concurrent.futures as cf
import itertools
import json
import re
import subprocess
import sys

DOCS = ["null", '{"a":null}', '{"a":1}', "[1]", '{"a":{"b":1}}', "true", "false"]
INITS = [".", "5", "null", ".a", "false"]
SRCS = ["(.a,.)", "(.,.a)", "(1,2)", ".a", ".", "(.a,.a)"]
BINDS = [
    "$x as {a:$a}",
    "$x as [$a]",
    "$x as $a",
    "$x as {a:{b:$a}}",
    ". as {a:$a}",
    "$x as {a:$a} ?// $a",
]
# What the bind ends on: the accumulator, a literal, the bound variable, a navigation, the outer marker.
TAILS = [".", "5", "$a", ".a", "$x", "(.,.)", "range(2)"]
# {S} is the bind plus its tail, parenthesised.
WRAPPERS = [
    "({S}) // 5",
    "({S}) // null",
    "({S}) // false",
    "({S}) // [1]",
    "({S}) // {}",
    "({S}) // (1,2)",
    # an operand that is not a constant: the loop's `$i` (a marker over a source that navigates), the outer `$x`, `.` and
    # navigations can be the register's node, which the emission's check accepts where a constant raises (#5167)
    "({S}) // $i",
    "({S}) // (6, $i)",
    "try (({S}) // $i)",
    "({S}) // $x",
    "({S}) // .a",
    "({S}) // .",
    "({S}) // not",
    "({S}) // length",
    "({S}) // ($i + 1)",
    "({S}) // .a?",
    "({S}) // (.a, 6)",
    "try (({S}) // .)",
    "(({S}) // 5) // 6",
    "(({S}) // 5) | .",
    # #5162: the `| .` spelling, over the other constants and with a `def` in the left operand
    "(({S}) // (1,2)) | .",
    "(({S}) // {}) | .",
    "(({S}) // [1]) | .",
    "({S}) // 5 | . | .",
    "(def f: .; {S}) // 5 | .",
    "5 | (({S}) // 6)",
    "try (({S}) // 5)",
    "(({S}) // 5)?",
    "(({S}), 7) // 5",
    "(7, ({S})) // 5",
    "(({S}), null) // 5",
    "(({S}), $i) // 5",
    "(({S}), .a) // 5",
    "({S}), 7",
    "7, ({S})",
    "if true then (({S}) // 5) else . end",
    "limit(1; ({S}) // 5)",
    "(try ({S})) // 5",
    # #5175: a right operand that holds a bind (jq runs it from the register the `//` was entered with, only after the left
    # operand backtracked) or a call
    "try (({S}) // .a as $q | 1)",
    "try (({S}) // (.a as $q | $q))",
    "try (({S}) // ($x as {a:$a} | .))",
    "({S}) // ($x as {a:$a} | .)",
    "try (({S}) // (def f: .; f))",
    # #5175 residuals: a comma, an `if` or a `?//` chain around the binds, a generator source
    "try (({S}) // ((.a as $q | $q), 3))",
    "try (({S}) // (if .a then (.a as $q | $q) else 1 end))",
    "try (({S}) // ($x as {a:$a} ?// $a | $a))",
    "try (({S}) // ((1,2) as $q | $q))",
    # #5175, the right operand beyond a bare chain: wrappers around chains and bind-free items, and a generator source
    "try (({S}) // ((.a as $q | $q), (.a as $q | 1)))",
    "try (({S}) // ((.a as $q | range(2)), 3))",
    "try (({S}) // (if true then (.a as $q | range(2)) else 1 end))",
    "try (({S}) // (if true then (.a as $q | $q) else 1 end))",
    "try (({S}) // (if $i then (.a as $q | $q) else ($x as {a:$a} | .) end))",
    "try (({S}) // (.a as $q | $q, 3))",
    "try (({S}) // (3, (.a as $q | .)))",
    "try (({S}) // ((1,2) as $q | .))",
    "try (({S}) // (.[] as $q | .))",
    "try (({S}) // (.[] as $q | $q))",
    "try (({S}) // ((.a, .) as $q | 1))",
    # #5175: a bind whose source navigates an outer `$x` (a source runs in a subexpression that tracks no path)
    "try (({S}) // ($x.a as $q | $q))",
    "try (({S}) // ($x.a.b as {c:$c} | .))",
    "try (({S}) // ($x.a as $q | $x.a as $r | 1))",
    "try (({S}) // (($x.a, .) as $q | 1))",
    # a navigated loop variable can be the register's node: the shipped resolver kept refusing it (#5226), the path-mode
    # interpreter (#5223) answers it as jq does
    "try (({S}) // ($i.a as $q | $q))",
    # controls: no `//`, so a change to the `//` code must not move them
    "{S}",
    "try ({S})",
]
EXTRACTS = ["$i", ".", ".a", "$x"]
CONTEXTS = ["path({E})", "del({E})", "({E}) |= 9"]
PREFIX = ". as $x | {R}"


def grid(fold):
    extracts = EXTRACTS if fold == "foreach" else [None]  # `foreach2` and `reduce` have no EXTRACT
    for init, src, bind, tail, wrapper, extract in itertools.product(
        INITS, SRCS, BINDS, TAILS, WRAPPERS, extracts
    ):
        upd = wrapper.replace("{S}", f"{bind} | {tail}")
        if fold == "foreach":
            body = f"foreach {src} as $i ({init}; {upd}; {extract})"
        elif fold == "foreach2":
            body = f"foreach {src} as $i ({init}; {upd})"
        else:
            body = f"reduce {src} as $i ({init}; {upd})"
        for ctx in CONTEXTS:
            expr = ctx.replace("{E}", PREFIX.replace("{R}", body))
            for doc in DOCS:
                yield doc, expr


def _mix64(x):
    """splitmix64's finalizer: a fixed, well-spread 64-bit hash, the same on every machine and Python."""
    x = (x + 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF
    x = ((x ^ (x >> 30)) * 0xBF58476D1CE4E5B9) & 0xFFFFFFFFFFFFFFFF
    x = ((x ^ (x >> 27)) * 0x94D049BB133111EB) & 0xFFFFFFFFFFFFFFFF
    return x ^ (x >> 31)


def sampled(rows, k, seed):
    """About one row in `k` of `rows`, chosen by a hash of its index and `seed`, without listing `rows`."""
    if k <= 1:
        yield from rows
        return
    for i, row in enumerate(rows):
        if _mix64((seed << 40) ^ i) % k == 0:
            yield row


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
    ap.add_argument("--fold", choices=["foreach", "foreach2", "reduce"], default="foreach")
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--show", type=int, default=10, help="rows printed per transition")
    ap.add_argument("--sample", type=int, default=1, help="keep about one row in K, by a seeded hash of its index")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--only", help="keep rows whose program contains one of these substrings")
    ap.add_argument("--dump", help="write every row of a transition worth reading, as JSON")
    ap.add_argument(
        "--jq-gate",
        action="store_true",
        help="also fail on any ACCEPT_WRONG or DIFF row of --bin against jq (the gate without --base, #5552)",
    )
    a = ap.parse_args()

    rows = grid(a.fold)
    if a.only:
        needles = a.only.split(",")
        rows = (r for r in rows if any(n in r[1] for n in needles))
    rows = list(sampled(rows, a.sample, a.seed))
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
                (kb == "MATCH" and kc != "MATCH")
                or (kb != "ACCEPT_WRONG" and kc == "ACCEPT_WRONG")
                or (kb == "REFUSE_WRONG" and kc == "DIFF")
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
        why = "a lost match, a new wrong accept" + (" or a wrong answer against jq" if a.jq_gate else "")
        print(f"\nFAIL: {why} (see above)", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
