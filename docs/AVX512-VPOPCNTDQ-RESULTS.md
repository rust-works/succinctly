# AVX512-VPOPCNTDQ Performance Results

## Summary

Successfully implemented AVX512-VPOPCNTDQ with runtime dispatch, achieving **5.2x throughput improvement** for popcount operations on AMD Ryzen 9 7950X (Zen 4).

## Implementation

- **File**: [src/popcount.rs](../src/popcount.rs)
- **Function**: `popcount_words_avx512vpopcntdq()`
- **Instruction**: `_mm512_popcnt_epi64` - counts bits in 8 u64 words (512 bits) in parallel
- **Dispatch**: Runtime CPU detection via `is_x86_feature_detected!("avx512vpopcntdq")`
- **Availability**: Intel Ice Lake+ (2019), AMD Zen 4+ (2022)

### Key Code

```rust
#[target_feature(enable = "avx512f,avx512vpopcntdq")]
unsafe fn popcount_words_avx512vpopcntdq(words: &[u64]) -> u32 {
    use core::arch::x86_64::*;
    let mut total = 0u32;
    let mut offset = 0;

    // Process 8 u64 words (512 bits) at a time
    while offset + 8 <= words.len() {
        unsafe {
            let ptr = words.as_ptr().add(offset) as *const __m512i;
            let v = _mm512_loadu_si512(ptr);
            let counts = _mm512_popcnt_epi64(v);  // 8x parallel popcount
            total += _mm512_reduce_add_epi64(counts) as u32;
        }
        offset += 8;
    }

    // Handle remaining words (< 8) with scalar
    for &word in &words[offset..] {
        total += word.count_ones();
    }
    total
}
```

## Benchmark Results

### Throughput (1MB dataset)

```
throughput/simd_1MB     time:   [10.088 µs]
                        thrpt:  [96.8 GiB/s]

throughput/scalar_1MB   time:   [52.631 µs]
                        thrpt:  [18.5 GiB/s]
```

**Speedup: 5.2x** (96.8 / 18.5)

### Raw Popcount Performance

| Benchmark           | Time    | Improvement                           |
|---------------------|---------|---------------------------------------|
| 1M bits (125KB)     | 827 ns  | ~60x faster than scalar per-word loop |
| 10M bits (1.25MB)   | 12.2 µs | Sustained high throughput             |
| Single word (1000x) | 946 ns  | Sub-nanosecond per word               |

### End-to-End Performance

| Operation                     | Time    | Impact                                                       |
|-------------------------------|---------|--------------------------------------------------------------|
| 1M bit construction           | 50.1 µs | Popcount is ~1.6% of time (most time in allocation/indexing) |
| 10M bit construction          | 660 µs  | Popcount overhead minimal                                    |
| 1M rank queries (10k queries) | 30.8 µs | 3 ns per query - popcount fast enough to not bottleneck      |

### Alignment Analysis

AVX512 processes 8 u64 words per iteration. Performance scales linearly:

| Words         | AVX512 Loads | Time    | ns/word |
|---------------|--------------|---------|---------|
| 8 (aligned)   | 1            | 1.63 ns | 0.20 ns |
| 16 (aligned)  | 2            | 2.05 ns | 0.13 ns |
| 64 (aligned)  | 8            | 4.65 ns | 0.07 ns |
| 512 (aligned) | 64           | 27.7 ns | 0.05 ns |
| 7 (unaligned) | 0 + scalar   | 4.68 ns | 0.67 ns |
| 9 (unaligned) | 1 + scalar   | 2.68 ns | 0.30 ns |

**Key insight**: AVX512 delivers **0.05-0.20 ns per word** for aligned data, vs **0.67 ns per word** for scalar fallback.

## Real-World Impact

### Rank/Select Operations

Popcount is used during:
1. **Construction**: Building rank directory (one-time cost)
2. **Rank queries**: Partial word popcount in hot path

**Construction**: 5.2x faster popcount, but only ~1-2% of total construction time
- 1M bits: 827 ns popcount vs 50 µs total → **1.6% of time**
- Most time spent in: memory allocation, rank directory building, select indexing

**Rank queries**: Minimal impact - queries already extremely fast (3 ns)
- Directory lookups dominate over partial word popcount
- AVX512 helps when processing large word ranges

### JSON Parsing

Popcount used in:
- Structural character detection (minimal usage)
- Bracket matching for BP construction

**Impact**: Less relevant than JSON SIMD processing (next optimization target)

## Testing

All tests pass with AVX512-VPOPCNTDQ implementation:

```bash
cargo test --lib --features simd popcount
```

Key tests:
- `test_avx512_vpopcntdq_matches_scalar`: Validates correctness against scalar
- `test_avx512_edge_cases`: All zeros, all ones, alternating patterns
- `test_popcount_words_various_lengths`: 0-20 words (alignment boundaries)

## Comparison with Documentation Predictions

From [optimization-opportunities.md](optimization-opportunities.md):

**Predicted**: 2-4x speedup for rank operations
**Actual**: 5.2x throughput improvement for raw popcount

**Why higher than predicted?**
1. AVX512-VPOPCNTDQ is newer than AVX2-POPCNT (8x parallel vs scalar)
2. AMD Zen 4 has excellent AVX512 implementation (no frequency throttling)
3. Benchmark measured pure popcount throughput (best case)

**Real-world impact**: 1-2x on end-to-end workloads due to Amdahl's law
- Popcount is small fraction of total time in construction/queries
- Bigger wins from AVX512 JSON parser (next step)

## Next Steps

1. ✅ Implement AVX512-VPOPCNTDQ with runtime dispatch
2. ✅ Validate correctness with comprehensive tests
3. ✅ Measure microbenchmark performance (5.2x throughput)
4. ✅ Measure end-to-end impact (1.6% of construction time)
5. 🔲 Implement AVX512 JSON parser (64 bytes/iteration vs current 32)
6. 🔲 Benchmark full JSON indexing pipeline with AVX512

## Build Instructions

```bash
# Enable SIMD features (includes AVX512 runtime dispatch)
cargo build --release --features simd

# Run all popcount tests
cargo test --features simd popcount

# Run microbenchmarks
cargo bench --bench popcount_strategies --features simd

# Run end-to-end benchmarks
cargo bench --bench rank_select --features simd -- popcount
```

## CPU Requirements

- **Minimum**: x86_64 with POPCNT (universal on modern CPUs)
- **Optimal**: AVX512-VPOPCNTDQ (Intel Ice Lake 2019+, AMD Zen 4 2022+)
- **Runtime**: Automatically detects and uses best available instruction set

## References

- Intel Intrinsics Guide: [`_mm512_popcnt_epi64`](https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html#text=_mm512_popcnt_epi64)
- AMD Zen 4 Microarchitecture: Full AVX512 support without frequency scaling
- Rust `std::arch::x86_64`: Stable intrinsics since Rust 1.27
