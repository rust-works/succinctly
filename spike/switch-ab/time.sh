#!/bin/bash
# Spike switch-ab, timing phase: ONE exclusive benchlock hold (cap.py 840 s). Run after GO.
# Arms: M (base), P (prototype, model on), P0 (P's binaries with SUCCINCTLY_READER_MODEL=0), each
# unseeded and under the 8 padding seeds. Pairs: P/M (two binaries), P0/M, P/P0 (same binary).
set -u -o pipefail
D=$HOME/wrk/bench-scratch/bughunt2/switch-ab
SH=$HOME/wrk/bench-scratch/bughunt2
LOCK=$HOME/wrk/bench-scratch/7260/benchlock.py
cd "$D" || exit 1
if [ "${1:-}" != inner ]; then
  (while sleep 30; do touch "$D/HEARTBEAT"; done) & HB=$!
  echo "$(date -u +%FT%TZ) timing start" >> "$D/STATUS"
  python3 "$D/cap.py" 840 python3 "$LOCK" switch-ab_time bash "$D/time.sh" inner > "$D/time.log" 2>&1
  rc=$?
  echo "$(date -u +%FT%TZ) switch-ab finished rc=$rc time du=$(du -sh "$D" | cut -f1)" >> "$SH/STATUS"
  echo "$(date -u +%FT%TZ) switch-ab left: $([ $rc = 0 ] && echo TIMED || echo "FAILED time")" >> "$SH/DONE"
  echo "$([ $rc = 0 ] && echo TIMED || echo "FAILED time")" > "$D/DONE"
  kill "$HB"; exit $rc
fi
H="$D/spike/scripts/layout-ab.py"
bins() {
  for X in M P; do
    echo --bin $X=$D/bins/$X/succinctly
    for s in 1 2 3 4 5 6 7 8; do echo --bin $X.s$s=$D/bins/$X.s$s/succinctly; done
  done
  echo --bin P0=$D/bins/P/succinctly
  for s in 1 2 3 4 5 6 7 8; do echo --bin P0.s$s=$D/bins/P.s$s/succinctly; done
}
mkdir -p "$D/res"
run() {  # row input reps [extra]
  echo "== $1 $2"
  python3 "$H" --row "$1" --input "$D/inputs/$2.json" --reps "$3" $(bins) --env P0:SUCCINCTLY_READER_MODEL=0 \
    --pair P/M --pair P0/M --pair P/P0 --heartbeat "$D/HEARTBEAT" --csv "$D/res/$1-$2.csv" ${4:-}
}
while read -r row input reps extra; do
  [ -z "$row" ] || [ "${row#\#}" != "$row" ] && continue
  run "$row" "$input" "$reps" "$extra"
done < "$D/rows.txt"
