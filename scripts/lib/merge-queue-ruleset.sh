# shellcheck shell=bash
#
# Shared helpers for scripts/enable-merge-queue.sh and
# scripts/disable-merge-queue.sh.
#
# Both scripts toggle one repository ruleset ("Protect main branch" by default)
# between two mutually exclusive states:
#
#   merge-queue    merge_queue rule present, strict status checks OFF
#                  (GitHub batches PRs and rebases them onto main itself, so
#                  requiring an up-to-date branch beforehand is redundant — the
#                  UI hides the option once a merge queue is required)
#
#   strict-rebase  merge_queue rule absent, strict status checks ON
#                  ("Require branches to be up to date before merging": a PR
#                  cannot merge until its head has main's tip as an ancestor)
#
# Everything else in the ruleset — the required check contexts, the pull_request
# rule, non_fast_forward, conditions, bypass actors — is left untouched. The PUT
# sends only `rules`, relying on GitHub leaving omitted body fields alone. The
# REST docs mark every field optional but never say what omitting one does, so
# mq_apply_rules re-reads the ruleset afterwards and fails loudly if anything it
# did not ask to change moved.
#
# Sourced, not executed. The sourcing script owns `set -euo pipefail`.

# Ruleset selected when --ruleset is not given.
MQ_DEFAULT_RULESET_NAME="Protect main branch"

# The merge_queue rule this repo runs with when the queue is on. Held here
# rather than recovered from the API so enable-merge-queue.sh can restore a
# known-good state even after the rule has been deleted. Keep in sync if the
# queue is retuned in the GitHub UI (enable-merge-queue.sh --dry-run will show
# the drift as a diff).
# shellcheck disable=SC2034  # read by enable-merge-queue.sh, which sources this
MQ_BASELINE_MERGE_QUEUE_RULE='{
  "type": "merge_queue",
  "parameters": {
    "merge_method": "REBASE",
    "max_entries_to_build": 5,
    "min_entries_to_merge": 1,
    "max_entries_to_merge": 5,
    "min_entries_to_merge_wait_minutes": 5,
    "grouping_strategy": "ALLGREEN",
    "check_response_timeout_minutes": 60
  }
}'

# Merge methods the pull_request rule allows in the merge-queue state. Only
# consulted to undo disable-merge-queue.sh --rebase-only.
# shellcheck disable=SC2034  # read by enable-merge-queue.sh, which sources this
MQ_BASELINE_MERGE_METHODS='["merge","squash","rebase"]'

# Common flags, populated by mq_parse_common_args.
MQ_REPO=""
MQ_RULESET="$MQ_DEFAULT_RULESET_NAME"
MQ_DRY_RUN=false
MQ_STATUS_ONLY=false
MQ_ASSUME_YES=false
MQ_EXTRA_ARGS=()

# Cleaned up on exit rather than with a RETURN trap: bash 3.2 (what macOS ships)
# keeps a function-scoped RETURN trap installed after the function returns.
MQ_TMP_PAYLOAD=""
trap 'rm -f "${MQ_TMP_PAYLOAD:-}"' EXIT

mq_die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

mq_note() {
  printf '%s\n' "$*" >&2
}

# Common flag block shared by both scripts' --help output.
mq_common_options_help() {
  cat <<'EOF'
  --repo OWNER/NAME   Repository to act on (default: the current checkout).
  --ruleset NAME|ID   Ruleset to edit (default: "Protect main branch").
  --status            Print the current state and exit without changing it.
  -n, --dry-run       Print the payload and the diff; make no API call.
  -y, --yes           Do not prompt for confirmation.
  -h, --help          Show this help.
EOF
}

# Consumes the flags every toggle script shares. Anything unrecognised is left
# in MQ_EXTRA_ARGS for the calling script to interpret or reject.
mq_parse_common_args() {
  MQ_EXTRA_ARGS=()
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --repo)
        [[ $# -ge 2 ]] || mq_die "--repo needs a value"
        MQ_REPO="$2"
        shift 2
        ;;
      --repo=*)
        MQ_REPO="${1#--repo=}"
        shift
        ;;
      --ruleset)
        [[ $# -ge 2 ]] || mq_die "--ruleset needs a value"
        MQ_RULESET="$2"
        shift 2
        ;;
      --ruleset=*)
        MQ_RULESET="${1#--ruleset=}"
        shift
        ;;
      --status)
        MQ_STATUS_ONLY=true
        shift
        ;;
      -n | --dry-run)
        MQ_DRY_RUN=true
        shift
        ;;
      -y | --yes)
        MQ_ASSUME_YES=true
        shift
        ;;
      -h | --help)
        mq_usage
        exit 0
        ;;
      *)
        MQ_EXTRA_ARGS+=("$1")
        shift
        ;;
    esac
  done
}

mq_require_tools() {
  command -v gh >/dev/null 2>&1 || mq_die "gh not found on PATH — https://cli.github.com"
  command -v jq >/dev/null 2>&1 || mq_die "jq not found on PATH"
  gh auth status >/dev/null 2>&1 ||
    mq_die "gh is not authenticated — run 'gh auth login'"
}

# Editing rulesets needs admin on the repository; say so up front rather than
# letting a 403 surface from the middle of the PUT.
mq_resolve_repo() {
  if [[ -z "$MQ_REPO" ]]; then
    MQ_REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner)" ||
      mq_die "could not determine the repository — pass --repo OWNER/NAME"
  fi
}

# Sets MQ_RULESET_ID and MQ_RULESET_JSON from MQ_RULESET (a numeric id or a name).
mq_fetch_ruleset() {
  local listing
  # --paginate because the listing is capped at 30 per page: without it a repo
  # with more rulesets than that reports the target as missing. --slurp nests
  # the pages one level, which flatten undoes (and is a no-op on one page).
  # gh's own error is left on stderr rather than suppressed — a typo in --repo,
  # a 404 and a network failure are not all missing admin access.
  listing="$(gh api --paginate --slurp "repos/$MQ_REPO/rulesets" | jq 'flatten')" ||
    mq_die "cannot list rulesets for $MQ_REPO (gh reported the error above); editing rulesets needs admin on the repository"

  if [[ "$MQ_RULESET" =~ ^[0-9]+$ ]]; then
    MQ_RULESET_ID="$MQ_RULESET"
  else
    MQ_RULESET_ID="$(jq -r --arg name "$MQ_RULESET" \
      'map(select(.name == $name)) | first | .id // empty' <<<"$listing")"
    if [[ -z "$MQ_RULESET_ID" ]]; then
      mq_note "error: no ruleset named \"$MQ_RULESET\" in $MQ_REPO. Available:"
      jq -r '.[] | "  \(.id)  \(.name) [\(.target)]"' <<<"$listing" >&2
      exit 1
    fi
  fi

  MQ_RULESET_JSON="$(gh api "repos/$MQ_REPO/rulesets/$MQ_RULESET_ID")" ||
    mq_die "cannot read ruleset $MQ_RULESET_ID in $MQ_REPO"
}

# Names the state a ruleset is in: merge-queue, strict-rebase, or mixed.
mq_mode_of() {
  jq -r '
    (.rules // []) as $rules
    | ($rules | map(select(.type == "merge_queue")) | length > 0) as $queued
    | ($rules
       | map(select(.type == "required_status_checks"))
       | first
       | .parameters.strict_required_status_checks_policy // false) as $strict
    | if   $queued and ($strict | not) then "merge-queue"
      elif ($queued | not) and $strict then "strict-rebase"
      else "mixed" end
  ' <<<"$1"
}

mq_print_status() {
  local json="$1"
  jq -r '
    (.rules // []) as $rules
    | ($rules | map(select(.type == "merge_queue")) | first) as $mq
    | ($rules | map(select(.type == "required_status_checks")) | first) as $rsc
    | ($rules | map(select(.type == "pull_request")) | first) as $pr
    | "  ruleset:       \(.name) (id \(.id), enforcement: \(.enforcement))",
      "  merge queue:   " + (
        if $mq == null then "off"
        else "on (\($mq.parameters.merge_method), "
             + "build \($mq.parameters.max_entries_to_build), "
             + "merge \($mq.parameters.min_entries_to_merge)-\($mq.parameters.max_entries_to_merge), "
             + "wait \($mq.parameters.min_entries_to_merge_wait_minutes)m, "
             + "\($mq.parameters.grouping_strategy))"
        end),
      "  strict checks: " + (
        if $rsc == null then "n/a (no required_status_checks rule)"
        elif ($rsc.parameters.strict_required_status_checks_policy // false)
        then "on (branch must be up to date with the base before merging)"
        else "off" end),
      "  merge methods: " + (
        if $pr == null then "n/a (no pull_request rule)"
        else (($pr.parameters.allowed_merge_methods // []) | join(", ")) end)
  ' <<<"$json"
  printf '  mode:          %s\n' "$(mq_mode_of "$json")"
}

# Writes the pre-change ruleset under .ai/scratch/ (git-ignored) so a botched
# toggle can be undone with:
#   gh api --method PUT repos/OWNER/NAME/rulesets/ID \
#     --input <(jq '{rules}' BACKUP.json)
mq_backup_ruleset() {
  local dir="$MQ_REPO_ROOT/.ai/scratch/merge-queue"
  mkdir -p "$dir"
  local stamp slug
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  # Backups land under the checkout the script lives in, which need not be the
  # repo being edited — --repo names another one, and with no --repo the target
  # comes from the cwd, which may be a different checkout entirely. Name the
  # file after the target so a backup is never ambiguous about what it restores.
  slug="${MQ_REPO//\//-}"
  MQ_BACKUP_FILE="$dir/ruleset-$slug-$MQ_RULESET_ID-$stamp.json"
  printf '%s\n' "$MQ_RULESET_JSON" >"$MQ_BACKUP_FILE"
}

# Shows what the new rules array changes, honours --dry-run and the confirmation
# prompt, then PUTs. Argument: the complete replacement rules array as JSON.
mq_apply_rules() {
  local new_rules="$1"
  local target_mode="$2"

  local before after before_canon after_canon
  before="$(jq -S '.rules // []' <<<"$MQ_RULESET_JSON")"
  after="$(jq -S '.' <<<"$new_rules")"

  # Compare on a canonical rule order. Position within the array carries no
  # meaning and is GitHub's to pick, but the rebuilt array always appends
  # merge_queue last, so a plain comparison would call a ruleset that already
  # holds the target state "changed" and re-PUT it whenever GitHub happened to
  # store that rule somewhere else. The diff below still uses the real order.
  before_canon="$(jq -S 'sort_by(.type)' <<<"$before")"
  after_canon="$(jq -S 'sort_by(.type)' <<<"$after")"

  if [[ "$before_canon" == "$after_canon" ]]; then
    printf 'Already in "%s" mode — nothing to do.\n' "$target_mode"
    mq_print_status "$MQ_RULESET_JSON"
    return 0
  fi

  printf 'Rule changes for %s ruleset %s:\n' "$MQ_REPO" "$MQ_RULESET_ID"
  diff -u \
    --label 'current' <(printf '%s\n' "$before") \
    --label 'proposed' <(printf '%s\n' "$after") || true
  printf '\n'

  if [[ "$MQ_DRY_RUN" == true ]]; then
    printf 'Dry run — no changes made. Request body would be:\n'
    jq -n --argjson rules "$new_rules" '{rules: $rules}'
    return 0
  fi

  # No tty means there is no way to ask, which is a reason to stop rather than a
  # reason to proceed: --yes is how a caller opts into applying unattended, and
  # gating the prompt on -t 0 would hand that out for free to cron, CI, a pipe
  # and anything run with stdin closed.
  if [[ "$MQ_ASSUME_YES" != true ]]; then
    [[ -t 0 ]] ||
      mq_die "stdin is not a terminal, so this cannot be confirmed — re-run with --yes to apply unattended"
    local reply
    read -r -p "Apply this to $MQ_REPO? [y/N] " reply
    [[ "$reply" == [yY] || "$reply" == [yY][eE][sS] ]] || mq_die "aborted"
  fi

  mq_backup_ruleset
  printf 'Backed up the current ruleset to %s\n' "$MQ_BACKUP_FILE"

  MQ_TMP_PAYLOAD="$(mktemp)"
  jq -n --argjson rules "$new_rules" '{rules: $rules}' >"$MQ_TMP_PAYLOAD"

  gh api --method PUT "repos/$MQ_REPO/rulesets/$MQ_RULESET_ID" \
    --input "$MQ_TMP_PAYLOAD" >/dev/null ||
    mq_die "PUT failed; the ruleset is unchanged. Backup: $MQ_BACKUP_FILE"

  local updated
  updated="$(gh api "repos/$MQ_REPO/rulesets/$MQ_RULESET_ID")"

  # The request body carried only `rules`, on the understanding that GitHub
  # leaves the fields you omit alone. Nothing in the API docs actually says so,
  # and if it were wrong the damage — this ruleset's bypass actors or the ref
  # pattern it applies to, silently dropped — sits exactly where mq_mode_of
  # cannot see it, since that reads `rules` and nothing else. So check.
  local field
  for field in name target enforcement bypass_actors conditions; do
    [[ "$(jq -Sc --arg f "$field" '.[$f]' <<<"$MQ_RULESET_JSON")" \
      == "$(jq -Sc --arg f "$field" '.[$f]' <<<"$updated")" ]] ||
      mq_die "the PUT changed .$field, which it was not asked to touch — restore from $MQ_BACKUP_FILE"
  done

  printf '\nUpdated %s:\n' "$MQ_REPO"
  mq_print_status "$updated"

  local got
  got="$(mq_mode_of "$updated")"
  if [[ "$got" != "$target_mode" ]]; then
    mq_die "expected mode \"$target_mode\" but the ruleset now reads \"$got\" — restore from $MQ_BACKUP_FILE"
  fi
}

# Shared entry sequence: both scripts do exactly this before computing rules.
mq_begin() {
  mq_require_tools
  mq_resolve_repo
  mq_fetch_ruleset
  if [[ "$MQ_STATUS_ONLY" == true ]]; then
    printf 'Current state of %s:\n' "$MQ_REPO"
    mq_print_status "$MQ_RULESET_JSON"
    exit 0
  fi
}
