#!/usr/bin/env python3
"""Interleaved wall/peak-RSS A/B for up to three succinctly binaries (base, head, holdout).

Per docs/guides/benchmarking.md: binaries alternate within each repetition, a non-zero exit
stops the row, output identity is gated before any number is read, and min-of-N is reported.
RSS via /usr/bin/time (-l on macOS, -v on Linux). `scripts/ab-cli.py` times wall-clock only;
this one exists for changes whose cost is peak memory -- a value-layout change first of all.

The corpus is the one #3182 measured ADR-0024's option D on (arrays of 200k/2M ints and short
strings, 10 MB of users/wide/three-key objects/strings, and a 10 MB YAML twin); the ROWS below
name the file each row reads, so any directory holding those files works:

    scripts/rss-ab.py --base succ-base --head succ-head [--holdout succ-holdout] \
        --corpus ~/wrk/bench-scratch/issue-3182 [--rows 'strs-2m,bind']

Written for #3182, committed with #3191, which added the per-element `as` bind rows: the one
shape where #3191's bind-time promotion allocates once per element."""
import argparse, os, platform, re, subprocess, sys, time, statistics

ROWS = [
  # label, file, tool, query, extra
  ("ints-200k .[(0,1)]=0",      "ints-200k.json",  "jq", ".[(0,1)] = 0", ["-c"]),
  ("ints-200k .[$k]=0",         "ints-200k.json",  "jq", ".[$k] = 0",    ["-c", "--argjson", "k", "0"]),
  ("ints-2m .[(0,1)]=0",        "ints-2m.json",    "jq", ".[(0,1)] = 0", ["-c"]),
  ("ints-2m .[0]=0",            "ints-2m.json",    "jq", ".[0] = 0",     ["-c"]),
  ("ints-2m .",                 "ints-2m.json",    "jq", ".",            ["-c"]),
  ("strs-200k .[(0,1)]=0",      "strs-200k.json",  "jq", ".[(0,1)] = 0", ["-c"]),
  ("strs-200k .[$k]=0",         "strs-200k.json",  "jq", ".[$k] = 0",    ["-c", "--argjson", "k", "0"]),
  ("strs-2m .[(0,1)]=0",        "strs-2m.json",    "jq", ".[(0,1)] = 0", ["-c"]),
  ("strs-2m .",                 "strs-2m.json",    "jq", ".",            ["-c"]),
  ("strs-2m sort",              "strs-2m.json",    "jq", "sort | .[0]",  ["-c"]),
  ("objs3-10mb .[(0,1)]=0",     "objs3-10mb.json", "jq", ".[(0,1)] = 0", ["-c"]),
  ("objs3-10mb .[0].a=1",       "objs3-10mb.json", "jq", ".[0].a = 1",   ["-c"]),
  ("arrs5-10mb sort_by",        "arrs5-10mb.json", "jq", "sort_by(.) | length", ["-c"]),
  ("users-10mb .users[].name",  "users-10mb.json", "jq", ".users[].name", ["-c"]),
  ("users-10mb names downcase", "users-10mb.json", "jq", "[.users[] | .name | ascii_downcase] | length", ["-c"]),
  ("users-10mb names concat",   "users-10mb.json", "jq", "[.users[] | .name + \"x\"] | length", ["-c"]),
  ("users-10mb reduce concat",  "users-10mb.json", "jq", "reduce .users[].name as $s (\"\"; . + $s) | length", ["-c"]),
  ("users-10mb del select",     "users-10mb.json", "jq", "del(.users[] | select(.score < 100))", ["-c"]),
  ("users-10mb scores=1",       "users-10mb.json", "jq", "(.users[] | .score) = 1", ["-c"]),
  ("users-10mb sort_by name",   "users-10mb.json", "jq", "[.users[] | .name] | sort | length", ["-c"]),
  ("users-10mb tostring",       "users-10mb.json", "jq", "[.users[] | .name | tostring] | length", ["-c"]),
  ("users-10mb csv",            "users-10mb.json", "jq", ".users[] | [.name, .score] | @csv", ["-r"]),
  ("users-10mb bind to_entries","users-10mb.json", "jq", ". as $x | [.users[] | to_entries | length] | add", ["-c"]),
  ("users-10mb bind per item",  "users-10mb.json", "jq", "[.users[] | .name as $n | {a: .name, b: .score, c: $n}] | length", ["-c"]),
  ("wide-10mb to_entries",      "wide-10mb.json",  "jq", "to_entries | length", ["-c"]),
  ("wide-10mb bind keys",       "wide-10mb.json",  "jq", ". as $x | to_entries | length", ["-c"]),
  ("strings-10mb .",            "strings-10mb.json","jq", ".",           ["-c"]),
  ("strings-10mb downcase",     "strings-10mb.json","jq", "[.. | strings | ascii_downcase] | length", ["-c"]),
  # #3191: an `as` bind over every element promotes each bound scalar (one `Rc` per
  # element), the only place promote-on-bind allocates.
  ("strs-2m bind each",         "strs-2m.json",    "jq", "[.[] as $s | $s] | length", ["-c"]),
  ("strs-2m bind each embed",   "strs-2m.json",    "jq", "[.[] as $s | {k: $s}] | length", ["-c"]),
  ("strs-2m bind each kept",    "strs-2m.json",    "jq", "[.[] as $s | $s]", ["-c"]),
  ("ints-2m bind each kept",    "ints-2m.json",    "jq", "[.[] as $n | $n]", ["-c"]),
  ("strs-2m bind each placed",  "strs-2m.json",    "jq", "[.[] as $s | {k: $s} | .k] | length", ["-c"]),
  ("users-10mb.yaml identity",  "users-10mb.yaml", "yq", ".",            ["-o", "json", "-I0"]),
  ("users-10mb.yaml dom write", "users-10mb.yaml", "yq", ".users[0].name = \"x\"", []),
]

def run(binary, tool, extra, query, path):
    if platform.system() == "Darwin": tcmd = ["/usr/bin/time", "-l"]
    else: tcmd = ["/usr/bin/time", "-v"]
    t0 = time.perf_counter()
    p = subprocess.run(tcmd + [binary, tool] + extra + [query, path], capture_output=True)
    wall = time.perf_counter() - t0
    err = p.stderr.decode(errors="replace")
    m = re.search(r"(\d+)\s+maximum resident set size", err) or re.search(r"Maximum resident set size \(kbytes\): (\d+)", err)
    rss = int(m.group(1)) if m else -1
    if platform.system() != "Darwin" and m: rss *= 1024
    return wall, rss, p.stdout, p.returncode

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", required=True); ap.add_argument("--head", required=True); ap.add_argument("--holdout")
    ap.add_argument("--corpus", required=True); ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--rows", default=None, help="comma list of row label substrings")
    a = ap.parse_args()
    bins = [("base", a.base), ("head", a.head)] + ([("holdout", a.holdout)] if a.holdout else [])
    print(f"machine: {platform.node()} {platform.machine()} reps={a.reps}")
    print("row | base wall ms | head Δ | holdout Δ | base RSS MB | head Δ | holdout Δ")
    for label, f, tool, q, extra in ROWS:
        if a.rows and not any(s in label for s in a.rows.split(",")): continue
        path = os.path.join(a.corpus, f)
        if not os.path.exists(path): print(f"{label}: missing {f}"); continue
        walls = {n: [] for n,_ in bins}; rsss = {n: [] for n,_ in bins}; outs = {}
        for rep in range(a.reps):
            order = bins if rep % 2 == 0 else list(reversed(bins))
            for n, b in order:
                w, r, out, rc = run(b, tool, extra, q, path)
                if rc != 0: print(f"{label}: {n} exit {rc}"); break
                walls[n].append(w); rsss[n].append(r); outs.setdefault(n, out)
        if len(outs) < len(bins): continue
        ident = all(outs[n] == outs["base"] for n,_ in bins)
        bw = min(walls["base"]); br = min(rsss["base"])
        def d(x, y): return f"{(x - y) / y * 100:+.1f}%"
        hw = d(min(walls["head"]), bw); hr = d(min(rsss["head"]), br)
        ow = d(min(walls["holdout"]), bw) if a.holdout else "-"
        orr = d(min(rsss["holdout"]), br) if a.holdout else "-"
        flag = "" if ident else "  OUTPUT DIFFERS"
        print(f"{label} | {bw*1000:.1f} | {hw} | {ow} | {br/1e6:.1f} | {hr} | {orr}{flag}", flush=True)

main()
