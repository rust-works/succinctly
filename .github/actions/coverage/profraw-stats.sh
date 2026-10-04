#!/usr/bin/env bash
# Log how many raw LLVM profile files a coverage run left and how big they are
# (#3750, A2). It reads and prints, never changes what the report step merges, and
# exits 0, with one exception: when pooling was requested (see B1 below) and the
# files are not pooled it exits 1, so that a pooling run which silently did
# nothing cannot pass for one that worked. Delete this file, and the
# `extra-test-commands` line in action.yml that runs it, once its figures are no
# longer wanted AND `pool-profile-files` is no longer used; while pooling is on,
# this is the only thing that checks it took effect.
#
# It answers what the issue needs before deciding anything about the merge step:
# how many files (thousands, per #3649), how big, and which binary writes them.
# Whether they dominate the disk the run consumes is the total below against the
# growth in used space between the two `df -h /` lines of the reclaim step and
# the one at the end of this block. Run it after the tests and before the first
# `cargo llvm-cov report`, which merges them and then removes them.
#
# Where: the action evals `cargo llvm-cov show-env --sh` before the tests, and
# that points the profiles at $CARGO_LLVM_COV_TARGET_DIR (`target/` with the
# cargo-llvm-cov in use when this was written), not `target/llvm-cov-target/`.
# The pattern is an absolute `<dir>/<package>-%p-%Nm.profraw`, so the files sit
# directly in that directory (or one level down, in the `llvm-cov-target/` the
# issue expected): the search stops at depth 2 rather than walking a target dir
# of hundreds of thousands of build files on every run.
#
# Names are `<package>-<pid>-<binary signature>_<pool slot>.profraw`, so the
# signature field groups the files by instrumented binary, without a per-binary
# run. A signature is a hash, not a name, but a group's count and size say which
# kind of process it is: one file per spawned CLI process, a handful for each
# test binary. That holds for a default run only. A pooled run (B1) writes
# `profile-<signature>_<slot>.profraw`, at most one file per CPU per binary, so its
# groups no longer say how many processes a binary spawned; its count is the
# evidence that pooling worked.
#
# B1: `pool-profile-files` exports LLVM_PROFILE_FILE_NAME for the rest of the job. A
# pooled run has about 70 files (one per test binary and doctest, plus the CLI's
# per-CPU pool: 67 non-CLI files were counted in an unpooled run); an unpooled one
# left 30,814 on 2026-10-04. More than 500 with the name set means the pool did not
# take effect, and the script fails after printing everything above.
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
find "$dir" -maxdepth 2 -name '*.profraw' -exec wc -c {} + 2>/dev/null |
  awk '{ size = $1; sub(/^ *[0-9]+ +/, ""); if ($0 != "total") print size "\t" $0 }' >"$list"

n=$(wc -l <"$list" | tr -d ' ')
echo "dir: $dir"
# What pattern this run's processes were told to write (#3750 B1): the second line
# is only set when `pool-profile-files` is on.
echo "LLVM_PROFILE_FILE=${LLVM_PROFILE_FILE-<unset>}"
echo "LLVM_PROFILE_FILE_NAME=${LLVM_PROFILE_FILE_NAME-<unset>}"
pool_failed=0
if [ -n "${LLVM_PROFILE_FILE_NAME:-}" ] && [ "$n" -gt 500 ]; then
  pool_failed=1
fi
if [ "$n" -eq 0 ]; then
  echo "no .profraw files found"
  echo "::endgroup::"
  exit 0
fi

# Size distribution: count, total, and percentiles of the file size.
sort -n "$list" | awk -F'\t' '
  { a[NR] = $1; total += $1 }
  # Nearest rank: the smallest value with at least p of the files at or below it.
  function pct(p,   r, i) { r = NR * p; i = int(r); if (i < r) i++; if (i < 1) i = 1; return a[i] }
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

# Free disk now, at the run's peak before the merge, for the comparison above.
df -h / 2>/dev/null || true

echo "::endgroup::"
if [ "$pool_failed" -eq 1 ]; then
  echo "::error::pool-profile-files is on (LLVM_PROFILE_FILE_NAME=$LLVM_PROFILE_FILE_NAME) but $n .profraw files exist: pooling did not take effect (#3750)"
  exit 1
fi
exit 0
