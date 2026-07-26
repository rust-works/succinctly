#!/usr/bin/env bash
#
# Switch main-branch protection from the merge queue to strict merges.
#
# Removes the `merge_queue` rule and turns on the strict required-status-checks
# policy, so a PR can only merge once its head branch has main's tip as an
# ancestor — i.e. it must be rebased (or otherwise brought up to date) onto main
# and re-run its checks against that state. GitHub calls this "Require branches
# to be up to date before merging"; it is the strict counterpart to the merge
# queue, which does the same rebase-and-recheck itself on a batch of PRs.
#
# Undo with ./scripts/enable-merge-queue.sh — the two scripts are a toggle pair
# and each fully reverses the other.
#
# Usage:
#   ./scripts/disable-merge-queue.sh --status     # what is set right now
#   ./scripts/disable-merge-queue.sh --dry-run    # show the diff, change nothing
#   ./scripts/disable-merge-queue.sh              # apply (prompts to confirm)
#   ./scripts/disable-merge-queue.sh --rebase-only  # also force rebase merges
#
# Requires the gh CLI authenticated with admin access to the repository.

set -euo pipefail

MQ_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# source-path so the directive resolves against this script's directory rather
# than whatever directory shellcheck happens to be invoked from.
# shellcheck source-path=SCRIPTDIR
# shellcheck source=lib/merge-queue-ruleset.sh
source "$MQ_REPO_ROOT/scripts/lib/merge-queue-ruleset.sh"

# Also narrow the pull_request rule to rebase-only merges, so a PR lands on main
# as a replay of its commits rather than a merge or squash commit. Off by
# default: strictness is about the branch being up to date, and silently taking
# merge methods away from a live repo is a bigger change than was asked for.
rebase_only=false

mq_usage() {
  cat <<EOF
Usage: $(basename "$0") [options]

Removes the merge queue from the main-branch ruleset and requires PR branches to
be up to date with main (rebased) before they can merge.

Options:
  --rebase-only       Also restrict the allowed merge methods to "rebase".
$(mq_common_options_help)
EOF
}

mq_parse_common_args "$@"
for arg in ${MQ_EXTRA_ARGS+"${MQ_EXTRA_ARGS[@]}"}; do
  case "$arg" in
    --rebase-only) rebase_only=true ;;
    *) mq_die "unknown option: $arg (try --help)" ;;
  esac
done

mq_begin

# Strict mode lives on the required_status_checks rule; without that rule there
# is nothing to make strict, and dropping the merge queue would leave main with
# no gate at all.
jq -e '(.rules // []) | any(.type == "required_status_checks")' \
  <<<"$MQ_RULESET_JSON" >/dev/null ||
  mq_die "ruleset $MQ_RULESET_ID has no required_status_checks rule — refusing to remove the merge queue, as that would leave main ungated"

new_rules="$(jq --argjson rebase_only "$rebase_only" '
  (.rules // [])
  | map(select(.type != "merge_queue"))
  | map(if .type == "required_status_checks"
        then .parameters.strict_required_status_checks_policy = true
        else . end)
  | if $rebase_only
    then map(if .type == "pull_request"
             then .parameters.allowed_merge_methods = ["rebase"]
             else . end)
    else . end
' <<<"$MQ_RULESET_JSON")"

mq_apply_rules "$new_rules" "strict-rebase"
