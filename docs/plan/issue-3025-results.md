# #3025 implementation evidence

The reindex bridge now preserves every non-NaN number literal verbatim,
including source tokens beyond the former 256-character limit. A bounded
owned-state path handles entries conversion and literal numeric updates to an
existing top-level object field. Its consuming update keeps a unique
copy-on-write object unique; a unit test checks that 32 updates leave the
200,000-character sibling at the same address. Unsupported expressions use
the lossless bridge. Array collection drives a loop's projection before
retaining the result, while still returning an array atomically.

## Correctness

The 54-case boundary/filter matrix from #3025 has zero mismatches against jq
1.7.1 on both hosts; the capped baseline has 29. All 30 zero-mantissa
scaling cases agree with the pinned jq binary on each host. CLI regression
tests cover 256-, 257-, and 300-character integer/decimal literals across
all nine issue filters, and 200,000-digit while/until/entries-reduce cases.
Six native/entries/with_entries checks agree with pinned yq v4.53.3. Direct
owned steps, their consuming variant, and the forced bridge agree in jq and
yq modes for the supported cases, including errors. Unsupported assignment
shapes decline without changing their input.

The final default test suite, jq and yq CLI suites, no-default-features test
suite, all-features clippy, rustdoc, default/SIMD builds, and
`scripts/build.sh` passed. The CI-feature coverage run passed with 93.63%
reported total coverage and 92.36% patch coverage (290/314 new lines). The build script
exposed an existing `Vec::with_capacity` disallowed-method lint in
`bench-compare`; this branch replaces that call with a checked reservation.

## Performance method

Both hosts built release binaries from base commit
`717833c17724a2db6281518af8cb9a3a175cbeb3`, the provisional lossless
patch, and the final implementation. Each within-host comparison used the
same compiler/profile; the scripts generated fixtures outside timing and
interleaved candidates over three repetitions. Values below are median
process wall time. The exact historical #1402 commands were unavailable;
the workload definitions and lossless baseline are in the
[spike report](issue-3025-spike.md).

- Apple M4 Pro, Darwin 25.5.0, Rust 1.96.0; `/usr/bin/jq` 1.7.1-apple.
- AMD Ryzen 9 7950X, WSL2 Linux 6.18.33.2, Rust 1.97.0; official jq 1.7.1 Linux binary.

For a 200,000-zero mantissa and 3,200 iterations, median milliseconds are
listed as capped baseline / unoptimized lossless / final:

- M4 Pro: while 11 / 3367 / 5.4; until 7.9 / 2049 / 6.1; entries-reduce 13.2 / 4096 / 21.7.
- Ryzen: while 11 / 4031 / 5.2; until 7.1 / 2680 / 5.9; entries-reduce 12.7 / 5272 / 24.0.

Every final case is below the 500 ms budget and over 8× faster than the
unoptimized lossless candidate. The capped baseline is incorrect for the
until and entries-reduce projections, so its timing is only a historical
cost reference.

At 800 iterations, increasing the zero run through 20,000 / 200,000 /
1,000,000 digits gives final while times of 3.3 / 4.8 / 11.3 ms on M4 Pro
and 1.8 / 4.3 / 14.3 ms on Ryzen. Entries-reduce takes 4.1 / 9.3 / 29.8 ms
and 2.7 / 9.9 / 40.9 ms respectively. Remaining copying and output work
scale with literal length; the per-update full-index rebuild seen in the
lossless spike is absent on the supported owned path.

The collecting while query peaks at 11.30 MB RSS on M4 Pro and 9.85 MB on
Ryzen, below the 64 MiB budget and far below the lossless spike's roughly
700/653 MB. Until peaks at 11.30/10.50 MB, below 32 MiB. The existing
streaming control peaks at 11.37/10.76 MB, below 32 MiB; the projected
collector no longer retains full intermediate states. That streaming control,
`while(...) | .i | select(. == 3199)`, still takes 1.33/2.06 seconds because
its remaining `Field | Select` pipe reindexes the full owned state for each
emitted value. [#3213](https://github.com/rust-works/succinctly/issues/3213)
tracks this composition outside the bounded owned-step set. By contrast,
`while(...) | .i` and `while(...) | .i | type` finish in under 10 ms on M4
Pro. The update itself no longer rebuilds the index.

Two ordinary 200,000-item workloads were run five times with interleaved
baseline A/A controls. M4 Pro medians in milliseconds (base A / base B /
final) were map-plus-sum 81.1 / 79.7 / 79.3 and map-filter 74.3 / 73.2 /
72.8. Ryzen was 108.9 / 108.5 / 108.5 and 81.3 / 81.4 / 80.5. Outputs
matched. No repeatable >5% end-to-end regression appeared outside the A/A
range on these representative ordinary workloads; this is a focused check,
not a universal performance guarantee.

## Scope left separate

General assignments and unsupported loop bodies still use the now-lossless
bridge and can pay its cost. #3040 concerns YAML source admission and is
independent. The spike also found two pre-existing jq final-formatting
mismatches for extreme underflow/overflow literals, visible on native `.n |
tostring` without any reindexing. [#3212](https://github.com/rust-works/succinctly/issues/3212) tracks them; this change
preserves the source token internally but does not change the final printer.
