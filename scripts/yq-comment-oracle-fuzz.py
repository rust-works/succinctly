#!/usr/bin/env python3
"""Randomised differential fuzz for standalone (head/foot) YAML comments
(#2795, #2811, #2518) -- the companion of `scripts/yq-alias-oracle-sweep.sh`.

Generates small block YAML documents with `#` lines at every position the
parser can attribute differently -- above/below keys and items, at column
0, at the node's column, between columns, blank-line detached on either
side, before the first node, at end of input, against a `---` boundary,
inside compact `- k: v` and nested `- - x` items, next to `&anchor`/`*alias`
and trailing ` # c` comments -- and runs each through the pinned oracle and
the built succinctly binary on every YAML-to-YAML route plus the getters.

Every mismatch is classified by *direction*, because the two directions are
not equally bad: a comment we print that yq does not is a fabrication a
reader downstream will trust, a comment yq prints that we drop is the
pre-#2795 status quo.

    same             byte-identical stdout and exit status
    we-emit-yq-doesnt  a `#` line in our output that is neither in yq's
                     output nor in the input -- a fabrication  (**gate**)
    yq-drops-we-keep   a `#` line from the input that yq's output lost and
                     ours kept: yq discarding data, which succinctly may
                     refuse to reproduce (ADR-0018) -- must be recorded
    yq-emits-we-dont   a `#` line in yq's output that is not in ours
    placement-differs  same `#` lines, different position or indent
    blank-differs      comments identical, only blank lines differ
    trailing-relocated a same-line ` # c` from the input that the DOM route
                     re-emits on its own line under a flow value it
                     re-rendered as block (pre-existing `-P` behaviour)
    attribution-differs the head/foot getters disagree (`--getters`)
    getter-blank-text  the getters agree once blank lines and indentation
                     inside a comment's text are ignored (yq keeps
                     `h1\n\nh2`, a trailing `\n`, and a header line's indent)
    yq-unsound       yq's own output does not re-parse to the same JSON as
                     its input (recorded, never reproduced -- ADR-0018 rule 4)
    values-differ    the non-comment text differs (a pre-existing divergence
                     outside this fuzz's claim; listed so it is not hidden)

**The alphabet is part of the claim** (#2041): `--self-test` prints the
pools so a shrink is visible. Deliberately *not* in the alphabet, each with
its own boundary: comments inside flow collections (yq rewrites them),
`--header-preprocess=false`, a leading `---` before the first document (yq
preserves it via header preprocessing; succinctly drops it on every route --
a separate gap), and multi-result filters (#1361 drops every comment).

Usage:
    cargo build --release --features cli
    ./scripts/yq-comment-oracle-fuzz.py [--bin PATH] [--yq PATH] [-n N] [--seed S]
                                        [--show K] [--getters] [--filters F1,F2]
Exit 1 on any we-emit-yq-doesnt.
"""
import argparse, json, random, subprocess, sys, collections

# ---------------------------------------------------------------- alphabet

SCALARS = ["1", "2", "x", "hello world", '"q"', "'s'", "true", "null", "1.5", "[1, 2]", "{k: v}"]
KEYS = ["a", "b", "c", "d", '"e"', "f g"]
COMMENT_TEXT = ["# c{}", "#c{}", "# c{} tail", "#", "# c{}  "]

# Where a standalone comment line's column is drawn from, relative to the
# block it sits in: its own indent, column 0, deeper, one column shallower
# (between two open blocks), or the parent's indent.
COLUMN_CHOICES = ["own", "own", "own", "zero", "deeper", "shallower", "parent"]

# Routes. `-o=json .` is the negative control: comments must not appear.
DEFAULT_FILTERS = [
    ([], "."),
    (["-P"], "."),
    ([], ".a = 5"),
    ([], "del(.b)"),
    ([], "select(true)"),
    (["-o=json"], "."),
]

GETTER_FILTER = (
    '[.. | {"p": path, "h": [head_comment], "f": [foot_comment],'
    ' "kh": [key | head_comment], "kf": [key | foot_comment]}]'
)


class Gen:
    def __init__(self, rng):
        self.rng = rng
        self.n = 0
        self.anchors = []

    def comment_line(self, own_indent, parent_indent):
        self.n += 1
        text = self.rng.choice(COMMENT_TEXT).format(self.n)
        col = self.rng.choice(COLUMN_CHOICES)
        if col == "own":
            c = own_indent
        elif col == "zero":
            c = 0
        elif col == "deeper":
            c = own_indent + 2
        elif col == "shallower":
            c = max(own_indent - 1, 0)
        else:
            c = parent_indent
        return " " * c + text

    def comment_block(self, own_indent, parent_indent, lines):
        """0-2 comment lines, optionally blank-detached before/after/inside."""
        r = self.rng.random()
        if r < 0.55:
            return
        if self.rng.random() < 0.3:
            lines.extend([""] * self.rng.choice([1, 1, 2]))
        k = 1 if self.rng.random() < 0.7 else 2
        for i in range(k):
            lines.append(self.comment_line(own_indent, parent_indent))
            if i + 1 < k and self.rng.random() < 0.25:
                lines.append("")
        if self.rng.random() < 0.3:
            lines.append("")

    def trailing(self):
        if self.rng.random() < 0.15:
            self.n += 1
            return " # t%d" % self.n
        return ""

    def scalar(self):
        s = self.rng.choice(SCALARS)
        if self.rng.random() < 0.08:
            name = "x%d" % len(self.anchors)
            self.anchors.append(name)
            return "&%s %s" % (name, s)
        if self.anchors and self.rng.random() < 0.08:
            return "*" + self.rng.choice(self.anchors)
        return s

    def mapping(self, indent, parent_indent, depth, lines, first_inline=None):
        pad = " " * indent
        keys = self.rng.sample(KEYS, self.rng.choice([1, 2, 2, 3]))
        for i, key in enumerate(keys):
            if not (i == 0 and first_inline is not None):
                self.comment_block(indent, parent_indent, lines)
            prefix = first_inline if (i == 0 and first_inline is not None) else pad
            self.entry(prefix, indent, depth, lines, key)
        self.comment_block(indent, parent_indent, lines)

    def entry(self, prefix, indent, depth, lines, key):
        r = self.rng.random()
        if depth < 3 and r < 0.3:
            lines.append("%s%s:%s" % (prefix, key, self.trailing()))
            self.block(indent + 2, indent, depth + 1, lines)
        else:
            lines.append("%s%s: %s%s" % (prefix, key, self.scalar(), self.trailing()))

    def sequence(self, indent, parent_indent, depth, lines, first_inline=None):
        pad = " " * indent
        for i in range(self.rng.choice([1, 2, 2, 3])):
            if not (i == 0 and first_inline is not None):
                self.comment_block(indent, parent_indent, lines)
            prefix = first_inline if (i == 0 and first_inline is not None) else pad
            self.item(prefix, indent, depth, lines)
        self.comment_block(indent, parent_indent, lines)

    def item(self, prefix, indent, depth, lines):
        r = self.rng.random()
        if depth < 3 and r < 0.2:
            # compact `- k: v` mapping item
            self.mapping(indent + 2, indent, depth + 1, lines, first_inline=prefix + "- ")
        elif depth < 3 and r < 0.3:
            # nested `- - x`
            self.sequence(indent + 2, indent, depth + 1, lines, first_inline=prefix + "- ")
        elif depth < 3 and r < 0.4:
            # deferred: `-` then an indented block
            lines.append(prefix + "-" + self.trailing())
            self.block(indent + 2, indent, depth + 1, lines)
        else:
            lines.append("%s- %s%s" % (prefix, self.scalar(), self.trailing()))

    def block(self, indent, parent_indent, depth, lines):
        if self.rng.random() < 0.6:
            self.mapping(indent, parent_indent, depth, lines)
        else:
            self.sequence(indent, parent_indent, depth, lines)

    def document(self, lines):
        # Leading block before the first node (document head).
        self.comment_block(0, 0, lines)
        r = self.rng.random()
        if r < 0.1:
            lines.append(self.scalar())
        else:
            self.block(0, 0, 0, lines)
        self.comment_block(0, 0, lines)


def gen_doc(rng):
    g = Gen(rng)
    lines = []
    ndocs = 1 if rng.random() < 0.75 else 2
    for d in range(ndocs):
        if d > 0:
            lines.append("---")
        if rng.random() < 0.05:
            # comment-only document
            lines.append(g.comment_line(0, 0))
            continue
        g.document(lines)
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------- running

def run(argv, stdin, timeout=10):
    try:
        p = subprocess.run(argv, input=stdin, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return -1, "", "timeout"


def comment_lines(text):
    """Every standalone `#` line as (stripped text, column), in order."""
    out = []
    for line in text.split("\n"):
        s = line.lstrip(" ")
        if s.startswith("#"):
            out.append((s.rstrip(), len(line) - len(s)))
    return out


def non_comment_text(text):
    return "\n".join(l for l in text.split("\n") if not l.lstrip(" ").startswith("#") and l.strip() != "")


def trailing_comments(doc):
    """Same-line ` # c` comments in the input (heuristic: no `#` inside a
    quoted scalar in the alphabet)."""
    out = set()
    for line in doc.split("\n"):
        if line.lstrip(" ").startswith("#"):
            continue
        i = line.find(" #")
        if i >= 0:
            out.add(line[i + 1:].rstrip())
    return out


def classify(yq_out, yq_rc, our_out, our_rc, is_json, doc):
    if yq_rc == our_rc and yq_out == our_out:
        return "same"
    if yq_rc != our_rc:
        return "values-differ"
    if is_json:
        return "values-differ"
    if non_comment_text(yq_out) != non_comment_text(our_out):
        return "values-differ"
    yc = collections.Counter(t for t, _ in comment_lines(yq_out))
    oc = collections.Counter(t for t, _ in comment_lines(our_out))
    if oc - yc:
        # A trailing ` # c` the DOM route relocates onto its own line under
        # a flow value it re-rendered as block (pre-existing `-P` behaviour,
        # not a standalone comment): reported, not gated.
        if all(t in trailing_comments(doc) for t in (oc - yc)):
            return "trailing-relocated"
        # Every extra line is a standalone comment the *input* holds and
        # yq's own output lost: yq discarding data, which ADR-0018 lets
        # succinctly refuse to reproduce -- reported, recorded, not gated.
        ic = collections.Counter(t for t, _ in comment_lines(doc))
        if not ((oc - yc) - (ic - yc)):
            return "yq-drops-we-keep"
        return "we-emit-yq-doesnt"
    if yc - oc:
        return "yq-emits-we-dont"
    if comment_lines(yq_out) != comment_lines(our_out):
        return "placement-differs"
    return "blank-differs"


def strip_blank_text(getter_json):
    """The getter output with every blank line inside a comment's text
    removed (yq keeps `h1\n\nh2` and a trailing `f\n`; the parser records
    only the `#` lines), so attribution can be compared on its own."""
    try:
        rows = json.loads(getter_json)
    except ValueError:
        return getter_json
    for row in rows:
        for k in ("h", "f", "kh", "kf"):
            row[k] = ["\n".join(l.strip() for l in v.split("\n") if l.strip()) for v in row.get(k, [])]
    return json.dumps(rows, sort_keys=True)


def yq_unsound(yq, doc, extra, filt, yq_out):
    """yq's own YAML output, re-parsed, is not the JSON yq gives for the
    same filter on the same input -- its emitter printed something its
    parser reads back differently (a comment swallowing a value, an alias
    before its anchor, ...)."""
    extra = [e for e in extra if e != "-P"]
    rc_a, a, _ = run([yq, "-o=json", "-I=0"] + extra + [filt], doc)
    rc_b, b, _ = run([yq, "-o=json", "-I=0", "."], yq_out)
    return rc_a != rc_b or a != b


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default="./target/release/succinctly")
    ap.add_argument("--yq", default="yq")
    ap.add_argument("-n", type=int, default=300)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--show", type=int, default=3, help="examples to print per class")
    ap.add_argument("--getters", action="store_true", help="also compare head/foot getters")
    ap.add_argument("--filters", default=None,
                    help="comma-separated subset of the routes, spelled as printed (e.g. '.,-P .')")
    ap.add_argument("--only", default=None, help="print examples for this class only")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        print("SCALARS:", SCALARS)
        print("KEYS:", KEYS)
        print("COMMENT_TEXT:", COMMENT_TEXT)
        print("COLUMN_CHOICES:", COLUMN_CHOICES)
        print("FILTERS:", DEFAULT_FILTERS)
        print("GETTER_FILTER:", GETTER_FILTER)
        rng = random.Random(args.seed)
        for _ in range(5):
            print("----")
            print(gen_doc(rng), end="")
        return 0

    filters = DEFAULT_FILTERS
    if args.filters:
        want = set(args.filters.split(","))
        filters = [f for f in filters if " ".join(f[0] + [f[1]]) in want]

    rng = random.Random(args.seed)
    counts = collections.Counter()
    examples = collections.defaultdict(list)
    for i in range(args.n):
        doc = gen_doc(rng)
        # Skip documents yq itself cannot parse (the generator can emit an
        # alias before its anchor across documents, etc.).
        rc, _, _ = run([args.yq, "-o=json", "."], doc)
        if rc != 0:
            counts["skipped-invalid"] += 1
            continue
        for extra, filt in filters:
            yrc, yout, _ = run([args.yq] + extra + [filt], doc)
            orc, oout, _ = run([args.bin, "yq"] + extra + [filt], doc)
            is_json = "-o=json" in extra
            cls = classify(yout, yrc, oout, orc, is_json, doc)
            if cls != "same" and not is_json and yrc == 0 and yq_unsound(args.yq, doc, extra, filt, yout):
                cls = "yq-unsound"
            key = (cls, " ".join(extra + [filt]))
            counts[key] += 1
            if cls != "same" and len(examples[key]) < args.show:
                examples[key].append((doc, yout, oout))
        if args.getters:
            yrc, yout, _ = run([args.yq, "-o=json", "-I=0", GETTER_FILTER], doc)
            orc, oout, _ = run([args.bin, "yq", "-o=json", "-I=0", GETTER_FILTER], doc)
            if yrc == orc and yout == oout:
                cls = "same"
            elif yrc == orc and strip_blank_text(yout) == strip_blank_text(oout):
                cls = "getter-blank-text"
            else:
                cls = "attribution-differs"
            key = (cls, "getters")
            counts[key] += 1
            if cls != "same" and len(examples[key]) < args.show:
                examples[key].append((doc, yout, oout))

    print("%-22s %-14s %s" % ("class", "count", "route"))
    for (cls, route), n in sorted(counts.items(), key=lambda kv: (str(kv[0][0]), str(kv[0][1]))):
        print("%-22s %-14d %s" % (cls, n, route))
    for (cls, route), exs in sorted(examples.items()):
        if args.only and cls != args.only:
            continue
        for doc, yout, oout in exs:
            print("\n=== %s  [%s]\n--- input:\n%s--- yq:\n%s--- succinctly:\n%s" % (cls, route, doc, yout, oout))
    bad = sum(n for (cls, _), n in counts.items() if cls == "we-emit-yq-doesnt")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
