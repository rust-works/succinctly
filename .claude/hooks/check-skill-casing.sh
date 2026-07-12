#!/usr/bin/env bash
# Stop hook: block the agent from ending its turn if any skill manifest is not
# named exactly SKILL.md.
#
# A lowercase (or mixed-case) skill.md is not discovered by skill loaders on
# case-sensitive Linux CI — see issue #189. exit 2 surfaces the message on
# stderr back to the model as a blocking error it must resolve (rename the
# file) before stopping.
set -u

cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0

input="$(cat)"

# Avoid loops: if this Stop hook already fired and is being asked again, defer.
[ "$(jq -r '.stop_hook_active // false' <<<"$input")" = "true" ] && exit 0

bad="$(find .claude/skills -maxdepth 2 -iname 'skill.md' ! -name 'SKILL.md' 2>/dev/null)"
[ -z "$bad" ] && exit 0

{
  echo "Skill manifests must be named exactly SKILL.md (lowercase is not discovered"
  echo "on Linux CI — see #189). Rename via git mv:"
  echo "$bad" | sed 's/^/  - /'
} >&2

exit 2
