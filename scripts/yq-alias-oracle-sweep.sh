#!/usr/bin/env bash
#
# Differential oracle sweep for yq-mode writes over alias-bearing YAML
# (#1351 and its split-outs #2497/#2498/#2499/#2500).
#
# Cross-products a small alphabet of anchor/alias document shapes with a set
# of write filters, runs each case through the built succinctly binary and
# the pinned real yq (v4.53.3), and classifies every difference *by
# direction* rather than reporting a bare mismatch count:
#
#   same              values and YAML text identical
#   yq-unsound        real yq's output does not re-read as YAML at all (e.g. a
#                     `*x` with no `&x` left) -- rule 4(a) territory, expected
#   yq-discards       real yq's values equal its *input* although the filter
#                     writes something -- yq silently dropped the write; rule
#                     4(b) territory, expected
#   presentation      values agree, YAML text differs (marks/style/comments)
#   values-differ     values differ and neither yq-side excuse applies --
#                     a succinctly bug until proven otherwise
#
# The alphabet deliberately includes the shapes yq itself gets wrong (see
# docs/compliance/yq/limitations.md), so the sweep reports them as expected
# divergences instead of hiding them -- a generator that cannot emit the
# reference's own bugs cannot tell a refusal from a regression.
#
# Usage:
#   cargo build --release --features cli
#   ./scripts/yq-alias-oracle-sweep.sh [--summary] [--all] [binary]
#
# Without --summary the full TSV record is printed:
#   class <TAB> doc <TAB> filter <TAB> yq_json <TAB> succ_json <TAB> yq_yaml <TAB> succ_yaml
# (newlines escaped as \n). With --summary only the per-class counts and the
# `values-differ`/`presentation` rows are shown. `--all` also lists the rows
# in the two expected-divergence classes.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SUMMARY=0
SHOW_ALL=0
SUCC=""
for arg in "$@"; do
  case "$arg" in
    --summary) SUMMARY=1 ;;
    --all) SHOW_ALL=1 ;;
    *) SUCC="$arg" ;;
  esac
done
SUCC="${SUCC:-$REPO_ROOT/target/release/succinctly}"
YQ="${YQ:-yq}"

if [[ ! -x "$SUCC" ]]; then
  echo "error: succinctly binary not found at $SUCC -- run: cargo build --release --features cli" >&2
  exit 1
fi
if ! "$YQ" --version 2>/dev/null | grep -q 'v4\.53\.3'; then
  echo "warning: $YQ is not the pinned v4.53.3: $("$YQ" --version 2>&1)" >&2
fi

# name|document (\n-escaped)
DOCS=(
  'map2|a: &x {p: 1, q: 2}\nb: *x\nc: *x\n'
  'seq|a: &x [1, 2]\nb: *x\n'
  'scalar|a: &x 1\nb: *x\n'
  'nested|a: &x\n  p: &y 1\n  q: *y\nb: *x\n'
  'multihop|x: &y {z: 0}\na: &x {q: *y}\nb: *x\n'
  'items|items:\n  - &t {n: 1}\n  - *t\n  - *t\n'
  'defout|x: &y 1\na: &x {q: *y}\nb: *x\n'
  'cont|a: &x {s: [1, 2]}\nb: *x\n'
)

# Filters applied to every document whose paths resolve (a path that does not
# apply to a shape is still run -- yq and succinctly should then agree on the
# no-op or error too).
FILTERS=(
  '.b.p = 9' '.b.p |= . + 1' '.b.p += 1' 'del(.b.p)' '.b.r = 3'
  '.b = 5' '.b |= 5' 'del(.b)' '.b = .c' '.c = .b' '.c = .a'
  '.b |= (.p = 9)' '.b |= del(.p)' '.b |= . + {"r": 3}' '.b |= . + 1' '.b += 1'
  '.[] .p += 1' '.[].p |= . + 1' '(.b, .c).p = 2'
  '.b.p = 9 | .c.q = 8' '.b.p = 9 | .b = 5' '.b = 5 | .b.p = 9' '.b = null | .a.p = 9'
  '.[].p = 9 | .a.q = 7' '.a.p = 9 | .b.p = 9 | .a.q = 7' '.b.p = 9 | .a.q = 7'
  'del(.a) | .b.p = 9' 'del(.a) | .b.p = 9 | .c'
  'setpath(["b","p"]; 9)' 'delpaths([["b","p"]])' 'del(.b.p, .c.q)'
  '.b[0] = 9' '.b[-1] = 5' '.b[2] = 3' 'del(.b[0])' '.b += [3]'
  '.b.s[0] = 9' '.b.q.z = 1' '.b.q = 5' '.b.p = 5' '.a.p = 5' '.x = 5'
  '.items[].n += 1' '.items[1].n = 9' '.items[1] = {"n": 3}' 'del(.items[1])'
  '.b = .a | .a.p = 9' '.c = .b | .d = .c' '.c = (.b + 1)' '.c = .b | .c = 1'
)

esc() { printf '%s' "$1" | awk 'BEGIN{ORS="\\n"} {print}' ; }

declare -A COUNT
ROWS=()
for entry in "${DOCS[@]}"; do
  name="${entry%%|*}"; doc="${entry#*|}"
  input="$(printf "$doc")"
  input_json="$(printf '%s' "$input" | "$YQ" -o=json -I=0 '.' 2>/dev/null || true)"
  for f in "${FILTERS[@]}"; do
    yq_json="$(printf '%s' "$input" | "$YQ" -o=json -I=0 "$f" 2>&1 || true)"
    succ_json="$(printf '%s' "$input" | "$SUCC" yq -o=json -I=0 "$f" 2>&1 || true)"
    yq_yaml="$(printf '%s' "$input" | "$YQ" "$f" 2>&1 || true)"
    succ_yaml="$(printf '%s' "$input" | "$SUCC" yq "$f" 2>&1 || true)"
    if [[ "$yq_json" == "$succ_json" && "$yq_yaml" == "$succ_yaml" ]]; then
      class=same
    elif ! printf '%s\n' "$yq_yaml" | "$YQ" '.' >/dev/null 2>&1; then
      class=yq-unsound
    elif [[ "$yq_json" == "$input_json" && "$succ_json" != "$input_json" ]]; then
      class=yq-discards
    elif [[ "$yq_json" == "$succ_json" ]]; then
      class=presentation
    else
      class=values-differ
    fi
    COUNT[$class]=$(( ${COUNT[$class]:-0} + 1 ))
    ROWS+=("$class	$name	$f	$(esc "$yq_json")	$(esc "$succ_json")	$(esc "$yq_yaml")	$(esc "$succ_yaml")")
  done
done

if [[ $SUMMARY -eq 0 ]]; then
  printf '%s\n' "${ROWS[@]}"
  exit 0
fi

echo "cases: ${#ROWS[@]}"
for c in same presentation values-differ yq-unsound yq-discards; do
  echo "  $c: ${COUNT[$c]:-0}"
done
echo
for row in "${ROWS[@]}"; do
  class="${row%%	*}"
  case "$class" in
    values-differ|presentation) ;;
    yq-unsound|yq-discards) [[ $SHOW_ALL -eq 1 ]] || continue ;;
    *) continue ;;
  esac
  IFS=$'\t' read -r cls name f yj sj yy sy <<<"$row"
  echo "[$cls] $name :: $f"
  echo "    yq   json: $yj"
  echo "    succ json: $sj"
  echo "    yq   yaml: $yy"
  echo "    succ yaml: $sy"
done
