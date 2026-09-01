#!/usr/bin/env bash
#
# Oracle sweep for the assignment/delete *path walkers* (`set_path`,
# `update_path`, `delete_at_path` in src/jq/eval.rs).
#
# Cross-products documents x path shapes x write operators x mode (jq/yq),
# recording stdout/stderr/exit for the built succinctly binary alongside the
# pinned oracle (jq 1.7.1, yq v4.53.3) for the identical case. Written for
# #1429 (converging `set_path`'s flatten-then-split pre-scan onto a
# peel-and-recurse walker), which is a pure restructuring: the primary gate
# is not "matches the oracle" but "the *succinctly* columns are byte-identical
# before and after", with the oracle columns present so a divergence can be
# classified as pre-existing rather than re-triaged by hand.
#
# Two uses:
#
#   # A/B a restructuring (the primary gate):
#   ./scripts/jq-assign-path-oracle-sweep.sh .ai/scratch/succinctly-pre > before.tsv
#   ./scripts/jq-assign-path-oracle-sweep.sh target/release/succinctly > after.tsv
#   diff before.tsv after.tsv && echo "byte-identical"
#
#   # Triage succinctly-vs-oracle divergence in one recording:
#   ./scripts/jq-assign-path-oracle-sweep.sh --summary target/release/succinctly
#
# The record is a stable, sorted-by-construction TSV so `diff` is meaningful:
#   mode <TAB> op <TAB> path <TAB> doc <TAB> succ_exit <TAB> succ_out <TAB> succ_err
#        <TAB> oracle_exit <TAB> oracle_out <TAB> oracle_err
# Newlines inside a field are escaped to \n so one case is always one line.
#
# `--no-oracle` skips the oracle columns (roughly halves the runtime) for a
# fast A/B when divergence triage is not needed. The oracle columns are
# identical between two recordings anyway, so a diff is unaffected either way
# — but only recordings made with the *same* flag are comparable.
#
# Usage:
#   cargo build --release --features cli,simd,regex,serde
#   ./scripts/jq-assign-path-oracle-sweep.sh [--summary] [--no-oracle] [binary]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PIN="$(cat "$REPO_ROOT/tests/data/jq-golden/JQ_VERSION")"

SUMMARY=0
USE_ORACLE=1
SUCC=""
for arg in "$@"; do
  case "$arg" in
    --summary) SUMMARY=1 ;;
    --no-oracle) USE_ORACLE=0 ;;
    *) SUCC="$arg" ;;
  esac
done
SUCC="${SUCC:-$REPO_ROOT/target/release/succinctly}"

if [[ ! -x "$SUCC" ]]; then
  echo "error: succinctly binary not found at $SUCC — run: cargo build --release --features cli,simd,regex,serde" >&2
  exit 1
fi

# Prefer the pinned oracle binary directly (macOS ships jq-1.7.1 at
# /usr/bin/jq; a PATH jq, e.g. Homebrew's, is often newer) — same reasoning as
# scripts/jq-fanout-oracle-sweep.sh and tests/jq_cli_tests.rs's own comments.
# For yq the pin is the opposite way round: Homebrew's *is* the pin.
JQ=""
YQ=""
if ((USE_ORACLE)); then
  if [[ -x /usr/bin/jq ]] && /usr/bin/jq --version | grep -q "^$PIN"; then
    JQ=/usr/bin/jq
  elif command -v jq >/dev/null 2>&1 && jq --version | grep -q "^$PIN"; then
    JQ="$(command -v jq)"
  else
    echo "error: no jq matching pin $PIN found at /usr/bin/jq or on PATH" >&2
    exit 1
  fi
  if command -v yq >/dev/null 2>&1 && yq --version | grep -q 'v4\.'; then
    YQ="$(command -v yq)"
  else
    echo "error: no yq v4 found on PATH" >&2
    exit 1
  fi
  echo "oracles: $JQ ($("$JQ" --version)), $YQ ($("$YQ" --version 2>&1 | tail -1))" >&2
fi
echo "succinctly: $SUCC" >&2

# --- matrix -----------------------------------------------------------------

# Documents. JSON throughout: every one of these is also valid YAML flow, so
# the same string feeds both modes without a second corpus to keep in sync.
DOCS=(
  'null'
  '5'
  '"str"'
  'true'
  '{}'
  '[]'
  '{"a":1}'
  '{"a":null}'
  '{"a":"notobj"}'
  '{"a":{"c":5}}'
  '{"a":{"b":[1,2,3]}}'
  '{"a":[1,2,3,4]}'
  '{"a":[{"b":1},{"b":2}]}'
  '[1,2,3]'
  '[[1,2],[3,4]]'
  '{"items":[{"env":[{"v":1}]},{"env":[{"v":2}]}]}'
)

# Path shapes. Grouped by what they exercise; the groups overlap on purpose —
# an `Iterate` before a `Slice` and a `Slice` before an `Iterate` are the two
# halves of the ordering invariant #1429 is about, and both have to be here.
PATHS=(
  # plain field/index chains, with `?` at each position
  '.a' '.a.b' '.a.b.c' '.a?' '.a?.b' '.a?.b.c' '.a.b?' '.a.b?.c' '.a.b.c?'
  '.[0]' '.[-1]' '.[5]' '.[-9]' '.a[0]' '.a[-1]' '.a[5]' '.a[-9]' '.a[0].b'
  # terminal and mid-chain iterate
  '.[]' '.[]?' '.a[]' '.a[]?' '.a[].b' '.a[]?.b' '.a[].b?' '.a[][0]' '.[][]'
  '.a[][]' '.a[].b[]' '.items[].env[]' '.items[].env[].v' '.items[]?.env[]?.v'
  # terminal and mid-chain slice
  '.[0:1]' '.[0:2]' '.a[0:2]' '.a[0:2]?' '.a[0:2].b' '.a[0:2].b?' '.a[0:2][0]'
  '.a[0:1][0:2]' '.a[0:1][0:2]?' '.a[0:1]?[0]' '.[0:1][]'
  # the ordering invariant: slice-before-iterate vs iterate-before-slice
  '.a[0:2][]' '.a[0:2][]?' '.a[][0:2]' '.a[]?[0:2]' '.a[0:1][]?.b' '.a[][0:1][]'
  # nested Paren/Pipe groups (the double-flatten case), incl. group-scoped `?`
  '(.a|.c)' '(.a|.c)[0]' '(.a|.c).d' '(.a|.c)?' '((.a|.c)? | .y)'
  '(.a[])[0]' '(.a[0:1])[0]' '(.a[0:1]?)[0]' '((.a[0:1])? | .[0])'
  '(.a|.b|.c)' '(.a|.b)[0:1]' '((.a|.b)? | .[])'
  # missing-parent / stranded-autovivify shapes (#1428)
  '.a[0:1][]?' '.a[0:1].c[]?' '.a[0:1][0]?.c[]?' '.a?.b[].c' '.a.b[0:1][]?'
)

# Write operators. `__P__` is the path. `|=`/`+=`/`del` route through
# `update_path`/`delete_at_path`, which #1429 does not restructure — they are
# controls: a diff in those columns means collateral damage.
OPS=(
  '__P__ = 9'
  '__P__ = [9]'
  '__P__ = null'
  '__P__ = {"z":1}'
  '__P__ |= 9'
  '__P__ |= empty'
  '__P__ += 1'
  'del(__P__)'
)

# --- runner -----------------------------------------------------------------

# Normalises the document's temp-file path out of captured stderr: jq and yq
# both name the input file in their error messages, and a per-run `mktemp`
# name would otherwise make every error row differ between two recordings —
# which is precisely what this record exists to compare.
esc() {
  printf '%s' "$1" \
    | tr '\t' ' ' \
    | sed "s|$DOC_FILE|DOC|g" \
    | awk 'BEGIN{ORS=""} NR>1{print "\\n"} {print}'
}

DOC_FILE="$(mktemp -t assign-sweep-doc)"
S_ERR="$(mktemp -t assign-sweep-serr)"
O_ERR="$(mktemp -t assign-sweep-oerr)"
trap 'rm -f "$DOC_FILE" "$S_ERR" "$O_ERR"' EXIT

total=0
diverged=0
declare -a div_keys=()

run_case() {
  local mode="$1" op="$2" path="$3" doc="$4" filter="$5"
  total=$((total + 1))
  printf '%s' "$doc" > "$DOC_FILE"

  local s_out s_code o_out o_code s_err o_err
  if [[ "$mode" == jq ]]; then
    s_out="$("$SUCC" jq -c "$filter" "$DOC_FILE" 2>"$S_ERR")" && s_code=0 || s_code=$?
  else
    s_out="$("$SUCC" yq -o=json -I=0 "$filter" "$DOC_FILE" 2>"$S_ERR")" && s_code=0 || s_code=$?
  fi
  s_err="$(cat "$S_ERR")"

  o_out=""; o_err=""; o_code="-"
  if ((USE_ORACLE)); then
    if [[ "$mode" == jq ]]; then
      o_out="$("$JQ" -c "$filter" "$DOC_FILE" 2>"$O_ERR")" && o_code=0 || o_code=$?
    else
      o_out="$("$YQ" -o=json -I=0 "$filter" "$DOC_FILE" 2>"$O_ERR")" && o_code=0 || o_code=$?
    fi
    o_err="$(cat "$O_ERR")"
    if [[ "$s_out" != "$o_out" || "$s_code" != "$o_code" ]]; then
      diverged=$((diverged + 1))
      div_keys+=("$mode/$op")
    fi
  fi

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$mode" "$op" "$path" "$(esc "$doc")" \
    "$s_code" "$(esc "$s_out")" "$(esc "$s_err")" \
    "$o_code" "$(esc "$o_out")" "$(esc "$o_err")"
}

for mode in jq yq; do
  for op in "${OPS[@]}"; do
    for path in "${PATHS[@]}"; do
      filter="${op//__P__/$path}"
      for doc in "${DOCS[@]}"; do
        run_case "$mode" "$op" "$path" "$doc" "$filter"
      done
    done
  done
done

# --- generated paths --------------------------------------------------------
#
# The curated matrix above only ever covers shapes its author already thought
# of, which is exactly how a restructuring slips through a hand-picked oracle
# check. These compose atoms mechanically instead, over a fixed seed so two
# recordings are comparable and a failure is reproducible. A generated filter
# that does not parse is still a useful A/B row — both recordings capture the
# same parse error, and a *change* in it is the signal.
#
# Deterministic LCG rather than `$RANDOM`/`awk srand()`: those vary by shell
# and libc, which would make `before.tsv` and `after.tsv` incomparable across
# a machine or shell change.
GEN_SEED=20260101
_rand_state=$GEN_SEED
next_rand() {
  _rand_state=$(( (_rand_state * 1103515245 + 12345) % 2147483648 ))
  echo $(( (_rand_state / 65536) % $1 ))
}

GEN_ATOMS=('.a' '.b' '.[0]' '.[-1]' '.[]' '.[0:2]' '.[1:]')
GEN_DOCS=(
  'null' '{"a":1}' '{"a":[1,2,3]}' '{"a":[{"b":1},{"b":2}]}' '[1,2,3]' '5'
)
GEN_OPS=('__P__ = 9' '__P__ |= 9' 'del(__P__)')
GEN_PATHS=()
while ((${#GEN_PATHS[@]} < 100)); do
  depth=$(( $(next_rand 3) + 2 ))
  parts=()
  for ((k = 0; k < depth; k++)); do
    atom="${GEN_ATOMS[$(next_rand ${#GEN_ATOMS[@]})]}"
    # ~1 in 3 components carries its own `?`.
    (( $(next_rand 3) == 0 )) && atom="${atom}?"
    parts+=("$atom")
  done
  # ~1 in 3 paths puts its first two components in a parenthesised
  # pipe group (`(.a|.b)[0]`), the nested-group shape that used to cost
  # two full flatten passes before finding nothing.
  if (( $(next_rand 3) == 0 )); then
    path="(${parts[0]}|${parts[1]})"
    for ((k = 2; k < depth; k++)); do path+="${parts[k]}"; done
  else
    path=""
    for part in "${parts[@]}"; do path+="$part"; done
  fi
  GEN_PATHS+=("$path")
done

for mode in jq yq; do
  for op in "${GEN_OPS[@]}"; do
    for path in "${GEN_PATHS[@]}"; do
      filter="${op//__P__/$path}"
      for doc in "${GEN_DOCS[@]}"; do
        run_case "$mode" "gen:$op" "$path" "$doc" "$filter"
      done
    done
  done
done

if ((SUMMARY)); then
  {
    echo "== $total cases, $diverged succinctly-vs-oracle divergences =="
    if ((diverged > 0)); then
      printf '%s\n' "${div_keys[@]}" | sort | uniq -c | sort -rn | sed 's/^/   /'
    fi
  } >&2
else
  echo "== $total cases recorded ($diverged vs-oracle divergences) ==" >&2
fi
