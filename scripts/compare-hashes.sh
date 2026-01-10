#!/bin/bash
# Compare jq output vs succinctly jq output piped through jq
# Outputs JSONL format with results as they become available
# Measures: peak memory (RSS), wall time, CPU time
# Usage: ./scripts/compare-hashes.sh

set -e

cd "$(dirname "$0")/.."

# Use md5 on macOS, md5sum on Linux
if command -v md5 &>/dev/null; then
  md5cmd() { md5 | tr -d ' '; }
elif command -v md5sum &>/dev/null; then
  md5cmd() { md5sum | cut -d' ' -f1; }
else
  echo "Error: neither md5 nor md5sum found" >&2
  exit 1
fi

# Detect platform for /usr/bin/time flags
# macOS: -l gives max RSS in bytes
# Linux: -v gives max RSS in KB
if [[ "$(uname)" == "Darwin" ]]; then
  TIME_CMD="/usr/bin/time -l"
  # Parse macOS time output: "maximum resident set size" line gives bytes
  parse_memory() {
    grep "maximum resident set size" | awk '{print $1}'
  }
  parse_user_time() {
    grep "user" | head -1 | awk '{print $1}'
  }
  parse_sys_time() {
    grep "sys" | head -1 | awk '{print $1}'
  }
  parse_wall_time() {
    grep "real" | head -1 | awk '{print $1}'
  }
else
  TIME_CMD="/usr/bin/time -v"
  # Parse Linux time output: "Maximum resident set size" in KB, convert to bytes
  parse_memory() {
    grep "Maximum resident set size" | awk '{print $6 * 1024}'
  }
  parse_user_time() {
    grep "User time" | awk '{print $4}'
  }
  parse_sys_time() {
    grep "System time" | awk '{print $4}'
  }
  parse_wall_time() {
    grep "Elapsed (wall clock)" | awk '{print $8}'
  }
fi

# Run command and capture timing/memory stats
# Usage: run_with_stats <output_var_prefix> <command...>
# Sets: ${prefix}_out, ${prefix}_mem, ${prefix}_wall, ${prefix}_user, ${prefix}_sys
run_with_stats() {
  local prefix=$1
  shift
  local tmpout=$(mktemp)
  local tmperr=$(mktemp)

  # Run with time, capturing stdout and stderr separately
  $TIME_CMD "$@" >"$tmpout" 2>"$tmperr" || true

  # Extract stats from stderr (time output)
  local mem=$(cat "$tmperr" | parse_memory)
  local wall=$(cat "$tmperr" | parse_wall_time)
  local user=$(cat "$tmperr" | parse_user_time)
  local sys=$(cat "$tmperr" | parse_sys_time)

  # Store results in global variables
  eval "${prefix}_out=\$(cat \"\$tmpout\")"
  eval "${prefix}_mem=\${mem:-0}"
  eval "${prefix}_wall=\${wall:-0}"
  eval "${prefix}_user=\${user:-0}"
  eval "${prefix}_sys=\${sys:-0}"

  rm -f "$tmpout" "$tmperr"
}

for pattern in arrays comprehensive literals mixed nested numbers pathological strings unicode users; do
  for size in 1kb 10kb 100kb 1mb 10mb 100mb; do
    f="data/bench/generated/$pattern/${size}.json"
    if [ -f "$f" ]; then
      filesize=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f" 2>/dev/null)

      # Run jq with timing/memory stats
      run_with_stats jq jq . "$f"
      h1=$(echo "$jq_out" | md5cmd)
      s1=$(echo "$jq_out" | wc -c | tr -d ' ')

      # Run succinctly with timing/memory stats
      run_with_stats succ ./target/release/succinctly jq . "$f"
      # Pipe through jq for hash comparison
      succ_out_piped=$(echo "$succ_out" | jq . 2>/dev/null)
      h2=$(echo "$succ_out_piped" | md5cmd)
      s2=$(echo "$succ_out_piped" | wc -c | tr -d ' ')

      if [ "$h1" = "$h2" ]; then
        status="match"
      else
        status="different"
      fi

      # Output JSONL with timing and memory stats
      printf '{"file":"%s","pattern":"%s","size":"%s","filesize":%s,"status":"%s",' \
        "$f" "$pattern" "$size" "$filesize" "$status"
      printf '"jq":{"hash":"%s","output_size":%s,"peak_memory_bytes":%s,"wall_time_sec":%s,"user_time_sec":%s,"sys_time_sec":%s},' \
        "$h1" "$s1" "${jq_mem}" "${jq_wall}" "${jq_user}" "${jq_sys}"
      printf '"succinctly":{"hash":"%s","output_size":%s,"peak_memory_bytes":%s,"wall_time_sec":%s,"user_time_sec":%s,"sys_time_sec":%s}}\n' \
        "$h2" "$s2" "${succ_mem}" "${succ_wall}" "${succ_user}" "${succ_sys}"
    fi
  done
done
