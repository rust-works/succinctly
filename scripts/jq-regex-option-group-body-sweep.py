#!/usr/bin/env python3
"""Differential sweep for #6241: an option group in the body of a lazy counted repeat against jq 1.7.1.

    scripts/jq-regex-option-group-body-sweep.py [--jq /usr/bin/jq] [--succinctly target/debug/succinctly]

Oniguruma writes `X{n,}?` out as copies or as a counted loop by the compiled length of the body
(#6128), and an option group (`(?s:\\w)`, `(?-i:\\p{L})`, `(?i:b)?`) must count as its body. This
runs fourteen body shapes around the ten-operation limit (the group repeated, counted or alone),
each with nineteen atoms (bare and grouped properties, classes, digits and letters), counts of one
to six, over nine texts with the `n` flag and without (28,728 cases) in jq and in `succinctly jq`. Exits 1 on a difference. One pattern is known
to differ (#6263): an inline `(?i-s:\\w)` as the body, where our translation of `\\w` into a bracketed
class is counted under case folding.
"""
import argparse
import os
import subprocess
import sys
import concurrent.futures as cf

ap = argparse.ArgumentParser()
ap.add_argument("--jq", default="/usr/bin/jq")
ap.add_argument("--succinctly", default=os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "target/debug/succinctly"))
args = ap.parse_args()
S = args.succinctly
atoms = [r"\d", r"(?s:\d)", r"\s", r"(?m:\w)", r"(?i-s:\d)", r"\p{L}", r"(?-i:\p{L})", r"(?i:\p{L})", r"(?s:\w)", r"(?-i:\w)", r"(?i:b)", r"(?-i:b)", r"(?s:b)", r"\w", r"[\p{L}]", r"b", r"\p{Lu}", r"(?-i:[a-c])", r"(?i-s:\w)"]
shapes = ["(?:a?|{A}){N,}?", "(?:{A}|a?){N,}?", "(?:a?|b?|{A}){N,}?", "(?:a?|{A}|b?|c?){N,}?", "(?:a?b?|{A}){N,}?", "(?:{A}?|a){N,}?", "(?:a?|{A}){N,M}?", "(?:a*|{A}){N,}?", "(?:a?|{A}{2}){N,}?", "(?:a?|{A}+){N,}?", "(?:(?s:a)*|{A}){N,}?", "(?:(?s:a)+|{A}){N,}?", "(?:(?s:a?)*|{A}){N,}?", "(?:(?s:a){2}|{A}){N,}?"]
texts = ["ca", "cab", "abc", "b", "", "aaa", "bca", "1a", "c1 "]
cases = []
for sh in shapes:
    for A in atoms:
        for N in (1, 2, 3, 4, 5, 6):
            p = sh.replace("{A}", A).replace("{N,M}", "{%d,%d}" % (N, N + 2)).replace("{N,}", "{%d,}" % N)
            cases.append(p)
def run(cmd, pat, t, fl):
    prog = '"%s" | [match("%s"; "%s") | [.offset,.string]]' % (t, pat.replace("\\", "\\\\"), fl)
    r = subprocess.run(cmd + ["-nc", prog], capture_output=True, text=True, timeout=20)
    if r.returncode not in (0, 5):
        sys.exit("%s exited %d on %s: %s" % (cmd[0], r.returncode, pat, r.stderr.strip()[:80]))
    # A refusal is compared by the fact of it, not by each tool's wording.
    return r.stdout.strip() if r.returncode == 0 else "REFUSED"
def job(c):
    pat, t, fl = c
    return c, run([args.jq], pat, t, fl), run([S, "jq"], pat, t, fl)
jobs = [(p, t, fl) for p in cases for t in texts for fl in ("gn", "g")]
bad = {}
with cf.ThreadPoolExecutor(8) as ex:
    for c, a, b in ex.map(job, jobs):
        if a != b:
            bad.setdefault(c[0], []).append((c[1], c[2], a, b))
print(len(jobs), "cases;", len(bad), "patterns differ")
# #6263: an inline `i` with `\w` as the body, at any count and shape.
KNOWN_ATOM = r"(?i-s:\w)"
known = {p for p in bad if KNOWN_ATOM in p}
for p, v in list(bad.items())[:60]:
    print(p, len(v), v[0], "(known, #6263)" if p in known else "")
sys.exit(1 if set(bad) - known else 0)
