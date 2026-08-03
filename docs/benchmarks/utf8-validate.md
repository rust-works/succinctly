# UTF-8 Validation Benchmarks

Benchmarks for `succinctly text validate utf8`, which validates UTF-8 with two
interchangeable engines:

- **AVX2 SIMD** (x86_64, default) — a first-principles port of the Keiser–Lemire
  "Validating UTF-8 In Less Than One Instruction Per Byte" algorithm
  ([arXiv:2010.03090](https://arxiv.org/abs/2010.03090)): 32-byte blocks with a
  fast accept scan that falls back to the scalar validator for the exact error
  position, so diagnostics are byte-identical either way. Selected at runtime via
  `is_x86_feature_detected!("avx2")`.
- **Scalar** — the portable validator, used on non-x86_64 targets, on x86_64 CPUs
  without AVX2, and whenever `--no-simd` is passed. Multi-byte sequences are
  checked one character at a time; runs of ASCII are skipped eight bytes at a time
  with a broadword (SWAR) test (`word & 0x8080_8080_8080_8080 == 0`), and
  line/column for an error are derived from its byte offset rather than tracked
  per byte (#133). This is the **only** path on ARM64 and in `no_std` builds.

`benches/utf8_validate_bench.rs` reports a `scalar` and a `simd` arm per input so
the two compare directly (`cargo bench --bench utf8_validate_bench`). Its
`utf8_corpus` group validates the real-workload corpus (#301): the fully synced
`data/bench/corpus/` when present, otherwise the always-committed seed under
`tests/data/bench-corpus/seed/`.

> **⚠️ SIMD figures not yet measured.** Every table below is the **scalar** engine
> only. AVX2 throughput must be measured on an AVX2-capable x86_64 host (CI's x86_64
> leg or the 7950X bench box) and — per the perf-cluster convention shared with #134
> (broadword + DFA) — gated on the #301 real-workload corpus before being cited.
> JSON-validation experience suggests several GiB/s, with the largest wins on
> ASCII-heavy and mixed content.

> **⚠️ Mixed vintage — read the per-platform dates.** The **Apple M4 Pro** section was
> re-measured on 2026-07-25 and reflects the #133 broadword ASCII fast path. The
> **AMD Ryzen 9 7950X** and **Apple M1 Max** sections still hold pre-#133 figures
> captured on a pre-rebase branch tip whose commits were orphaned by a rebase, so
> their **Commit** references no longer resolve; they are byte-per-byte slower than
> what the current scalar validator achieves and must be re-measured before being
> cited as authoritative. Do **not** compare an M4 Pro row against a 7950X or M1 Max
> row — they are different code.

## Summary

Scalar engine at 1MB. Rows are **not** comparable to each other — see the vintage
warning above.

| Platform                   | Code      | ASCII (GiB/s) | CJK (GiB/s) | Emoji (GiB/s) | Mixed (GiB/s) |
|----------------------------|-----------|---------------|-------------|---------------|---------------|
| Apple M4 Pro (ARM)         | post-#133 | 29.2          | 2.7         | 3.2           | 3.2           |
| AMD Ryzen 9 7950X (x86_64) | pre-#133  | 2.2           | 1.3         | 1.0           | 1.8           |
| Apple M1 Max (ARM)         | pre-#133  | 1.3           | 1.1         | 1.2           | 1.2           |

**Key Finding**: on ASCII-dominant input the broadword fast path is the whole story —
M4 Pro scalar validation went from 2.2 to 29.2 GiB/s (#133). Multi-byte content
improves far less (roughly 1.3-1.6x), because every multi-byte character is still
decoded one at a time; closing that gap is #134's job.

## Apple M4 Pro (ARM)

**Date**: 2026-07-25
**Commit**: `1104d93a` + the #133 broadword change (branch `issue-133-perf-text-broadword-swar-optimizations`)
**CPU**: Apple M4 Pro (12 cores)
**OS**: macOS 15.6.1
**Rust**: 1.96.0
**Method**: `cargo bench --bench utf8_validate_bench`, two full repetitions per side,
run sequentially on an otherwise idle machine; the two trees differed only in
`src/text/utf8.rs`. Figures are the median of the two repetitions.

ARM64 has no NEON UTF-8 kernel, so **the scalar validator is the only path here** —
these numbers are what `validate_utf8` actually delivers on Apple Silicon.

### Performance by Pattern Type (1MB)

| Pattern        | Before (µs) | After (µs) | Throughput (GiB/s) | Change |
|----------------|-------------|------------|--------------------|--------|
| ASCII          | 446.39      | 33.42      | 29.22              | -92.5% |
| Mixed          | 491.83      | 309.67     | 3.15               | -37.0% |
| CJK (3-byte)   | 534.82      | 368.98     | 2.65               | -31.0% |
| Emoji (4-byte) | 395.94      | 307.69     | 3.17               | -22.3% |
| 2-byte Latin   | 417.14      | 325.77     | 3.00               | -21.9% |

### Detailed Results by Pattern

#### ASCII (Pure 7-bit)

| Size  | Before    | After     | Throughput (GiB/s) | Change |
|-------|-----------|-----------|--------------------|--------|
| 1KB   | 454.1 ns  | 41.5 ns   | 23.01              | -90.9% |
| 10KB  | 4.48 µs   | 342.6 ns  | 27.83              | -92.4% |
| 100KB | 43.17 µs  | 3.29 µs   | 29.02              | -92.4% |
| 1MB   | 446.39 µs | 33.42 µs  | 29.22              | -92.5% |
| 10MB  | 4.64 ms   | 343.45 µs | 28.43              | -92.6% |

#### Mixed (Realistic content)

| Size  | Before    | After     | Throughput (GiB/s) | Change |
|-------|-----------|-----------|--------------------|--------|
| 1KB   | 513.0 ns  | 287.3 ns  | 3.32               | -44.0% |
| 10KB  | 4.95 µs   | 3.00 µs   | 3.18               | -39.4% |
| 100KB | 48.68 µs  | 30.25 µs  | 3.15               | -37.9% |
| 1MB   | 491.83 µs | 309.67 µs | 3.15               | -37.0% |
| 10MB  | 4.98 ms   | 3.18 ms   | 3.07               | -36.0% |

#### CJK (3-byte sequences)

| Size  | Before    | After     | Throughput (GiB/s) | Change |
|-------|-----------|-----------|--------------------|--------|
| 1KB   | 410.5 ns  | 364.8 ns  | 2.61               | -11.1% |
| 10KB  | 4.03 µs   | 3.22 µs   | 2.96               | -20.0% |
| 100KB | 44.91 µs  | 31.71 µs  | 3.01               | -29.4% |
| 1MB   | 534.82 µs | 368.98 µs | 2.65               | -31.0% |
| 10MB  | 4.31 ms   | 3.32 ms   | 2.94               | -22.9% |

#### Emoji (4-byte sequences)

| Size  | Before    | After     | Throughput (GiB/s) | Change |
|-------|-----------|-----------|--------------------|--------|
| 1KB   | 366.8 ns  | 288.6 ns  | 3.30               | -21.3% |
| 10KB  | 3.67 µs   | 2.93 µs   | 3.25               | -20.2% |
| 100KB | 39.39 µs  | 36.10 µs  | 2.64               | -8.4%  |
| 1MB   | 395.94 µs | 307.69 µs | 3.17               | -22.3% |
| 10MB  | 3.71 ms   | 3.07 ms   | 3.18               | -17.2% |

The 100KB row is the noisiest arm in the suite (the two repetitions disagreed,
-30.1% and +17.6%); the other four sizes agree closely, so treat -8.4% as a
lower bound rather than a per-size characteristic.

#### Latin Extended (2-byte sequences)

| Size  | Before    | After     | Throughput (GiB/s) | Change |
|-------|-----------|-----------|--------------------|--------|
| 1KB   | 471.3 ns  | 283.6 ns  | 3.36               | -39.8% |
| 10KB  | 4.24 µs   | 3.14 µs   | 3.04               | -26.0% |
| 100KB | 45.31 µs  | 30.79 µs  | 3.10               | -32.1% |
| 1MB   | 417.14 µs | 325.77 µs | 3.00               | -21.9% |
| 10MB  | 4.40 ms   | 3.20 ms   | 3.05               | -27.2% |

#### Early exit on an invalid byte near end-of-input

`line`/`column` are no longer tracked per byte; they are derived from the error
offset by one broadword newline scan when an error is found (#133). That extra
scan is far cheaper than the per-byte bookkeeping it replaced, so the error path
got faster too:

| Size  | Before    | After     | Change |
|-------|-----------|-----------|--------|
| 1KB   | 460.3 ns  | 97.4 ns   | -78.8% |
| 10KB  | 4.62 µs   | 1.03 µs   | -77.6% |
| 100KB | 50.11 µs  | 10.13 µs  | -79.8% |
| 1MB   | 513.02 µs | 112.01 µs | -78.2% |

### Real-workload corpus (#301)

The `utf8_corpus` group over the synced corpus (`./scripts/sync-bench-corpus.sh`).
Real configuration and data files are overwhelmingly ASCII, so they track the
ASCII pattern closely — which is the point of gating claims on them rather than on
the synthetic ladder alone.

| File                                     | Bytes | Before   | After    | Throughput (GiB/s) | Change |
|------------------------------------------|-------|----------|----------|--------------------|--------|
| `json/geojson/us-election.geojson`       | 99715 | 50.38 µs | 3.48 µs  | 26.70              | -93.1% |
| `dsv/gapminder/gapminder-five-year.csv`  | 82079 | 37.40 µs | 2.72 µs  | 28.15              | -92.7% |
| `yaml/actions/prometheus-ci.yml`         | 18935 | 9.60 µs  | 649.9 ns | 27.13              | -93.2% |
| `dsv/world-gdp/world-gdp-with-codes.csv` | 4515  | 2.08 µs  | 157.8 ns | 26.65              | -92.4% |
| `yaml/actions/stale.yml`                 | 1290  | 607.5 ns | 53.1 ns  | 22.63              | -91.3% |
| `yaml/compose/nginx-flask-mysql.yaml`    | 1239  | 598.7 ns | 52.3 ns  | 22.06              | -91.3% |
| `yaml/actions/codeql-analysis.yml`       | 925   | 394.9 ns | 34.0 ns  | 25.37              | -91.4% |
| `yaml/k8s/nginx-deployment.yml`          | 836   | 398.5 ns | 30.9 ns  | 25.16              | -92.2% |
| `yaml/compose/wordpress.yaml`            | 815   | 392.4 ns | 30.8 ns  | 24.62              | -92.1% |
| `json/charts/bullet-data.json`           | 548   | 263.1 ns | 23.7 ns  | 21.54              | -91.0% |

### Sequence Type Comparison (1MB)

| Sequence Type     | Before (µs) | After (µs) | Throughput (GiB/s) | Change |
|-------------------|-------------|------------|--------------------|--------|
| ASCII (1-byte)    | 481.16      | 34.56      | 28.26              | -92.8% |
| Extended (2-byte) | 447.45      | 324.38     | 3.01               | -27.5% |
| CJK (3-byte)      | 397.03      | 337.00     | 2.90               | -15.1% |
| Emoji (4-byte)    | 346.66      | 318.58     | 3.07               | -8.1%  |
| Mixed             | 473.18      | 319.01     | 3.06               | -32.6% |

**Observation**: the pre-#133 pattern — longer sequences validating *faster* per
byte — is gone. ASCII is now an order of magnitude ahead of everything else, and
the multi-byte patterns have converged on ~3 GiB/s, the cost of decoding each
character individually.

### Where the gains come from

All 44 benchmark arms improved; none regressed. Three changes contribute:

1. **8-byte ASCII skip** — `word & 0x8080_8080_8080_8080 == 0` clears eight bytes
   per test. LLVM auto-vectorises this word loop into NEON, so the realised ASCII
   gain (~13x) far exceeds the ~8x fewer iterations the technique implies on paper.
2. **Line/column off the hot path** — the old loop read `input[pos - 1]` on *every*
   byte to maintain `line`/`line_start`. Deriving both from the error offset removes
   that load entirely and is what lifts the multi-byte patterns, which never touch
   the ASCII fast path at all.
3. **Keeping the ASCII skip out of line** — `#[inline(never)]` on `skip_ascii`.
   Inlining it bloats the enclosing dispatch loop and costs multi-byte input 5-12%;
   across three repetitions the inlined form ranged from -10.6% to +11.8% versus
   baseline (regressing pure emoji in three of four runs), while the out-of-line
   form won on all twelve (benchmark, repetition) pairs at a median of -13.0%. The
   call is amortised over a whole ASCII run, costing the ASCII path under 1%.

---

## AMD Ryzen 9 7950X (x86_64)

**Date**: 2026-02-06
**Commit**: _orphaned by rebase — provisional, pending re-measurement_
**CPU**: AMD Ryzen 9 7950X 16-Core Processor
**OS**: Ubuntu 22.04.5 LTS
**Rust**: 1.92.0

### Performance by Pattern Type

| Pattern        | 1KB (MiB/s) | 10KB (MiB/s) | 100KB (MiB/s) | 1MB (MiB/s) | 10MB (MiB/s) | 100MB (MiB/s) |
|----------------|-------------|--------------|---------------|-------------|--------------|---------------|
| ascii          | 2,271       | 2,299        | 2,308         | 2,308       | 2,245        | 2,262         |
| log_file       | 1,434       | 1,461        | 2,226         | 2,176       | 2,147        | 2,137         |
| pathological   | 1,358       | 1,416        | 2,165         | 2,149       | 2,147        | 2,153         |
| source_code    | 1,953       | 2,095        | 2,055         | 1,987       | 1,984        | 1,979         |
| json_like      | 1,915       | 1,307        | 1,278         | 1,911       | 1,873        | 1,879         |
| mixed          | 967         | 1,278        | 1,936         | 1,856       | 1,823        | 1,819         |
| cjk            | 1,684       | 1,852        | 1,386         | 1,358       | 1,368        | 1,359         |
| greek_cyrillic | 904         | 1,680        | 1,192         | 1,147       | 1,134        | 1,119         |
| emoji          | 651         | 1,225        | 1,946         | 1,040       | 1,061        | 1,036         |
| latin          | 1,528       | 1,531        | 894           | 848         | 874          | 860           |
| all_lengths    | 1,221       | 1,094        | 536           | 500         | 502          | 492           |

### Detailed Results by Pattern

#### ASCII (Pure 7-bit)

| Size  | Time (ms) | Throughput (MiB/s) |
|-------|-----------|--------------------|
| 1KB   | 0.00      | 2,271              |
| 10KB  | 0.00      | 2,299              |
| 100KB | 0.04      | 2,308              |
| 1MB   | 0.43      | 2,308              |
| 10MB  | 4.45      | 2,245              |
| 100MB | 44.22     | 2,262              |

#### CJK (3-byte sequences)

| Size  | Time (ms) | Throughput (MiB/s) |
|-------|-----------|--------------------|
| 1KB   | 0.00      | 1,684              |
| 10KB  | 0.01      | 1,852              |
| 100KB | 0.07      | 1,386              |
| 1MB   | 0.74      | 1,358              |
| 10MB  | 7.31      | 1,368              |
| 100MB | 73.58     | 1,359              |

#### Emoji (4-byte sequences)

| Size  | Time (ms) | Throughput (MiB/s) |
|-------|-----------|--------------------|
| 1KB   | 0.00      | 651                |
| 10KB  | 0.01      | 1,225              |
| 100KB | 0.05      | 1,946              |
| 1MB   | 0.96      | 1,040              |
| 10MB  | 9.43      | 1,061              |
| 100MB | 96.56     | 1,036              |

#### Mixed (Realistic content)

| Size  | Time (ms) | Throughput (MiB/s) |
|-------|-----------|--------------------|
| 1KB   | 0.00      | 967                |
| 10KB  | 0.01      | 1,278              |
| 100KB | 0.05      | 1,936              |
| 1MB   | 0.54      | 1,856              |
| 10MB  | 5.49      | 1,823              |
| 100MB | 54.98     | 1,819              |

#### Latin Extended (2-byte sequences)

| Size  | Time (ms) | Throughput (MiB/s) |
|-------|-----------|--------------------|
| 1KB   | 0.00      | 1,528              |
| 10KB  | 0.01      | 1,531              |
| 100KB | 0.11      | 894                |
| 1MB   | 1.18      | 848                |
| 10MB  | 11.45     | 874                |
| 100MB | 116.34    | 860                |

#### All Lengths (Uniform 1-4 byte mix)

| Size  | Time (ms) | Throughput (MiB/s) |
|-------|-----------|--------------------|
| 1KB   | 0.00      | 1,221              |
| 10KB  | 0.01      | 1,094              |
| 100KB | 0.18      | 536                |
| 1MB   | 2.00      | 500                |
| 10MB  | 19.94     | 502                |
| 100MB | 203.44    | 492                |

---

## Apple M1 Max (ARM)

**Date**: 2026-02-06
**Commit**: _orphaned by rebase — provisional, pending re-measurement_
**CPU**: Apple M1 Max
**OS**: macOS 26.1
**Rust**: 1.89.0
**Benchmark**: Criterion (micro-benchmark)

### Performance by Pattern Type

| Pattern   | 1KB (GiB/s) | 10KB (GiB/s) | 100KB (GiB/s) | 1MB (GiB/s) | 10MB (GiB/s) |
|-----------|-------------|--------------|---------------|-------------|--------------|
| ascii     | 1.34        | 1.35         | 1.34          | 1.35        | 1.36         |
| mixed     | 1.19        | 1.21         | 1.23          | 1.22        | 1.20         |
| cjk       | 1.05        | 1.06         | 1.07          | 1.07        | 1.03         |
| emoji     | 1.17        | 1.17         | 1.17          | 1.18        | 1.15         |
| 2-byte    | 1.02        | 1.00         | 1.01          | 1.00        | 1.00         |
| error_end | 1.25        | 1.25         | 1.28          | 1.32        | -            |

### Detailed Results

#### ASCII (Pure 7-bit)

| Size  | Time (µs) | Throughput (GiB/s) |
|-------|-----------|--------------------|
| 1KB   | 0.71      | 1.34               |
| 10KB  | 7.06      | 1.35               |
| 100KB | 71.4      | 1.34               |
| 1MB   | 722       | 1.35               |
| 10MB  | 7,195     | 1.36               |

#### Mixed (Realistic content)

| Size  | Time (µs) | Throughput (GiB/s) |
|-------|-----------|--------------------|
| 1KB   | 0.80      | 1.19               |
| 10KB  | 7.87      | 1.21               |
| 100KB | 77.8      | 1.23               |
| 1MB   | 798       | 1.22               |
| 10MB  | 8,109     | 1.20               |

#### CJK (3-byte sequences)

| Size  | Time (µs) | Throughput (GiB/s) |
|-------|-----------|--------------------|
| 1KB   | 0.91      | 1.05               |
| 10KB  | 8.97      | 1.06               |
| 100KB | 89.0      | 1.07               |
| 1MB   | 912       | 1.07               |
| 10MB  | 9,463     | 1.03               |

#### Emoji (4-byte sequences)

| Size  | Time (µs) | Throughput (GiB/s) |
|-------|-----------|--------------------|
| 1KB   | 0.82      | 1.17               |
| 10KB  | 8.13      | 1.17               |
| 100KB | 81.5      | 1.17               |
| 1MB   | 830       | 1.18               |
| 10MB  | 8,501     | 1.15               |

#### 2-byte (Latin Extended)

| Size  | Time (µs) | Throughput (GiB/s) |
|-------|-----------|--------------------|
| 1KB   | 0.94      | 1.02               |
| 10KB  | 9.49      | 1.00               |
| 100KB | 94.8      | 1.01               |
| 1MB   | 972       | 1.00               |
| 10MB  | 9,724     | 1.00               |

#### Sequence Types Comparison (1MB)

| Sequence Type     | Time (µs) | Throughput (GiB/s) |
|-------------------|-----------|--------------------|
| ascii (1-byte)    | 740       | 1.32               |
| extended (2-byte) | 979       | 1.00               |
| cjk (3-byte)      | 937       | 1.04               |
| emoji (4-byte)    | 871       | 1.12               |
| mixed             | 820       | 1.19               |

### Key Observations

- **Consistent throughput**: All patterns show stable throughput from 1KB to 10MB
- **ASCII is fastest**: ~1.35 GiB/s, single-byte validation is simplest
- **Emoji faster than CJK**: 1.17 vs 1.05 GiB/s - fewer characters per MB means fewer state transitions
- **2-byte is slowest multi-byte**: 1.00 GiB/s - highest character density for multi-byte sequences

## Key Findings

### Cross-Platform Comparison (1MB)

> **Not currently possible.** Only the M4 Pro has been re-measured since #133; the
> 7950X and M1 Max figures are pre-#133 code. A cross-platform table would compare
> two different validators and read as a hardware result when it is really a code
> result. Restore this section once the other two platforms are re-measured.

Pre-#133 the comparison read: M4 Pro **1.9-2.9x** faster than the 7950X on
multi-byte sequences, with the 7950X marginally ahead on ASCII (2.2 vs 2.0 GiB/s).

### Throughput by Character Type (AMD Ryzen 9 7950X, pre-#133)

1. **ASCII-dominant content** (~2.2-2.3 GiB/s): Pure ASCII, log files, source code, pathological
2. **Mixed content** (~1.8-1.9 GiB/s): JSON-like, mixed prose
3. **Multi-byte content** (~1.0-1.4 GiB/s): CJK (3-byte), emoji (4-byte), Greek/Cyrillic (2-byte)
4. **Uniform multi-byte** (~0.5 GiB/s): All-lengths pattern with maximum byte diversity

### Throughput by Character Type (Apple M4 Pro, post-#133)

1. **ASCII / ASCII-dominant** (~23-29 GiB/s): pure ASCII and the real-workload corpus
2. **Mixed prose** (~3.1-3.3 GiB/s): ASCII-dominant with sparse multi-byte characters
3. **Uniform multi-byte** (~2.6-3.2 GiB/s): 2-byte Latin, CJK, emoji — all converge, since
   each character is decoded individually regardless of its length

### Performance Characteristics

- **ASCII dominates post-#133**: an order of magnitude ahead of multi-byte content on
  M4 Pro. The pre-#133 inversion (multi-byte validating faster than ASCII) is gone
- **Multi-byte is the remaining bottleneck**: ~3 GiB/s across 2/3/4-byte patterns is
  the per-character decode cost; a DFA (#134) is the next lever
- **Ryzen 9 (pre-#133) follows the expected pattern**: ASCII fastest, multi-byte slower
- **Memory bandwidth**: still not the limit. Post-#133 ASCII sustains ~28 GiB/s at
  10MB (past cache), roughly 30 GB/s — an order of magnitude under M4 Pro's peak

### Scaling (Apple M4 Pro, post-#133)

- ASCII throughput plateaus by 100KB (29.02 GiB/s) and holds through 10MB (28.43),
  so the fast path is not cache-limited at these sizes
- 1KB is the outlier at 23.01 GiB/s: at 41 ns per validation, per-call overhead is a
  visible share of the measurement
- Multi-byte patterns are flat across all five sizes (~2.6-3.3 GiB/s), consistent
  with a per-character cost that no amount of input length amortises

## Running the Benchmarks

```bash
# Generate test files
./target/release/succinctly text generate-suite

# Fetch the real-workload corpus so the `utf8_corpus` group covers the larger
# files; without this it falls back to the small committed seed
./scripts/sync-bench-corpus.sh

# Run CLI benchmark
./target/release/succinctly dev bench utf8

# Run Criterion benchmark (all groups, both engines)
cargo bench --bench utf8_validate_bench

# Just the real-workload corpus group
cargo bench --bench utf8_validate_bench -- utf8_corpus

# Via unified runner
./target/release/succinctly bench run utf8_bench
```

## Test Data Patterns

| Pattern        | Description                                       |
|----------------|---------------------------------------------------|
| ascii          | Pure 7-bit ASCII (single-byte sequences)          |
| latin          | Latin Extended characters (2-byte sequences)      |
| greek_cyrillic | Greek and Cyrillic (2-byte sequences)             |
| cjk            | Chinese/Japanese/Korean (3-byte sequences)        |
| emoji          | Emoji and symbols (4-byte sequences)              |
| mixed          | Realistic prose with occasional non-ASCII         |
| all_lengths    | Uniform mix of all sequence lengths (1-4 bytes)   |
| log_file       | Log file style (mostly ASCII with timestamps)     |
| source_code    | Source code style (ASCII with unicode in strings) |
| json_like      | JSON-like structure with unicode strings          |
| pathological   | Maximum multi-byte density                        |
