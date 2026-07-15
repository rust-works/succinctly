# Rust Succinct Library Comparison

[Home](../../) > [Docs](../) > [Benchmarks](./) > Rust Succinct Libraries

Benchmark comparison of succinctly's `BitVec` rank/select against the other maintained pure-Rust
succinct-structure crates.

**Platform**: ARM64 (Apple M5 Max, 18 cores, 128 GB), macOS 26.5.2, rustc 1.97.0
**Date**: 2026-07-15

> **Why this page exists**: [issue #47](https://github.com/rust-works/succinctly/issues/47) asked whether
> existing crates were evaluated before succinctly built its own rank/select, and whether anyone had
> benchmarked them. This page is the measurement; [ADR-0011](../adrs/adr-0011.md) is the decision it informs.
> The short version: **succinctly's `BitVec` is not the fastest structure here, and it is far from the
> smallest.** It is kept for reasons that these benchmarks do not measure.

## Libraries Compared

Each library is given its best-available acceleration, so no crate is measured with a fast path switched
off:

| Library        | Version | Features enabled | Structure benchmarked                              |
|----------------|---------|------------------|----------------------------------------------------|
| **succinctly** | 0.7.0   | `simd`           | `BitVec` (3-level `RankDirectory` + `SelectIndex`) |
| **vers-vecs**  | 1.10.1  | `simd`           | `RsVec`                                            |
| **sucds**      | 0.8.3   | `intrinsics`     | `Rank9Sel`                                         |
| **sux**        | 0.14.0  | (none relevant)  | `SelectAdapt<Rank9<AddNumBits<BitVec>>>`           |

### Not benchmarked, and why

Three of the crates named in issue #47 are excluded. They are ruled out on maintenance or dependency
grounds *before* performance becomes relevant, so benchmarking them would pad the tables without
informing the decision:

| Library    | Version | Reason for exclusion                                                             |
|------------|---------|----------------------------------------------------------------------------------|
| `succinct` | 0.5.2   | Last published August 2019; effectively unmaintained                             |
| `fid`      | 0.1.7   | Repository archived February 2025                                                |
| `bio`      | 4.0.1   | Bioinformatics toolkit; rank/select incidental, ~30 direct dependencies to adopt |

## Limitations

**These results are ARM64-only, and that matters more than usual here** — several of the fastest paths in
this comparison are x86_64-specific and therefore never executed:

- **succinctly's x86_64 popcount acceleration is AVX-512 VPOPCNTDQ** (`popcount_words_x86` in
  [src/bits/popcount.rs](../../src/bits/popcount.rs)); there is no AVX2 variant. On ARM64 the `simd`
  feature selects the NEON path instead. The x86_64 path is untested by this page.
- **vers-vecs' SIMD select is gated on `x86_64` + `avx512vl` + `avx512bw`**. On ARM64 its `simd` feature
  is a no-op, so its select ran on the scalar fallback. On an AVX-512 machine its select numbers below
  could improve substantially.
- **sucds' `intrinsics`** is arch-neutral (`count_ones`/`trailing_zeros`), so it is unaffected by
  platform.

Treat the **placings as ARM64 findings, not universal ones.** Reproducing this on x86_64 (Zen 4, where
the project already documents other benchmarks) is a genuine gap; the `select1` ordering in particular
should not be assumed to carry over.

**Run-to-run variance**: `select1` is memory-bound and noisy on this machine — repeated runs moved
individual cells by up to ~40% (succinctly at 1M/50% measured 125 µs, 181 µs, and 175 µs across three
runs). The *placings* were stable across all three, but do not read the `select1` ratios as precise.
`rank1`, `construction`, and resident size were all stable or exact.

## Methodology

All four structures are built from the **same seeded random words** (`ChaCha8Rng`, fixed seed), so every
library sees identical bits. Each builder takes `&[u64]` and returns a fully owned rank+select structure:
no library is charged for, or credited with, borrowing the caller's buffer.

The benchmark asserts all four libraries **agree on every `rank1`/`select1` answer** before timing them —
a comparison of structures that disagree would measure nothing. See `verify_agreement` in
[bench-compare/benches/succinct_libs.rs](../../bench-compare/benches/succinct_libs.rs).

Sizes and densities mirror [benches/rank_select.rs](../../benches/rank_select.rs) so numbers are comparable
with the in-crate rank/select benchmarks. Query benchmarks issue 10,000 random positions per iteration.

**Fairness caveat on construction**: succinctly (`BitVec::from_words`) and vers-vecs (`BitVec::from_vec`)
both accept an owned `Vec<u64>` by move. sucds and sux have **no bulk-word constructor** and must be filled
one bit at a time. Their construction numbers below reflect that API limitation, not an algorithmic deficit.

## Results

### Resident size (rank + select structure)

Measured with a tracking global allocator, read while the structure is alive. Overhead is over the raw
bits. These figures are deterministic — identical across every run.

| Size / density | succinctly | vers-vecs | sucds | sux   |
|----------------|------------|-----------|-------|-------|
| 1M / 10%       | 27.5%      | **5.6%**  | 36.1% | 36.3% |
| 1M / 50%       | 37.5%      | **4.7%**  | 36.1% | 39.1% |
| 1M / 90%       | 47.5%      | **5.6%**  | 36.1% | 37.7% |
| 10M / 10%      | 27.5%      | **6.5%**  | 99.0% | 36.3% |
| 10M / 50%      | 37.5%      | **5.2%**  | 99.0% | 32.0% |
| 10M / 90%      | 47.5%      | **6.5%**  | 99.0% | 37.7% |

**vers-vecs is 5-8x more compact than succinctly.** Two things drive succinctly's number: the rank
directory spends 128 bits of metadata per 512 bits of data (~25% by design — see
[src/bits/rank.rs](../../src/bits/rank.rs)), and the sampled `SelectIndex` adds an entry per 256 set bits,
which is why succinctly's overhead **climbs with density** (27.5% → 47.5%) where the others stay flat.

This is the strongest result on the page: it is exact, platform-independent, and not subject to the
variance that affects the timings.

### Construction (build the full rank+select structure)

| Size / density | succinctly | vers-vecs    | sucds    | sux      |
|----------------|------------|--------------|----------|----------|
| 1M / 10%       | 21.5 µs    | **19.9 µs**  | 735.5 µs | 1.66 ms  |
| 1M / 50%       | 24.0 µs    | **19.7 µs**  | 757.1 µs | 3.13 ms  |
| 1M / 90%       | 25.0 µs    | **20.1 µs**  | 735.5 µs | 1.69 ms  |
| 10M / 10%      | 212.4 µs   | **209.8 µs** | 7.47 ms  | 17.29 ms |
| 10M / 50%      | 242.7 µs   | **215.3 µs** | 7.58 ms  | 29.81 ms |
| 10M / 90%      | 249.9 µs   | **191.3 µs** | 7.76 ms  | 16.79 ms |

vers-vecs builds **1.0-1.31x faster** than succinctly (the two are near-parity at 10M/10%). sucds and sux
are 30-120x slower here, but per the caveat above that is their missing bulk-word constructor, not their
algorithms.

### rank1 (10,000 random queries)

| Size / density | succinctly | vers-vecs | sucds   | sux         |
|----------------|------------|-----------|---------|-------------|
| 1M / 10%       | 14.6 µs    | 33.5 µs   | 16.3 µs | **9.2 µs**  |
| 1M / 50%       | 14.9 µs    | 34.0 µs   | 16.0 µs | **9.0 µs**  |
| 1M / 90%       | 14.2 µs    | 32.6 µs   | 16.8 µs | **9.1 µs**  |
| 10M / 10%      | 16.2 µs    | 30.4 µs   | 16.3 µs | **10.1 µs** |
| 10M / 50%      | 16.0 µs    | 30.0 µs   | 16.1 µs | **10.6 µs** |
| 10M / 90%      | 17.9 µs    | 32.6 µs   | 17.9 µs | **10.8 µs** |

This is succinctly's best showing, and the most stable timing on the page. It is **~1.8-2.3x faster than
vers-vecs** and level with sucds — the cache-aligned directory is buying the query speed it was designed
to buy. But **sux is 1.5-1.7x faster still**, at comparable space.

### select1 (10,000 random queries)

Read the ordering, not the ratios — see the variance note under [Limitations](#limitations).

| Size / density | succinctly | vers-vecs | sucds    | sux         |
|----------------|------------|-----------|----------|-------------|
| 1M / 10%       | 195.9 µs   | 174.4 µs  | 122.2 µs | **55.9 µs** |
| 1M / 50%       | 181.0 µs   | 372.2 µs  | 79.7 µs  | **55.8 µs** |
| 1M / 90%       | 240.2 µs   | 554.1 µs  | 70.8 µs  | **69.3 µs** |
| 10M / 10%      | 278.3 µs   | 182.9 µs  | 139.7 µs | **70.2 µs** |
| 10M / 50%      | 279.8 µs   | 382.1 µs  | 90.3 µs  | **78.7 µs** |
| 10M / 90%      | 276.7 µs   | 575.6 µs  | 83.6 µs  | **63.0 µs** |

succinctly's select is O(log n) by construction (sample every 256 ones, then search), so losing here is
expected rather than surprising. **sux is roughly 3-4x faster** and sucds 1.6-3.4x faster; both keep select
near-flat across density where succinctly's and vers-vecs' degrade. vers-vecs is the only library that is
*worse* than succinctly here, and only at 50-90% density — but on ARM64 its SIMD select path does not
exist, so this is the cell most likely to change on x86_64.

## Summary

| Dimension    | Winner        | succinctly's placing                      |
|--------------|---------------|-------------------------------------------|
| Space        | **vers-vecs** | 4th of 4 at high density (5-8x vers-vecs) |
| Construction | **vers-vecs** | 2nd of 4                                  |
| rank1        | **sux**       | 2nd of 4                                  |
| select1      | **sux**       | 3rd of 4                                  |

succinctly's `BitVec` wins no category on ARM64. `sux` is faster at both rank and select at comparable
space; `vers-vecs` is far smaller and faster to build. On these measurements alone, a third-party crate
would be the better choice.

**The reason succinctly nevertheless implements its own is not on this page**: it is `no_std` (CI-enforced,
and not satisfied by any of these crates), plus structures none of them provide — balanced-parentheses tree
navigation and hinted select. See [ADR-0011](../adrs/adr-0011.md) for the full argument, including where it
does not hold.

## Reproducing Benchmarks

```bash
# From the repository root
succinctly bench run succinct_libs

# Or directly
cd bench-compare
cargo bench --bench succinct_libs
```

Feature selection is pinned in `bench-compare/Cargo.toml` (see [Libraries Compared](#libraries-compared)),
so the command above reproduces the configuration measured here. The resident-size table is printed by the
benchmark itself, after the criterion groups.

Per [STYLE-0008](../STYLE_GUIDE.md), these numbers are regenerated rather than hand-edited, and are labelled
with the CPU they were measured on. **An x86_64 run is outstanding** and would be a genuine addition — it is
the only way to exercise succinctly's AVX-512 popcount and vers-vecs' AVX-512 select.
