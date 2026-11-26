#!/bin/bash
# Spike switch-ab, build phase on terminus. Does a same-binary runtime switch A/B agree with a
# two-binary A/B, and does it remove the layout term? M = f884a5148 (the reader prototype's base),
# P = M + the prototype (design-input-reader 5884ba609, one diff), P0 = P run with
# SUCCINCTLY_READER_MODEL=0. Both linked through the layout-seeds spike's link.py with their own
# symbols.txt, saved, and relinked under 8 padding seeds. Each step under its own shared benchlock.
set -u -o pipefail
D=$HOME/wrk/bench-scratch/bughunt2/switch-ab
SH=$HOME/wrk/bench-scratch/bughunt2
CLONE=$HOME/wrk/rust-works/succinctly
SPIKE=$D/spike
LOCK=$HOME/wrk/bench-scratch/7260/benchlock.py
cd "$D" || exit 1
status() { echo "$(date -u +%FT%TZ) $*" >> "$D/STATUS"; }

step_fetch() {
  git -C "$CLONE" cat-file -e "f884a5148^{commit}" 2>/dev/null ||
    timeout 600 git -C "$CLONE" fetch -q https://github.com/rust-works/succinctly.git main || return 1
  for X in M P; do
    [ -d "$D/$X/src" ] && continue
    mkdir -p "$D/$X" && git -C "$CLONE" archive f884a5148 | tar -x -C "$D/$X" || return 1
  done
  [ -f "$D/P/.reader-applied" ] && return 0
  (cd "$D/P" && git apply --whitespace=nowarn "$D/reader.diff" && touch .reader-applied) || return 1
  grep -q SUCCINCTLY_READER_MODEL "$D/P/src/bin/succinctly/jq_runner/reader_model.rs" || return 1
}

step_build() {
  X=$1
  [ -x "$D/bins/$X.cargo/succinctly" ] && return 0
  while [ "$(free -g | awk '/^Mem:/{print $7}')" -lt 4 ]; do status "build $X waiting: under 4 GB available"; sleep 60; done
  ( cd "$D/$X" &&
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$SPIKE/scripts/link-order/cc" \
    SUCCINCTLY_LINK_ORDER_LIST="$D/$X/scripts/link-order/symbols.txt" \
    SUCCINCTLY_LINK_SAVE_DIR="$D/save-$X" \
    nice -n 5 cargo build --release --features cli,simd,regex,serde -j 12 ) > "$D/build-$X.log" 2>&1 || return 1
  [ -s "$D/save-$X/link.json" ] || { status "build $X: no saved link"; return 1; }
  mkdir -p "$D/bins/$X.cargo" && cp "$D/$X/target/release/succinctly" "$D/bins/$X.cargo/succinctly" || return 1
  status "build $X: $(find "$D/$X/target/release" -name 'succinctly*.link-order.log' | head -1 | xargs -r cat | tr '\n' ' ')"
}

step_relinks() {
  R="$SPIKE/scripts/link-order/relink.py"
  for X in M P; do
    python3 "$R" "$D/save-$X" "$D/bins/$X/succinctly" > "$D/relink-$X.log" 2>&1 || return 1
    cmp -s "$D/bins/$X/succinctly" "$D/bins/$X.cargo/succinctly" &&
      status "relink control $X: byte-identical" || status "relink control $X: differs from cargo's binary"
    for s in 1 2 3 4 5 6 7 8; do
      python3 "$R" "$D/save-$X" "$D/bins/$X.s$s/succinctly" --seed $s --mode pad >> "$D/relink-$X.log" 2>&1 || return 1
    done
  done
}

# Inputs, then a probe: which inputs make the model run (its trace), and whether P and P0 answer alike.
step_probe() {
  G="$D/bins/M/succinctly"; I="$D/inputs"; mkdir -p "$I"
  for spec in users:users ndjson:ndjson strings:strings; do
    id=${spec%%:*}; pat=${spec#*:}
    for sz in 1mb 10mb; do
      [ -s "$I/$id-$sz.json" ] || "$G" json generate $sz -p $pat -s 1 -o "$I/$id-$sz.json" > /dev/null || return 1
    done
  done
  for sz in 1mb 10mb; do   # trigger candidates
    { cat "$I/ndjson-$sz.json"; printf '\0'; } > "$I/ndjson-nul-eof-$sz.json"
    python3 - "$I/ndjson-$sz.json" "$I/ndjson-nul-mid-$sz.json" <<'EOF'
import sys
b = open(sys.argv[1], "rb").read(); i = b.index(b"\n", len(b) // 2) + 1
open(sys.argv[2], "wb").write(b[:i] + b"\0" + b[i:])
EOF
    { printf '\xef'; cat "$I/ndjson-$sz.json"; } > "$I/ndjson-ef-$sz.json"
  done
  P="$D/bins/P/succinctly"
  for f in users-1mb ndjson-1mb ndjson-nul-eof-1mb ndjson-nul-mid-1mb ndjson-ef-1mb; do
    for q in . ; do
      a=$(SUCCINCTLY_READER_MODEL_TRACE=1 "$P" jq -c "$q" "$I/$f.json" 2>"$D/trace-$f.txt" | sha256sum | cut -c1-12; echo "rc=${PIPESTATUS[0]}")
      b=$(SUCCINCTLY_READER_MODEL=0 "$P" jq -c "$q" "$I/$f.json" 2>/dev/null | sha256sum | cut -c1-12; echo "rc=${PIPESTATUS[0]}")
      m=$("$G" jq -c "$q" "$I/$f.json" 2>/dev/null | sha256sum | cut -c1-12; echo "rc=${PIPESTATUS[0]}")
      status "probe $f '$q': P $(echo $a) P0 $(echo $b) M $(echo $m) trace_lines=$(wc -l < "$D/trace-$f.txt")"
    done
  done
  for f in users-1mb ndjson-1mb ndjson-nul-mid-1mb; do
    for arm in M P P0; do
      bin="$D/bins/${arm%0}/succinctly"; envv=; [ $arm = P0 ] && envv="SUCCINCTLY_READER_MODEL=0"
      env $envv valgrind --tool=cachegrind --cache-sim=no --cachegrind-out-file=/dev/null "$bin" jq -c . "$I/$f.json" \
        > /dev/null 2> "$D/cg-$arm-$f.txt"
      status "Ir $arm jq -c . $f: $(grep -o 'I *refs: *[0-9,]*' "$D/cg-$arm-$f.txt")"
    done
  done
}

step_clean() { for X in M P; do (cd "$D/$X" && cargo clean -q) || return 1; done; }

cap_of() { case $1 in fetch) echo 900 ;; build-*) echo 1800 ;; relinks) echo 900 ;; probe) echo 1200 ;; *) echo 600 ;; esac; }

if [ "${1:-}" = step ]; then
  prlimit --pid $$ --core=1:1
  case $2 in
    fetch) step_fetch ;; build-M) step_build M ;; build-P) step_build P ;;
    relinks) step_relinks ;; probe) step_probe ;; clean) step_clean ;;
    *) echo "unknown step $2"; exit 2 ;;
  esac
  exit $?
fi

prlimit --pid $$ --core=1:1
(while sleep 30; do touch "$D/HEARTBEAT"; done) & HB=$!
touch "$D/HEARTBEAT"
status "driver start, pid $$"
# One of my builds at a time on terminus: wait while layout-seeds' build steps hold the lock.
other_build() { ps -eo args | grep -E '[b]enchlock\.py --shared layout-seeds_build-' | head -1; }
for st in fetch build-M build-P relinks probe clean; do
  case $st in build-*)
    while [ -n "$(other_build)" ]; do status "step $st waiting for layout-seeds' build"; sleep 60; done ;;
  esac
  status "step $st start (cap $(cap_of $st)s)"
  python3 "$D/cap.py" "$(cap_of $st)" python3 "$LOCK" --shared "switch-ab_$st" bash "$D/build.sh" step "$st" > "$D/step-$st.log" 2>&1
  rc=$?
  echo "$(date -u +%FT%TZ) switch-ab finished rc=$rc $st du=$(du -sh "$D" | cut -f1)" >> "$SH/STATUS"
  status "step $st rc=$rc"
  if [ $rc -ne 0 ]; then
    echo "$(date -u +%FT%TZ) switch-ab left: FAILED $st" >> "$SH/DONE"
    echo "FAILED $st" > "$D/DONE"; kill "$HB"; exit 1
  fi
done
echo "$(date -u +%FT%TZ) switch-ab left: READY" >> "$SH/DONE"
echo READY > "$D/DONE"
status "READY for timing"
kill "$HB"
