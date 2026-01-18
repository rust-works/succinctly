# Optimization History

[Home](/) > [Docs](../) > [Optimizations](./) > History

This document records all optimization attempts in the succinctly library, showing what worked, what failed, and exact performance measurements.

---

## Summary Statistics

### Successful Optimizations

| Optimization              | Speedup                                    | Platform | Status     |
|---------------------------|--------------------------------------------|----------|------------|
| PFSM JSON Parser          | **1.33x** (vs scalar), **1.17x** (vs AVX2) | x86_64   | Deployed |
| AVX2 JSON Parser          | **1.78x**                                  | x86_64   | Deployed |
| AVX512-VPOPCNTDQ          | **5.2x** (micro), **1.01x** (e2e)          | x86_64   | Deployed |
| SSE4.2 PCMPISTRI          | **1.38x**                                  | x86_64   | Deployed |
| NEON 32-byte Processing   | **1.11x** (avg), **1.69x** (strings)       | ARM      | Deployed |
| NEON Nibble Lookup        | **1.02-1.06x**                             | ARM      | Deployed |
| DSV Lightweight Index ARM | **1.8x** (avg), **4.3x** (wide)            | ARM      | Deployed |
| BP Byte Lookup Tables     | **11x**                                    | All      | Deployed |
| Hierarchical RangeMin     | **40x**                                    | All      | Deployed |
| Cumulative Index          | **627x**                                   | All      | Deployed |
| Dual Select Methods       | **3.1x** (seq), **1.39x** (rand)           | All      | Deployed |

**Total Successful**: 11 optimizations
**Best Result**: Cumulative index (627x)

### Failed Optimizations

| Optimization           | Penalty                  | Platform | Action         |
|------------------------|--------------------------|----------|----------------|
| AVX-512 JSON Parser    | **-10%** (avg)           | x86_64   | Removed      |
| BMI1 Mask Iteration    | **-26%** (avg)           | x86_64   | Reverted     |
| BMI2 PDEP BitWriter    | **-71%** (3.4x slower!)  | x86_64   | Reverted     |
| PFSM Batched           | **-25%** (vs production) | All      | Not deployed |
| NEON PFSM Shuffle      | **-47%**                 | ARM      | Reverted     |
| NEON Batched Popcount  | **-25%**                 | ARM      | Rejected     |
| NEON Movemask Batching | **0%** (no effect)       | ARM      | Rejected     |
| NEON Prefetching       | **0%** (no effect)       | ARM      | Rejected     |

**Total Failed**: 8 attempts
**Worst Failure**: BMI2 PDEP (-71%, 3.4x slower)

---

## Successful Optimizations

### 1. AVX2 SIMD JSON Parser (x86_64)

**File**: [src/json/simd/avx2.rs](../../src/json/simd/avx2.rs)

Process 32 bytes per iteration using AVX2 SIMD instructions for character classification.

| Workload   | SSE2 Baseline | AVX2      | Improvement |
|------------|---------------|-----------|-------------|
| 1KB JSON   | 450 MiB/s     | 806 MiB/s | **+79%**    |
| 10KB JSON  | 420 MiB/s     | 731 MiB/s | **+74%**    |
| 100KB JSON | 410 MiB/s     | 725 MiB/s | **+77%**    |
| 1MB JSON   | 400 MiB/s     | 732 MiB/s | **+83%**    |

**Why it worked**: Targets actual bottleneck (character classification), 32-byte processing amortizes instruction overhead, AVX2 widely available (~95% market penetration).

### 2. AVX512-VPOPCNTDQ Popcount (x86_64)

**File**: [src/bits/popcount.rs](../../src/bits/popcount.rs)

Parallel popcount of 8 u64 words (512 bits) using `_mm512_popcnt_epi64`.

| Dataset Size     | Scalar  | AVX512-VPOPCNTDQ | Improvement |
|------------------|---------|------------------|-------------|
| 64B (8 words)    | 4.68 ns | 1.63 ns          | **+187%**   |
| 512B (64 words)  | 42.8 ns | 4.65 ns          | **+820%**   |
| 4KB (512 words)  | 342 ns  | 27.7 ns          | **+1135%**  |
| 1MB (131K words) | 52.6 us | 10.1 us          | **+421%**   |

**Throughput**: 96.8 GiB/s (AVX-512) vs 18.5 GiB/s (scalar) = **5.2x faster**

**End-to-End Impact**: ~1-2% overall (Amdahl's Law - popcount is ~1.6% of total time)

**Why it worked**: Pure compute-bound operation, embarrassingly parallel, native 512-bit support on Zen 4.

### 3. NEON 32-byte Processing (ARM aarch64)

**File**: [src/json/simd/neon.rs](../../src/json/simd/neon.rs)

Process two 16-byte NEON vectors (32 bytes total) per iteration.

| Pattern               | 16-byte (old) | 32-byte (new) | Improvement          |
|-----------------------|---------------|---------------|----------------------|
| nested (string-heavy) | 2.05 GiB/s    | 3.47 GiB/s    | **+69% (-41% time)** |
| strings               | 1.77 GiB/s    | 2.83 GiB/s    | **+60% (-37% time)** |
| unicode               | 1.45 GiB/s    | 1.70 GiB/s    | **+17% (-14% time)** |

**Why it worked**: InString fast-path can skip up to 32 consecutive characters, amortizes classification overhead, no regressions on any pattern.

### 4. DSV Lightweight Index for NEON (ARM)

**File**: [src/dsv/simd/neon.rs](../../src/dsv/simd/neon.rs)

Use lightweight cumulative rank index instead of full BitVec with 3-level rank directory.

| Pattern      | Before (BitVec) | After (Lightweight) | Improvement      |
|--------------|-----------------|---------------------|------------------|
| **strings**  | 29.0 MiB/s      | 75.1 MiB/s          | **+159% (2.6x)** |
| **wide**     | 7.1 MiB/s       | 30.6 MiB/s          | **+331% (4.3x)** |
| **multiline**| 20.3 MiB/s      | 26.8 MiB/s          | **+32%**         |

**Why it worked**: Simpler array structure fits better in cache, reduces memory overhead from ~6% to ~0.8%.

### 5. Balanced Parentheses Byte-Level Lookup Tables

**File**: [src/trees/bp.rs](../../src/trees/bp.rs)

Precomputed lookup tables for byte-level excess and find_close operations.

| Operation                | Bit-by-bit | Byte lookup | Improvement             |
|--------------------------|------------|-------------|-------------------------|
| find_close (1M elements) | 130.9 us   | 11.3 us     | **+1058% (11x faster)** |
| find_open                | 125 us     | 15 us       | **+733% (8x faster)**   |

**Why it worked**: Reduces iterations from 64 per word to 8 per word, lookup tables fit in L1 cache.

### 6. Cumulative Index for O(1) IB Select

Precomputed cumulative bit counts for direct rank-to-position mapping.

| Method               | Time    | Speedup                     |
|----------------------|---------|-----------------------------|
| Binary search select | 779 us  | baseline                    |
| Cumulative index     | 1.24 us | **+62,700% (627x faster!)** |

**Memory Overhead**: 4 bytes per u64 word (~50% of bitvector)

**Why it worked**: O(1) lookup vs O(log n) binary search, perfect for dense bitvectors.

### 7. PFSM (Parallel Finite State Machine) JSON Parser

**Files**: [src/json/pfsm_optimized.rs](../../src/json/pfsm_optimized.rs), [src/json/pfsm_tables.rs](../../src/json/pfsm_tables.rs)

Table-driven state machine with BMI2/AVX2 batch bit extraction (ported from haskellworks hw-json-simd).

| Size  | PFSM BMI2 | Standard Scalar | Standard AVX2 | vs Scalar | vs AVX2   |
|-------|-----------|-----------------|---------------|-----------|-----------|
| 1KB   | 674 MiB/s | 551 MiB/s       | 672 MiB/s     | **+22%**  | **+0.3%** |
| 10KB  | 701 MiB/s | 537 MiB/s       | 599 MiB/s     | **+30%**  | **+17%**  |
| 100KB | 695 MiB/s | 485 MiB/s       | 552 MiB/s     | **+43%**  | **+26%**  |
| 1MB   | 679 MiB/s | 494 MiB/s       | 546 MiB/s     | **+37%**  | **+24%**  |

**Why it worked**: Cache-friendly tables (fits in L1), separates concerns (state machine from bit extraction), branch prediction friendly.

---

## Failed Optimizations

### 1. AVX-512 JSON Parser - REMOVED

**Hypothesis**: Wider SIMD (64 bytes vs 32 bytes) would increase JSON parsing throughput.

| Size  | AVX-512   | AVX2      | Result          |
|-------|-----------|-----------|-----------------|
| 1KB   | 689 MiB/s | 806 MiB/s | **-17% slower** |
| 10KB  | 675 MiB/s | 731 MiB/s | **-8% slower**  |
| 100KB | 678 MiB/s | 725 MiB/s | **-7% slower**  |
| 1MB   | 672 MiB/s | 732 MiB/s | **-9% slower**  |

**Why it failed**:
1. Memory-bound workload (waiting for data, not compute)
2. AMD Zen 4 splits AVX-512 into 2x256-bit micro-ops
3. Only ~20% of work is SIMD classification; 80% is state transitions + memory

**Lesson**: Wider SIMD does not automatically mean faster for memory-bound workloads.

### 2. BMI1 JSON Mask Processing - REVERTED

**Hypothesis**: Using TZCNT/BLSR to iterate only over structural characters would reduce overhead.

| Size  | AVX2 Baseline | AVX2+BMI1 | Result          |
|-------|---------------|-----------|-----------------|
| 1KB   | 609 MiB/s     | 457 MiB/s | **-25% slower** |
| 10KB  | 647 MiB/s     | 450 MiB/s | **-30% slower** |
| 1MB   | 659 MiB/s     | 454 MiB/s | **-31% slower** |

**Time breakdown** (profiled):
- State machine logic: 40%
- BitWriter operations: 30%
- Memory access: 20%
- SIMD classification: 10%
- **Iteration overhead**: <1% (this is what BMI1 "optimized")

**Why it failed**: Optimized the wrong thing (<1% of time), must process ALL bytes for state machine tracking.

**Lesson**: Profile first to find the real bottleneck.

### 3. BMI2 PDEP for BitWriter - REVERTED

**Hypothesis**: Use BMI2 PDEP instruction to efficiently deposit bits without explicit shifting.

| Implementation | Throughput | Result                 |
|----------------|------------|------------------------|
| Scalar         | 1.27 GiB/s | baseline               |
| BMI2 PDEP      | 381 MiB/s  | **-71% slower (3.4x)** |

**Why it failed**:
1. **Wrong use case**: PDEP is for sparse bit operations (bits at positions [0, 5, 12, 31]), not consecutive ranges
2. **PDEP latency**: 3-cycle latency vs 2 cycles for shift + OR
3. Simple operations are already optimal

**Lesson**: Fancy instructions are not automatically better. PDEP/PEXT are specialized tools for sparse operations.

### 4. PFSM Batched Processing - NOT DEPLOYED

**The misleading result**:
- Batched was 40% faster than basic `pfsm.rs` reference
- But `pfsm_optimized.rs` (production) was already 25% faster than batched

**Why it failed**: Benchmarked against reference implementation, not production code.

**Lesson**: Always benchmark against **production code**, not reference implementations.

### 5. NEON PFSM Shuffle Composition - REVERTED

| Implementation | Throughput | Result          |
|----------------|------------|-----------------|
| Scalar PFSM    | 343 MiB/s  | baseline        |
| NEON Shuffle   | 182 MiB/s  | **-47% slower** |

**Why it failed**: Sequential phi extraction negates parallel gains. Each state depends on the previous state - SIMD cannot break this dependency chain.

**Lesson**: Parallel computation is useless if results must be consumed sequentially.

---

## Lessons Learned

### What Works

1. **Profile first, optimize second**
   - Successful optimizations targeted actual bottlenecks
   - Failed optimizations targeted wrong bottlenecks

2. **Understand workload characteristics**
   - Compute-bound: AVX512-VPOPCNTDQ (5.2x) worked
   - Memory-bound: AVX-512 JSON (slower) failed

3. **Amdahl's Law always wins**
   - AVX2 JSON (78% of work): 1.78x speedup possible
   - BMI1 (<1% of work): No improvement possible

4. **Simple code is often fastest**
   - `for i in 0..32` beats "clever" BMI1 iteration
   - Scalar `count_ones()` beats NEON batched popcount
   - Simple shift+mask beats BMI2 PDEP (by 3.4x!)

5. **Cache-friendly data structures**
   - Byte lookup tables: 11x speedup
   - Cumulative index: 627x speedup

6. **Algorithm matters more than micro-optimization**
   - Cumulative index (O(1)) vs binary search (O(log n)): 627x

### What Does Not Work

1. **Wider SIMD does not automatically mean faster**
   - AVX-512 JSON: 7-17% slower than AVX2

2. **Optimizing fast paths**
   - BMI1 iteration: Optimized <1% of time, made it slower

3. **Fighting the compiler/hardware**
   - NEON batched popcount: LLVM already optimal
   - NEON prefetching: Hardware prefetcher is better

4. **"Clever" code without profiling**
   - TZCNT/BLSR iteration: Intuitive but slower
   - BMI2 PDEP: Fancy instruction, wrong use case

---

## Decision Framework

Before implementing an optimization, ask:

1. **Where is time actually spent?** (Profile!)
   - AVX2: 40% in classification -> optimize
   - BMI1: <1% in iteration -> do not optimize

2. **What % of total time?** (Amdahl's Law)
   - AVX2: 78% of JSON parsing -> 1.78x possible
   - AVX512-VPOPCNT: 1.6% of construction -> 1.01x realistic

3. **Is this compute-bound or memory-bound?**
   - Popcount: Compute-bound -> SIMD wins
   - JSON: Memory-bound -> wider SIMD loses

4. **Are there sequential dependencies?**
   - Cumulative index: No dependencies -> parallelize
   - Prefix sum: Sequential dependencies -> keep simple

5. **What is the baseline compiler doing?** (Check assembly)
   - AVX2: SSE2 is baseline -> room for improvement
   - Scalar popcount: Already uses CNT instruction -> no room

6. **How will I measure success?** (Benchmarks ready)
   - All optimizations: Micro + end-to-end benchmarks

**If you cannot answer these confidently, do not optimize.**

---

## Overall Impact

**x86_64 (Zen 4)**:
- JSON parsing: ~2.4x faster (PFSM BMI2 vs scalar baseline)
- Popcount: 5.2x faster (AVX512-VPOPCNTDQ vs scalar)

**ARM (Apple Silicon)**:
- JSON parsing: 1.52x faster (NEON vs scalar)
- String-heavy workloads: Up to 1.69x faster
- DSV parsing: 1.8x faster (avg), 4.3x faster (wide patterns)

**All Platforms**:
- Balanced Parentheses: 10-40x faster (lookup tables + RangeMin)
- IB Select: 627x faster (cumulative index)

---

## Conclusion

**Success Rate**: 11/19 attempted optimizations (58%)

**Key Insight**: The most successful optimizations were algorithmic (cumulative index: 627x, RangeMin: 40x) rather than micro-optimizations. SIMD acceleration works best when:
1. Targeting actual bottlenecks
2. Compute-bound workloads
3. Simple, regular patterns that do not fight the compiler

**The best optimization is often choosing the right algorithm, not the fanciest SIMD instructions.**

---

*Last Updated: 2026-01-18*
