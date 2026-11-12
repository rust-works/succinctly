#!/usr/bin/env python3
"""Where the comment block at the top of the first document goes behind a UTF-8 byte-order mark (#7084).

yq sets no header aside after a mark (#6762), so the block above the first document is placed by go-yaml's
ordinary rules. This builds every leading block of up to four lines (a comment, a blank line, a `---`, at most
one marker) before six bodies (a map, a map with a line comment, a nested map, a sequence, a scalar, a flow
collection), alone, followed by a second document and followed by a foot comment, and runs three outputs of
each (the head and foot comment of the document and of every key and item, `.`, and `-o=props`) through
the pinned `yq` and through `succinctly yq`: 4,320 runs, a few minutes.

    scripts/yq-bom-leading-block-sweep.py TARGET_BIN [BASE_BIN]

With a second binary (a build of `main`) each difference is classed: `fixed` (base differed, TARGET matches),
`REGRESSION` (base matched, TARGET differs: the line that fails the run), `both` (neither matches, and they
differ from each other) and `unchanged`. `SHOW=n` prints the first n differences (default 40). Exit 1 on a
regression. Standard library only; run under `.ai/scratch/memcap.py` like any other run of succinctly.
"""
import itertools, os, subprocess, sys, tempfile
BIN = sys.argv[1]
BASE = sys.argv[2] if len(sys.argv) > 2 else None
YQ = "yq"
F = '[head_comment, foot_comment] + [.. | select(tag=="!!map") | keys[] | [head_comment, foot_comment]] + [.. | select(tag=="!!seq") | .[] | [head_comment, foot_comment]]'
PIECES = {"C": "# c%d\n", "B": "\n", "M": "---\n"}
BODIES = {"map": "a: 1\n", "map1": "a: 1 # one\n", "nested": "a:\n  b: 1\n", "seq": "- 1\n", "scalar": "5\n", "flow": "{a: 1}\n"}
TAILS = {"": "", "doc2": "---\n# t1\n\n# t2\nb: 2\n", "footc": "# f1\n"}
def run(cmd, path):
    p = subprocess.run(cmd + [path], capture_output=True, timeout=30)
    return p.stdout + b"|" + p.stderr[:200], p.returncode
total = diff = 0
counts = {}
seen = {}
with tempfile.TemporaryDirectory() as d:
    path = os.path.join(d, "x.yaml")
    for n in range(0, 5):
        for combo in itertools.product("CBM", repeat=n):
            if combo.count("M") > 1: continue
            k = 0
            lead = ""
            for p in combo:
                lead += PIECES[p] % (k + 1) if p == "C" else PIECES[p]
                if p == "C": k += 1
            for bname, body in BODIES.items():
                for tname, tail in TAILS.items():
                    text = "\ufeff".encode() + (lead + body + tail).encode()
                    open(path, "wb").write(text)
                    for label, args in (("F", ["-o=json", "-I0", F]), ("dot", ["."]), ("props", ["-o=props", "."])):
                        a = run([YQ] + args, path); b = run([BIN, "yq"] + args, path)
                        total += 1
                        base = run([BASE, "yq"] + args, path) if BASE else None
                        cls = "same" if a == b else "diff"
                        if base is not None:
                            if a == b and base != a: cls = "fixed"
                            elif a != b and base == a: cls = "REGRESSION"
                            elif a != b and base != a: cls = "both" if base != b else "unchanged"
                        counts[cls] = counts.get(cls, 0) + 1
                        if a != b and cls in ("diff", "REGRESSION", "both", "unchanged"):
                            diff += 1
                            key = (cls, label, "".join(combo), bname, tname)
                            seen[key] = (text, a, b)
for (cls, label, combo, bname, tname), (text, a, b) in sorted(seen.items())[:int(os.environ.get("SHOW", "40"))]:
    print(cls, label, combo or "-", bname, tname or "-", repr(text[3:]))
    print("   yq  :", a[0][:160], a[1])
    print("   succ:", b[0][:160], b[1])
print("total", total, "diffs", diff, counts)
sys.exit(1 if counts.get("REGRESSION") else 0)
