#!/usr/bin/env python3
"""Differential sweep for #6412 and #6419: random streams of directive lines (`%FOO`, `%TAG ...`, `%YAML`) around `---` and `...`.

    scripts/yq-directive-name-sweep.py SEED COUNT SUBJECT [BASELINE]

Each stream is two to five lines drawn from a pool (well-formed and malformed `%TAG` and `%YAML` lines, unknown and empty directive
names, `%YAML..` lines yq's header slurp takes, markers, comments, and content that leaves a node pending) and is run through yq v4.53.3 and SUBJECT (a
`succinctly` binary, run as `yq`). A stream agrees when both refuse it, or neither does and the JSON matches. It prints
how many streams agree and lists the first that do not. With a BASELINE binary it also lists the streams BASELINE
read as yq does and SUBJECT does not.
"""
import subprocess, random, sys
ver = subprocess.run(["yq", "--version"], capture_output=True, text=True).stdout
assert "v4.53.3" in ver, "the oracle is yq v4.53.3, not " + ver.strip()
seed = int(sys.argv[1]); count = int(sys.argv[2]); subject = sys.argv[3]
baseline = sys.argv[4] if len(sys.argv) > 4 else None
rng = random.Random(seed)
LINES = ["%YAML 1.2", "%YAML 1.1", "%YAMLL", "%YA", "%TAG ! x:", "%TAG !! y:", "%TAG !e! z", "%TAG !a-b_1! z",
         "%TAG !e! w", "%TAG", "%TAG !", "%TAG ! x: # c", "%TAG ! x: y", "%TAG ! x:# c", "%TAG x y", "%TAG !a b! x",
         "%TAG\t!\tx", "%TAGX ! x:", "%FOO", "%FOO bar", "%foo", "%", "% FOO", "%1", "%-", "%FOO # c", "%FOO#c",
         "---", "...", "# c", "a: 1", "x", "", "  ", " %TAG ! x:", "--- x",
         "%YAML 1.1 # c", "%YAML 1.1#c", "%YAML 1.2", "%YAML 1", "%YAML 1.1 x", "%YAML\t1.1", "%YAML 01.1",
         "!!str", "&a", "a:", "- ", "- a", "? a", "a: |", "  x", "a: \"x\"", "a: [x]", "[a]", "\"x\""]
def run(binary, text):
    r = subprocess.run(binary.split() + ["-o=json", "-I0", "."], input=text.encode(), capture_output=True)
    return ("ERR" if (r.returncode != 0 or b"Error" in r.stderr) else "OK"), r.stdout.decode(errors="replace")
seen = set(); rows = []
while len(rows) < count:
    text = "\n".join(rng.choice(LINES) for _ in range(rng.choice([2, 3, 3, 4, 5]))) + "\n"
    if text not in seen:
        seen.add(text); rows.append(text)
def agrees(want, got):
    return got[0] == want[0] and (want[0] == "ERR" or got[1] == want[1])
agree = 0; bad = []; broke = []
for text in rows:
    want, got = run("yq", text), run(subject + " yq", text)
    if agrees(want, got):
        agree += 1
    else:
        bad.append((text, want, got))
        if baseline and agrees(want, run(baseline + " yq", text)):
            broke.append((text, want, got))
print("streams", len(rows), "agree", agree, "differ", len(bad), *(["broke", len(broke)] if baseline else []))
for text, want, got in (broke if baseline else bad)[:12]:
    print(repr(text)); print("   yq  ", want); print("   ours", got)
