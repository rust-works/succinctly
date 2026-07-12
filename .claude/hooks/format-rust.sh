#!/usr/bin/env bash
# PostToolUse hook: rustfmt the Rust source file the agent just edited.
#
# Non-blocking: always exits 0 so a transient fmt failure never interrupts
# editing. Pins --edition 2021 to match the crate (there is no rustfmt.toml;
# bare rustfmt defaults to edition 2015 and can misformat 2021 syntax).
set -u

cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0

file_path="$(jq -r '.tool_input.file_path // empty')"

case "$file_path" in
  *.rs) [ -f "$file_path" ] && rustfmt --edition 2021 "$file_path" >/dev/null 2>&1 || true ;;
esac

exit 0
