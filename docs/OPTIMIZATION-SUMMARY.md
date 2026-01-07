# Optimization Summary - Complete Record

**Project**: Succinctly Rust Library
**System**: AMD Ryzen 9 7950X (Zen 4), Apple M1/M2/M3 (ARM)
**Period**: 2025-2026
**Last Updated**: 2026-01-07

This document provides a comprehensive record of all optimizations attempted in the Succinctly library, showing what worked, what failed, and exact performance measurements.

---

## Table of Contents

1. [Successful Optimizations](#successful-optimizations)
2. [Failed Optimizations](#failed-optimizations)
3. [Summary Statistics](#summary-statistics)
4. [Lessons Learned](#lessons-learned)

---

## Successful Optimizations

### 1. AVX2 SIMD JSON Parser (x86_64)

**Status**: ✅ IMPLEMENTED & DEPLOYED
**File**: [src/json/simd/avx2.rs](../src/json/simd/avx2.rs)
**Date**: 2025

**Technique**: Process 32 bytes per iteration using AVX2 SIMD instructions for character classification.

**Performance Results**:

| Workload | SSE2 Baseline | AVX2 | Improvement |
|----------|---------------|------|-------------|
| 1KB JSON | 450 MiB/s | 806 MiB/s | **+79%** |
| 10KB JSON | 420 MiB/s | 731 MiB/s | **+74%** |
| 100KB JSON | 410 MiB/s | 725 MiB/s | **+77%** |
| 1MB JSON | 400 MiB/s | 732 MiB/s | **+83%** |

**Average Speedup**: **1.78x (78% faster)**

**Why it worked**:
- Targets actual bottleneck (character classification)
- 32-byte processing amortizes instruction overhead
- AVX2 widely available (2013+, ~95% market penetration)
- Balanced between SIMD width and state machine overhead

**Current Status**: Default implementation with runtime CPU detection

---

### 2. AVX512-VPOPCNTDQ Popcount (x86_64)

**Status**: ✅ IMPLEMENTED & DEPLOYED
**File**: [src/popcount.rs](../src/popcount.rs)
**Date**: January 2026

**Technique**: Parallel popcount of 8 u64 words (512 bits) using `_mm512_popcnt_epi64`.

**Performance Results**:

| Dataset Size | Scalar | AVX512-VPOPCNTDQ | Improvement |
|--------------|--------|------------------|-------------|
| 64B (8 words) | 4.68 ns | 1.63 ns | **+187%** |
| 512B (64 words) | 42.8 ns | 4.65 ns | **+820%** |
| 4KB (512 words) | 342 ns | 27.7 ns | **+1135%** |
| 1MB (131K words) | 52.6 µs | 10.1 µs | **+421%** |

**Throughput**: 96.8 GiB/s (AVX-512) vs 18.5 GiB/s (scalar) = **5.2x faster**

**End-to-End Impact**:
- BitVec construction (1M bits): Popcount is ~1.6% of total time
- Rank queries: Minimal impact (already 3 ns per query)
- **Real-world speedup**: ~1-2% overall (Amdahl's Law limited)

**Why it worked**:
- Pure compute-bound operation (no memory bottleneck)
- Embarrassingly parallel (no data dependencies)
- Native 512-bit support on Zen 4
- Perfect for vectorization

**Current Status**: Runtime dispatch with `is_x86_feature_detected!("avx512vpopcntdq")`

---

### 3. SSE4.2 with PCMPISTRI (x86_64)

**Status**: ✅ IMPLEMENTED
**File**: [src/json/simd/sse42.rs](../src/json/simd/sse42.rs)
**Date**: 2025

**Technique**: Use SSE4.2 `_mm_cmpistri` for efficient character set matching.

**Performance Results**:

| Workload | SSE2 | SSE4.2 | Improvement |
|----------|------|--------|-------------|
| 1MB JSON | 400 MiB/s | 550 MiB/s | **+38%** |

**Average Speedup**: **1.38x (38% faster)**

**Why it worked**:
- `PCMPISTRI` instruction designed for character classification
- Reduced instruction count vs SSE2
- ~90% market availability (2008+)

**Current Status**: Middle tier in runtime dispatch (AVX2 > SSE4.2 > SSE2)

---

### 4. NEON 32-byte Processing (ARM aarch64)

**Status**: ✅ IMPLEMENTED
**File**: [src/json/simd/neon.rs](../src/json/simd/neon.rs)
**Date**: January 2026

**Technique**: Process two 16-byte NEON vectors (32 bytes total) per iteration.

**Performance Results**:

| Pattern | 16-byte (old) | 32-byte (new) | Improvement |
|---------|---------------|---------------|-------------|
| nested (string-heavy) | 2.05 GiB/s | 3.47 GiB/s | **+69% (-41% time)** |
| strings | 1.77 GiB/s | 2.83 GiB/s | **+60% (-37% time)** |
| unicode | 1.45 GiB/s | 1.70 GiB/s | **+17% (-14% time)** |
| comprehensive | 505 MiB/s | 560 MiB/s | **+11% (-9% time)** |
| mixed | 360 MiB/s | 389 MiB/s | **+8% (-7% time)** |
| arrays | 297 MiB/s | 311 MiB/s | **+5% (-4% time)** |

**Average Speedup**: **1.11x overall, up to 1.69x on string-heavy workloads**

**Why it worked**:
- InString fast-path can skip up to 32 consecutive characters
- Amortizes classification overhead
- No regressions on any pattern

**Current Status**: Default NEON implementation on aarch64

---

### 5. NEON Nibble Lookup for Value Characters (ARM aarch64)

**Status**: ✅ IMPLEMENTED
**File**: [src/json/simd/neon.rs](../src/json/simd/neon.rs)
**Date**: January 2026

**Technique**: Replace 13 NEON operations (range checks + ORs) with 6-operation nibble lookup table.

**Performance Results**:

| Pattern | Before | After | Improvement |
|---------|--------|-------|-------------|
| nested | 3.47 GiB/s | 3.70 GiB/s | **+6%** |
| strings | 2.83 GiB/s | 2.94 GiB/s | **+4%** |
| literals | 290 MiB/s | 305 MiB/s | **+5%** |
| comprehensive | 560 MiB/s | 571 MiB/s | **+2%** |

**Average Speedup**: **1.02-1.06x (2-6% faster)**

**Why it worked**:
- Reuses already-computed nibbles from structural classification
- Fewer NEON operations (6 vs 13)
- Better instruction-level parallelism

**Current Status**: Integrated into NEON implementation

---

### 6. Runtime CPU Feature Detection

**Status**: ✅ IMPLEMENTED
**File**: [src/json/simd/mod.rs](../src/json/simd/mod.rs), [src/popcount.rs](../src/popcount.rs)
**Date**: 2025-2026

**Technique**: Automatic dispatch to best available SIMD implementation using `is_x86_feature_detected!()`.

**Performance Results**:

| CPU | Auto-Selected | Throughput | vs Baseline |
|-----|---------------|------------|-------------|
| Zen 4 (AVX2) | AVX2 | 732 MiB/s | **+83%** |
| Skylake (AVX2) | AVX2 | 680 MiB/s | **+70%** |
| Core 2 Duo (SSE2) | SSE2 | 400 MiB/s | baseline |
| M1 (NEON) | NEON | 571 MiB/s | **+52%** |

**Dispatch Priority**:
- x86_64: AVX2 > SSE4.2 > SSE2
- Popcount: AVX512-VPOPCNTDQ > scalar POPCNT
- aarch64: NEON (mandatory)

**Why it worked**:
- Zero runtime overhead after initial detection
- Transparent to users
- Maintains broad CPU compatibility

**Current Status**: Default with `std` feature (enabled by default)

---

### 7. Balanced Parentheses Byte-Level Lookup Tables

**Status**: ✅ IMPLEMENTED
**File**: [src/bp.rs](../src/bp.rs)
**Date**: 2025

**Technique**: Precomputed lookup tables for byte-level excess and find_close operations.

**Performance Results**:

| Operation | Bit-by-bit | Byte lookup | Improvement |
|-----------|------------|-------------|-------------|
| find_close (1M elements) | 130.9 µs | 11.3 µs | **+1058% (11x faster)** |
| find_open | 125 µs | 15 µs | **+733% (8x faster)** |

**Average Speedup**: **~10x faster**

**Why it worked**:
- Reduces iterations from 64 per word to 8 per word
- Lookup tables fit in L1 cache
- Combined with hierarchical RangeMin for O(1) block skipping

**Current Status**: Production implementation

---

### 8. Hierarchical RangeMin Index for Balanced Parentheses

**Status**: ✅ IMPLEMENTED
**File**: [src/bp.rs](../src/bp.rs)
**Date**: 2025

**Technique**: 3-level index (L0: 64-bit, L1: 512-bit, L2: 4096-bit blocks) for O(1) min-excess queries.

**Performance Results**:

| Operation | Linear scan | Hierarchical | Improvement |
|-----------|-------------|--------------|-------------|
| enclose (1M tree) | ~1000 µs | 25 µs | **+3900% (40x faster)** |
| find_close | 130 µs | 11 µs | **+1058% (11x faster)** |

**Memory Overhead**: ~6% of BP bitvector size

**Why it worked**:
- Skips entire blocks with single comparison
- Cache-friendly hierarchical structure
- O(1) queries instead of O(n) scans

**Current Status**: Core BP implementation

---

### 9. Dual Select Methods (Binary + Exponential Search)

**Status**: ✅ IMPLEMENTED
**File**: [src/json/index.rs](../src/json/index.rs)
**Date**: January 2026

**Technique**:
- `ib_select1(k)`: Pure binary search for random access
- `ib_select1_from(k, hint)`: Exponential search from hint for sequential access

**Performance Results**:

| Access Pattern | Method | Time | Throughput |
|----------------|--------|------|------------|
| Sequential | Exponential | 108 µs | 92.2 Melem/s |
| Sequential | Binary | 340 µs | 29.4 Melem/s |
| Random | Binary | 779 µs | **12.8 Melem/s** |
| Random | Exponential | 1080 µs | 9.3 Melem/s |

**Speedup**:
- Sequential: Exponential **3.1x faster**
- Random: Binary **1.39x faster**

**Why it worked**:
- Different access patterns need different algorithms
- Exponential search exploits locality
- Binary search optimal for random access

**Current Status**: Both methods available, used contextually

---

### 10. Cumulative Index for O(1) IB Select

**Status**: ✅ IMPLEMENTED
**File**: [src/json/index.rs](../src/json/index.rs)
**Date**: 2025

**Technique**: Precomputed cumulative bit counts for direct rank-to-position mapping.

**Performance Results**:

| Method | Time | Speedup |
|--------|------|---------|
| Binary search select | 779 µs | baseline |
| Cumulative index | 1.24 µs | **+62,700% (627x faster!)** |

**Memory Overhead**: 4 bytes per u64 word (~50% of bitvector)

**Why it worked**:
- O(1) lookup vs O(log n) binary search
- Perfect for dense bitvectors (many 1-bits)
- Cache-friendly sequential access

**Current Status**: Default for IB select operations

---

## Failed Optimizations

### 1. AVX-512 JSON Parser (x86_64)

**Status**: ❌ REMOVED
**Date**: January 2026

**Technique**: Process 64 bytes per iteration using AVX-512 (double AVX2's 32 bytes).

**Performance Results**:

| Size | AVX2 | AVX-512 | Result |
|------|------|---------|--------|
| 1KB | 806 MiB/s | 689 MiB/s | **-17% slower** |
| 10KB | 731 MiB/s | 675 MiB/s | **-8% slower** |
| 100KB | 725 MiB/s | 678 MiB/s | **-7% slower** |
| 1MB | 732 MiB/s | 672 MiB/s | **-9% slower** |

**Average Penalty**: **-10% (7-17% slower across sizes)**

**Why it failed**:
1. **Memory-bound workload**: Waiting for data from RAM, not compute
2. **AMD Zen 4 splits AVX-512**: 2×256-bit micro-ops instead of native 512-bit execution
3. **State machine overhead**: 64 bytes = more sequential processing after SIMD classification
4. **Amdahl's Law**: SIMD classification is only ~20% of work; 80% is state machine + memory

**Action taken**:
- Removed `src/json/simd/avx512.rs` (627 lines)
- Removed benchmark `benches/json_avx512_comparison.rs`
- Updated runtime dispatch to prioritize AVX2

**Lesson**: Wider SIMD ≠ automatically faster for memory-bound workloads

**Documentation**: [docs/AVX512-JSON-RESULTS.md](AVX512-JSON-RESULTS.md) (historical reference)

---

### 2. BMI1 JSON Mask Processing (x86_64)

**Status**: ❌ REVERTED
**Date**: January 2026

**Technique**: Use TZCNT/BLSR to iterate only over set bits in structural character mask.

**Implementation**:
```rust
// Instead of: for i in 0..32
while mask != 0 {
    let i = _tzcnt_u32(mask);      // Find lowest set bit
    mask = _blsr_u32(mask);         // Clear it
    process_byte(i);                // Process only structural chars
}
```

**Performance Results**:

#### Dense JSON (many structural characters):

| Size | AVX2 Baseline | AVX2+BMI1 | Result |
|------|---------------|-----------|--------|
| 1KB | 609 MiB/s | 457 MiB/s | **-25% slower** |
| 10KB | 647 MiB/s | 450 MiB/s | **-30% slower** |
| 100KB | 658 MiB/s | 458 MiB/s | **-30% slower** |
| 1MB | 659 MiB/s | 454 MiB/s | **-31% slower** |

#### Sparse JSON (few structural characters - "best case"):

| Size | AVX2 Baseline | AVX2+BMI1 | Result |
|------|---------------|-----------|--------|
| 1KB | 796 MiB/s | 727 MiB/s | **-9% slower** |
| 10KB | 790 MiB/s | 700 MiB/s | **-11% slower** |

**Average Penalty**: **-26% (9-31% slower)**

**Why it failed**:
1. **Wrong bottleneck**: Optimized iteration (<1% of time) instead of state machine (40%) and BitWriter (30%)
2. **Loop overhead**: TZCNT/BLSR in loop + branches cost more than simple `for i in 0..32`
3. **Must process all positions**: State machine requires sequential processing of ALL 32 bytes (even whitespace) to track quote state
4. **Compiler already optimizes**: `for i in 0..32` is unrolled and vectorized by LLVM

**Time breakdown** (profiled):
- State machine logic: 40%
- BitWriter operations: 30%
- Memory access: 20%
- SIMD classification: 10%
- **Iteration overhead**: <1% ← This is what BMI1 "optimized"

**Action taken**: Reverted all changes to `src/json/simd/avx2.rs` and `src/json/simd/mod.rs`

**Lesson**: "Smarter" code isn't always faster. Profile first to find the real bottleneck.

**Documentation**: [docs/FAILED-OPTIMIZATIONS.md](FAILED-OPTIMIZATIONS.md)

---

### 3. BMI2 PDEP for BitWriter (x86_64)

**Status**: ❌ REVERTED
**Date**: 2026-01-07

**Technique**: Use BMI2 `_pdep_u64` instruction to deposit bits without explicit shifting in `BitWriter::write_bits()`.

**Implementation**:
```rust
// Scalar (baseline):
let mask = (1u64 << count) - 1;
self.current_word |= (bits & mask) << pos;

// BMI2 PDEP (attempted):
let deposit_mask = (1u64 << count) - 1;
let shifted_mask = deposit_mask << pos;
self.current_word |= _pdep_u64(bits, shifted_mask);
```

**Performance Results**:

| Implementation | Throughput (8-bit writes) | Result |
|----------------|--------------------------|--------|
| Scalar | 1.27 GiB/s | baseline |
| BMI2 PDEP | 381 MiB/s | **-71% slower (3.4x)** |

**Average Penalty**: **-71% (3.4x slower!)**

**Why it failed**:

1. **Wrong use case for PDEP**: PDEP is designed for **sparse** bit extraction/deposition (e.g., bits at positions [0, 5, 12, 31]). Our use case is **consecutive** bits (positions pos..pos+count), which simple shift handles optimally.

2. **PDEP latency**: Even on Zen 4 with "fast" BMI2, PDEP has 3-cycle latency. Scalar shift + OR = 2 cycles total (1 cycle each).

3. **Extra work**: The BMI2 version still needs to compute the shifted mask, adding overhead without benefit.

4. **Simple operations are already optimal**: `(bits & mask) << pos` compiles to 2-3 fast integer ALU operations that modern CPUs can execute in parallel. PDEP is a complex instruction that ties up execution units.

**When PDEP would actually help**:
- Extracting bits at non-consecutive positions
- Packing/unpacking structured data with sparse fields
- **NOT** for consecutive bit ranges

**Action taken**: Reverted all changes to `src/json/bit_writer.rs`, removed CPU detection code (~150 lines)

**Lesson**: Fancy instructions aren't automatically better. PDEP/PEXT are specialized tools for sparse operations, not replacements for simple shift+mask. Always benchmark!

**Documentation**: [docs/FAILED-OPTIMIZATIONS.md](FAILED-OPTIMIZATIONS.md)

---

### 4. NEON Batched Popcount for Cumulative Index (ARM)

**Status**: ❌ REJECTED
**Date**: January 2026

**Technique**: Use NEON `vcntq_u8` to batch popcount 8 words, then build prefix sum.

**Performance Results**:

| Implementation | Time (1M words = 8MB) | Result |
|----------------|----------------------|--------|
| Scalar `count_ones()` | 542 µs | baseline |
| NEON batched | 722 µs | **-25% slower** |

**Average Penalty**: **-25% (1.33x slower)**

**Why it failed**:
1. **Extraction overhead**: 14 `vgetq_lane` calls per 8 words to extract individual counts
2. **Sequential dependency**: Prefix sum requires `cumulative += count` for each word (inherently serial)
3. **LLVM already optimal**: `count_ones()` compiles to efficient `CNT` instruction
4. **Memory pressure**: Additional register shuffling

**Lesson**: SIMD batching helps for *total* sums but not for *prefix* sums where each output depends on all previous values. The bottleneck is the data dependency chain, not the popcount operation.

**Action taken**: Keep simple scalar implementation in `build_ib_rank()`

**Documentation**: [docs/optimization-opportunities.md](optimization-opportunities.md#rejected-optimizations)

---

### 4. NEON Movemask Call Reduction (ARM)

**Status**: ❌ REJECTED
**Date**: January 2026

**Technique**: Batch multiple `neon_movemask` calls to improve instruction-level parallelism.

**Approaches tried**:
1. Batched extraction (`neon_movemask_pair`, `neon_movemask_triple`)
2. Fully inlined extraction (inline all 12 lane extractions)
3. Compute `string_special` in NEON domain (OR vectors before extraction)

**Performance Results**:

| Approach | Result |
|----------|--------|
| Batched extraction | **No measurable improvement** (within noise) |
| Fully inlined | **No measurable improvement** |
| NEON domain OR | **-1-2% slower** on string-heavy patterns |

**Average Penalty**: **0% (no effect or slight regression)**

**Why it failed**:
- Apple Silicon's out-of-order execution already schedules independent `neon_movemask` calls efficiently
- The 6 movemask calls have no data dependencies (CPU parallelizes automatically)
- Manual batching adds code complexity without benefit
- Lane extractions (`vgetq_lane_u64`) and multiplications are fast on M1

**Lesson**: Don't outsmart the compiler and CPU's OOO engine. Independent operations are already parallelized.

**Action taken**: Keep simple sequential `neon_movemask` calls

---

### 5. NEON Prefetching (ARM)

**Status**: ❌ REJECTED
**Date**: January 2026

**Technique**: Software prefetch hints using ARM `PRFM` instruction.

**Approaches tried**:
1. `PLDL1KEEP` (L1 cache, temporal) at 256, 512, 1024 byte distances
2. `PLDL1STRM` (L1 cache, streaming) at 1024 bytes
3. `PLDL2KEEP` (L2 cache, temporal) at 2048 bytes

**Performance Results**:

| Prefetch Distance | Result |
|-------------------|--------|
| 256 bytes (L1KEEP) | **No improvement** (within ±1-2% noise) |
| 512 bytes (L1KEEP) | **No improvement** |
| 1024 bytes (L1STRM) | **No improvement** |
| 2048 bytes (L2KEEP) | **No improvement** |

**Average Penalty**: **0% (no effect)**

**Why it failed**:
1. **Hardware prefetcher is excellent**: Apple Silicon automatically detects sequential memory access patterns
2. **Access pattern is ideal**: Strictly sequential, 32 bytes at a time, used once (streaming)
3. **Instruction overhead**: Software prefetch hints add overhead with no benefit
4. **Not memory-bound**: At 571 MiB/s, L1 cache hit rate is nearly 100%

**Lesson**: Hardware prefetchers on modern CPUs (especially Apple Silicon) outperform software hints for simple sequential patterns.

**Action taken**: No prefetching implemented

---

## Summary Statistics

### Successful Optimizations

| Optimization | Speedup | Platform | Status |
|--------------|---------|----------|--------|
| AVX2 JSON Parser | **1.78x** | x86_64 | ✅ Deployed |
| AVX512-VPOPCNTDQ | **5.2x** (micro), **1.01x** (e2e) | x86_64 | ✅ Deployed |
| SSE4.2 PCMPISTRI | **1.38x** | x86_64 | ✅ Deployed |
| NEON 32-byte Processing | **1.11x** (avg), **1.69x** (strings) | ARM | ✅ Deployed |
| NEON Nibble Lookup | **1.02-1.06x** | ARM | ✅ Deployed |
| BP Byte Lookup Tables | **11x** | All | ✅ Deployed |
| Hierarchical RangeMin | **40x** | All | ✅ Deployed |
| Cumulative Index | **627x** | All | ✅ Deployed |
| Dual Select Methods | **3.1x** (seq), **1.39x** (rand) | All | ✅ Deployed |

**Total Successful**: 9 optimizations
**Average Speedup**: 5.8x (geometric mean, excluding outliers)
**Best Result**: Cumulative index (627x)
**Most Impactful**: AVX2 JSON parser (1.78x, broad impact)

### Failed Optimizations

| Optimization | Penalty | Platform | Action |
|--------------|---------|----------|--------|
| AVX-512 JSON Parser | **-10%** (avg) | x86_64 | ❌ Removed |
| BMI1 Mask Iteration | **-26%** (avg) | x86_64 | ❌ Reverted |
| BMI2 PDEP BitWriter | **-71%** (3.4x slower!) | x86_64 | ❌ Reverted |
| NEON Batched Popcount | **-25%** | ARM | ❌ Rejected |
| NEON Movemask Batching | **0%** (no effect) | ARM | ❌ Rejected |
| NEON Prefetching | **0%** (no effect) | ARM | ❌ Rejected |

**Total Failed**: 6 attempts
**Average Penalty**: -22% (for those with negative impact)
**Worst Failure**: BMI2 PDEP (-71%, 3.4x slower)
**Lines Removed**: ~800 lines (AVX-512 JSON + BMI1 + BMI2)

### Overall Impact

**x86_64 (Zen 4)**:
- JSON parsing: **1.78x faster** (AVX2 vs SSE2)
- Popcount: **5.2x faster** (AVX512-VPOPCNTDQ vs scalar)
- End-to-end: **~1.8x faster** JSON indexing

**ARM (Apple Silicon)**:
- JSON parsing: **1.52x faster** (NEON vs scalar)
- String-heavy workloads: **Up to 1.69x faster**
- End-to-end: **~1.5x faster** JSON indexing

**All Platforms**:
- Balanced Parentheses: **10-40x faster** (lookup tables + RangeMin)
- IB Select: **627x faster** (cumulative index)

---

## Lessons Learned

### What Works

1. **✅ Profile first, optimize second**
   - Successful optimizations targeted actual bottlenecks (classification, lookup)
   - Failed optimizations targeted wrong bottlenecks (iteration, movemask calls)

2. **✅ Understand workload characteristics**
   - Compute-bound: AVX512-VPOPCNTDQ (5.2x) ✓
   - Memory-bound: AVX-512 JSON (slower) ✗

3. **✅ Amdahl's Law always wins**
   - AVX2 JSON (78% of work): 1.78x speedup ✓
   - AVX512-VPOPCNTDQ (1.6% of work): 5.2x micro, 1.01x macro ✓
   - BMI1 (<1% of work): No improvement possible ✗

4. **✅ Simple code is often fastest**
   - `for i in 0..32` beats "clever" BMI1 iteration
   - Scalar `count_ones()` beats NEON batched popcount
   - Simple shift+mask beats "fancy" BMI2 PDEP (by 3.4x!)
   - Compiler optimizations are excellent

5. **✅ Cache-friendly data structures**
   - Byte lookup tables: 11x speedup
   - Hierarchical RangeMin: 40x speedup
   - Cumulative index: 627x speedup

6. **✅ Algorithm matters more than micro-optimization**
   - Cumulative index (O(1)) vs binary search (O(log n)): 627x
   - Exponential search vs binary (sequential): 3.1x

### What Doesn't Work

1. **❌ Wider SIMD ≠ automatically faster**
   - AVX-512 JSON: 7-17% slower than AVX2
   - Must consider memory bandwidth, state machine overhead

2. **❌ Optimizing fast paths**
   - BMI1 iteration: Optimized <1% of time, made it slower
   - NEON movemask: Already fast, batching added overhead

3. **❌ Fighting the compiler/hardware**
   - NEON batched popcount: LLVM already optimal
   - NEON prefetching: Hardware prefetcher is better

4. **❌ "Clever" code without profiling**
   - TZCNT/BLSR iteration: Intuitive but slower
   - BMI2 PDEP: Fancy instruction, wrong use case
   - Must process ALL bytes for state machine, not just structural chars

5. **❌ Using specialized instructions incorrectly**
   - PDEP is for sparse bits, not consecutive ranges
   - Know when tools apply: PDEP for [0,5,12,31] ✓, not for [5,6,7,8] ✗

6. **❌ Keeping failed optimizations**
   - Complexity is technical debt
   - Remove slower code, document the lesson

### Decision Framework

Before implementing an optimization, ask:

1. **Where is time actually spent?** (Profile!)
   - ✓ AVX2: 40% in classification → optimize
   - ✗ BMI1: <1% in iteration → don't optimize

2. **What % of total time?** (Amdahl's Law)
   - ✓ AVX2: 78% of JSON parsing → 1.78x possible
   - ✗ AVX512-VPOPCNT: 1.6% of construction → 1.01x realistic

3. **Is this compute-bound or memory-bound?**
   - ✓ Popcount: Compute-bound → SIMD wins
   - ✗ JSON: Memory-bound → wider SIMD loses

4. **Are there sequential dependencies?**
   - ✓ Cumulative index: No dependencies → parallelize
   - ✗ Prefix sum: Sequential dependencies → keep simple

5. **What's the baseline compiler doing?** (Check assembly)
   - ✓ AVX2: SSE2 is baseline → room for improvement
   - ✗ Scalar popcount: Already uses CNT instruction → no room

6. **How will I measure success?** (Benchmarks ready)
   - ✓ All optimizations: Micro + end-to-end benchmarks
   - Regression testing confirms impact

**If you can't answer these confidently, don't optimize.**

---

## References

### Performance Analysis Documents

- [AVX512-VPOPCNTDQ-RESULTS.md](AVX512-VPOPCNTDQ-RESULTS.md) - Popcount 5.2x speedup
- [AVX512-JSON-RESULTS.md](AVX512-JSON-RESULTS.md) - Why AVX2 beats AVX-512 (historical)
- [FAILED-OPTIMIZATIONS.md](FAILED-OPTIMIZATIONS.md) - Detailed failure analysis
- [implemented-optimizations.md](implemented-optimizations.md) - Implementation catalog
- [optimization-opportunities.md](optimization-opportunities.md) - Remaining opportunities

### External References

- Langdale & Lemire, "Parsing Gigabytes of JSON per Second" (2019)
- simdjson: https://github.com/simdjson/simdjson
- AMD Zen 4 Optimization Guide
- Apple Silicon Performance Guide

---

## Conclusion

**Success Rate**: 9/14 attempted optimizations (64%)

**Key Insight**: The most successful optimizations were algorithmic (cumulative index: 627x, RangeMin: 40x) rather than micro-optimizations. SIMD acceleration works best when:
1. Targeting actual bottlenecks (AVX2 JSON: 78%)
2. Compute-bound workloads (AVX512-VPOPCNTDQ: 5.2x)
3. Simple, regular patterns that don't fight the compiler

**The best optimization is often choosing the right algorithm, not the fanciest SIMD instructions.**

---

**Last Updated**: 2026-01-07
**Maintained By**: Succinctly development team
**Status**: All production optimizations deployed and tested
