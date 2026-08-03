# Select Word Scan (#40)

[Home](../../) > [Docs](../) > [Optimizations](./) > Select scan

Investigation of [#40](https://github.com/rust-works/succinctly/issues/40): should the
word scan inside `select` be vectorised?

**Outcome: the SIMD kernel is not justified — but finding that out uncovered an
O(n²) defect in the YAML query path worth 3.6–23× (see [O5](#o5-ib-cursor-reset--accepted-)).**

## The proposal

A [Reddit commenter](https://www.reddit.com/r/rust/comments/1qleizg/comment/o1e0fyo/)
suggested SIMD-popcounting several words at once, prefix-summing them, and
jumping straight to the word holding the k-th set bit, instead of walking words
one at a time.

The issue framed this as a *batch select* API for multi-index patterns like
`.users[0, 5, 10]`. That framing was weak on arrival: the real-workload corpus
puts JSON array size at p50 = 2, p90 = 2 elements
([corpus-shape.md](../benchmarks/corpus-shape.md)), so there is nothing to batch.
The issue also stated that tree navigation uses rank rather than select, implying
no hot call site existed.

Both premises were wrong in an interesting way. Select **is** hot —
`YamlCursor::text_position()` reaches `AdvancePositions::get_sequential`, which
contains a literal per-word scan, once per node during `yq` streaming. So the
optimisation had a real target; it just was not the one the issue named.

## Step 1: measure the scan lengths

A SIMD block only pays off if scans are long. The `select-stats` feature
(`succinctly dev select-stats`, see [benchmarking.md](../guides/benchmarking.md))
records how many words each scan traverses. On the real-workload corpus:

| site                                    | calls | mean  | p50 | p90 | max | calls <4 | work >=4 |
| --------------------------------------- | ----- | ----- | --- | --- | --- | -------- | -------- |
| `yaml/advance_positions get_sequential` | 744   | 59.97 | 3   | 232 | 296 | 50.7%    | 99.0%    |
| `json/light ib_select1_from` (probes)   | 17146 | 16.78 | 17  | 19  | 20  | 0.8%     | 99.9%    |
| the other four scan sites               | 0     | –     | –   | –   | –   | –        | –        |

Sharply bimodal: half the calls are trivial, but **99% of the words popcounted
happen in scans of 4+ words**. Counting calls alone would have rejected this;
counting work said proceed. That is why `select-stats` reports both.

Four of the five scan sites recorded nothing at all — `BitVec::select1`,
BP's select support (`WithSelect::select1`, superseded by `WithCsPoppy::select1`
in #64) and `EliasFano::select1` are reached only by `locate` tooling or have no
in-crate consumer.

## Step 2: the kernel, and the crossover

[`bits::scan`](../../src/bits/scan.rs) provides one `scan_select` helper shared
by all five sites, replacing five copies of the same loop. Rather than the
issue's per-iteration prefix sum, each iteration computes a single block
**total** and either skips the block or drops into a short scalar scan to
pinpoint — a scan needs to locate only once, at the end.

A scalar prologue of `BLOCK` words runs first, so any scan short enough to
finish inside it executes exactly the old code. That makes "no regression on
short scans" structural rather than a tuning claim.

Micro-benchmark (`select_scan_micro`, Apple M4 Pro, NEON, sparse words):

| scan distance | scalar   | `scan_select` | speedup |
| ------------- | -------- | ------------- | ------- |
| 1 word        | 1.11 ns  | 0.93 ns       | 1.19×   |
| 4 words       | 1.88 ns  | 1.85 ns       | 1.02×   |
| 8 words       | 2.89 ns  | 3.43 ns       | 0.84×   |
| 16 words      | 5.32 ns  | 3.55 ns       | 1.50×   |
| 64 words      | 18.62 ns | 7.09 ns       | 2.63×   |
| 256 words     | 89.72 ns | 23.68 ns      | 3.79×   |
| corpus mix    | 492 ns   | 143 ns        | 3.44×   |

Crossover sits between 8 and 16 words. Explicit NEON beat the portable block
popcount by ~1.3×, so LLVM's auto-vectorisation was not already sufficient.

At this point the optimisation looked strongly justified, and end-to-end
`yaml_query` showed 1.8–3.2×.

## Step 3: the number was measuring a bug

The end-to-end win was suspiciously uniform — short-scalar fixtures, where scans
should be a single word, sped up 1.9×. Measuring the fixtures directly showed
why: mean scan length was 284 words, not 1. Sweeping file size:

| keys | bytes   | mean scan | total words popcounted |
| ---- | ------- | --------- | ---------------------- |
| 250  | 4 390   | 34.1      | 8 564                  |
| 1000 | 17 890  | 139.4     | 139 489                |
| 4000 | 74 890  | 578.8     | 2 315 578              |
| 8000 | 150 890 | 1171.5    | 9 373 251              |

Mean scan length doubles when the file doubles: **total scan work is O(n²)**.

Instrumenting the cursor's path selection found the cause. For a 4000-key
mapping the traversal took the sequential fast path twice, the forward-gap path
4000 times, and the random path 4000 times — and `get_random` set
`ib_word_idx: 0, ib_ones_before: 0` while advancing `next_open_idx`. It computed
the IB position, then threw it away, so every following scan restarted from word
0 and walked the whole prefix.

## O5: IB cursor reset — ACCEPTED ✅

Carrying the position `get_random` already computed, instead of zeroing it:

```rust
let (ib_word_idx, ib_ones_before) = match result {
    Some(pos) => {
        let w = pos as usize / 64;
        (w, self.ib_rank[w] as usize)
    }
    None => (0, 0),
};
```

Scan length becomes flat in file size — O(n²) → O(n):

| keys | mean scan before | after | total words before | after  |
| ---- | ---------------- | ----- | ------------------ | ------ |
| 250  | 34.1             | 1.27  | 8 564              | 318    |
| 1000 | 139.4            | 1.28  | 139 489            | 1 281  |
| 4000 | 578.8            | 1.29  | 2 315 578          | 5 161  |
| 8000 | 1171.5           | 1.29  | 9 373 251          | 10 321 |

At 8000 keys that is a **908× reduction** in scan work.

End-to-end `yaml_query` (Apple M4 Pro, medians):

| case          | baseline | + cursor fix     | + cursor fix + `scan_select` |
| ------------- | -------- | ---------------- | ---------------------------- |
| value_8b      | 361 µs   | 99.1 µs (3.6×)   | 99.6 µs (−0.5%)              |
| value_32b     | 666 µs   | 145 µs (4.6×)    | 140 µs (+3.3%)               |
| value_128b    | 1.81 ms  | 265 µs (6.8×)    | 258 µs (+2.7%)               |
| value_512b    | 6.33 ms  | 702 µs (9.0×)    | 677 µs (+3.5%)               |
| value_2048b   | 24.3 ms  | 2.52 ms (9.6×)   | 2.38 ms (+5.6%)              |
| records_1k    | 658 µs   | 183 µs (3.6×)    | 185 µs (−1.3%)               |
| records_10k   | 43.2 ms  | 1.86 ms (23.2×)  | 1.89 ms (−1.5%)              |
| nested_d6_w4  | 1.77 ms  | 303 µs (5.8×)    | 307 µs (−1.2%)               |
| nested_d4_w8  | 1.39 ms  | 251 µs (5.5×)    | 256 µs (−2.0%)               |

Output is byte-identical across 127 query/file pairs, and the pinned-`yq` golden
suite passes unchanged.

## Verdict on #40

**The SIMD batch select is not justified.** Once the cursor is fixed, the mean
scan is 1.29 words — every scan completes inside the scalar prologue and the
vector path is never entered. `scan_select` contributes −2% to +5.6% on top of
the cursor fix, which is noise.

The 1.8–3.2× that `scan_select` appeared to deliver was entirely an artifact of
the defect: it made a quadratic scan 3× cheaper, where the quadratic scan should
not have existed. Vectorising it would have locked in the bug behind a
respectable-looking benchmark.

`scan_select` retains non-performance value — five copies of the same loop
became one helper with property tests — but its NEON and AVX2 kernels are dead
weight at the measured scan lengths.

## Lessons

- **A hot loop is not the same as necessary work.** The scan-length distribution
  said "long scans, lots of work, good SIMD target" and was correct about all
  three. It could not say the work was avoidable. Asking *why* the tail was long,
  rather than only how long, is what turned a 3× into a 23×.
- **Beware a speedup that is too evenly distributed.** Uniform gains across
  fixtures that should differ is a signal that something other than the intended
  mechanism is being measured.
- **Measure the fixture, not just the benchmark.** The synthetic fixtures had a
  scan distribution far worse than the real corpus. Checking them against the
  corpus numbers is what exposed the gap.
- The repo's existing lesson held again: *algorithmic improvements beat
  micro-optimisations* — here by roughly a factor of 300.

## See also

- [`src/bits/scan.rs`](../../src/bits/scan.rs) — the shared helper
- [`src/util/select_stats.rs`](../../src/util/select_stats.rs) — instrumentation
- [`benches/select_scan_micro.rs`](../../benches/select_scan_micro.rs) — crossover
- [`benches/yaml_query.rs`](../../benches/yaml_query.rs) — query-path coverage
- [end-positions.md](end-positions.md) — O1/O2, the earlier cursor work
- [access-patterns.md](access-patterns.md) — sequential vs random access
