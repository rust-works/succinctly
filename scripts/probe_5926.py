#!/usr/bin/env python3
"""Throwaway (#5926): per key of the two libm cases, which elements differ between the committed
golden, this host's jq 1.7.1 and this host's succinctly."""
import json, pathlib, subprocess, sys

jq, succ = sys.argv[1], sys.argv[2]
root = pathlib.Path("tests/data/jq-golden/cases")


def run(cmd, case):
    d = root / case
    out = subprocess.run(cmd + ["-c", "-f", str(d / "filter"), str(d / "input.json")], capture_output=True, text=True)
    return out.stdout, out.returncode, out.stderr


def elems(v):
    return v if isinstance(v, list) else [v]


for case in ("math_libm_family", "math_platform_libm_bits"):
    golden = json.loads((root / case / "expected.out").read_text())
    j, jrc, jerr = run([jq], case)
    s, src, serr = run([succ, "jq"], case)
    print(f"##### {case}: jq rc={jrc} succinctly rc={src}", jerr[:200], serr[:200])
    jv, sv = json.loads(j), json.loads(s)
    print("jq26 == succinctly26 (whole output):", jv == sv, "; jq26 == golden:", jv == golden, "; succ26 == golden:", sv == golden)
    for key in golden:
        g, a, b = golden[key], jv.get(key), sv.get(key)
        if g == a == b:
            continue
        ga, gb, gg = elems(g), elems(a), elems(b)
        print(f"-- {key}: golden={g if len(str(g)) < 160 else '...'}")
        for i in range(max(len(ga), len(gb), len(gg))):
            x = ga[i] if i < len(ga) else None
            y = gb[i] if i < len(gb) else None
            z = gg[i] if i < len(gg) else None
            if not (x == y == z):
                print(f"   [{i}] golden={json.dumps(x)} jq26={json.dumps(y)} succ26={json.dumps(z)}")
