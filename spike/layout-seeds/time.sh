#!/bin/bash
# Spike layout-seeds, timing phase: ONE exclusive benchlock hold (<= 15 min, cap.py 840 s).
# Usage: time.sh linux|mac   (run after Machines' GO)
set -u -o pipefail
OS=$1
D=$HOME/wrk/bench-scratch/bughunt2/layout-seeds
SH=$HOME/wrk/bench-scratch/bughunt2
LOCK=$HOME/wrk/bench-scratch/7260/benchlock.py
cd "$D" || exit 1
if [ "${2:-}" != inner ]; then
  (while sleep 30; do touch "$D/HEARTBEAT"; done) & HB=$!
  echo "$(date -u +%FT%TZ) timing start ($OS)" >> "$D/STATUS"
  python3 "$D/cap.py" 840 python3 "$LOCK" layout-seeds_time bash "$D/time.sh" "$OS" inner > "$D/time-$OS.log" 2>&1
  rc=$?
  echo "$(date -u +%FT%TZ) layout-seeds finished rc=$rc time-$OS du=$(du -sh "$D" | cut -f1)" >> "$SH/STATUS"
  echo "$(date -u +%FT%TZ) layout-seeds left: $([ $rc = 0 ] && echo TIMED || echo "FAILED time-$OS")" >> "$SH/DONE"
  echo "$([ $rc = 0 ] && echo TIMED || echo "FAILED time-$OS")" > "$D/DONE"
  kill "$HB"; exit $rc
fi
H="$D/spike/scripts/layout-ab.py"
bins() {  # $1 = mode (pad|shuf)
  echo --bin A=$D/bins/A/succinctly --bin Ap=$D/bins/A.cargo/succinctly --bin B=$D/bins/B/succinctly --bin C=$D/bins/C/succinctly
  for X in A B C; do for s in 1 2 3 4 5 6 7 8; do echo --bin $X.s$s=$D/bins/$X.$1.s$s/succinctly; done; done
}
mkdir -p "$D/res"
run() {  # mode row input reps
  echo "== $1 $2 $3"
  python3 "$H" --row "$2" --input "$D/inputs/$3.json" --reps "$4" $(bins "$1") \
    --pair Ap/A --pair B/A --pair C/A --heartbeat "$D/HEARTBEAT" --csv "$D/res/$OS-$1-$2-$3.csv"
}
if [ "$OS" = linux ]; then
  for mode in pad shuf; do
    run $mode strings_upcase strings-1mb 25
    run $mode strings_upcase strings-10mb 15
    run $mode strings_upcase strings-100mb 6
  done
else
  for mode in shuf; do
    run $mode ndjson_slurp ndjson-1mb 20
    run $mode ndjson_slurp ndjson-10mb 12
    run $mode seq_slurp jsonseq-1mb 20
    run $mode seq_slurp jsonseq-10mb 12
    run $mode strings_upcase strings-1mb 20
    run $mode strings_upcase strings-10mb 12
  done
fi
