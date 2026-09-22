#!/usr/bin/env bash
# Format Rust files changed by a Codex edit.
# This hook is advisory: formatting failures must not interrupt the edit.
set -u

root="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0
input="$(cat)"

while IFS= read -r file_path; do
  case "$file_path" in
    /*) candidate="$file_path" ;;
    *) candidate="$root/$file_path" ;;
  esac

  candidate="$(realpath "$candidate" 2>/dev/null)" || continue
  case "$candidate" in
    "$root"/*.rs) [ -f "$candidate" ] && rustfmt --edition 2021 "$candidate" >/dev/null 2>&1 || true ;;
  esac
done < <(
  jq -r '
    (.tool_input.file_path // empty),
    (.tool_input.command // "" | split("\n")[] |
      select(test("^\\*\\*\\* (Update|Add) File: ")) |
      sub("^\\*\\*\\* (Update|Add) File: "; ""))
  ' <<<"$input" 2>/dev/null
)

exit 0
