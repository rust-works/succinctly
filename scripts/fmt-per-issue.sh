#!/usr/bin/env bash
# Check the formatting of the per-issue test files (#5697).
#
# `tests/{jq,yq}_cli/mod.rs` declare their modules with `automod::dir!`, a macro, and rustfmt
# reaches module files only by following `mod` declarations, so `cargo fmt --all -- --check`
# no longer sees `tests/jq_cli/iN.rs` and friends. This formats every `.rs` file in each
# directory whose `mod.rs` uses `automod::dir!` directly instead. CI's `Format` job and
# `scripts/pre-push.sh` run it next to `cargo fmt --all -- --check`.
#
#   scripts/fmt-per-issue.sh          check (exit 1 and print a diff when a file would change)
#   scripts/fmt-per-issue.sh --fix    rewrite the files in place (what `cargo fmt` did before)
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

mode=(--check)
if [[ "${1:-}" == "--fix" ]]; then
  mode=()
elif [[ $# -gt 0 ]]; then
  echo "usage: $0 [--fix]" >&2
  exit 2
fi

edition=$(sed -n 's/^edition *= *"\([0-9]*\)".*/\1/p' Cargo.toml | head -1)
if [[ -z "$edition" ]]; then
  echo "fmt-per-issue: no edition in Cargo.toml" >&2
  exit 2
fi

files=()
for modrs in tests/*/mod.rs; do
  grep -q 'automod::dir!' "$modrs" || continue
  for f in "$(dirname "$modrs")"/*.rs; do
    files+=("$f")
  done
done

# A glob that matched nothing would make the check pass over zero files.
if [[ ${#files[@]} -lt 2 ]]; then
  echo "fmt-per-issue: found ${#files[@]} files; expected the per-issue test modules" >&2
  exit 2
fi

exec rustfmt ${mode[@]+"${mode[@]}"} --edition "$edition" "${files[@]}"
