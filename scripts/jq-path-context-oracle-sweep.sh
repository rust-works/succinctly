#!/usr/bin/env bash
#
# Oracle sweep for the *path-context* evaluators — #2416 phase 0's "net".
#
# #2416 retires the eager `eval_stage_with_path_context` (`src/jq/eval.rs`) by
# migrating its arms one at a time into `try_path_context_cursor_walk`
# (`src/jq/eval_generic.rs`, #2061). Every migration has to be checked against
# the **oracle**, not against the in-tree bridge it replaces: #2388 already
# proved the two in-tree evaluators disagree with each other, so "the walk
# matches the bridge" is not evidence that either matches jq/yq.
#
# Promoted from `.ai/scratch/sweep-path-fold-differential.py`, which swept
# `path(foreach|reduce ...)` against jq and classified diffs by direction. It
# was gitignored scratch: the one asset that de-risks the whole migration was
# not durable. This is that sweep generalised — both modes, both oracles, and
# an alphabet that can actually emit the shapes #2416 is about.
#
# **The generator alphabet is part of the claim.** A fuzzer whose pool cannot
# emit the shape a bug lives in proves nothing (#2041). The alphabet here is a
# full cross product of
#
#   prefix x path-context leaf x consumer wrapper x outer form
#
# where the leaves are the `needs_path_context` family in yq mode
# (`key`, `parent`, `path`, `file_index`) and the path-*expression* family in
# jq mode (`path(f)`, `paths`, `getpath(p)`, `path(..)` — jq 1.7.1 has no
# `key`/`parent`/bare `path`/`file_index` at all), and the wrappers are every
# construct #2416 names: `limit`, `first`, `last`, `foreach`, `reduce`,
# `getpath`, object construction, `select`, comma, `if`, `try`, `label`/`break`,
# plus `path(...)` and `?` as outer forms. `--self-test` asserts every
# combination class is present, so a future alphabet shrink fails loudly.
#
# **yq comma/pipe precedence (#2420).** Real yq v4.53.3 parses `a, b | c` as
# `a, (b | c)`, where jq parses it as `(a, b) | c`. succinctly used to apply
# jq's grouping in both modes; since #2420 `succinctly yq` follows yq's own
# precedence table (`pipeOpType` 30, `unionOpType` 10) and the two agree.
# Every yq-mode case here nevertheless parenthesises each comma branch, so
# that a case's meaning does not silently depend on which grouping rule is in
# force — the corpus is about path context, not about parsing. `--self-test`
# enforces it mechanically: no yq-mode filter may contain a `,` and a `|` at
# the same bracket depth.
#
# Usage:
#   cargo build --release --features cli,simd,regex,serde
#   ./scripts/jq-path-context-oracle-sweep.sh                 # sweep, TSV to stdout
#   ./scripts/jq-path-context-oracle-sweep.sh --summary       # + per-category counts
#   ./scripts/jq-path-context-oracle-sweep.sh --list-cases    # generated corpus only
#   ./scripts/jq-path-context-oracle-sweep.sh --self-test     # alphabet + precedence checks
#
# `--list-cases` and `--self-test` need no binary at all (neither succinctly
# nor an oracle), which is what lets `tests/jq_path_context_alphabet_tests.rs`
# assert the alphabet's coverage on every CI leg.
#
# The record is a stable, sorted-by-construction TSV so two recordings diff
# meaningfully (the `scripts/jq-assign-path-oracle-sweep.sh` convention):
#   mode <TAB> class <TAB> filter <TAB> succ_exit <TAB> succ_out <TAB> succ_err
#        <TAB> oracle_exit <TAB> oracle_out <TAB> oracle_err
#
# Divergence is judged on **stdout + exit code**, with stderr recorded for
# triage but not compared: diagnostic wording has its own oracle
# (`tests/data/jq-error-messages.tsv` + `scripts/sync-jq-error-messages.sh`),
# and folding it in here would bury the signal under known message drift.
#
# Divergences attributable to an already-tracked gap are counted separately
# (see `classify_divergence`), so the headline number stays a pass/fail signal
# — the `scripts/jq-fanout-oracle-sweep.sh` convention. Exits non-zero if any
# *unexpected* divergence appears.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
JQ_PIN="$(cat "$REPO_ROOT/tests/data/jq-golden/JQ_VERSION")"
YQ_PIN="$(cat "$REPO_ROOT/tests/data/yq-golden/YQ_VERSION")"

MODE=sweep
SUMMARY=0
RUN_JQ=1
RUN_YQ=1
SUCC=""
for arg in "$@"; do
  case "$arg" in
    --list-cases) MODE=list ;;
    --self-test) MODE=selftest ;;
    --summary) SUMMARY=1 ;;
    --jq-only) RUN_YQ=0 ;;
    --yq-only) RUN_JQ=0 ;;
    -h|--help) sed -n '2,60p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) SUCC="$arg" ;;
  esac
done
SUCC="${SUCC:-$REPO_ROOT/target/release/succinctly}"

# --- alphabet ---------------------------------------------------------------
#
# Kept as parallel id/template arrays rather than `id|template` strings: the
# templates are jq filters and half of them contain a literal `|`, so any
# single-character field split would corrupt them.

# Navigation prefixes. Each generated filter is `<prefix> | <outer>`, so the
# leaf always sits at a non-root position with a real accumulated path — the
# thing `eval_stage_with_path_context` exists to carry.
PREFIX_IDS=(root  field  iter    seqiter descend deep)
PREFIX_TPL=('.'   '.a'   '.a[]'  '.d[]'  '..'    '.a.b')

# Path-context leaves, per mode.
#
# jq 1.7.1 has none of `key`/`parent`/bare `path`/`file_index` — confirmed
# live, and `src/jq/eval.rs`'s `needs_path_context` is exactly that family, so
# jq mode cannot observe the eager evaluator through them at all. What jq
# *does* have is the path-expression family, which reaches the sibling
# `resolve_node`/`Builtin::Path` machinery (#1952, phase 4) and the same pipe
# plumbing around it.
JQ_LEAF_IDS=(path_f  paths  getpath        path_dd)
JQ_LEAF_TPL=('path(.a)' 'paths' 'getpath(["a","b"])' 'path(..)')

YQ_LEAF_IDS=(key   parent   path   file_index)
YQ_LEAF_TPL=('key' 'parent' 'path' 'file_index')

# Consumer wrappers. `__I__` is the leaf.
#
# Every construct #2416's phase-0 checklist names. yq's own lexer rejects most
# of them outright (confirmed live against v4.53.3: `limit`, `last`, `foreach`,
# `reduce`, `getpath`, `if`, `try`, `label`/`break` are all "lexer: invalid
# input text"), which is itself an oracle fact worth sweeping — succinctly yq
# accepting them is a divergence, tracked as a category below rather than
# quietly dropped from the alphabet.
WRAP_IDS=(bare limit first last foreach reduce getpath object select comma if try label)
WRAP_TPL=(
  '__I__'
  'limit(2; __I__)'
  'first(__I__)'
  'last(__I__)'
  'foreach (__I__) as $x (null; $x)'
  'reduce (__I__) as $x ([]; . + [$x])'
  'getpath([__I__])'
  '{"k": __I__}'
  'select((__I__) != null)'
  '((__I__), 1)'
  'if (__I__) then 1 else 2 end'
  'try (__I__) catch "e"'
  'label $o | ((__I__), break $o)'
)

# Outer forms. `__W__` is the wrapped leaf.
OUTER_IDS=(plain path       opt)
OUTER_TPL=('__W__' 'path(__W__)' '(__W__)?')

# --- generation -------------------------------------------------------------

# `CASES` entries are "mode<TAB>class<TAB>filter". Built once and reused by
# every mode of this script, so `--self-test` and `--list-cases` describe
# exactly the corpus the sweep runs.
CASES=()

emit_case() {
  CASES+=("$1	$2	$3")
}

generate_mode() {
  local mode="$1"
  shift
  local -a leaf_ids leaf_tpl
  if [[ "$mode" == jq ]]; then
    leaf_ids=("${JQ_LEAF_IDS[@]}"); leaf_tpl=("${JQ_LEAF_TPL[@]}")
  else
    leaf_ids=("${YQ_LEAF_IDS[@]}"); leaf_tpl=("${YQ_LEAF_TPL[@]}")
  fi

  local li wi oi pi leaf wrapped outer filter class
  for ((li = 0; li < ${#leaf_ids[@]}; li++)); do
    leaf="${leaf_tpl[li]}"
    for ((wi = 0; wi < ${#WRAP_IDS[@]}; wi++)); do
      wrapped="${WRAP_TPL[wi]//__I__/$leaf}"
      for ((oi = 0; oi < ${#OUTER_IDS[@]}; oi++)); do
        outer="${OUTER_TPL[oi]//__W__/$wrapped}"
        class="${leaf_ids[li]}/${WRAP_IDS[wi]}/${OUTER_IDS[oi]}"
        for ((pi = 0; pi < ${#PREFIX_IDS[@]}; pi++)); do
          filter="${PREFIX_TPL[pi]} | $outer"
          emit_case "$mode" "$class/${PREFIX_IDS[pi]}" "$filter"
        done
      done
    done
  done
}

# Deterministic LCG rather than `$RANDOM`/`awk srand()`: those vary by shell
# and libc, which would make two recordings incomparable across a machine or
# shell change (the `scripts/jq-assign-path-oracle-sweep.sh` convention).
GEN_SEED=20260905
_rand_state=$GEN_SEED
next_rand() {
  _rand_state=$(( (_rand_state * 1103515245 + 12345) % 2147483648 ))
  echo $(( (_rand_state / 65536) % $1 ))
}

# Mechanically composed shapes on top of the curated cross product above: two
# nested wrappers and a randomly chained prefix, so the sweep covers
# compositions its author did not individually think of. A generated filter
# that does not parse is still a useful row — the oracle's own rejection is
# the expected answer, and a *change* in it is the signal.
GEN_PREFIX_ATOMS=('.a' '.d' '.[]' '.a[]' '.b' '..')
generate_random() {
  local mode="$1" count="$2"
  local -a leaf_ids leaf_tpl
  if [[ "$mode" == jq ]]; then
    leaf_ids=("${JQ_LEAF_IDS[@]}"); leaf_tpl=("${JQ_LEAF_TPL[@]}")
  else
    leaf_ids=("${YQ_LEAF_IDS[@]}"); leaf_tpl=("${YQ_LEAF_TPL[@]}")
  fi
  local n li w1 w2 oi depth k prefix body class
  for ((n = 0; n < count; n++)); do
    li=$(next_rand ${#leaf_ids[@]})
    w1=$(next_rand ${#WRAP_IDS[@]})
    w2=$(next_rand ${#WRAP_IDS[@]})
    oi=$(next_rand ${#OUTER_IDS[@]})
    depth=$(( $(next_rand 2) + 1 ))
    prefix=""
    for ((k = 0; k < depth; k++)); do
      prefix+="${GEN_PREFIX_ATOMS[$(next_rand ${#GEN_PREFIX_ATOMS[@]})]}"
    done
    body="${WRAP_TPL[w1]//__I__/${leaf_tpl[li]}}"
    body="${WRAP_TPL[w2]//__I__/($body)}"
    body="${OUTER_TPL[oi]//__W__/$body}"
    class="gen/${leaf_ids[li]}/${WRAP_IDS[w1]}+${WRAP_IDS[w2]}/${OUTER_IDS[oi]}"
    emit_case "$mode" "$class" "$prefix | $body"
  done
}

((RUN_JQ)) && { generate_mode jq; generate_random jq 120; }
((RUN_YQ)) && { generate_mode yq; generate_random yq 120; }

# --- self-checks ------------------------------------------------------------

# #2420: no yq-mode filter may hold a `,` and a `|` at the same bracket depth.
# Real yq groups `a, b | c` as `a, (b | c)` where succinctly (both modes)
# groups `(a, b) | c`, so such a case records a *parser* divergence that says
# nothing about path context. Checked per depth level rather than per filter,
# so a properly parenthesised `(a, b) | c` passes and a nested `((a, b | c))`
# does not.
yq_comma_pipe_conflict() {
  local f="$1" i ch depth=0
  local -a has_comma=() has_pipe=()
  for ((i = 0; i < ${#f}; i++)); do
    ch="${f:i:1}"
    case "$ch" in
      '('|'['|'{') depth=$((depth + 1)); has_comma[depth]=0; has_pipe[depth]=0 ;;
      ')'|']'|'}') depth=$((depth > 0 ? depth - 1 : 0)) ;;
      ',') has_comma[depth]=1; [[ "${has_pipe[depth]:-0}" == 1 ]] && return 0 ;;
      '|') has_pipe[depth]=1; [[ "${has_comma[depth]:-0}" == 1 ]] && return 0 ;;
    esac
  done
  return 1
}

self_test() {
  local failures=0 mode class filter line

  # 1. Alphabet coverage: every leaf x wrapper x outer combination class must
  #    be present in the generated corpus, per mode. A future alphabet shrink
  #    — the #2041 failure mode — then fails here instead of silently proving
  #    less than the sweep claims.
  local -a modes=()
  ((RUN_JQ)) && modes+=(jq)
  ((RUN_YQ)) && modes+=(yq)
  local m li wi oi want found
  for m in "${modes[@]}"; do
    local -a leaf_ids
    if [[ "$m" == jq ]]; then leaf_ids=("${JQ_LEAF_IDS[@]}"); else leaf_ids=("${YQ_LEAF_IDS[@]}"); fi
    for ((li = 0; li < ${#leaf_ids[@]}; li++)); do
      for ((wi = 0; wi < ${#WRAP_IDS[@]}; wi++)); do
        for ((oi = 0; oi < ${#OUTER_IDS[@]}; oi++)); do
          want="$m	${leaf_ids[li]}/${WRAP_IDS[wi]}/${OUTER_IDS[oi]}/"
          found=0
          for line in "${CASES[@]}"; do
            [[ "$line" == "$want"* ]] && { found=1; break; }
          done
          if ((!found)); then
            echo "missing combination class: $m ${leaf_ids[li]}/${WRAP_IDS[wi]}/${OUTER_IDS[oi]}" >&2
            failures=$((failures + 1))
          fi
        done
      done
    done
  done

  # 2. yq comma/pipe precedence (#2420).
  for line in "${CASES[@]}"; do
    IFS=$'\t' read -r mode class filter <<<"$line"
    [[ "$mode" == yq ]] || continue
    if yq_comma_pipe_conflict "$filter"; then
      echo "yq-mode case mixes ',' and '|' at one bracket depth (#2420): [$class] $filter" >&2
      failures=$((failures + 1))
    fi
  done

  if ((failures > 0)); then
    echo "FAIL: $failures self-test failure(s)" >&2
    exit 1
  fi
  echo "self-test ok: ${#CASES[@]} cases, all combination classes present, no #2420 precedence hazards" >&2
}

case "$MODE" in
  list)
    printf '%s\n' "${CASES[@]}"
    exit 0
    ;;
  selftest)
    self_test
    exit 0
    ;;
esac

self_test

# --- oracles ----------------------------------------------------------------

# Prefer the pinned oracle binary directly (macOS ships jq-1.7.1 at
# /usr/bin/jq; a PATH jq, e.g. Homebrew's, is often newer) — same reasoning as
# scripts/jq-fanout-oracle-sweep.sh. For yq the pin is the opposite way round:
# Homebrew's *is* the pin.
JQ=""
YQ=""
if ((RUN_JQ)); then
  if [[ -x /usr/bin/jq ]] && /usr/bin/jq --version | grep -q "^$JQ_PIN"; then
    JQ=/usr/bin/jq
  elif command -v jq >/dev/null 2>&1 && jq --version | grep -q "^$JQ_PIN"; then
    JQ="$(command -v jq)"
  else
    echo "error: no jq matching pin $JQ_PIN found at /usr/bin/jq or on PATH" >&2
    exit 1
  fi
fi
if ((RUN_YQ)); then
  if command -v yq >/dev/null 2>&1 && yq --version | grep -qF "version $YQ_PIN"; then
    YQ="$(command -v yq)"
  else
    echo "error: no yq matching pin $YQ_PIN found on PATH" >&2
    exit 1
  fi
fi

if [[ ! -x "$SUCC" ]]; then
  echo "error: succinctly binary not found at $SUCC — run: cargo build --release --features cli,simd,regex,serde" >&2
  exit 1
fi
echo "oracles: ${JQ:-<skipped>} ${YQ:-<skipped>}; succinctly: $SUCC" >&2

# --- documents --------------------------------------------------------------

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# JSON that is also valid YAML flow, so one shape feeds both modes.
cat > "$WORK/doc.json" <<'JSON'
{"a":{"b":1,"c":2},"d":[10,20]}
JSON
cp "$WORK/doc.json" "$WORK/one.yaml"
cat > "$WORK/two.yaml" <<'JSON'
{"a":{"b":3,"c":4},"d":[30,40]}
JSON

# The same two documents in block style. A block sequence item sits behind a
# `-` wrapper node that the flow form has no counterpart for, and #2455 was a
# path-context bug only that wrapper could expose (`.d[] | key == 0` answered
# nothing on `- 10` and `true` on `[10, 20]`), so every yq-mode case runs on
# both forms. Hand-written rather than derived with `yq -P`, so the sweep
# does not depend on the oracle's own pretty-printer; the runner checks the
# block files decode to the flow ones before any case runs.
cat > "$WORK/one-block.yaml" <<'YAML'
a:
  b: 1
  c: 2
d:
  - 10
  - 20
YAML
cat > "$WORK/two-block.yaml" <<'YAML'
a:
  b: 3
  c: 4
d:
  - 30
  - 40
YAML

# --- runner -----------------------------------------------------------------

esc() {
  printf '%s' "$1" \
    | tr '\t' ' ' \
    | sed "s|$WORK|WORK|g" \
    | awk 'BEGIN{ORS=""} NR>1{print "\\n"} {print}'
}

# Known divergences live in a manifest, not in this script: they are a
# scoreboard the migration has to shrink, and the check is two-sided the way
# tests/data/jq-golden-known-failures.txt is — an unlisted divergence fails,
# and a listed pattern that never matched fails too, so a fix cannot be
# absorbed by a stale entry.
KNOWN_FILE="$REPO_ROOT/tests/data/jq-path-context-sweep-known-divergences.txt"
declare -a KNOWN_PATTERNS=()
declare -a KNOWN_REASONS=()
declare -a KNOWN_HITS=()
while IFS= read -r line || [[ -n "$line" ]]; do
  line="${line%%$'\r'}"
  [[ -z "${line// /}" || "$line" == \#* ]] && continue
  pattern="${line%%[[:space:]]*}"
  rest="${line#"$pattern"}"
  KNOWN_PATTERNS+=("$pattern")
  KNOWN_REASONS+=("$(printf '%s' "$rest" | sed 's/^[[:space:]]*//')")
  KNOWN_HITS+=(0)
done < "$KNOWN_FILE"
((${#KNOWN_PATTERNS[@]} > 0)) || {
  echo "error: $KNOWN_FILE has no entries — a truncated manifest would turn every known gap into a failure" >&2
  exit 1
}

# Attribute a divergence to an already-tracked gap, or return 1 for "this is
# new, look at it". The label lands in `CLASSIFY_LABEL` rather than on stdout:
# a `$(...)` capture would run this in a subshell, where the `KNOWN_HITS`
# bookkeeping the two-sided staleness check depends on would be discarded.
CLASSIFY_LABEL=""
classify_divergence() {
  local key="$1" o_err="$2" i
  CLASSIFY_LABEL=""

  # yq refusing the jq-only surface outright is not a path-context question at
  # all — it is the `--jq-extensions` gating question (#1512 lineage). Two
  # distinct wordings, both captured live from v4.53.3:
  #
  #   "lexer: invalid input text"  — `limit`, `last`, `foreach`, `reduce`,
  #                                  `getpath`, `if`, `try`, `label`/`break`,
  #                                  `paths`, ... are not tokens at all.
  #   "bad expression, ..."        — `path` parses, but only in its bare form:
  #                                  real yq has no `path(f)`, so even
  #                                  `path(.a)` is refused.
  #
  # Matched on the oracle's own wording rather than a bare non-zero exit, so a
  # genuine yq *evaluation* error still counts as a divergence.
  if [[ "$o_err" == *"lexer: invalid input text"* \
     || "$o_err" == *"bad expression, please check expression syntax"* ]]; then
    CLASSIFY_LABEL='#1512 (yq rejects jq-only surface outright; succinctly evaluates it)'
    return 0
  fi

  for ((i = 0; i < ${#KNOWN_PATTERNS[@]}; i++)); do
    # shellcheck disable=SC2053 -- the manifest entry is a glob on purpose.
    if [[ "$key" == ${KNOWN_PATTERNS[i]} ]]; then
      KNOWN_HITS[i]=1
      CLASSIFY_LABEL="${KNOWN_PATTERNS[i]}  ${KNOWN_REASONS[i]}"
      return 0
    fi
  done

  return 1
}

total=0
diverged=0
unexpected=0
divergence_log=""
declare -a known_labels=()
declare -a unexpected_classes=()

S_ERR="$WORK/s.err"
O_ERR="$WORK/o.err"

run_case() {
  local mode="$1" class="$2" filter="$3" variant="${4:-}"
  total=$((total + 1))

  local s_out s_code o_out o_code s_err o_err
  if [[ "$mode" == jq ]]; then
    s_out="$("$SUCC" jq -c "$filter" "$WORK/doc.json" 2>"$S_ERR")" && s_code=0 || s_code=$?
    s_err="$(cat "$S_ERR")"
    o_out="$("$JQ" -c "$filter" "$WORK/doc.json" 2>"$O_ERR")" && o_code=0 || o_code=$?
    o_err="$(cat "$O_ERR")"
  else
    # Two input files, deliberately: `file_index` is a leaf in this alphabet
    # and is identically 0 with one file, so a single-file sweep could not
    # tell a correct implementation from a stubbed constant.
    s_out="$("$SUCC" yq -o=json -I=0 "$filter" "$WORK/one$variant.yaml" "$WORK/two$variant.yaml" 2>"$S_ERR")" && s_code=0 || s_code=$?
    s_err="$(cat "$S_ERR")"
    o_out="$("$YQ" -o=json -I=0 "$filter" "$WORK/one$variant.yaml" "$WORK/two$variant.yaml" 2>"$O_ERR")" && o_code=0 || o_code=$?
    o_err="$(cat "$O_ERR")"
  fi

  if [[ "$s_out" != "$o_out" || "$s_code" != "$o_code" ]]; then
    diverged=$((diverged + 1))
    if classify_divergence "$mode/$class" "$o_err"; then
      known_labels+=("$CLASSIFY_LABEL")
    else
      unexpected=$((unexpected + 1))
      unexpected_classes+=("$mode/$class${variant:+ (block)}")
      divergence_log+="[$mode $class${variant:+ block}] $filter
  oracle:      out=$(esc "$o_out") err=$(esc "$o_err") exit=$o_code
  succinctly:  out=$(esc "$s_out") err=$(esc "$s_err") exit=$s_code
"
    fi
  fi

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$mode${variant:+ block}" "$class" "$(esc "$filter")" \
    "$s_code" "$(esc "$s_out")" "$(esc "$s_err")" \
    "$o_code" "$(esc "$o_out")" "$(esc "$o_err")"
}

# The block files must be the same documents as the flow ones, or a block
# divergence could be the fixture rather than the evaluator.
if [[ -n "${YQ:-}" ]]; then
  for f in one two; do
    if [[ "$("$YQ" -o=json -I=0 . "$WORK/$f-block.yaml")" != "$(cat "$WORK/$f.yaml")" ]]; then
      echo "error: $WORK/$f-block.yaml does not decode to $WORK/$f.yaml" >&2
      exit 1
    fi
  done
fi

for line in "${CASES[@]}"; do
  IFS=$'\t' read -r c_mode c_class c_filter <<<"$line"
  run_case "$c_mode" "$c_class" "$c_filter"
  if [[ "$c_mode" == yq ]]; then
    run_case "$c_mode" "$c_class" "$c_filter" -block
  fi
done

{
  printf '%s' "$divergence_log"
  echo "== $total cases: $unexpected unexpected, $((diverged - unexpected)) known =="
  if ((diverged > unexpected)); then
    printf '%s\n' "${known_labels[@]}" | sort | uniq -c | sort -rn | sed 's/^/   known: /'
  fi
  if ((SUMMARY && unexpected > 0)); then
    printf '%s\n' "${unexpected_classes[@]}" | sort | uniq -c | sort -rn | sed 's/^/   unexpected: /'
  fi
} >&2

stale=0
for ((i = 0; i < ${#KNOWN_PATTERNS[@]}; i++)); do
  if [[ "${KNOWN_HITS[i]}" == 0 ]]; then
    echo "STALE: no divergence matched ${KNOWN_PATTERNS[i]} — if it is fixed, drop the line from" >&2
    echo "       tests/data/jq-path-context-sweep-known-divergences.txt" >&2
    stale=$((stale + 1))
  fi
done

if ((unexpected > 0 || stale > 0)); then
  echo "FAIL: $unexpected unexpected divergence(s), $stale stale manifest entry/entries — see above" >&2
  exit 1
fi
echo "OK: every divergence is on record and every record still diverges" >&2
