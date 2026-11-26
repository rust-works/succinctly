#!/bin/bash
# Spike layout-seeds, macOS build phase on johns-mac-mini (shuffle seeds only: ld64 has no padding). Driver: each step runs under its own shared
# benchlock hold (`benchlock.py --shared layout-seeds_<step> ...`), so an exclusive waiter can get
# in between steps. A = e43c3f9c4, B = 6362c4bc4 (#7933's pair), C = A plus a planted real cost in
# ascii_upcase (positive control). Each is linked through the spike's link.py with its own
# symbols.txt and the link saved; each is then relinked under 8 padding and 8 shuffle seeds.
# Usage: build.sh            (the driver)
#        build.sh step NAME  (one step; the driver calls this under benchlock)
set -u -o pipefail
D=$HOME/wrk/bench-scratch/bughunt2/layout-seeds
SH=$HOME/wrk/bench-scratch/bughunt2          # shared STATUS / DONE logs
CLONE=$HOME/wrk/rust-works/succinctly
SPIKE=$D/spike
LOCK=$HOME/wrk/bench-scratch/7260/benchlock.py
cd "$D" || exit 1
status() { echo "$(date -u +%FT%TZ) $*" >> "$D/STATUS"; }

sha_of() { case $1 in A|C) echo e43c3f9c4 ;; B) echo 6362c4bc4 ;; esac; }

step_fetch() {
  for c in e43c3f9c4 6362c4bc4; do
    git -C "$CLONE" cat-file -e "$c^{commit}" 2>/dev/null ||
      git -C "$CLONE" fetch -q https://github.com/rust-works/succinctly.git main || return 1
  done
  for X in A B C; do
    [ -d "$D/$X/src" ] && continue
    mkdir -p "$D/$X" && git -C "$CLONE" archive "$(sha_of $X)" | tar -x -C "$D/$X" || return 1
  done
  grep -q "spike positive control" "$D/C/src/jq/eval.rs" && return 0
  python3 - "$D/C/src/jq/eval.rs" <<'EOF'
import sys
p = sys.argv[1]; s = open(p).read()
old = "                let uppered: String = cow.chars().map(|c| c.to_ascii_uppercase()).collect();\n"
assert s.count(old) == 1, s.count(old)
new = old + ("                // spike positive control: one extra pass over the result\n"
             "                core::hint::black_box(uppered.bytes().fold(0u32, |a, b| a.rotate_left(5) ^ b as u32));\n")
open(p, "w").write(s.replace(old, new))
EOF
}

step_build() {
  X=$1
  [ -x "$D/bins/$X.cargo/succinctly" ] && return 0
  ( cd "$D/$X" &&
    CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER="$SPIKE/scripts/link-order/cc" \
    SUCCINCTLY_LINK_ORDER_LIST="$D/$X/scripts/link-order/symbols.txt" \
    SUCCINCTLY_LINK_SAVE_DIR="$D/save-$X" \
    nice -n 5 cargo build --release --features cli,simd,regex,serde -j 6 ) > "$D/build-$X.log" 2>&1 || return 1
  [ -s "$D/save-$X/link.json" ] || { status "build $X: no saved link (wrapper not used?)"; return 1; }
  mkdir -p "$D/bins/$X.cargo" && cp "$D/$X/target/release/succinctly" "$D/bins/$X.cargo/succinctly" || return 1
  status "build $X: $(find "$D/$X/target/release" -name 'succinctly*.link-order.log' | head -1 | xargs -r cat | tr '\n' ' ')"
}

step_relinks() {
  R="$SPIKE/scripts/link-order/relink.py"
  for X in A B C; do
    python3 "$R" "$D/save-$X" "$D/bins/$X/succinctly" > "$D/relink-$X.log" 2>&1 || return 1
    if cmp -s "$D/bins/$X/succinctly" "$D/bins/$X.cargo/succinctly"; then
      status "relink control $X: byte-identical to cargo's binary"
    else
      status "relink control $X: differs from cargo's binary"
    fi
    for s in 1 2 3 4 5 6 7 8; do
      python3 "$R" "$D/save-$X" "$D/bins/$X.shuf.s$s/succinctly" --seed $s --mode shuffle >> "$D/relink-$X.log" 2>&1 || return 1
    done
    status "relinks $X: $(ls -d "$D"/bins/$X.* | wc -l) binaries"
  done
}

step_inputs_cg() {
  G="$D/bins/A/succinctly"
  mkdir -p "$D/inputs"
  for spec in strings:strings ndjson:ndjson jsonseq:json-seq; do
    id=${spec%%:*}; pat=${spec#*:}
    for sz in 1mb 10mb; do
      f="$D/inputs/$id-$sz.json"
      [ -s "$f" ] || "$G" json generate $sz -p $pat -s 1 -o "$f" > /dev/null || return 1
      status "input $id-$sz sha256 $(shasum -a 256 "$f" | cut -c1-16)"
    done
  done
}

step_clean() {
  for X in A B C; do (cd "$D/$X" && cargo clean -q) || return 1; done
}

if [ "${1:-}" = step ]; then
  case $2 in
    fetch) step_fetch ;;
    build-A) step_build A ;; build-B) step_build B ;; build-C) step_build C ;;
    relinks) step_relinks ;;
    cachegrind) step_inputs_cg ;;
    clean) step_clean ;;
    *) echo "unknown step $2"; exit 2 ;;
  esac
  exit $?
fi

# Driver. Each step runs under cap.py: RLIMIT_CORE (1,1) by setrlimit, a wall cap (estimate + 50%),
# its own process group killed by id at the cap.
(while sleep 30; do touch "$D/HEARTBEAT"; done) & HB=$!
touch "$D/HEARTBEAT"
status "driver start, pid $$"
cap_of() { case $1 in fetch) echo 900 ;; build-*) echo 1800 ;; relinks) echo 1200 ;; *) echo 600 ;; esac; }
# One release build at a time on the mini across tags: wait while another tag's build holds the lock.
other_build() { ps -Ao command | grep -E '[b]enchlock\.py --shared s7957_build_' | head -1; }
for st in fetch build-A build-B build-C relinks cachegrind clean; do  # cachegrind = inputs here
  case $st in build-*)
    while [ -n "$(other_build)" ]; do status "step $st waiting for another tag's build: $(other_build | cut -c1-80)"; sleep 60; done ;;
  esac
  status "step $st start (cap $(cap_of $st)s)"
  python3 "$D/cap.py" "$(cap_of $st)" python3 "$LOCK" --shared "layout-seeds_$st" bash "$D/build-mac.sh" step "$st" > "$D/step-$st.log" 2>&1
  rc=$?
  echo "$(date -u +%FT%TZ) layout-seeds finished rc=$rc $st du=$(du -sh "$D" | cut -f1)" >> "$SH/STATUS"
  status "step $st rc=$rc"
  if [ $rc -ne 0 ]; then
    echo "$(date -u +%FT%TZ) layout-seeds left: FAILED $st" >> "$SH/DONE"
    echo "FAILED $st" > "$D/DONE"
    kill "$HB"; exit 1
  fi
done
echo "$(date -u +%FT%TZ) layout-seeds left: READY" >> "$SH/DONE"
echo READY > "$D/DONE"
status "READY for timing"
kill "$HB"
