#!/usr/bin/env python3
"""Replay strings against a real yq and `succinctly yq` to check `src/jq/go_time.rs` (#5651).

For each string S it compares `"S" < "x"` (is S a timestamp, and the error text of the date layout
when it is) and `"2001-01-01T00:00:00Z" < "S"` (how a right operand reads), plus an instant
ordering of S against a fixed timestamp, and prints every difference.

    scripts/yq-time-fuzz.py --succinctly target/debug/succinctly [--count 2000] [--seed 1]
"""
import argparse
import random
import subprocess

SEEDS = [
    "2001-01-01T00:00:00Z", "2001-01-01T00:00:00+10:00", "2001-01-01T00:00:00.123456789Z",
    "2001-01-01T00:00:00,5Z", "2001-01-01", "2001-01-01T1:00:00Z", "2001-01-01T00:00:00-24:00",
    "2000-02-29T23:59:59-23:59Z", "2001-01-01T00:00:00+00:60", "0001-01-01", "9999-12-31T23:59:59Z",
]
ALPHABET = list("0123456789-:TZtz+.,/ _x") + ["é", "\t"]
# Seeds and a few whole-field replacements reach other years, long fractions and offsets.
SEEDS += ["1999-12-31T23:59:59.9999999999999Z", "2024-02-29T12:00:00+23:59", "1970-01-01T00:00:00Z"]


def mutate(rng, s):
    s = list(s)
    for _ in range(rng.choice([0, 1, 1, 2, 3])):
        op = rng.choice(["del", "ins", "rep", "dup"])
        i = rng.randrange(len(s) + 1)
        if op == "del" and s:
            del s[min(i, len(s) - 1)]
        elif op == "ins":
            s.insert(i, rng.choice(ALPHABET))
        elif op == "rep" and s:
            s[min(i, len(s) - 1)] = rng.choice(ALPHABET)
        elif op == "dup" and s:
            j = min(i, len(s) - 1)
            s.insert(j, s[j])
    return "".join(s)


def run(cmd, expr):
    p = subprocess.run(cmd + ["-n", expr], capture_output=True, text=True, timeout=30)
    return (p.returncode == 0, (p.stdout + p.stderr).strip().removeprefix("Error: ").strip())


def lit(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"').replace("\t", "\\t") + '"'


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--succinctly", required=True)
    ap.add_argument("--count", type=int, default=2000)
    ap.add_argument("--seed", type=int, default=1)
    args = ap.parse_args()
    rng = random.Random(args.seed)
    yq, succ = ["yq"], [args.succinctly, "yq"]
    cases = set(SEEDS)
    # Bounded: the reachable set is finite, so a count beyond it stops at the attempt cap.
    for _ in range(args.count * 20):
        if len(cases) >= args.count:
            break
        cases.add(mutate(rng, rng.choice(SEEDS)))
    bad = 0
    for s in sorted(cases):
        for expr in (
            f'{lit(s)} < "x"',
            f'"2001-01-01T00:00:00Z" < {lit(s)}',
            f'{lit(s)} < "2001-01-01T00:00:00Z"',
            f'{lit(s)} >= "2001-06-15T12:30:00+05:30"',
        ):
            a, b = run(yq, expr), run(succ, expr)
            if a != b:
                bad += 1
                print(f"DIFF {expr}\n   yq:   {a}\n   succ: {b}")
    print(f"{len(cases)} strings, {bad} differences")
    raise SystemExit(1 if bad else 0)


if __name__ == "__main__":
    main()
