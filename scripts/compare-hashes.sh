#!/bin/bash
# Compare jq output vs succinctly jq output piped through jq
# Outputs JSONL format with results as they become available
# Usage: ./scripts/compare-hashes.sh

set -e

cd "$(dirname "$0")/.."

for pattern in arrays comprehensive literals mixed nested numbers pathological strings unicode users; do
  for size in 1kb 10kb 100kb 1mb 10mb 100mb; do
    f="data/bench/generated/$pattern/${size}.json"
    if [ -f "$f" ]; then
      filesize=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f" 2>/dev/null)

      # Get output from jq directly
      jq_out=$(jq . "$f" 2>/dev/null)
      h1=$(echo "$jq_out" | md5 | tr -d ' ')
      s1=$(echo "$jq_out" | wc -c | tr -d ' ')

      # Get output from succinctly piped through jq
      succ_out=$(./target/release/succinctly jq . "$f" 2>/dev/null | jq . 2>/dev/null)
      h2=$(echo "$succ_out" | md5 | tr -d ' ')
      s2=$(echo "$succ_out" | wc -c | tr -d ' ')

      if [ "$h1" = "$h2" ]; then
        status="match"
      else
        status="different"
      fi

      # Output JSONL
      printf '{"file":"%s","pattern":"%s","size":"%s","filesize":%s,"status":"%s","jq_hash":"%s","jq_output_size":%s,"succinctly_hash":"%s","succinctly_output_size":%s}\n' \
        "$f" "$pattern" "$size" "$filesize" "$status" "$h1" "$s1" "$h2" "$s2"
    fi
  done
done
