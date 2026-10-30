#!/usr/bin/env bash
# Run the CI gates a diff can fail before pushing it (#5585). See scripts/pre_push.py.
#
#   scripts/pre-push.sh [run] [--full] [--base REF]
#   scripts/pre-push.sh list
#
# `.claude/settings.json` runs `hook` on every Bash call an agent makes, which blocks `git push`
# while this fails.
exec python3 "$(dirname "${BASH_SOURCE[0]}")/pre_push.py" "$@"
