#!/usr/bin/env bash
#
# Regenerate the yq-mode `tonumber` grammar fixture (#2960).
#
# Real yq's `tonumber` accepts a materially wider grammar than RFC 8259 --
# underscored/hex/octal integers, Go hex floats, and inf/nan words, all
# ported from yq's `tryConvertToNumber` (pkg/yqlib/operator_to_number.go +
# lib.go). This captures the pinned yq's answer for a curated seed list (every
# row from the #2960 triage's reference table, plus the rounding/subnormal/
# overflow boundary vectors) and a deterministic pseudo-random sweep over the
# same alphabet, into tests/data/yq-tonumber-fixture.tsv, which
# tests/yq_tonumber_fixture_tests.rs then replays against succinctly.
#
# Usage:
#   ./scripts/sync-yq-tonumber-fixture.sh              # regenerate the fixture
#   ./scripts/sync-yq-tonumber-fixture.sh --check       # verify it matches pinned yq
#
# To move to a newer yq, bump the pin below, install that version, run this,
# and review the diff.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$REPO_ROOT/tests/data/yq-tonumber-fixture.tsv"
PIN="v4.53.3"

check_only=false
[[ "${1:-}" == "--check" ]] && check_only=true

command -v yq >/dev/null 2>&1 || {
  echo "error: yq not found on PATH — install mikefarah/yq $PIN" >&2
  echo "  https://github.com/mikefarah/yq/releases/tag/$PIN" >&2
  exit 1
}
version_line="$(yq --version)"
if [[ "$version_line" != *"$PIN"* ]]; then
  echo "error: yq on PATH is '$version_line' but the fixture is pinned to $PIN" >&2
  exit 1
fi

# ── Seed list: every row named in the #2960 triage's reference table, plus
# the rounding/subnormal/overflow boundary vectors from its Tests section. ──
SEEDS=(
  # underscore placement (int stage: strip unconditionally)
  '_1000' '1000_' '1__000' '0x_10' '0x10_' '0_0' '_' '0x_'
  # hex prefix + sign-after-prefix (int stage)
  '0x10' '0X1F' '0x-10' '0x+10' '0x-8000000000000000' '0x10e'
  '-0x10' '+0x10' '0x' '0x8000000000000000' '00x10'
  # octal prefix, lowercase only (int stage)
  '0o17' '0o1_7' '0o-17' '0O17' '0o8' '0b101' '0B101'
  # decimal (int stage): sign, leading zeros
  '017' '018' '08' '+017' '-017' '00' '+0' '-0' '9223372036854775807'
  '9223372036854775808' '-9223372036854775808' '-9223372036854775809'
  # float stage: underscoreOK (between digits only; prefix counts as a digit)
  '1_000.5' '1_0e3' '1e1_0' '1.5e3_0' '1_000_000_000_000_000_000'
  '1_e3' '1._5' '1_000e' '_1.5' '1.5_'
  # Go hex floats: mandatory p exponent, optional underscore after prefix
  '0x1p3' '0X1p3' '0x1P3' '0x1.8p1' '0x.8p1' '0x1.p1' '0x_1p3' '0x1p-2000'
  '0x10.5' '0xp1' '0x1p2000' '0o1p3' '-0x1p3' '+0x1p3' '0x1p+3'
  # hex-float structural rejects: invalid mantissa digit, missing/non-digit
  # exponent, and a genuinely zero mantissa (skips the bignum loop entirely)
  '0x1.gp1' '0xg.1p3' '0x1p' '0x1pA' '0x1p+' '0x0p5' '0x0.0p5' '0x00p9'
  # exponent magnitude far past EXP2_CLAMP in both directions -- short-circuits
  # before the bignum scaling loop runs at all, rather than looping ~5000 times
  '0x1p999999' '0x1p-999999'
  # Go special words: sign + inf/infinity, unsigned-only nan, case-insensitive
  'inf' 'Inf' '+inf' '-inf' '-Infinity' 'INFINITY' 'infinity' 'NaN' 'nan'
  'NAN' 'nAn' '+nan' '-nan' '.inf' '.nan' 'infin' 'infinityx' 'infinities'
  # decimal float overflow/underflow (ErrRange only rejects overflow) -- both
  # via the pre-existing RFC 8259 literal-preserving arm (no underscore) and
  # via yq_parse_float's own path (underscored, so RFC 8259 never sees it)
  '1e-400' '1e999' '-1e999' '1e309' '1.7976931348623157e+308'
  '1.7976931348623159e+308' '5e-324' '4.9e-324' '1e-324' '1e99_9' '-1e99_9'
  '+1e999'
  # no trimming in yq mode
  ' 1 ' '1 ' "$(printf '\t1')" ' 0x10 ' "$(printf '1\t')"
  # already-agreeing rows that must not move (RFC 8259 literal-preserving arms)
  '2.50' '1e3' '1e+3' '-0.0' '99999999999999999999'
  # spelling-differs rows (#1356 territory; accept/value only, not spelling)
  '017x' '+2.0' '2.' '.5' '5.'
  # hex-float rounding/subnormal/overflow boundary vectors
  '0x1.fffffffffffff8p0' '0x1.00000000000008p0' '0x1p-1074' '0x1p-1075'
  '0x1.fffffffffffffp1023' '0x1.fffffffffffff8p1023' '0x1.ffffffffffffep1023'
  # plainly invalid
  'abc' '' '1 2' '[1,2]' 'true' 'null' "$(printf '\xef\xbf\xbd')"
)

# ── Deterministic pseudo-random sweep over the same alphabet, length <= 7. ──
ALPHABET='0123456789abcdefABCDEFxXoObBpPeE._+-inty'
gen_random() {
  local count="$1" seed="$2"
  awk -v count="$count" -v seed="$seed" -v alphabet="$ALPHABET" '
    BEGIN {
      srand(seed)
      n = length(alphabet)
      for (i = 0; i < count; i++) {
        len = int(rand() * 7) + 1
        s = ""
        for (j = 0; j < len; j++) {
          idx = int(rand() * n) + 1
          s = s substr(alphabet, idx, 1)
        }
        print s
      }
    }'
}

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

all_inputs="$work_dir/inputs.txt"
printf '%s\n' "${SEEDS[@]}" > "$all_inputs"
gen_random 400 2960 >> "$all_inputs"
# De-duplicate while preserving first occurrence.
awk '!seen[$0]++' "$all_inputs" > "$work_dir/inputs-dedup.txt"
mv "$work_dir/inputs-dedup.txt" "$all_inputs"

esc() {
  # Escape for a single TSV field: backslash, tab, newline.
  printf '%s' "$1" | awk '
    BEGIN { ORS = "" }
    {
      s = $0
      gsub(/\\/, "\\\\", s)
      printf "%s", s
    }' | perl -pe 's/\t/\\t/g; s/\n/\\n/g' 2>/dev/null || printf '%s' "$1"
}

out="$work_dir/table.tsv"
{
  echo "# GENERATED by ./scripts/sync-yq-tonumber-fixture.sh from yq $PIN —"
  echo "# do not hand-edit. Columns: <escaped-input>	<accept 0|1>	<tag-or-empty>	<json-value-or-empty>"
  echo "# Seeds + a deterministic pseudo-random sweep live inline in the"
  echo "# generator script; the consumer is tests/yq_tonumber_fixture_tests.rs."
} > "$out"

count=0
while IFS= read -r input; do
  # `strenv(NAME)`, not `env(NAME)`: `env()` applies yq's own YAML scalar
  # type resolution to the variable's text *before* `tonumber` ever sees it
  # (confirmed live: `env(...)` on `"018"` is already tag `!!float`, on
  # `"017"` already `!!int` -- go-yaml's own bare-scalar resolution, nothing
  # to do with `tonumber`'s grammar). `strenv()` is the genuine, unresolved
  # string `tonumber`'s grammar actually has to decide, matching what a
  # quoted filter literal (`"<s>" | tonumber`) receives, which is what
  # tests/yq_tonumber_fixture_tests.rs actually runs against succinctly.
  #
  # Ground truth is arithmetic (`+0`), not `tonumber | tag` or a JSON
  # marshal: `tag` is a *looser* check than the number tonumber's own
  # `CreateReplacement` then carries forward unvalidated -- `-0x10`/
  # `0x8000000000000000` (i64 overflow)/`0O17`/`0b101` all report
  # `tag == !!int` yet error on ordinary arithmetic (confirmed live), which
  # is what the #2960 triage's source-derived reference table already
  # predicted as rejected. A JSON marshal has the opposite problem in the
  # other direction: it errors on any genuinely-accepted non-finite value
  # (`inf`/`nan`) purely because Go's `encoding/json` cannot represent
  # `+Inf`/`NaN` at all -- unrelated to `tonumber`'s own grammar, and the
  # same limitation succinctly's JSON output has today (#1071/#2579,
  # tracked separately). Arithmetic has neither failure mode.
  if ! YQ_PROBE_INPUT="$input" yq -n '(strenv(YQ_PROBE_INPUT) | tonumber) + 0' >/dev/null 2>&1; then
    printf '%s\t0\t\t\n' "$(esc "$input")" >> "$out"
    count=$((count + 1))
    continue
  fi
  accept=1
  tag_field="$(YQ_PROBE_INPUT="$input" yq -n 'strenv(YQ_PROBE_INPUT) | tonumber | tag' 2>&1 || true)"
  # The JSON value is the real comparison target for canonicalized rows
  # (yq's own `-o=json` re-parses+reformats through Go's number marshaler,
  # e.g. "0x10" -> 16, matching the plan's "int stage gives OwnedValue::Int"
  # design) -- except when it fails purely on the non-finite limitation
  # above, in which case the field is left empty and the consumer test
  # checks accept/reject only, not the exact value, for that row.
  if json_out="$(YQ_PROBE_INPUT="$input" yq -n -o=json -I=0 'strenv(YQ_PROBE_INPUT) | tonumber' 2>&1)"; then
    json_field="$(esc "$json_out")"
  else
    json_field=""
  fi
  printf '%s\t%s\t%s\t%s\n' "$(esc "$input")" "$accept" "$tag_field" "$json_field" >> "$out"
  count=$((count + 1))
done < "$all_inputs"

if $check_only; then
  if diff -q "$out" "$OUT" >/dev/null 2>&1; then
    echo "OK: $count rows match $OUT"
    exit 0
  else
    echo "STALE: $OUT does not match the pinned yq $PIN — run without --check to regenerate" >&2
    diff "$OUT" "$out" || true
    exit 1
  fi
fi

cp "$out" "$OUT"
echo "Wrote $count rows to $OUT"
