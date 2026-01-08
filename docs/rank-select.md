# High-Performance Rank/Select on ARM

This document covers alternative approaches for implementing high-performance rank/select operations on ARM architectures, where PDEP/PEXT (BMI2) instructions are not available.

## Background

Rank and select are fundamental operations for succinct data structures:

- **rank(i)**: Count the number of 1-bits in positions 0 through i
- **select(j)**: Find the position of the j-th 1-bit

The `hw-rankselect` package in the haskell-works ecosystem relies on PDEP/PEXT (BMI2) for optimal x86_64 performance. These instructions have no direct ARM equivalent, requiring alternative approaches.

## Target Platforms

| Platform              | SIMD      | Notes                      |
|-----------------------|-----------|----------------------------|
| aarch64-apple-darwin  | NEON      | Apple M1/M2/M3/M4          |
| aarch64-unknown-linux | NEON, SVE | AWS Graviton, Ampere Altra |
| x86_64 (baseline)     | SSE4.2+   | Reference for comparison   |

## Hardware Options

| Approach           | Availability               | Performance            | Notes                                   |
|--------------------|----------------------------|------------------------|-----------------------------------------|
| ARM SVE2 BDEP/BEXT | Optional in ARMv9          | Near-native PDEP/PEXT  | Not available on most current ARM chips |
| ARM NEON popcount  | All ARMv8 (aarch64)        | Fast (1.8 cycles/byte) | Via `vcnt` instruction                  |
| ARM SVE popcount   | Graviton3+, Apple M-series | 3x faster than NEON    | Limited availability                    |

### ARM SVE2 BDEP/BEXT

SVE2 (Scalable Vector Extension 2) provides direct equivalents to x86 PDEP/PEXT:

| SVE2 Instruction | x86 Equivalent | Operation                              |
|--------------------|------------------|------------------------------------------|
| BDEP             | PDEP           | Scatter lower-bits into masked positions |
| BEXT             | PEXT           | Gather bits from masked positions      |
| BGRP             | (compound)     | `BEXT(a,b) | (BEXT(a,~b) << popcount(b))` |

**Availability**: SVE2 is optional in ARMv9 and not yet widely deployed. All currently announced SVE2 cores only support 128-bit vectors (same width as NEON). Operations occur in vector registers rather than general-purpose registers.

**Processors with SVE2**: Some Arm Cortex-X2/X3/X4, Cortex-A710/A715/A720, Neoverse V2.

### ARM NEON Popcount

NEON provides efficient population count via `vcnt`:

```c
// NEON popcount for 16 bytes
uint8x16_t data = vld1q_u8(ptr);
uint8x16_t counts = vcntq_u8(data);
// Sum the byte counts
```

Performance: ~1.8 cycles/byte (benchmark from Daniel Lemire).

### ARM SVE Popcount

SVE provides scalable popcount that adapts to vector width:

```c
// SVE popcount (width-agnostic)
svuint8_t data = svld1_u8(pg, ptr);
svuint8_t counts = svcnt_u8_x(pg, data);
```

Performance: Up to 3x faster than NEON on AWS Graviton3 (from libpopcnt benchmarks).

## NEON Popcount for 512-bit Blocks

The hot inner operation for rank queries is counting bits in a 512-bit (64-byte) block. This maps well to NEON:

### Basic NEON Popcount Pattern

```rust
use core::arch::aarch64::*;

/// Popcount of 64 bytes (512 bits) using NEON.
/// Requires: ptr aligned to 16 bytes, valid for 64-byte read
#[target_feature(enable = "neon")]
unsafe fn popcount_512_neon(ptr: *const u8) -> u32 {
    // Load 4 x 128-bit chunks
    let v0 = vld1q_u8(ptr);
    let v1 = vld1q_u8(ptr.add(16));
    let v2 = vld1q_u8(ptr.add(32));
    let v3 = vld1q_u8(ptr.add(48));

    // Per-byte popcount
    let c0 = vcntq_u8(v0);
    let c1 = vcntq_u8(v1);
    let c2 = vcntq_u8(v2);
    let c3 = vcntq_u8(v3);

    // Sum byte counts (stay in vector to avoid pipeline stalls)
    let sum01 = vaddq_u8(c0, c1);
    let sum23 = vaddq_u8(c2, c3);
    let sum = vaddq_u8(sum01, sum23);

    // Horizontal sum to scalar
    vaddvq_u8(sum) as u32
}
```

### Performance Characteristics

| Approach                     | Throughput       | Notes                      |
|------------------------------|------------------|----------------------------|
| Basic NEON (vcntq + vaddvq)  | ~1.8 cycles/byte | Single accumulator         |
| Unrolled + multi-accumulator | ~1.5 cycles/byte | Better ILP, reduces stalls |
| Scalar u64::count_ones()     | ~3-4 cycles/byte | Baseline fallback          |

**Key insight**: The basic NEON implementation gives 2x speedup over scalar. Using multiple accumulators with instruction-level parallelism can squeeze out another 20% on large blocks.

### Accumulator Overflow Prevention

For counting more than 255 bytes (2040 bits), the 8-bit accumulators can overflow. Use widening operations periodically:

```rust
// Widen 8-bit counts to 16-bit, then 32-bit, then 64-bit
let wide = vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(acc)));
```

## Software Polyfills for PDEP/PEXT

When hardware support is unavailable, software emulation is required.

| Polyfill              | Technique                      | Performance vs Native | ARM Support         |
|-----------------------|--------------------------------|-----------------------|---------------------|
| ZP7                   | Parallel-prefix-popcount       | ~3-5x slower          | Portable C fallback |
| Multiplication tricks | Bit concentration via multiply | Mask-dependent        | Pure arithmetic     |
| Naive loop            | Bit-by-bit iteration           | ~10-20x slower        | Fully portable      |

### ZP7 (Zach's Peppy Parallel-Prefix-Popcountin' PEXT/PDEP Polyfill)

[github.com/zwegner/zp7](https://github.com/zwegner/zp7)

ZP7 is a branchless PEXT/PDEP polyfill using parallel prefix algorithms:

- **Branchless**: Avoids branch misprediction penalties
- **Portable**: Works on any architecture when no CPU-specific flags are set
- **Optional acceleration**: Can use CLMUL (carryless multiply) on x86 for ~2x speedup
- **Performance**: ~3-5x slower than native PDEP/PEXT, but much faster than naive loops

```c
// ZP7 usage
uint64_t result = zp7_pext_64(value, mask);
uint64_t deposited = zp7_pdep_64(value, mask);
```

### Multiplication-Based PEXT

For compile-time-known masks with few bit groups, PEXT can be emulated efficiently:

```c
// Example: extracting 8 bits spaced 8 apart
// pext(x, 0x8040201008040201)
uint64_t mask = 0x8040201008040201ULL;
uint64_t result = (((x & mask) + (0x8080808080808080ULL - mask))
                   * 0x0002040810204081ULL) >> 56;
```

This technique:
- Works well for masks with evenly-spaced bits
- Performance depends on mask structure
- Can be generated at compile time for known patterns

### Performance Comparison

| Implementation           | Latency (cycles) | Notes                      |
|--------------------------|------------------|----------------------------|
| Intel PDEP/PEXT (native) | 3                | Haswell and later          |
| AMD PDEP/PEXT (native)   | ~18              | Zen 1-2; improved in Zen 3 |
| ZP7 (software)           | ~15-20           | Portable, branchless       |
| Naive loop               | ~50-100          | Bit-by-bit, branch-heavy   |

## Rank/Select Data Structures (No PDEP/PEXT Required)

Modern rank/select structures achieve high performance through cache-aware layout and algorithmic innovations, not PDEP/PEXT.

| Structure     | Space Overhead | Rank Time | Select Time | Key Technique                              |
|---------------|----------------|-----------|-------------|--------------------------------------------|
| Rank9         | 25%            | Very fast | Moderate    | 2-level interleaved index                  |
| Rank9.v2      | 6.3%           | Fast      | Moderate    | Compressed 2-level                         |
| Poppy         | 3%             | Fast      | Fast        | 3-level with interleaving                  |
| CS-Poppy      | 3.51%          | Fast      | Fast        | Poppy + sampling for select                |
| SPIDER (2024) | 3.82%          | Best ≥8Gb | Very fast   | Interleaved metadata + ML-inspired predict |
| RRR           | Compressed     | O(1)      | O(1)        | Block-based with lookup tables             |

### SPIDER (2024) - State of the Art

[arxiv.org/abs/2405.05214](https://arxiv.org/abs/2405.05214)

SPIDER (Succinct Predictions for Indexed Data Efficiently Ranked) is the current state-of-the-art:

**Key innovations**:
1. **Interleaved metadata**: Metadata stored alongside bit vector data improves cache locality
2. **Prediction-based select**: ML-inspired predictions nearly eliminate linear scan cost
3. **Minimal space**: Only 3.82% overhead while achieving best-in-class performance

**Performance improvements**:
- Rank queries: 22% faster than previous state-of-the-art
- Select queries: 41% faster for structures using <5% space
- Best rank query time for datasets ≥8 billion bits

**Architecture independence**: SPIDER's techniques (interleaving, prediction) work on any architecture.

### Rank9

Proposed by Sebastiano Vigna, Rank9 uses:
- Two-level index structure
- Interleaved first and second layer entries (128-bit "words")
- Broadword programming for efficient popcount

**Rank9.v2** reduces space from 25% to 6.3% while maintaining performance.

### Poppy / CS-Poppy

[Space-Efficient, High-Performance Rank & Select (Zhou et al., SEA 2013)](https://www.cs.cmu.edu/~dga/papers/zhou-sea2013.pdf)

Poppy uses:
- Three-level structure
- Interleaved last two levels for cache efficiency
- Carefully implemented bit tricks

CS-Poppy extends Poppy with:
- Sampling-based select acceleration
- Support for up to 2^64 bits
- ~3.51% space overhead

### RRR (Raman-Raman-Rao)

[RRR: A Succinct Rank/Select Index](https://www.alexbowe.com/rrr/)

RRR provides compression alongside rank/select:
- Divides bitmap into fixed-size blocks
- Encodes each block as (class, offset) pair
- Class = popcount of block; offset = which pattern with that popcount
- Lookup tables for small block rank queries

Best for: Sparse or dense bitmaps where compression matters.

## Broadword/SWAR Select-in-Word

ARM lacks PDEP/PEXT, but the broadword (SWAR) approach from Vigna's paper works well on any 64-bit architecture.

### Key Constants

```rust
const L8: u64 = 0x0101_0101_0101_0101;  // 1 in each byte's LSB
const H8: u64 = 0x8080_8080_8080_8080;  // 1 in each byte's MSB
const L9: u64 = 0x0040_2010_0804_0201;  // For multiplication tricks
```

### Select-in-Word Algorithm

The algorithm finds the position of the k-th set bit in a 64-bit word without branching:

1. **Compute byte popcounts**: Use SWAR to count bits in each byte
2. **Compute prefix sums**: Parallel prefix sum of byte popcounts
3. **Locate target byte**: Compare prefix sums with k to find the byte
4. **Locate bit in byte**: Use similar technique within the target byte

```rust
/// Select the k-th set bit (0-indexed) in word x.
/// Returns bit position 0-63, or 64 if fewer than k+1 bits are set.
fn select_in_word(x: u64, k: u32) -> u32 {
    // Byte popcounts via SWAR
    let byte_counts = {
        let x = x - ((x >> 1) & 0x5555_5555_5555_5555);
        let x = (x & 0x3333_3333_3333_3333) + ((x >> 2) & 0x3333_3333_3333_3333);
        (x + (x >> 4)) & 0x0F0F_0F0F_0F0F_0F0F
    };

    // Prefix sum of byte counts (cumulative popcount per byte)
    let prefix = byte_counts.wrapping_mul(L8);

    // Find which byte contains the k-th bit
    let k_spread = (k as u64 + 1) * L8;  // Broadcast k+1 to all bytes
    let ge_mask = ((prefix | H8) - k_spread) & H8;  // Bytes where prefix >= k+1
    let byte_idx = (ge_mask.wrapping_mul(L8) >> 56) as u32;  // Count bytes

    // Position within the target byte
    let byte_offset = byte_idx * 8;
    let target_byte = ((x >> byte_offset) & 0xFF) as u8;
    let bits_before = if byte_idx == 0 { 0 } else {
        (prefix >> ((byte_idx - 1) * 8)) as u32 & 0xFF
    };
    let k_in_byte = k - bits_before;

    byte_offset + select_in_byte(target_byte, k_in_byte)
}

/// Select k-th bit in a byte (lookup table or computation)
fn select_in_byte(byte: u8, k: u32) -> u32 {
    SELECT_IN_BYTE_TABLE[(byte as usize) * 8 + (k as usize)]
}
```

### Alternative: CTZ Loop for Sparse Words

When the word is sparse (few bits set), a counting-trailing-zeros loop can be faster:

```rust
fn select_in_word_sparse(mut x: u64, mut k: u32) -> u32 {
    loop {
        let t = x.trailing_zeros();
        if k == 0 { return t; }
        k -= 1;
        x &= x - 1;  // Clear lowest set bit
    }
}
```

Use profiling to choose the threshold (typically k < 4 for sparse path).

## Sampled Select Implementation

For select queries on large bitvectors, use sampling to avoid linear scans:

### Structure

```rust
pub struct SampledSelect {
    /// Position of every SAMPLE_RATE-th 1-bit
    samples: Vec<u64>,
    /// Sample rate (e.g., 256 or 512)
    sample_rate: u32,
}
```

### Query Algorithm

1. **Jump to neighborhood**: `samples[k / sample_rate]` gives approximate position
2. **Scan blocks**: Accumulate popcount until reaching the target
3. **Select in word**: Use broadword select for the final word

```rust
fn select1(&self, bitvec: &BitVec, k: usize) -> usize {
    // 1. Jump using samples
    let sample_idx = k / self.sample_rate as usize;
    let mut pos = if sample_idx > 0 {
        self.samples[sample_idx - 1] as usize
    } else {
        0
    };
    let mut remaining = (k % self.sample_rate as usize) as u32;

    // 2. Scan 512-bit blocks
    while remaining > 0 {
        let block_pop = popcount_512(&bitvec.data[pos..pos + 64]);
        if block_pop <= remaining {
            remaining -= block_pop;
            pos += 512;
        } else {
            break;
        }
    }

    // 3. Scan 64-bit words within block
    let words = &bitvec.words[pos / 64..];
    for (i, &word) in words.iter().enumerate() {
        let pop = word.count_ones();
        if pop > remaining {
            return pos + i * 64 + select_in_word(word, remaining) as usize;
        }
        remaining -= pop;
    }

    unreachable!()
}
```

### Sample Rate Trade-offs

| Sample Rate | Space Overhead | Select Speed | Notes                  |
|-------------|----------------|--------------|------------------------|
| 64          | ~12.5%         | Very fast    | More memory, less scan |
| 256         | ~3%            | Fast         | Good balance           |
| 512         | ~1.5%          | Moderate     | Space-efficient        |
| 1024        | ~0.8%          | Slower       | Minimal overhead       |

## Rust Libraries for ARM Rank/Select

| Crate           | ARM Support | SIMD         | Notes                                      |
|-----------------|-------------|--------------|--------------------------------------------|
| vers-vecs       | Yes (pure)  | x86 optional | Fastest pure-Rust; no ARM-specific opts    |
| indexed-bitvec  | Yes (pure)  | None         | Based on Zhou-Andersen-Kaminsky paper      |
| succinct        | Yes (pure)  | None         | O(lg lg n) select via binary search        |
| sdsl (bindings) | Partial     | Via C++      | Wraps C++ sdsl-lite; vgteam fork for ARM   |
| rust-bio        | Yes (pure)  | None         | Bioinformatics focus; succinct rank/select |

### vers-vecs (Recommended)

[lib.rs/crates/vers-vecs](https://lib.rs/crates/vers-vecs)

- Pure-Rust implementation with optional x86 SIMD
- Among the fastest publicly available implementations
- Lower memory overhead than competitors
- Uses broadword techniques that work well on ARM's 64-bit registers

```rust
use vers_vecs::BitVec;

let bv = BitVec::from_bits(&[true, false, true, true, false]);
let rank = bv.rank1(3);  // Count 1s in positions 0..=3
let pos = bv.select1(2); // Position of 2nd 1-bit
```

### indexed-bitvec

[github.com/DarkOtter/indexed-bitvec-rs](https://github.com/DarkOtter/indexed-bitvec-rs)

Based on the Zhou-Andersen-Kaminsky paper (Poppy). Pure Rust, no SIMD dependencies.

### sdsl-lite (via bindings or vgteam fork)

[github.com/vgteam/sdsl-lite](https://github.com/vgteam/sdsl-lite)

The vgteam fork includes proper 64-bit ARM support. Build with `BUILD_PORTABLE=1` to avoid x86-specific instructions.

## ARM Rank/Select Strategy for Rust Port

### Recommended Approach

1. **Popcount**: Use `u64::count_ones()` intrinsic
   - Compiles to ARM `cnt` instruction or NEON `vcnt`
   - No manual SIMD required

2. **Rank structure**: Implement SPIDER or Poppy
   - Neither requires PDEP/PEXT
   - Cache-aware interleaving is the key optimization

3. **Select structure**: Use prediction-based approach (SPIDER) or sampling (CS-Poppy)
   - Predictions work on any architecture
   - Avoid linear scans in hot paths

4. **SIMD byte comparison**: Use NEON for parallel operations
   ```rust
   #[cfg(target_arch = "aarch64")]
   use std::arch::aarch64::*;

   unsafe {
       let a = vld1q_u8(ptr_a);
       let b = vdupq_n_u8(needle);
       let eq = vceqq_u8(a, b);  // 16-byte parallel compare
   }
   ```

5. **Future-proof**: Add SVE2 BDEP/BEXT path with `cfg` guards
   ```rust
   #[cfg(all(target_arch = "aarch64", target_feature = "sve2-bitperm"))]
   fn pext_native(value: u64, mask: u64) -> u64 {
       // Use SVE2 BEXT when available
   }

   #[cfg(not(all(target_arch = "aarch64", target_feature = "sve2-bitperm")))]
   fn pext_native(value: u64, mask: u64) -> u64 {
       // Software fallback (ZP7 algorithm)
   }
   ```

### Code Organization

```
src/
├── rank/
│   ├── mod.rs
│   ├── poppy.rs      # Poppy/CS-Poppy implementation
│   ├── spider.rs     # SPIDER implementation
│   └── rank9.rs      # Rank9 implementation
├── select/
│   ├── mod.rs
│   ├── binary.rs     # Binary search over ranks
│   ├── sampling.rs   # CS-Poppy sampling approach
│   └── predict.rs    # SPIDER prediction approach
├── bits/
│   ├── mod.rs
│   ├── popcount.rs   # Platform-optimized popcount
│   ├── pdep.rs       # PDEP with fallback
│   └── pext.rs       # PEXT with fallback
└── simd/
    ├── mod.rs
    ├── x86.rs        # AVX2/SSE4.2 implementations
    ├── neon.rs       # ARM NEON implementations
    └── sve2.rs       # ARM SVE2 implementations (optional)
```

## Rust Runtime Feature Detection

On AArch64, NEON is enabled by default on most targets, but runtime detection is still useful for optional features like SVE.

### NEON (Default on AArch64)

```rust
// NEON is always available on aarch64-unknown-linux-gnu
// and aarch64-apple-darwin, so no runtime check needed
#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn neon_popcount(data: &[u8]) -> u32 {
    // NEON implementation
}
```

### SVE/SVE2 (Optional)

```rust
#[cfg(target_arch = "aarch64")]
fn popcount_dispatch(data: &[u8]) -> u32 {
    // SVE2 has best performance but limited availability
    if std::arch::is_aarch64_feature_detected!("sve2") {
        unsafe { sve2_popcount(data) }
    } else if std::arch::is_aarch64_feature_detected!("sve") {
        unsafe { sve_popcount(data) }
    } else {
        // NEON fallback (always available)
        unsafe { neon_popcount(data) }
    }
}
```

### Performance Note

Runtime detection adds ~10ns overhead per call. For hot paths:
- Use `#[inline]` and let the compiler hoist checks out of loops
- Or use `target-cpu=native` at compile time to eliminate checks entirely

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

## Benchmark Plan

### Test Parameters

| Parameter      | Values                          |
|----------------|---------------------------------|
| Bitvector size | 10^6, 10^7, 10^8 bits           |
| Bit density    | 1%, 10%, 50%, 90%               |
| Query pattern  | Random, sequential, adversarial |
| Platforms      | M1/M2, Graviton3, x86 baseline  |

### Metrics

- **Throughput**: queries/second
- **Latency**: ns/query (p50, p99)
- **Cache behavior**: L1/L2 miss rate (via `perf` or `dtrace`)

### Benchmark Code Structure

```rust
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};

fn bench_rank(c: &mut Criterion) {
    let mut group = c.benchmark_group("rank");

    for size in [1_000_000, 10_000_000, 100_000_000] {
        for density in [0.01, 0.1, 0.5, 0.9] {
            let bv = generate_bitvec(size, density);
            let queries: Vec<usize> = (0..10000)
                .map(|_| rand::random::<usize>() % size)
                .collect();

            group.bench_with_input(
                BenchmarkId::new(format!("{}M/{:.0}%", size/1_000_000, density*100.0), ""),
                &(&bv, &queries),
                |b, (bv, queries)| {
                    b.iter(|| {
                        queries.iter().map(|&i| bv.rank1(i)).sum::<usize>()
                    })
                },
            );
        }
    }
    group.finish();
}
```

## Acceptance Criteria

### Correctness

- [ ] Property tests vs reference implementation (random bitvectors + random queries)
- [ ] Edge cases: empty vector, all-zeros, all-ones
- [ ] Boundary conditions: word/block/superblock edges
- [ ] Consistency: `select1(rank1(i)) == i` for positions with 1-bits

### Performance

- [ ] NEON path measurably faster than scalar on AArch64 (≥1.5x for large vectors)
- [ ] Select performance competitive with baselines (sampled jump + scan)
- [ ] No regressions on x86_64 compared to existing implementations

### Maintainability

- [ ] Single safe public API
- [ ] Unsafe isolated to small architecture-specific modules
- [ ] Clear `cfg`/feature gates and runtime dispatch
- [ ] Comprehensive documentation with examples

## Performance Reference: simdjson on ARM

The [simdjson](https://simdjson.org/) library demonstrates that ARM NEON can achieve competitive performance for SIMD-intensive workloads:

| Platform    | SIMD Width | Performance vs x86 AVX2 |
|-------------|------------|-------------------------|
| Apple A12   | 128-bit    | ~50-80% of Skylake      |
| Apple M1    | 128-bit    | Competitive with AVX2   |
| Graviton2/3 | 128-bit    | Near-parity with AVX2   |

The 128-bit NEON vector width (vs 256-bit AVX2) is offset by:
- ARM's efficient instruction dispatch
- Lower memory latency on modern ARM chips
- Better power efficiency (more cores available)

## Public API Proposal

```rust
/// Trait for rank/select operations on bitvectors
pub trait RankSelect {
    /// Count 1-bits in positions [0, i)
    fn rank1(&self, i: usize) -> usize;

    /// Count 0-bits in positions [0, i)
    fn rank0(&self, i: usize) -> usize {
        i - self.rank1(i)
    }

    /// Position of the k-th 1-bit (0-indexed)
    /// Returns None if fewer than k+1 ones exist
    fn select1(&self, k: usize) -> Option<usize>;

    /// Position of the k-th 0-bit (0-indexed)
    fn select0(&self, k: usize) -> Option<usize>;
}

/// Bitvector with rank/select support
pub struct BitVecRankSelect {
    words: Vec<u64>,
    len: usize,
    // Rank directory (superblocks + subblocks)
    rank_directory: RankDirectory,
    // Optional select samples
    select_samples: Option<SelectSamples>,
}

impl BitVecRankSelect {
    /// Create from raw u64 words
    pub fn from_words(words: Vec<u64>, len: usize) -> Self;

    /// Create from bytes
    pub fn from_bytes(bytes: &[u8]) -> Self;

    /// Create from iterator of bits
    pub fn from_bits(bits: impl Iterator<Item = bool>) -> Self;

    /// Build rank index (required for rank queries)
    pub fn build_rank_index(&mut self);

    /// Build select index with given sample rate
    pub fn build_select_index(&mut self, sample_rate: u32);

    /// Number of bits
    pub fn len(&self) -> usize;

    /// Access bit at position i
    pub fn get(&self, i: usize) -> bool;
}
```

## Implementation Tasks

- [ ] Add rank/select module with baseline scalar rank9-like structure
- [ ] Add NEON popcount512 primitive with dispatch
- [ ] Implement sampled select + SWAR select-in-u64
- [ ] Add criterion benchmarks
- [ ] Add correctness property tests (proptest/quickcheck)
- [ ] Evaluate layout tweaks (block size, directory packing)
- [ ] Optional: SPIDER-style metadata interleaving
- [ ] Optional: Prediction-based select (SPIDER)
- [ ] Optional: SVE2 BDEP/BEXT path for future hardware

## References

### Papers

- **SPIDER (2024)**: Laws, M.D. "SPIDER: Improved Succinct Rank and Select Performance" [arxiv.org/abs/2405.05214](https://arxiv.org/abs/2405.05214)
- **Poppy (2013)**: Zhou, D., Andersen, D.G., Kaminsky, M. "Space-Efficient, High-Performance Rank & Select Structures on Uncompressed Bit Sequences" [PDF](https://www.cs.cmu.edu/~dga/papers/zhou-sea2013.pdf)
- **Rank9**: Vigna, S. "Broadword Implementation of Rank/Select Queries"
- **RRR**: Raman, R., Raman, V., Rao, S.S. "Succinct Indexable Dictionaries"

### Libraries and Tools

- **ZP7**: [github.com/zwegner/zp7](https://github.com/zwegner/zp7) - PEXT/PDEP polyfill
- **vers-vecs**: [lib.rs/crates/vers-vecs](https://lib.rs/crates/vers-vecs) - Rust rank/select
- **sdsl-lite**: [github.com/simongog/sdsl-lite](https://github.com/simongog/sdsl-lite) - C++ succinct library
- **libpopcnt**: [github.com/kimwalisch/libpopcnt](https://github.com/kimwalisch/libpopcnt) - Fast popcount
- **simdjson**: [simdjson.org](https://simdjson.org/) - SIMD JSON parser (ARM NEON reference)

### Architecture References

- **PDEP/PEXT across architectures**: [gist.github.com/Validark/9455d410ccc8e4bfd1c1c5c4fa38f934](https://gist.github.com/Validark/9455d410ccc8e4bfd1c1c5c4fa38f934)
- **ARM NEON intrinsics**: [developer.arm.com/architectures/neon](https://developer.arm.com/Architectures/Neon)
- **ARM NEON intrinsics reference**: [arm-software.github.io/acle/neon_intrinsics](https://arm-software.github.io/acle/neon_intrinsics/advsimd.html)
- **ARM SVE2 introduction**: [developer.arm.com/documentation/102340](https://developer.arm.com/documentation/102340/latest/)
- **Rust AArch64 intrinsics**: [doc.rust-lang.org/core/arch/aarch64](https://doc.rust-lang.org/core/arch/aarch64/index.html)
- **ARM NEON in Rust blog**: [developer.arm.com/.../rust-neon-intrinsics](https://developer.arm.com/community/arm-community-blogs/b/architectures-and-processors-blog/posts/rust-neon-intrinsics)

### Rust NEON Intrinsic Examples

- `vld1q_u8`: [doc.rust-lang.org/core/arch/aarch64/fn.vld1q_u8.html](https://doc.rust-lang.org/core/arch/aarch64/fn.vld1q_u8.html)
- `vcntq_u8`: [doc.rust-lang.org/core/arch/aarch64/fn.vcntq_u8.html](https://doc.rust-lang.org/core/arch/aarch64/fn.vcntq_u8.html)
- `vaddvq_u8`: [doc.rust-lang.org/core/arch/aarch64/fn.vaddvq_u8.html](https://doc.rust-lang.org/beta/core/arch/aarch64/fn.vaddvq_u8.html)
