#!/usr/bin/env bash
# Log how many raw LLVM profile files a coverage run left and how big they are
# (#3750, A2). OBSERVATION ONLY: it reads, prints and exits 0, so it can never
# fail the job or change what the report step merges. Delete this file, and the
# `extra-test-commands` line in action.yml that runs it, once a run's log has
# been read.
#
# It answers what the issue needs before deciding anything about the merge step:
# how many files (thousands, per #3649), how big, which binary writes them, and
# whether they dominate the disk the run consumes. Run it after the tests and
# before the first `cargo llvm-cov report`, which merges them and then removes
# them.
#
# Where: the action evals `cargo llvm-cov show-env --sh` before the tests, and
# that points the profiles at $CARGO_LLVM_COV_TARGET_DIR (`target/` with the
# cargo-llvm-cov in use when this was written), not `target/llvm-cov-target/`.
#
# Names are `<package>-<pid>-<binary signature>_<pool slot>.profraw`, so the
# signature field groups the files by instrumented binary, without a per-binary
# run. A signature is a hash, not a name, but a group's count and size say which
# kind of process it is: one file per spawned CLI process, a handful for each
# test binary.
#
# Only portable tools (`wc -c`, not GNU `find -printf`), so it also runs on a
# laptop for a local check.

set -uo pipefail
export LC_ALL=C

dir="${CARGO_LLVM_COV_TARGET_DIR:-target}"
list="$(mktemp)"
trap 'rm -f "$list"' EXIT

echo "::group::profraw stats (#3750 A2)"

# `<bytes>\t<path>` per file. `wc -c ... +` may print several `total` lines.
find "$dir" -name '*.profraw' -exec wc -c {} + 2>/dev/null |
  awk '{ size = $1; sub(/^ *[0-9]+ +/, ""); if ($0 != "total") print size "\t" $0 }' >"$list"

n=$(wc -l <"$list" | tr -d ' ')
echo "dir: $dir"
if [ "$n" -eq 0 ]; then
  echo "no .profraw files found"
  echo "::endgroup::"
  exit 0
fi

# Size distribution: count, total, and percentiles of the file size.
sort -n "$list" | awk -F'\t' '
  { a[NR] = $1; total += $1 }
  function pct(p,   i) { i = int(NR * p); if (i < 1) i = 1; return a[i] }
  END {
    printf "files: %d  total: %.0f bytes (%.2f GiB)\n", NR, total, total / 1073741824
    printf "size bytes: min=%.0f p50=%.0f p90=%.0f p99=%.0f max=%.0f\n", a[1], pct(0.50), pct(0.90), pct(0.99), a[NR]
  }'

# Histogram over fixed size bands.
awk -F'\t' '
  BEGIN {
    nb = split("1024 4096 16384 65536 262144 1048576 4194304 16777216", bound, " ")
    split("<=1KiB <=4KiB <=16KiB <=64KiB <=256KiB <=1MiB <=4MiB <=16MiB", name, " ")
    name[nb + 1] = ">16MiB"
  }
  {
    b = nb + 1
    for (i = 1; i <= nb; i++) if ($1 <= bound[i]) { b = i; break }
    cnt[b]++; bytes[b] += $1
  }
  END {
    print "size histogram:"
    for (i = 1; i <= nb + 1; i++)
      if (cnt[i]) printf "  %-8s %7d files  %10.1f MiB\n", name[i], cnt[i], bytes[i] / 1048576
  }' "$list"

# Group by binary signature: the field after the last `-`, up to the `_`. Counted
# from the end because the package name can itself contain hyphens.
echo "by binary signature (top 10 by bytes):"
awk -F'\t' '
  {
    base = $2; sub(/^.*\//, "", base); sub(/\.profraw$/, "", base)
    k = split(base, f, "-"); split(f[k], s, "_")
    cnt[s[1]]++; bytes[s[1]] += $1
  }
  END { for (g in cnt) printf "%.0f\t%d\t%s\n", bytes[g], cnt[g], g }' "$list" |
  sort -rn | head -10 |
  awk -F'\t' '{ printf "  %-22s %7d files  %10.1f MiB  avg %9.1f KiB\n", $3, $2, $1 / 1048576, $1 / $2 / 1024 }'

# What share of the disk the run took: the profile files against the whole
# target dir (build artifacts included) and what is left.
echo "target dir: $(du -sk "$dir" 2>/dev/null | awk '{ printf "%.2f GiB", $1 / 1048576 }') on disk"
df -h / 2>/dev/null || true

echo "::endgroup::"
exit 0
