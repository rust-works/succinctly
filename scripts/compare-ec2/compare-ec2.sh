#!/usr/bin/env bash
# Run `succinctly dev compare jq` on a fresh EC2 instance, unattended, from a baked image.
#
#   compare-ec2.sh setup                  create the S3 bucket, IAM role/instance profile and a
#                                         security group with no inbound rules (idempotent)
#   compare-ec2.sh bake [options]         provision a stock Amazon Linux 2023 instance and save it
#                                         as the benchmark image
#   compare-ec2.sh run [options]          launch an instance from the image, run the comparison at
#                                         a commit, wait, and download the results
#   compare-ec2.sh wait|fetch|shell|cancel <run-id>
#   compare-ec2.sh extend <run-id> <hours> [--hold]   move the hard cap to <hours> from now;
#                                         --hold also pauses the idle watchdog until then
#   compare-ec2.sh status                 instances and recent runs
#   compare-ec2.sh images [--prune N]     list images; keep only the newest N per architecture
#   compare-ec2.sh teardown --yes [--delete-bucket]
#   compare-ec2.sh operator-policy        print the IAM policy the operating user needs
#
# Configuration: ${COMPARE_EC2_CONFIG:-~/.config/succinctly/compare-ec2.env}, a shell file of
# assignments; scripts/compare-ec2/compare-ec2.env.example lists them. Nothing account-specific
# lives in the repository. The full procedure is docs/guides/reproducing-the-jq-comparison.md.
#
# Requires: bash, the AWS CLI v2 with credentials, git; the Session Manager plugin only for
# `shell`.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
CONFIG="${COMPARE_EC2_CONFIG:-${XDG_CONFIG_HOME:-$HOME/.config}/succinctly/compare-ec2.env}"

die() { echo "error: $*" >&2; exit 1; }
note() { echo "== $*" >&2; }

# Retry a command through a transient failure (a laptop waking up, a dropped network): every
# long wait here only polls AWS, so it can stop and resume at any point without harming a run.
retry() {
  local n=0
  until "$@"; do
    n=$(( n + 1 ))
    [ "$n" -lt 10 ] || return 1
    sleep $(( n * 10 ))
  done
}

[ -f "$CONFIG" ] || die "no configuration at $CONFIG (copy $HERE/compare-ec2.env.example there)"
# shellcheck source=/dev/null
. "$CONFIG"
: "${COMPARE_REGION:?set COMPARE_REGION in $CONFIG}"
: "${COMPARE_BUCKET:?set COMPARE_BUCKET in $CONFIG}"
NAME="${COMPARE_NAME:-succinctly-compare}"
REPO_URL="${COMPARE_REPO_URL:-https://github.com/rust-works/succinctly.git}"
DEFAULT_TYPE="${COMPARE_INSTANCE_TYPE:-c8g.2xlarge}"
ROOT_GB="${COMPARE_ROOT_GB:-40}"
[ -n "${AWS_PROFILE:-}" ] && export AWS_PROFILE

aws() { command aws --region "$COMPARE_REGION" "$@"; }
s3url() { echo "s3://$COMPARE_BUCKET/$1"; }
param_path() { echo "/$NAME/$1/tailscale-authkey"; }

# ---------------------------------------------------------------------------------------------
# Lookups
# ---------------------------------------------------------------------------------------------

instance_arch() { # instance type -> arm64 | x86_64
  aws ec2 describe-instance-types --instance-types "$1" \
    --query 'InstanceTypes[0].ProcessorInfo.SupportedArchitectures[0]' --output text
}

instance_mem_mib() {
  aws ec2 describe-instance-types --instance-types "$1" --query 'InstanceTypes[0].MemoryInfo.SizeInMiB' --output text
}

pick_subnet() { # COMPARE_SUBNET, or a default subnet in an AZ that offers the instance type
  if [ -n "${COMPARE_SUBNET:-}" ]; then echo "$COMPARE_SUBNET"; return; fi
  local azs
  azs="$(aws ec2 describe-instance-type-offerings --location-type availability-zone \
    --filters "Name=instance-type,Values=$1" --query 'InstanceTypeOfferings[].Location' --output text | tr '\t' ',')"
  [ -n "$azs" ] || die "$1 is not offered in $COMPARE_REGION"
  local subnet
  subnet="$(aws ec2 describe-subnets --filters Name=default-for-az,Values=true "Name=availability-zone,Values=$azs" \
    --query 'Subnets[0].SubnetId' --output text)"
  [ "$subnet" != None ] || die "no default subnet in $azs; set COMPARE_SUBNET to a public subnet"
  echo "$subnet"
}

vpc_of_subnet() { aws ec2 describe-subnets --subnet-ids "$1" --query 'Subnets[0].VpcId' --output text; }

security_group() { # the run's security group in the VPC of subnet $1
  local sg
  sg="$(aws ec2 describe-security-groups --filters "Name=group-name,Values=$NAME" "Name=vpc-id,Values=$(vpc_of_subnet "$1")" \
    --query 'SecurityGroups[0].GroupId' --output text)"
  [ "$sg" != None ] || die "no security group $NAME in that VPC; run '$0 setup'"
  echo "$sg"
}

latest_image() { # newest available image this pipeline baked for an architecture
  local ami
  ami="$(aws ec2 describe-images --owners self \
    --filters "Name=tag:Purpose,Values=$NAME" "Name=architecture,Values=$1" Name=state,Values=available \
    --query 'sort_by(Images,&CreationDate)[-1].ImageId' --output text)"
  [ "$ami" != None ] || die "no $1 image baked yet; run '$0 bake --instance-type <a $1 type>'"
  echo "$ami"
}

instance_of_run() { # live (not terminated) instance tagged with the run id
  aws ec2 describe-instances --filters "Name=tag:RunId,Values=$1" \
    Name=instance-state-name,Values=pending,running,stopping,stopped,shutting-down \
    --query 'Reservations[].Instances[0].InstanceId | [0]' --output text
}

resolve_commit() { # a ref or sha that must exist on $REPO_URL -> full sha
  local ref="$1" sha
  if printf '%s' "$ref" | grep -Eq '^[0-9a-f]{40}$'; then echo "$ref"; return; fi
  sha="$(git ls-remote "$REPO_URL" "refs/heads/$ref" "refs/tags/$ref^{}" "refs/tags/$ref" | awk 'NR==1{print $1}')"
  if [ -z "$sha" ]; then
    sha="$(git -C "$REPO_ROOT" rev-parse --verify --quiet "$ref^{commit}" || true)"
    [ -n "$sha" ] || die "cannot resolve $ref"
    note "resolved $ref locally to $sha; the instance clones $REPO_URL, so it must be pushed there"
  fi
  echo "$sha"
}

# ---------------------------------------------------------------------------------------------
# Launch
# ---------------------------------------------------------------------------------------------

user_data() { # mode prefix assignments... -> launch script on stdout
  local mode="$1" prefix="$2"; shift 2
  echo '#!/bin/bash'
  echo 'set -euo pipefail'
  echo 'install -d -m 755 /etc/succinctly-compare /opt/succinctly-compare/tooling'
  echo "cat >/etc/succinctly-compare/params.env <<'PARAMS'"
  printf 'MODE=%q\nBUCKET=%q\nPREFIX=%q\nREGION=%q\nREPO_URL=%q\n' "$mode" "$COMPARE_BUCKET" "$prefix" "$COMPARE_REGION" "$REPO_URL"
  local kv
  for kv in "$@"; do printf '%s=%q\n' "${kv%%=*}" "${kv#*=}"; done
  echo 'PARAMS'
  # The instance profile can take a few seconds to reach a new instance.
  printf 'for i in 1 2 3 4 5 6 7 8 9 10; do aws --region %q s3 cp --only-show-errors %q /opt/succinctly-compare/tooling/instance.sh && break; sleep 10; done\n' \
    "$COMPARE_REGION" "$(s3url "$prefix/tooling/instance.sh")"
  echo 'chmod 755 /opt/succinctly-compare/tooling/instance.sh'
  echo "exec /opt/succinctly-compare/tooling/instance.sh $mode"
}

launch() { # id image type shutdown-behaviour user-data-file -> instance id
  local id="$1" ami="$2" type="$3" behaviour="$4" ud="$5" subnet sg rootdev
  subnet="$(pick_subnet "$type")"
  sg="$(security_group "$subnet")"
  rootdev="$(aws ec2 describe-images --image-ids "$ami" --query 'Images[0].RootDeviceName' --output text)"
  aws ec2 run-instances --image-id "$ami" --instance-type "$type" \
    --subnet-id "$subnet" --security-group-ids "$sg" --associate-public-ip-address \
    --iam-instance-profile "Name=$NAME" \
    --instance-initiated-shutdown-behavior "$behaviour" \
    --metadata-options HttpTokens=required,HttpEndpoint=enabled \
    --block-device-mappings "[{\"DeviceName\":\"$rootdev\",\"Ebs\":{\"VolumeSize\":$ROOT_GB,\"VolumeType\":\"gp3\",\"DeleteOnTermination\":true}}]" \
    --tag-specifications \
    "ResourceType=instance,Tags=[{Key=Name,Value=$NAME-$id},{Key=Purpose,Value=$NAME},{Key=RunId,Value=$id}]" \
    "ResourceType=volume,Tags=[{Key=Purpose,Value=$NAME},{Key=RunId,Value=$id}]" \
    --user-data "file://$ud" --query 'Instances[0].InstanceId' --output text
}

upload_tooling() { aws s3 cp --only-show-errors "$HERE/instance.sh" "$(s3url "$1/tooling/instance.sh")"; }

read_status() { aws s3 cp --only-show-errors "$(s3url "$1/status.json")" - 2>/dev/null || true; }

follow() { # prefix run-id: print state changes until an outcome, or until the instance is gone
  local prefix="$1" id="$2" last="" st state polls=0
  while :; do
    st="$(read_status "$prefix")"
    state="$(printf '%s' "$st" | sed -n 's/.*"state":"\([^"]*\)".*/\1/p')"
    if [ -n "$state" ] && [ "$state" != "$last" ]; then note "$(date -u +%H:%M:%SZ) $state"; last="$state"; fi
    case "$state" in done | failed | idle-timeout) echo "$state"; return 0 ;; esac
    # No outcome yet: the run is alive as long as its instance is. One that vanished without an
    # outcome hit its hard cap (or was terminated by hand); check again once for a late upload.
    polls=$(( polls + 1 ))
    if [ $(( polls % 5 )) -eq 0 ] && [ "$(instance_of_run "$id")" = None ]; then
      sleep 30
      st="$(read_status "$prefix")"
      state="$(printf '%s' "$st" | sed -n 's/.*"state":"\([^"]*\)".*/\1/p')"
      case "$state" in done | failed | idle-timeout) echo "$state"; return 0 ;; esac
      echo "instance-gone-during-${state:-startup}"; return 0
    fi
    sleep 60
  done
}

# ---------------------------------------------------------------------------------------------
# Commands
# ---------------------------------------------------------------------------------------------

cmd_setup() {
  local acct
  acct="$(aws sts get-caller-identity --query Account --output text)"
  if ! aws s3api head-bucket --bucket "$COMPARE_BUCKET" >/dev/null 2>&1; then
    note "creating bucket $COMPARE_BUCKET"
    if [ "$COMPARE_REGION" = us-east-1 ]; then
      aws s3api create-bucket --bucket "$COMPARE_BUCKET" >/dev/null
    else
      aws s3api create-bucket --bucket "$COMPARE_BUCKET" \
        --create-bucket-configuration "LocationConstraint=$COMPARE_REGION" >/dev/null
    fi
  fi
  aws s3api put-public-access-block --bucket "$COMPARE_BUCKET" --public-access-block-configuration \
    BlockPublicAcls=true,IgnorePublicAcls=true,BlockPublicPolicy=true,RestrictPublicBuckets=true

  if ! aws iam get-role --role-name "$NAME" >/dev/null 2>&1; then
    note "creating IAM role $NAME"
    aws iam create-role --role-name "$NAME" --description "succinctly dev compare jq benchmark instances" \
      --assume-role-policy-document '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"ec2.amazonaws.com"},"Action":"sts:AssumeRole"}]}' >/dev/null
  fi
  # Session Manager (the troubleshooting shell), the bucket, and this pipeline's parameters only.
  aws iam attach-role-policy --role-name "$NAME" --policy-arn arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore
  aws iam put-role-policy --role-name "$NAME" --policy-name "$NAME-access" --policy-document "$(cat <<JSON
{"Version":"2012-10-17","Statement":[
 {"Effect":"Allow","Action":"s3:ListBucket","Resource":"arn:aws:s3:::$COMPARE_BUCKET"},
 {"Effect":"Allow","Action":["s3:GetObject","s3:PutObject"],"Resource":"arn:aws:s3:::$COMPARE_BUCKET/*"},
 {"Effect":"Allow","Action":"ssm:GetParameter","Resource":"arn:aws:ssm:$COMPARE_REGION:$acct:parameter/$NAME/*"}
]}
JSON
)"
  if ! aws iam get-instance-profile --instance-profile-name "$NAME" >/dev/null 2>&1; then
    note "creating instance profile $NAME"
    aws iam create-instance-profile --instance-profile-name "$NAME" >/dev/null
    aws iam add-role-to-instance-profile --instance-profile-name "$NAME" --role-name "$NAME"
    sleep 15 # a new instance profile is not usable by run-instances for a few seconds
  fi

  local subnet vpc sg
  subnet="$(pick_subnet "$DEFAULT_TYPE")"
  vpc="$(vpc_of_subnet "$subnet")"
  sg="$(aws ec2 describe-security-groups --filters "Name=group-name,Values=$NAME" "Name=vpc-id,Values=$vpc" \
    --query 'SecurityGroups[0].GroupId' --output text)"
  if [ "$sg" = None ]; then
    note "creating security group $NAME (no inbound rules)"
    aws ec2 create-security-group --group-name "$NAME" --vpc-id "$vpc" \
      --description "succinctly compare: no inbound; reached through SSM or Tailscale" >/dev/null
  fi
  note "setup complete"
}

cmd_bake() {
  local type="$DEFAULT_TYPE" ref="main" keep=0 resume="" toolchain=stable
  while [ $# -gt 0 ]; do
    case "$1" in
    --instance-type) type="$2"; shift 2 ;;
    --ref) ref="$2"; shift 2 ;;
    --keep-on-failure) keep=1; shift ;;
    --resume) resume="$2"; shift 2 ;;
    --rust-toolchain) toolchain="$2"; shift 2 ;;
    *) die "bake: unknown option $1" ;;
    esac
  done
  if [ -n "$resume" ]; then bake_finish "$resume"; return; fi
  local arch base base_name sha stamp id ud iid
  arch="$(instance_arch "$type")"
  base="$(aws ssm get-parameter --name "/aws/service/ami-amazon-linux-latest/al2023-ami-kernel-default-$arch" \
    --query Parameter.Value --output text)"
  base_name="$(aws ec2 describe-images --image-ids "$base" --query 'Images[0].Name' --output text)"
  sha="$(resolve_commit "$ref")"
  stamp="$(date -u +%Y%m%d-%H%M%S)"
  id="bake-$stamp-$arch"
  note "baking $arch on $type from $base_name, reference builds from $sha"
  upload_tooling "bakes/$id"
  # Everything bake_finish needs, so `bake --resume` can finish a bake this machine stopped
  # following (it slept, lost the network, or the shell was closed).
  printf 'arch=%s\nbase_name=%s\nsha=%s\nstamp=%s\nkeep=%s\n' "$arch" "$base_name" "$sha" "$stamp" "$keep" |
    aws s3 cp --only-show-errors - "$(s3url "bakes/$id/bake.env")"
  ud="$(mktemp)"
  user_data bake "bakes/$id" "BAKE_REF=$sha" "MAX_MINUTES=120" "IDLE_MINUTES=30" "KEEP_ON_FAILURE=$keep" \
    "RUST_TOOLCHAIN=$toolchain" >"$ud"
  iid="$(launch "$id" "$base" "$type" stop "$ud")"
  rm -f "$ud"
  note "bake $id: instance $iid (if this machine sleeps or loses the network, finish with: $0 bake --resume $id)"
  bake_finish "$id"
}

bake_finish() { # bake-id: wait for the bake, save the stopped instance as an image, terminate it
  local id="$1" env arch base_name sha stamp keep iid outcome ami
  env="$(retry aws s3 cp --only-show-errors "$(s3url "bakes/$id/bake.env")" -)" || die "no bake $id"
  arch="$(printf '%s\n' "$env" | sed -n 's/^arch=//p')"
  base_name="$(printf '%s\n' "$env" | sed -n 's/^base_name=//p')"
  sha="$(printf '%s\n' "$env" | sed -n 's/^sha=//p')"
  stamp="$(printf '%s\n' "$env" | sed -n 's/^stamp=//p')"
  keep="$(printf '%s\n' "$env" | sed -n 's/^keep=//p')"
  ami="$(retry aws ec2 describe-images --owners self --filters "Name=name,Values=$NAME-$arch-$stamp" \
    --query 'Images[0].ImageId' --output text)"
  if [ "$ami" = None ]; then
    iid="$(retry instance_of_run "$id")"
    [ "$iid" != None ] || die "bake $id has no image and no instance left"
    outcome="$(follow "bakes/$id" "$id")"
    if [ "$outcome" != "done" ]; then
      aws s3 cp --only-show-errors "$(s3url "bakes/$id/log.txt")" - 2>/dev/null | tail -40 >&2 || true
      [ "$keep" = 1 ] || retry aws ec2 terminate-instances --instance-ids "$iid" >/dev/null
      die "bake $outcome (log: $(s3url "bakes/$id/log.txt"))"
    fi
    until [ "$(aws ec2 describe-instances --instance-ids "$iid" --query 'Reservations[0].Instances[0].State.Name' --output text 2>/dev/null)" = stopped ]; do sleep 15; done
    ami="$(retry aws ec2 create-image --instance-id "$iid" --name "$NAME-$arch-$stamp" \
      --description "succinctly dev compare jq image; base $base_name; references from $sha" \
      --tag-specifications \
      "ResourceType=image,Tags=[{Key=Purpose,Value=$NAME},{Key=BaseImage,Value=$base_name},{Key=BakeRef,Value=$sha}]" \
      "ResourceType=snapshot,Tags=[{Key=Purpose,Value=$NAME}]" --query ImageId --output text)"
  fi
  note "image $ami registering (several minutes)"
  local state=""
  until [ "$state" = available ]; do
    state="$(aws ec2 describe-images --image-ids "$ami" --query 'Images[0].State' --output text 2>/dev/null || true)"
    [ "$state" != failed ] || die "image $ami failed to register"
    [ "$state" = available ] || sleep 30
  done
  iid="$(retry instance_of_run "$id")"
  [ "$iid" = None ] || retry aws ec2 terminate-instances --instance-ids "$iid" >/dev/null
  note "image $ami ready; bake instance terminated"
}

cmd_run() {
  local type="$DEFAULT_TYPE" ref="main" args="--full --huge --write-docs" ts_key="" ts_host="" keep=0 hours="" idle=60 wait=1 ami="" toolchain=stable
  while [ $# -gt 0 ]; do
    case "$1" in
    --commit) ref="$2"; shift 2 ;;
    --instance-type) type="$2"; shift 2 ;;
    --args) args="$2"; shift 2 ;;
    --tailscale-key-file) ts_key="$2"; shift 2 ;;
    --tailscale-hostname) ts_host="$2"; shift 2 ;;
    --keep-on-failure) keep=1; shift ;;
    --max-hours) hours="$2"; shift 2 ;;
    --idle-minutes) idle="$2"; shift 2 ;;
    --rust-toolchain) toolchain="$2"; shift 2 ;;
    --image) ami="$2"; shift 2 ;;
    --no-wait) wait=0; shift ;;
    *) die "run: unknown option $1" ;;
    esac
  done
  # Hard cap: ~2.5-3x the expected duration (--full is ~3 h on a Graviton4, --huge adds ~1 h, plus
  # ~15 min of setup). A run that ends powers off at once, so the padding only costs anything if
  # the run is still busy at the expected time; a hung run is caught by the idle watchdog.
  if [ -z "$hours" ]; then
    case " $args " in
    *" --huge "*) hours=12 ;;
    *" --full "*) hours=8 ;;
    *) hours=3 ;;
    esac
  fi
  local arch sha id ud iid mem
  arch="$(instance_arch "$type")"
  mem="$(instance_mem_mib "$type")"
  case " $args " in *" --huge "*)
    [ "$mem" -ge 15000 ] || die "$type has $mem MiB; the 1 GB tier needs ~16 GiB (jq holds a 1 GB document at ~9-11 GB)" ;;
  esac
  [ -n "$ami" ] || ami="$(latest_image "$arch")"
  sha="$(resolve_commit "$ref")"
  id="run-$(date -u +%Y%m%d-%H%M%S)-$arch-$(printf '%s' "$sha" | cut -c1-9)"
  local extra=()
  if [ -n "$ts_key" ]; then
    [ -f "$ts_key" ] || die "no file $ts_key"
    aws ssm put-parameter --name "$(param_path "$id")" --type SecureString --value "file://$ts_key" >/dev/null
    extra+=("TS_PARAM=$(param_path "$id")" "TS_HOSTNAME=${ts_host:-$NAME-$arch}")
    note "Tailscale enabled: the instance joins as ${ts_host:-$NAME-$arch}"
  fi
  note "run $id: $sha on $type ($arch, image $ami), dev compare jq $args"
  note "power-off: when the run ends; at the latest after $hours h (hard cap) or $idle min without CPU progress"
  upload_tooling "runs/$id"
  ud="$(mktemp)"
  user_data run "runs/$id" "COMMIT=$sha" "COMPARE_ARGS=$args" "MAX_MINUTES=$(( hours * 60 ))" \
    "IDLE_MINUTES=$idle" "KEEP_ON_FAILURE=$keep" "RUST_TOOLCHAIN=$toolchain" ${extra[@]+"${extra[@]}"} >"$ud"
  iid="$(launch "$id" "$ami" "$type" terminate "$ud")"
  rm -f "$ud"
  note "instance $iid launched; it terminates itself when the run ends"
  note "change the cap with: $0 extend $id <hours-from-now> [--hold]"
  if [ "$wait" = 1 ]; then cmd_wait "$id"; else note "follow with: $0 wait $id"; fi
}

cmd_wait() {
  local id="${1:?run id}" outcome
  outcome="$(follow "runs/$id" "$id")"
  note "run $id: $outcome"
  cmd_fetch "$id"
  [ "$outcome" = "done" ]
}

cmd_fetch() {
  local id="${1:?run id}" dir
  dir="$REPO_ROOT/data/compare/ec2/$id"
  mkdir -p "$dir"
  retry aws s3 sync --only-show-errors "$(s3url "runs/$id")" "$dir" --exclude 'tooling/*'
  [ ! -f "$dir/results.tar.gz" ] || tar -xzf "$dir/results.tar.gz" -C "$dir"
  aws ssm delete-parameter --name "$(param_path "$id")" >/dev/null 2>&1 || true
  note "downloaded to $dir"
  [ -f "$dir/status.json" ] && cat "$dir/status.json" >&2
  if [ -d "$dir/out/docs" ] && [ -n "$(ls -A "$dir/out/docs" 2>/dev/null)" ]; then
    note "per-platform docs written by --write-docs:"
    ls "$dir/out/docs" >&2
    note "apply: cp $dir/out/docs/* docs/compare/ && ./target/release/succinctly dev compare jq --render-docs"
  fi
}

cmd_extend() { # run-id hours [--hold]: set the hard cap to <hours> from now
  local id="${1:?run id}" hours="${2:?hours from now}" hold=0 cap now req
  [ "${3:-}" = --hold ] && hold=1
  printf '%s' "$hours" | grep -Eq '^[0-9]+([.][0-9]+)?$' || die "hours must be a number"
  [ "$(instance_of_run "$id")" != None ] || die "no live instance for $id"
  now="$(date +%s)"
  req="$now-$$"
  cap=$(( now + $(awk -v h="$hours" 'BEGIN{printf "%d", h * 3600}') ))
  local prefix=runs
  case "$id" in bake-*) prefix=bakes ;; esac
  {
    echo "cap_epoch=$cap"
    if [ "$hold" = 1 ]; then echo "hold_until_epoch=$cap"; fi
    echo "request=$req"
  } | aws s3 cp --only-show-errors - "$(s3url "$prefix/$id/control.txt")"
  note "requested: power off at the latest $hours h from now$([ "$hold" = 1 ] && echo ', idle watchdog paused until then')"
  note "the instance applies it within 5 minutes; waiting for its acknowledgement (cap.txt)"
  local applied
  for _ in 1 2 3 4 5 6 7; do
    sleep 60
    applied="$(aws s3 cp --only-show-errors "$(s3url "$prefix/$id/cap.txt")" - 2>/dev/null || true)"
    if printf '%s\n' "$applied" | grep -qx "request=$req"; then printf '%s\n' "$applied" >&2; return 0; fi
  done
  die "no acknowledgement after 7 minutes; check '$0 status' and the run's log"
}

cmd_shell() {
  local id="${1:?run id}" iid
  iid="$(instance_of_run "$id")"
  [ "$iid" != None ] || die "no live instance for $id"
  command -v session-manager-plugin >/dev/null || die "install the AWS Session Manager plugin, or use Tailscale SSH"
  aws ssm start-session --target "$iid"
}

cmd_cancel() {
  local id="${1:?run id}" iid
  iid="$(instance_of_run "$id")"
  [ "$iid" = None ] || aws ec2 terminate-instances --instance-ids "$iid" >/dev/null
  aws ssm delete-parameter --name "$(param_path "$id")" >/dev/null 2>&1 || true
  note "cancelled $id"
}

cmd_status() { # shellcheck disable=SC2016 # JMESPath backticks, not shell
  aws ec2 describe-instances --filters "Name=tag:Purpose,Values=$NAME" \
    Name=instance-state-name,Values=pending,running,stopping,stopped,shutting-down \
    --query 'Reservations[].Instances[].[Tags[?Key==`RunId`]|[0].Value,InstanceType,State.Name,LaunchTime]' --output table
  local p
  for p in $(aws s3 ls "$(s3url runs/)" | awk '{print $2}' | sed 's#/$##' | tail -10); do
    printf '%-48s %s %s\n' "$p" "$(read_status "runs/$p")" \
      "$(aws s3 cp --only-show-errors "$(s3url "runs/$p/cap.txt")" - 2>/dev/null | sed -n 's/^cap=/cap /p')"
  done
}

cmd_images() {
  local keep=""
  [ "${1:-}" = --prune ] && keep="${2:?--prune N}"
  aws ec2 describe-images --owners self --filters "Name=tag:Purpose,Values=$NAME" \
    --query 'sort_by(Images,&CreationDate)[].[ImageId,Architecture,CreationDate,Name]' --output table
  [ -n "$keep" ] || return 0
  local arch ami snaps
  for arch in arm64 x86_64; do
    for ami in $(aws ec2 describe-images --owners self --filters "Name=tag:Purpose,Values=$NAME" "Name=architecture,Values=$arch" \
      --query 'reverse(sort_by(Images,&CreationDate))[].ImageId' --output text | tr '\t' '\n' | tail -n "+$(( keep + 1 ))"); do
      snaps="$(aws ec2 describe-images --image-ids "$ami" --query 'Images[0].BlockDeviceMappings[].Ebs.SnapshotId' --output text)"
      aws ec2 deregister-image --image-id "$ami"
      for s in $snaps; do aws ec2 delete-snapshot --snapshot-id "$s"; done
      note "deleted $ami and its snapshots"
    done
  done
}

cmd_teardown() {
  local yes=0 bucket=0
  while [ $# -gt 0 ]; do
    case "$1" in --yes) yes=1 ;; --delete-bucket) bucket=1 ;; *) die "teardown: unknown option $1" ;; esac
    shift
  done
  [ "$yes" = 1 ] || die "teardown deletes the role, instance profile and security group; pass --yes"
  aws iam remove-role-from-instance-profile --instance-profile-name "$NAME" --role-name "$NAME" 2>/dev/null || true
  aws iam delete-instance-profile --instance-profile-name "$NAME" 2>/dev/null || true
  aws iam delete-role-policy --role-name "$NAME" --policy-name "$NAME-access" 2>/dev/null || true
  aws iam detach-role-policy --role-name "$NAME" --policy-arn arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore 2>/dev/null || true
  aws iam delete-role --role-name "$NAME" 2>/dev/null || true
  local sg
  for sg in $(aws ec2 describe-security-groups --filters "Name=group-name,Values=$NAME" --query 'SecurityGroups[].GroupId' --output text); do
    aws ec2 delete-security-group --group-id "$sg"
  done
  [ "$bucket" = 0 ] || aws s3 rb --force "s3://$COMPARE_BUCKET"
  note "teardown complete (images are kept; '$0 images --prune 0' deletes them)"
}

cmd_operator_policy() { # least privilege for the user (or role) that runs this script
  local acct
  acct="$(aws sts get-caller-identity --query Account --output text)"
  cat <<JSON
{
  "Version": "2012-10-17",
  "Statement": [
    {"Sid": "Ec2", "Effect": "Allow", "Resource": "*", "Action": [
      "ec2:RunInstances", "ec2:TerminateInstances", "ec2:CreateTags", "ec2:CreateImage",
      "ec2:DeregisterImage", "ec2:DeleteSnapshot", "ec2:CreateSecurityGroup",
      "ec2:DeleteSecurityGroup", "ec2:DescribeInstances", "ec2:DescribeImages",
      "ec2:DescribeInstanceTypes", "ec2:DescribeInstanceTypeOfferings", "ec2:DescribeSubnets",
      "ec2:DescribeSecurityGroups"]},
    {"Sid": "Bucket", "Effect": "Allow", "Resource": "arn:aws:s3:::$COMPARE_BUCKET", "Action": [
      "s3:CreateBucket", "s3:PutBucketPublicAccessBlock", "s3:ListBucket", "s3:DeleteBucket"]},
    {"Sid": "Objects", "Effect": "Allow", "Resource": "arn:aws:s3:::$COMPARE_BUCKET/*",
     "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"]},
    {"Sid": "InstanceRole", "Effect": "Allow",
     "Resource": ["arn:aws:iam::$acct:role/$NAME", "arn:aws:iam::$acct:instance-profile/$NAME"], "Action": [
      "iam:GetRole", "iam:CreateRole", "iam:DeleteRole", "iam:AttachRolePolicy",
      "iam:DetachRolePolicy", "iam:PutRolePolicy", "iam:DeleteRolePolicy",
      "iam:GetInstanceProfile", "iam:CreateInstanceProfile", "iam:DeleteInstanceProfile",
      "iam:AddRoleToInstanceProfile", "iam:RemoveRoleFromInstanceProfile"]},
    {"Sid": "PassInstanceRole", "Effect": "Allow", "Action": "iam:PassRole",
     "Resource": "arn:aws:iam::$acct:role/$NAME",
     "Condition": {"StringEquals": {"iam:PassedToService": "ec2.amazonaws.com"}}},
    {"Sid": "PublicParameters", "Effect": "Allow", "Action": "ssm:GetParameter",
     "Resource": "arn:aws:ssm:*::parameter/aws/service/*"},
    {"Sid": "TailscaleKeyParameters", "Effect": "Allow",
     "Action": ["ssm:PutParameter", "ssm:DeleteParameter"],
     "Resource": "arn:aws:ssm:*:$acct:parameter/$NAME/*"},
    {"Sid": "SessionOnPipelineInstances", "Effect": "Allow", "Action": "ssm:StartSession",
     "Resource": "arn:aws:ec2:*:$acct:instance/*",
     "Condition": {"StringEquals": {"ssm:resourceTag/Purpose": "$NAME"}}},
    {"Sid": "SessionDocument", "Effect": "Allow", "Action": "ssm:StartSession",
     "Resource": "arn:aws:ssm:*:*:document/SSM-SessionManagerRunShell"},
    {"Sid": "OwnSessions", "Effect": "Allow", "Action": ["ssm:TerminateSession", "ssm:ResumeSession"],
     "Resource": "arn:aws:ssm:*:$acct:session/\${aws:username}-*"}
  ]
}
JSON
}

case "${1:-}" in
setup | bake | run | wait | fetch | extend | shell | cancel | status | images | teardown)
  cmd="$1"; shift; "cmd_$cmd" "$@" ;;
operator-policy) cmd_operator_policy ;;
*) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
