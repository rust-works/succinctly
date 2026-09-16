#!/usr/bin/env bash
#
# Oracle sweep for the floating-point math builtins (#3045).
#
# Runs every libm-backed jq builtin (`sin` ... `atanh`, `exp*`, `log*`, `pow`,
# `atan2`, `sqrt`) over 400-point input sweeps through both the pinned jq and
# the built succinctly binary, and reports, per function, how many outputs
# differ *bit for bit* (compared as parsed numbers, not text, so a formatting
# difference cannot hide or fake a mismatch). Real jq calls the platform libm,
# and libms differ in the last bit on ordinary inputs; since #3045 succinctly
# calls the same platform libm under `std`, so the expected result is
# `0 mismatches` on every row against the *same platform's* jq 1.7.1.
#
# This is a verification tool, not a CI gate: the hermetic golden case
# `tests/data/jq-golden/cases/math_platform_libm_bits` and the unit tests in
# `src/jq/math.rs` are what CI enforces. Run this after touching
# `src/jq/math.rs`, or on a platform the goldens were not captured on.
#
# **Power check.** Before #3045 (the pure-Rust `libm` crate everywhere) this
# script reported, against `/usr/bin/jq` 1.7.1-apple on Apple silicon:
# tan 163, sqrt 161 (a Newton iteration), cosh 92, sinh 79, exp 43, exp10 44,
# pow 40, asin 36, atan 23, log 20, sin 17, cos 11 ... out of 400 each (1092
# in total). A run that prints all zeros is only meaningful because it did not
# then; re-run it against a pre-#3045 build if that ever needs re-proving.
#
# Note the oracle is *this platform's* jq: Apple's libm and glibc disagree
# with each other on the same inputs (tan 161/400), so comparing a Linux
# succinctly against goldens captured on macOS would report real jq's own
# platform dependence, not a succinctly bug. The libm's version counts too:
# the static `jq-linux-amd64` release binary embeds glibc 2.35, and glibc
# 2.39 rewrote `exp10`, so on a 2.39 host that binary reports exp10 137/400
# against a dynamically linked succinctly while the distro's own jq 1.7.1
# (dynamic, same glibc) reports 0. Every other row is version-independent.
#
# Usage:
#   cargo build --release --features cli
#   ./scripts/jq-libm-oracle-sweep.sh [path-to-succinctly-binary]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PIN="$(cat "$REPO_ROOT/tests/data/jq-golden/JQ_VERSION")"
SUCC="${1:-$REPO_ROOT/target/release/succinctly}"

if [[ ! -x "$SUCC" ]]; then
  echo "error: succinctly binary not found at $SUCC — run: cargo build --release --features cli" >&2
  exit 1
fi

# Prefer the pinned oracle binary directly (macOS ships jq-1.7.1 at
# /usr/bin/jq; a PATH jq, e.g. Homebrew's, is often a newer version).
if [[ -x /usr/bin/jq ]] && /usr/bin/jq --version | grep -q "^$PIN"; then
  JQ=/usr/bin/jq
elif command -v jq >/dev/null 2>&1 && jq --version | grep -q "^$PIN"; then
  JQ="$(command -v jq)"
else
  echo "error: no jq matching pin $PIN found at /usr/bin/jq or on PATH" >&2
  exit 1
fi

echo "oracle:     $JQ ($("$JQ" --version))"
echo "succinctly: $SUCC"
echo

# Input sweeps, as two IEEE operations over `range(1;401)` so both sides
# compute the exact same doubles. Non-finite results (NaN prints as null,
# ±inf as ±1.7976931348623157e+308) are excluded from the count.
A='. * 0.137 - 20'          # -19.863 .. 34.8
P='. * 0.137'               # 0.137 .. 54.8      (log*, sqrt)
Q='. * 0.137 + 1'           # 1.137 .. 55.8      (acosh)
U='. * 0.005 - 1.0025'      # -0.9975 .. 0.9975  (asin, acos, atanh)

total_mismatch=0
row() { # label filter
  local label="$1" filter="[range(1;401) | $2]"
  local want got n
  want="$("$JQ" -nc "$filter")"
  got="$("$SUCC" jq -nc "$filter")"
  n="$("$JQ" -n --argjson a "$want" --argjson b "$got" '
    [range($a | length) as $i
     | select($a[$i] != null and $b[$i] != null
              and ($a[$i] | fabs) != 1.7976931348623157e+308
              and $a[$i] != $b[$i])]
    | length')"
  printf '%-10s %4s/400 mismatches\n' "$label" "$n"
  total_mismatch=$((total_mismatch + n))
}

row sin    "$A | sin";     row cos   "$A | cos";   row tan   "$A | tan"
row asin   "$U | asin";    row acos  "$U | acos";  row atan  "$A | atan"
row sinh   "$A | sinh";    row cosh  "$A | cosh";  row tanh  "$A | tanh"
row asinh  "$A | asinh";   row acosh "$Q | acosh"; row atanh "$U | atanh"
row exp    "$A | exp";     row exp2  "$A | exp2";  row exp10 "$A | exp10"
row log    "$P | log";     row log10 "$P | log10"; row log2  "$P | log2"
row sqrt   "$P | sqrt"
row pow    "pow($P; ($A) / 3)"
row atan2  "atan2($A; 1.7)"
row atan2- "atan2($A; -0.3)"

echo
if [[ $total_mismatch -eq 0 ]]; then
  echo "OK: 0 mismatches against $JQ"
else
  echo "FAIL: $total_mismatch mismatches against $JQ" >&2
  exit 1
fi
