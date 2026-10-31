#!/usr/bin/env python3
"""Grammar-based differential fuzz of `succinctly jq` against the pinned jq (#5552, item B).

Random jq programs are generated from a typed grammar with a fixed seed, run on random JSON
documents under both tools, and compared on exit status, stdout and the *normalized* first line
of stderr. A divergence is minimized by replacing subtrees with their children or with a small
literal while the same kind of divergence persists, then printed as a one-line repro.

The grammar is broad and shallow on purpose: paths and slices, arithmetic and comparison,
`//`, `?`, `try`, `if`, `reduce`/`foreach`, `as` bindings and destructuring, object/array/string
construction, assignment operators, format strings, and about sixty builtins. Every child is
parenthesized when rendered, so precedence is never the thing under test. Programs that loop
forever are cut off by `--timeout`, and a timeout on either side is not evidence.

Classification:

    agree        same exit, same stdout, same normalized error
    mismatch     both exit 0 and the stdout differs, or both fail with a different stdout/error
    ours-only    succinctly answers (exit 0) where jq refuses
    theirs-only  jq answers where succinctly refuses
    skip         the program does not compile in jq (a generator bug, not evidence), a timeout,
                 or an oracle run that crashed

Usage:
    cargo build --release --features cli,simd,regex,serde
    scripts/jq-grammar-fuzz.py [--bin PATH] [--jq PATH] [-n N] [--seed S] [--show K] [--depth D]
    scripts/jq-grammar-fuzz.py --self-test        # the alphabet, so a shrink is visible
Exit 1 on any divergence. `--jq` defaults to /usr/bin/jq, which must be jq 1.7.1.
"""
import argparse
import json
import random
import re
import subprocess
import sys
import time

KEYS = ["a", "b", "c"]
SMALL_NUMBERS = ["0", "1", "2", "-1", "3", "0.5", "10"]
STRINGS = ['""', '"a"', '"abc"', '"b,c"', '"1"', '"x y"', '"A"']

ARITY0 = [
    "length", "keys", "values", "type", "add", "any", "all", "flatten", "floor", "sqrt", "tostring",
    "tonumber", "ascii_downcase", "ascii_upcase", "explode", "implode", "reverse", "sort", "unique",
    "min", "max", "to_entries", "from_entries", "tojson", "fromjson", "not", "first", "last",
    "transpose", "empty", "keys_unsorted", "arrays", "objects", "scalars", "strings", "numbers",
    "nulls", "booleans", "iterables", "ltrimstr(\"a\")", "tostream", "paths",
    "fabs", "utf8bytelength", "infinite", "nan", "isnormal", "isnan", "isinfinite",
]
ARITY1 = [
    "map({})", "select({})", "has({})", "contains({})", "inside({})", "index({})", "rindex({})",
    "indices({})", "split({})", "join({})", "test({})", "startswith({})", "endswith({})",
    "sort_by({})", "group_by({})", "unique_by({})", "min_by({})", "max_by({})", "first({})",
    "last({})", "recurse({})", "with_entries({})", "walk({})", "del({})", "path({})", "paths({})",
    "getpath({})", "delpaths({})", "to_entries", "limit(2; {})", "any({})", "all({})",
    "map_values({})", "ltrimstr({})", "rtrimstr({})", "splits({})", "ascii", "tojson",
    "add({})", "flatten({})", "range({})", "pick({})", "error({})", "isempty({})",
    "getpath([{}])", "splits({}; \"g\")", "sub({}; \"X\")", "gsub({}; \"X\")",
    "ltrimstr(\"x\") | {}", "tostream | {}", "@base64d", "limit({}; .[])", "nth({}; .[])",
    "walk(if type == \"number\" then . + 1 else . end) | {}",
]
FORMATS = ["@text", "@json", "@csv", "@tsv", "@html", "@uri", "@sh", "@base64", "@base64d"]
BINOPS = ["+", "-", "*", "/", "%", "==", "!=", "<", "<=", ">", ">=", "and", "or", "//"]
ASSIGN = ["=", "|=", "+=", "-=", "*=", "/=", "//="]
PATHS = [".", ".a", ".b", ".c", ".[0]", ".[1]", ".[-1]", ".[]", ".[]?", "..", ".a.b", ".a[0]", ".[0].a",
         ".[:2]", ".[1:]", ".[-1:]", ".a?", ".a[]?", ".[\"a\"]", ".a // .b"]


class Node:
    """A program fragment: `fmt` with one `{}` per child, rendered with every child in parens."""

    __slots__ = ("fmt", "kids")

    def __init__(self, fmt, kids=()):
        self.fmt = fmt
        self.kids = list(kids)

    def render(self):
        return self.fmt.format(*[wrap(k) for k in self.kids])


SIMPLE = re.compile(r'^[\w.$\[\]?"]+$')


def wrap(node):
    """A child in parentheses unless it is a bare token, so precedence is never under test."""
    text = node.render()
    return text if not node.kids and SIMPLE.match(text) else "(" + text + ")"


def atom(text):
    return Node(text.replace("{", "{{").replace("}", "}}"))


def gen(rng, depth, env):
    """A random expression of at most `depth` levels; `env` is the set of bound `$names`."""
    if depth <= 0 or rng.random() < 0.18:
        choices = PATHS + SMALL_NUMBERS + STRINGS + ["null", "true", "false", "[]", "{}", "[1,2,3]",
                                                      '{"a":1}', '[{"a":1},{"a":2}]', '"\\(.)"']
        choices += ["$" + v for v in env] * 3
        return atom(rng.choice(choices))
    d = depth - 1
    k = rng.randrange(0, 26)
    sub = lambda: gen(rng, d, env)
    if k == 0:
        return Node("{} | {}", [sub(), sub()])
    if k == 1:
        return Node("{} , {}", [sub(), sub()])
    if k in (2, 3):
        return Node("{} " + rng.choice(BINOPS) + " {}", [sub(), sub()])
    if k == 4:
        return Node("[{}]", [sub()])
    if k == 5:
        return Node("{{a: {}}}", [sub()])
    if k == 6:
        return Node("{{(" + rng.choice(['"a"', '"b"']) + "): {}, b: {}}}", [sub(), sub()])
    if k == 7:
        return Node("{}?", [sub()])
    if k == 8:
        return Node("try {} catch {}", [sub(), sub()])
    if k == 9:
        return Node("try {}", [sub()])
    if k == 10:
        return Node("if {} then {} else {} end", [sub(), sub(), sub()])
    if k == 11:
        return Node("if {} then {} end", [sub(), sub()])
    if k == 12:
        v = "x" + str(len(env))
        return Node("reduce {} as $" + v + " ({}; {})", [sub(), sub(), gen(rng, d, env | {v})])
    if k == 13:
        v = "x" + str(len(env))
        e2 = env | {v}
        if rng.random() < 0.5:
            return Node("foreach {} as $" + v + " ({}; {}; {})", [sub(), sub(), gen(rng, d, e2), gen(rng, d, e2)])
        return Node("foreach {} as $" + v + " ({}; {})", [sub(), sub(), gen(rng, d, e2)])
    if k == 14:
        v = "v" + str(len(env))
        return Node("{} as $" + v + " | {}", [sub(), gen(rng, d, env | {v})])
    if k == 15:
        v1, v2 = "p" + str(len(env)), "q" + str(len(env))
        pat = rng.choice(["[$%s, $%s]" % (v1, v2), "{{a: $%s, b: $%s}}" % (v1, v2)])
        return Node("{} as " + pat + " | {}", [sub(), gen(rng, d, env | {v1, v2})])
    if k in (16, 17):
        return Node("{} " + rng.choice(ASSIGN) + " {}", [Node(rng.choice(PATHS)), sub()])
    if k == 18:
        return Node("{} | " + rng.choice(FORMATS), [sub()])
    if k == 19:
        return Node("{} | " + rng.choice(ARITY0), [sub()])
    if k in (20, 21, 22):
        tpl = rng.choice(ARITY1)
        n = tpl.count("{}")
        return Node("{} | " + tpl.replace("{}", "\0", n).replace("\0", "{}"), [sub() for _ in range(n + 1)])
    if k == 23:
        return Node('"\\({})-\\({})"', [sub(), sub()])
    if k == 24:
        return Node("{}[{}:{}]", [sub(), sub(), sub()])
    return Node("{}[{}]", [sub(), sub()])


def gen_json(rng, depth):
    if depth <= 0 or rng.random() < 0.3:
        return rng.choice([None, True, False, 0, 1, 2, -1, 3, 0.5, 10, "", "a", "abc", "b,c", "1", "x y"])
    if rng.random() < 0.5:
        return [gen_json(rng, depth - 1) for _ in range(rng.randrange(0, 4))]
    return {k: gen_json(rng, depth - 1) for k in rng.sample(KEYS, rng.randrange(0, 4))}


def normalize_err(text):
    """First stderr line without the position and the value rendering: only the fault's class."""
    line = text.strip().split("\n")[0] if text.strip() else ""
    line = re.sub(r"\(at [^)]*\)", "(at X)", line)
    line = re.sub(r"jq: error \(at X\)( \(not a string\))?: ", "jq: error: ", line)
    line = re.sub(r"\s*\([^()]*\)$", "", line) if line.startswith("jq: error") else line
    # A rendered value or key follows the fault's words, and jq cuts it (and sometimes the words)
    # at a length of its own: compare the fault up to the first quote or bracket only.
    line = re.split(r' ["\[{]|\("|: ["\[{]', line)[0]
    line = re.sub(r"( with result | of | and | near attempt to \w+ \w+ of )\S.*$", r"\1...", line)
    return line


def run(cmd, program, doc, timeout):
    try:
        r = subprocess.run(cmd + ["-c", program], input=doc.encode(), capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None
    return r.returncode, r.stdout.decode("utf-8", "replace"), normalize_err(r.stderr.decode("utf-8", "replace"))


def classify(ours, theirs):
    if ours is None or theirs is None:
        return "skip"
    if theirs[0] == 3:
        return "skip"  # jq does not compile it: the generator's mistake, not evidence
    if theirs[0] not in (0, 5) or ours[0] in (3, 4):
        # jq crashed or halted differently, or we refused to compile what jq compiled
        return "theirs-only" if theirs[0] in (0, 5) else "skip"
    if ours == theirs:
        return "agree"
    if ours[0] == 0 and theirs[0] != 0:
        return "ours-only"
    if ours[0] != 0 and theirs[0] == 0:
        return "theirs-only"
    return "mismatch"


def shrink(node, still_diverges):
    """Greedy tree reduction: replace a subtree by a child or a small literal while it still does."""
    changed = True
    while changed:
        changed = False
        for parent, idx, child in list(walk(node)):
            size = len(child.render())
            options = [o for o in list(child.kids) + [atom("."), atom("null"), atom("1")]
                       if len(o.render()) < size]
            for opt in options:
                if parent is None:
                    trial_root = opt
                else:
                    old = parent.kids[idx]
                    parent.kids[idx] = opt
                    trial_root = node
                ok = still_diverges(trial_root)
                if ok:
                    if parent is None:
                        node = trial_root
                    changed = True
                    break
                if parent is not None:
                    parent.kids[idx] = old
            if changed:
                break
    return node


def walk(node, parent=None, idx=0):
    yield parent, idx, node
    for i, k in enumerate(list(node.kids)):
        yield from walk(k, node, i)


def signature(res, ours, theirs):
    """What a divergence looks like without the program: one finding per signature is minimized."""
    return (res, ours[0] if ours else None, theirs[0] if theirs else None,
            (theirs[2] or "")[:30], (ours[2] or "")[:30], bool(ours and theirs and ours[1] == theirs[1]))


def main():
    from concurrent.futures import ThreadPoolExecutor

    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default="target/release/succinctly")
    ap.add_argument("--jq", default="/usr/bin/jq")
    ap.add_argument("-n", type=int, default=2000)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--depth", type=int, default=3)
    ap.add_argument("--show", type=int, default=12, help="findings to print")
    ap.add_argument("--shrink", type=int, default=24, help="distinct signatures to minimize")
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--timeout", type=float, default=2.0)
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        print("arity0:", len(ARITY0), "arity1:", len(ARITY1), "formats:", len(FORMATS), "paths:", len(PATHS))
        rng = random.Random(a.seed)
        for _ in range(8):
            print(gen(rng, a.depth, set()).render())
        return 0
    ver = subprocess.run([a.jq, "--version"], capture_output=True, text=True).stdout.strip()
    if not ver.startswith("jq-1.7.1"):
        print(f"oracle is {ver!r}, not jq 1.7.1", file=sys.stderr)
        return 2
    ours_cmd, theirs_cmd = [a.bin, "jq"], [a.jq]
    rng = random.Random(a.seed)
    cases = []
    for _ in range(a.n):
        node = gen(rng, a.depth, set())
        cases.append((node, json.dumps(gen_json(rng, 3))))

    def both(program, doc):
        return run(ours_cmd, program, doc, a.timeout), run(theirs_cmd, program, doc, a.timeout)

    t0 = time.time()
    with ThreadPoolExecutor(a.jobs) as pool:
        results = list(pool.map(lambda c: both(c[0].render(), c[1]), cases))
    counts, groups = {}, {}
    for (node, doc), (ours, theirs) in zip(cases, results):
        res = classify(ours, theirs)
        counts[res] = counts.get(res, 0) + 1
        if res == "agree" and theirs[0] == 0:
            counts["agree-ok"] = counts.get("agree-ok", 0) + 1  # both answered: the yield that counts
        if res not in ("agree", "skip"):
            groups.setdefault(signature(res, ours, theirs), []).append((node, doc, res))
    findings = []
    for sig, members in sorted(groups.items(), key=lambda kv: -len(kv[1]))[: a.shrink]:
        node, doc, res = min(members, key=lambda m: len(m[0].render()))

        def same_kind(root, res=res, doc=doc, sig=sig):
            o, t = both(root.render(), doc)
            return classify(o, t) == res and signature(res, o, t) == sig

        small = shrink(node, same_kind)
        text = small.render()
        o, t = both(text, doc)
        findings.append((len(members), res, text, doc, o, t))
    print(f"seed={a.seed} n={a.n} depth={a.depth} {time.time() - t0:.0f}s  " +
          "  ".join(f"{k}={v}" for k, v in sorted(counts.items())) +
          f"  signatures={len(groups)}")
    for count, res, text, doc, ours, theirs in findings[: a.show]:
        print(f"\n[{res} x{count}] {text!r}  <<< {doc}\n  jq  : {theirs}\n  ours: {ours}")
    print(f"\n{len(groups)} distinct divergence signature(s); {len(findings)} minimized")
    return 1 if groups else 0


if __name__ == "__main__":
    sys.exit(main())
