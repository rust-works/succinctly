#!/usr/bin/env bash
# Runs ON the EC2 instance; `compare-ec2.sh` uploads it and the launch script starts it as root.
#
#   instance.sh bake   provision a stock Amazon Linux 2023 instance into the benchmark image,
#                      then power off so the driver can snapshot it
#   instance.sh run    on an instance launched from that image: build the commit under test,
#                      run `succinctly dev compare jq`, upload the results, power off (the
#                      driver launches run instances with shutdown behaviour `terminate`)
#
# Parameters come from /etc/succinctly-compare/params.env, which the launch script writes (see
# `user_data` in compare-ec2.sh). Progress goes to s3://$BUCKET/$PREFIX/: `status.json`
# (state, exit code, instance id), `log.txt` (refreshed every 5 minutes) and, for a run,
# `results.tar.gz`; `cap.txt` shows the current hard cap, and `control.txt`, written by
# `compare-ec2.sh extend`, moves it. The harness and every build run as the unprivileged ec2-user, never root:
# several CLI probes test permission errors, which root does not see.
set -euo pipefail

PARAMS=/etc/succinctly-compare/params.env
# shellcheck source=/dev/null
. "$PARAMS"
: "${MODE:?}" "${BUCKET:?}" "${PREFIX:?}" "${REGION:?}" "${REPO_URL:?}"
export AWS_DEFAULT_REGION="$REGION"

OPT=/opt/succinctly-compare
LOG=/var/log/succinctly-compare.log
USER_NAME=ec2-user
USER_HOME=/home/$USER_NAME
SELF="$(readlink -f "$0")"
S3="s3://$BUCKET/$PREFIX"

imds() {
  local tok
  tok="$(curl -fsS -X PUT http://169.254.169.254/latest/api/token \
    -H 'X-aws-ec2-metadata-token-ttl-seconds: 300')"
  curl -fsS -H "X-aws-ec2-metadata-token: $tok" "http://169.254.169.254/latest/meta-data/$1"
}

put_status() { # state [exit-code]; called by root and by the ec2-user phases
  local rc="${2:-null}" f
  f="$(mktemp)"
  printf '{"state":"%s","rc":%s,"time":"%s","instance":"%s","mode":"%s"}\n' \
    "$1" "$rc" "$(date -u +%FT%TZ)" "${INSTANCE_ID:-$(imds instance-id || echo unknown)}" "$MODE" >"$f"
  aws s3 cp --only-show-errors "$f" "$S3/status.json" || true
  rm -f "$f"
  echo "== status: $1${2:+ (exit $2)}"
}

push_log() { aws s3 cp --only-show-errors "$LOG" "$S3/log.txt" || true; }

as_user() { # run this script's user phase as ec2-user with a login environment
  sudo -u "$USER_NAME" -H env MODE="$MODE" bash -lc "\"$SELF\" $*"
}

# ---------------------------------------------------------------------------------------------
# Root phases
# ---------------------------------------------------------------------------------------------

# Three ways an instance stops, so it can neither run up a bill nor be cut off mid-run:
#   1. the run ends (success or failure): `finish` powers off at once;
#   2. a hard cap, MAX_MINUTES after boot, scheduled with shutdown(8) so it fires even if every
#      script here has died; the driver sets it to ~2.5-3x the expected duration, and
#      `compare-ec2.sh extend` moves it while the instance is up;
#   3. the idle watchdog in `monitor`: no CPU progress for IDLE_MINUTES means a hang, not a slow
#      run (every timed row keeps a core busy, and the harness's own per-run deadline is 15
#      minutes), so it powers off early instead of waiting out the cap.
# For troubleshooting, `compare-ec2.sh extend <run-id> <hours> --hold` (or, on the box,
# `touch /run/succinctly-compare-hold`) pauses the watchdog; the hard cap still applies.
root_common() {
  # An image carries its bake's log; a run starts its own (the bake's is also $OPT/bake.log).
  if [ "$MODE" = run ] && [ -f "$LOG" ]; then mv "$LOG" "$LOG.image"; fi
  exec > >(tee -a "$LOG") 2>&1
  INSTANCE_ID="$(imds instance-id || echo unknown)"
  export INSTANCE_ID
  # set_cap cancels the image's own safety net (`succinctly-compare-safety-shutdown.service`,
  # 60 minutes after boot) and replaces it with this run's cap.
  set_cap $(( $(date +%s) + ${MAX_MINUTES:-480} * 60 )) 0 boot
  echo "== idle watchdog: ${IDLE_MINUTES:-60} minutes without CPU progress"
  monitor &
  MONITOR_PID=$!
}

set_cap() { # epoch hold-until-epoch request-id: (re)schedule the hard cap and publish it as cap.txt
  local mins=$(( ($1 - $(date +%s) + 59) / 60 ))
  [ "$mins" -ge 1 ] || mins=1
  shutdown -c 2>/dev/null || true
  shutdown -h "+$mins" "succinctly-compare hard cap" || true
  echo "== hard cap: power off at $(date -u -d "@$1" +%FT%TZ)"
  {
    echo "cap=$(date -u -d "@$1" +%FT%TZ)"
    if [ "$2" -gt 0 ]; then echo "watchdog_held_until=$(date -u -d "@$2" +%FT%TZ)"; fi
    echo "applied=$(date -u +%FT%TZ)"
    echo "request=$3" # what `extend` waits for: the id of the control.txt this answers
  } >/tmp/cap.txt
  aws s3 cp --only-show-errors /tmp/cap.txt "$S3/cap.txt" || true
}

busy_jiffies() { awk '/^cpu /{print $2 + $3 + $4 + $7 + $8; exit}' /proc/stat; } # user nice system irq softirq

monitor() { # every 5 minutes: upload the log, apply `compare-ec2.sh extend`, watch for a hang
  local interval=300 limit=$(( ${IDLE_MINUTES:-60} * 60 )) idle=0 prev now ctl applied="" cap hold=0 req
  # Busy means at least 15 CPU-seconds per 5 minutes (5% of one core): well above this loop's
  # own uploads, far below any timed row or build step.
  local threshold=$(( 15 * $(getconf CLK_TCK) ))
  prev="$(busy_jiffies)"
  while sleep "$interval"; do
    nice -n 19 aws s3 cp --only-show-errors "$LOG" "$S3/log.txt" || true
    ctl="$(nice -n 19 aws s3 cp --only-show-errors "$S3/control.txt" - 2>/dev/null || true)"
    if [ -n "$ctl" ] && [ "$ctl" != "$applied" ]; then
      applied="$ctl"
      cap="$(printf '%s\n' "$ctl" | sed -n 's/^cap_epoch=//p')"
      hold="$(printf '%s\n' "$ctl" | sed -n 's/^hold_until_epoch=//p')"
      req="$(printf '%s\n' "$ctl" | sed -n 's/^request=//p')"
      case "$hold" in '' | *[!0-9]*) hold=0 ;; esac
      case "$cap" in
      '' | *[!0-9]*) echo "== ignoring a malformed control.txt" ;;
      *) echo "== compare-ec2.sh extend ($req):"; set_cap "$cap" "$hold" "$req" ;;
      esac
    fi
    now="$(busy_jiffies)"
    if [ -e /run/succinctly-compare-hold ] || [ "$(date +%s)" -lt "$hold" ] || [ $(( now - prev )) -ge "$threshold" ]; then
      idle=0
    else
      idle=$(( idle + interval ))
    fi
    prev="$now"
    if [ "$idle" -ge "$limit" ]; then
      echo "== idle watchdog: no CPU progress for ${IDLE_MINUTES:-60} minutes; powering off"
      put_status idle-timeout
      push_log
      poweroff
    fi
  done
}

finish() { # trap: record the outcome, upload the log, power off unless asked to stay up
  local rc=$?
  set +e
  # A failed run kept up for troubleshooting keeps its monitor: log uploads, extend, watchdog.
  if [ "$rc" -ne 0 ] && [ "${KEEP_ON_FAILURE:-0}" = 1 ]; then :; else kill "${MONITOR_PID:-}" 2>/dev/null; fi
  if [ "$rc" -eq 0 ]; then put_status "done" 0; else put_status failed "$rc"; fi
  push_log
  if [ "$rc" -ne 0 ] && [ "${KEEP_ON_FAILURE:-0}" = 1 ]; then
    echo "== keeping the instance up for troubleshooting; the hard cap and the idle watchdog still apply (compare-ec2.sh extend <run-id> <hours> --hold)"
    exit "$rc"
  fi
  if command -v tailscale >/dev/null && tailscale status >/dev/null 2>&1; then tailscale logout || true; fi
  poweroff
}

maybe_tailscale() { # optional: join the tailnet with Tailscale SSH, for troubleshooting only
  [ -n "${TS_PARAM:-}" ] || return 0
  # Troubleshooting access must never cost the run: a key that cannot be read or is spent (a
  # single-use key used before) is a warning, and the run goes on reachable through SSM only.
  local key
  if ! key="$(aws ssm get-parameter --with-decryption --name "$TS_PARAM" --query Parameter.Value --output text)"; then
    echo "== WARNING: could not read $TS_PARAM; continuing without Tailscale"
    return 0
  fi
  systemctl enable --now tailscaled
  if timeout 120 tailscale up --auth-key="$key" --ssh --hostname="${TS_HOSTNAME:-succinctly-compare}"; then
    echo "== joined the tailnet as ${TS_HOSTNAME:-succinctly-compare}"
  else
    echo "== WARNING: tailscale up failed (an expired or already-used key?); continuing without Tailscale"
    systemctl disable --now tailscaled || true
  fi
  key=""
}

quiet_machine() { # stop periodic jobs that would run during the timed rows
  systemctl stop dnf-makecache.timer 2>/dev/null || true
  systemctl stop update-motd.timer 2>/dev/null || true
}

bake_root() {
  root_common
  trap finish EXIT
  put_status provisioning
  dnf -y install gcc make git valgrind perf patch tar gzip xz python3 procps-ng util-linux

  # Tailscale is installed but left off and stateless: an image must never carry a node
  # identity, or every instance launched from it would claim to be the same node.
  curl -fsSL https://tailscale.com/install.sh | sh
  systemctl disable --now tailscaled
  rm -rf /var/lib/tailscale/*

  install -d "$OPT"
  chown "$USER_NAME:$USER_NAME" "$OPT" # the bootstrap created it as root for the tooling
  cat >/etc/systemd/system/succinctly-compare-safety-shutdown.service <<'UNIT'
[Unit]
Description=Power off 60 minutes after boot unless a benchmark run takes over (cost safety net)
After=network.target
Before=cloud-final.service

[Service]
Type=oneshot
ExecStart=/sbin/shutdown -h +60 "succinctly-compare safety net: nothing claimed this instance within 60 minutes of boot"

[Install]
WantedBy=multi-user.target
UNIT
  systemctl enable succinctly-compare-safety-shutdown.service

  put_status building-references
  as_user bake-user

  # Leave nothing instance-specific in the image.
  dnf clean all
  rm -f "$USER_HOME/.ssh/authorized_keys" /root/.ssh/authorized_keys
  rm -rf /etc/succinctly-compare
  truncate -s 0 /etc/machine-id
  cloud-init clean --logs
  cp "$LOG" "$OPT/bake.log"
}

run_root() {
  root_common
  trap finish EXIT
  put_status starting
  quiet_machine
  maybe_tailscale
  put_status building
  as_user run-user
  put_status uploading
  tar -C "$USER_HOME/work" -czf /tmp/results.tar.gz out
  aws s3 cp --only-show-errors /tmp/results.tar.gz "$S3/results.tar.gz"
  local rc
  rc="$(cat "$USER_HOME/work/out/compare-exit-code")"
  return "$rc"
}

# ---------------------------------------------------------------------------------------------
# User phases (ec2-user)
# ---------------------------------------------------------------------------------------------

install_rust() {
  local tc="${RUST_TOOLCHAIN:-stable}"
  if ! command -v rustup >/dev/null; then
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$tc"
    # shellcheck source=/dev/null
    . "$HOME/.cargo/env"
  fi
  rustup toolchain install "$tc" --profile minimal
  rustup default "$tc"
  rustc -vV
}

clone_at() { # url ref dir
  rm -rf "$3"
  git clone --quiet --filter=blob:none "$1" "$3"
  git -C "$3" checkout --quiet --detach "$2"
  echo "== $3 at $(git -C "$3" rev-parse HEAD)"
}

manifest_versions() { # the jq versions comparison/jq/manifest.toml pins
  awk '/^\[\[reference\]\]/{r=1} r && $1=="version"{gsub(/"/,"",$3); print $3; r=0}' "$1/comparison/jq/manifest.toml"
}

bake_user() {
  install_rust
  local src="$OPT/src"
  clone_at "$REPO_URL" "${BAKE_REF:-main}" "$src"
  # The pinned jq builds are hermetic and their sources sha256-pinned, so baking them keeps the
  # reference binary byte-identical across runs; a run whose manifest pins another version
  # builds that one itself (`--build-jq`).
  for v in $(manifest_versions "$src"); do
    COMPARE_JQ_DEST="$OPT/jq/jq-$v" "$src/scripts/compare-jq-build.sh" "$v"
    "$OPT/jq/jq-$v/bin/jq" --version
  done
  "$src/scripts/sync-bench-corpus.sh"
  rm -rf "$OPT/corpus"; cp -a "$src/data/bench/corpus" "$OPT/corpus"
  cp "$src/tests/data/bench-corpus/manifest.json" "$OPT/corpus.manifest.json"
  (cd "$src" && cargo fetch --locked) # warm the registry cache for the first run's build
  {
    echo "bake_ref=$(git -C "$src" rev-parse HEAD)"
    echo "baked_at=$(date -u +%FT%TZ)"
    echo "base_ami=$(imds ami-id)"
    echo "instance_type=$(imds instance-type)"
    rustc -V
  } >"$OPT/image.txt"
  rm -rf "$src"
}

run_user() {
  install_rust
  local work="$HOME/work" src="$HOME/work/src" out="$HOME/work/out"
  rm -rf "$work"; mkdir -p "$out"
  clone_at "$REPO_URL" "$COMMIT" "$src"
  [ "$(git -C "$src" rev-parse HEAD)" = "$COMMIT" ] || { echo "checkout is not $COMMIT" >&2; exit 1; }

  mkdir -p "$src/target/compare" "$src/data/bench"
  cp -a "$OPT/jq/." "$src/target/compare/"
  cp -a "$OPT/corpus" "$src/data/bench/corpus"
  if ! cmp -s "$OPT/corpus.manifest.json" "$src/tests/data/bench-corpus/manifest.json"; then
    echo "== the corpus manifest changed since the image was baked; syncing"
    "$src/scripts/sync-bench-corpus.sh"
  fi

  local features
  features="$(awk '/^\[succinctly\]/{s=1} s && $1=="features"{gsub(/"/,"",$3); print $3; exit}' "$src/comparison/jq/manifest.toml")"
  (cd "$src" && cargo build --release --locked --features "$features,bench-runner")

  write_provenance "$src" >"$out/provenance.txt"
  put_status comparing

  # Word-split COMPARE_ARGS on purpose: it is a flag list such as `--full --huge`.
  local rc=0
  # shellcheck disable=SC2086
  (cd "$src" && ./target/release/succinctly dev compare jq $COMPARE_ARGS --build-jq --out "$out/run") || rc=$?
  echo "$rc" >"$out/compare-exit-code"
  echo "== dev compare jq exited $rc"

  # --write-docs output, when the run asked for it and it was valid: only the per-platform files
  # are this machine's; the index and the blocks quoted elsewhere are re-rendered locally with
  # `dev compare jq --render-docs` once every platform's files are in place.
  mkdir -p "$out/docs"
  git -C "$src" status --porcelain --untracked-files=all >"$out/tree-changed.txt"
  # A run without --write-docs (or an invalid one) changes no docs: grep then matches nothing,
  # which must not fail the run under pipefail and lose its results.
  awk '{print $NF}' "$out/tree-changed.txt" |
    { grep -E '^docs/compare/jq-[a-z]+-[a-z0-9_]+\.(md|json)$' || true; } |
    while read -r f; do cp "$src/$f" "$out/docs/"; done
  # The harness's exit status is the run's; reaching here means collecting it worked.
  return 0
}

write_provenance() { # what the run was built and measured with
  echo "commit=$(git -C "$1" rev-parse HEAD)"
  echo "compare_args=$COMPARE_ARGS"
  echo "instance_type=$(imds instance-type)"
  echo "image=$(imds ami-id)"
  echo "kernel=$(uname -r)"
  grep -E '^(PRETTY_NAME|VERSION_ID)=' /etc/os-release
  cat "$OPT/image.txt"
  rustc -vV
  cargo -V
  gcc --version | head -1
  echo "nproc=$(nproc)"
  free -m | sed -n 2p
  echo "--- rpm -qa"
  rpm -qa | sort
}

case "${1:-}" in
bake) bake_root ;;
run) run_root ;;
bake-user) bake_user ;;
run-user) run_user ;;
*) echo "usage: $0 bake|run" >&2; exit 2 ;;
esac
