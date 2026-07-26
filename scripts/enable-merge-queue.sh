#!/usr/bin/env bash
#
# Restore the main-branch merge queue as this project runs it.
#
# Adds back the `merge_queue` rule (rebase merges, batches of up to 5, all-green
# grouping) and turns the strict required-status-checks policy off, because the
# queue already rebases each batch onto main's tip and re-runs the required
# checks there — requiring the branch to be up to date beforehand is redundant
# and GitHub hides the option once a queue is required.
#
# Undo with ./scripts/disable-merge-queue.sh — the two scripts are a toggle pair
# and each fully reverses the other.
#
# Usage:
#   ./scripts/enable-merge-queue.sh --status     # what is set right now
#   ./scripts/enable-merge-queue.sh --dry-run    # show the diff, change nothing
#   ./scripts/enable-merge-queue.sh              # apply (prompts to confirm)
#
# The queue's settings come from MQ_BASELINE_MERGE_QUEUE_RULE in
# scripts/lib/merge-queue-ruleset.sh; --params-file overrides them for a one-off.
#
# Requires the gh CLI authenticated with admin access to the repository.

set -euo pipefail

MQ_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# source-path so the directive resolves against this script's directory rather
# than whatever directory shellcheck happens to be invoked from.
# shellcheck source-path=SCRIPTDIR
# shellcheck source=lib/merge-queue-ruleset.sh
source "$MQ_REPO_ROOT/scripts/lib/merge-queue-ruleset.sh"

params_file=""
# disable-merge-queue.sh --rebase-only narrows the allowed merge methods, so the
# reverse restores them. Opt out when the repo has deliberately diverged.
restore_merge_methods=true

mq_usage() {
  cat <<EOF
Usage: $(basename "$0") [options]

Adds the merge_queue rule back to the main-branch ruleset and turns off the
strict up-to-date requirement that ./scripts/disable-merge-queue.sh sets.

Options:
  --params-file FILE  Read merge_queue settings from FILE instead of the
                      baseline. Accepts a whole rule object or a bare
                      parameters object.
  --keep-merge-methods
                      Leave the pull_request rule's allowed_merge_methods as
                      they are (default: restore $MQ_BASELINE_MERGE_METHODS).
$(mq_common_options_help)
EOF
}

mq_parse_common_args "$@"
extra=(${MQ_EXTRA_ARGS+"${MQ_EXTRA_ARGS[@]}"})
i=0
while [[ $i -lt ${#extra[@]} ]]; do
  case "${extra[$i]}" in
    --params-file)
      i=$((i + 1))
      [[ $i -lt ${#extra[@]} ]] || mq_die "--params-file needs a value"
      params_file="${extra[$i]}"
      ;;
    --params-file=*) params_file="${extra[$i]#--params-file=}" ;;
    --keep-merge-methods) restore_merge_methods=false ;;
    *) mq_die "unknown option: ${extra[$i]} (try --help)" ;;
  esac
  i=$((i + 1))
done

mq_begin

# Accept either shape so a file captured from `gh api .../rulesets/ID` (a rule
# object) and a hand-written settings blob both work.
if [[ -n "$params_file" ]]; then
  [[ -f "$params_file" ]] || mq_die "no such file: $params_file"
  merge_queue_rule="$(jq '{type: "merge_queue", parameters: (.parameters // .)}' \
    "$params_file")" || mq_die "$params_file is not valid JSON"
else
  merge_queue_rule="$MQ_BASELINE_MERGE_QUEUE_RULE"
fi

# Appending the queue rule last reproduces the order GitHub stores it in, which
# keeps disable -> enable a byte-identical round trip.
new_rules="$(jq \
  --argjson mq "$merge_queue_rule" \
  --argjson methods "$MQ_BASELINE_MERGE_METHODS" \
  --argjson restore "$restore_merge_methods" '
  (.rules // [])
  | map(select(.type != "merge_queue"))
  | map(if .type == "required_status_checks"
        then .parameters.strict_required_status_checks_policy = false
        else . end)
  | if $restore
    then map(if .type == "pull_request"
             then .parameters.allowed_merge_methods = $methods
             else . end)
    else . end
  | . + [$mq]
' <<<"$MQ_RULESET_JSON")"

mq_apply_rules "$new_rules" "merge-queue"
