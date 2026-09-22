#!/usr/bin/env bash
# Keep skill manifests discoverable on case-sensitive filesystems.
set -u

root="$(git rev-parse --show-toplevel 2>/dev/null)" || { printf '{}\n'; exit 0; }
input="$(cat)"

# A second Stop attempt should not create a continuation loop.
if jq -e '.stop_hook_active == true' <<<"$input" >/dev/null 2>&1; then
  printf '{}\n'
  exit 0
fi

skills_dir="$root/.agents/skills"
[ -d "$skills_dir" ] || { printf '{}\n'; exit 0; }

bad="$(find "$skills_dir" -maxdepth 2 -iname 'skill.md' ! -name 'SKILL.md')"
if [ -z "$bad" ]; then
  printf '{}\n'
  exit 0
fi

{
  echo "Skill manifests must be named exactly SKILL.md:"
  echo "$bad" | sed 's/^/  - /'
} >&2
exit 2
