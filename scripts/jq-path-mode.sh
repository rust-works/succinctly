#!/usr/bin/env bash
# The #5223 path-mode interpreter under jq's argv, so a differential sweep can
# take it as its `--bin`/`--candidate` unchanged: `jq-path-mode.sh jq -c PROG`
# runs `succinctly dev jq-path-mode -c -- PROG` (stdin is passed through).
#
# The binary is $SUCCINCTLY_PATH_MODE_BIN, else target/release/succinctly in this
# checkout (build it with `cargo build --release --features cli`). An option
# the interpreter does not take exits 64 with a `path-mode: unsupported` line, the
# status `scripts/jq-path-mode-3way.py` counts apart from verdicts.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
bin=${SUCCINCTLY_PATH_MODE_BIN:-$root/target/release/succinctly}
if [[ ${1:-} == jq ]]; then shift; fi
opts=()
while (($#)); do
  case $1 in
    -c | -r | -n | --compact-output | --raw-output | --null-input) opts+=("$1") ;;
    -[crn][crn] | -[crn][crn][crn]) for ((i = 1; i < ${#1}; i++)); do opts+=("-${1:i:1}"); done ;;
    --) shift; break ;;
    -[a-zA-Z]* | --[a-z]*) echo "path-mode: unsupported: option $1" >&2; exit 64 ;;
    *) break ;;
  esac
  shift
done
# `${opts[@]+...}`: bash 3.2 (macOS) treats an empty array as unset under `set -u`.
exec "$bin" dev jq-path-mode ${opts[@]+"${opts[@]}"} -- "$@"
