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
| NEON 256-byte Popcount    | **1.10-1.15x** (micro), **1.02-1.04x** (e2e) | ARM    | Deployed |
| NEON VMINV L1 Building    | **2.8x** (BP construction)                 | ARM      | Deployed |
| NEON VMINV L2 Building    | **1-3%** (large data, 100M+ nodes)         | ARM      | Deployed |
| SSE4.1 PHMINPOSUW L1/L2   | **1-3%** (10M+ nodes)                      | x86_64   | Deployed |
| DSV Lightweight Index ARM | **1.8x** (avg), **4.3x** (wide)            | ARM      | Deployed |
| BP Byte Lookup Tables     | **11x**                                    | All      | Deployed |
| Hierarchical RangeMin     | **40x**                                    | All      | Deployed |
| Cumulative Index          | **627x**                                   | All      | Deployed |
| Dual Select Methods       | **3.1x** (seq), **1.39x** (rand)           | All      | Deployed |
| P12-A Build Mitigation    | **11-85%** (yaml_bench build)               | All      | Deployed |

**Total Successful**: 16 optimizations
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
| BP L0 Index Unrolling  | **-19%** (e2e)           | All      | Rejected     |

**Total Failed**: 9 attempts
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

### 6. BP L0 Index Unrolling - REJECTED

**Hypothesis**: Unroll `word_min_excess` loop and batch all 8 byte lookups upfront for better cache locality.

**Micro-benchmark results** (BP construction only):

| Size | Before | After | Improvement |
|------|--------|-------|-------------|
| 10K  | 2.65 µs | 2.02 µs | **+24%** |
| 100K | 25.5 µs | 19.5 µs | **+23%** |
| 1M   | 294.8 µs | 193.7 µs | **+34%** |

**End-to-end results** (JSON semi-indexing):

| Size | Before | After | Regression |
|------|--------|-------|------------|
| 1KB  | 883 MiB/s | 714 MiB/s | **-19%** |
| 10KB | 872 MiB/s | 647 MiB/s | **-26%** |
| 100KB| 677 MiB/s | 580 MiB/s | **-14%** |

**Why it failed**:
1. **Instruction cache pollution**: Unrolled code is larger, evicts other hot code
2. **Register pressure**: Explicit variables interfere with compiler's allocation
3. **Inlining disruption**: Larger function body may prevent beneficial inlining
4. **Compiler already optimal**: Loop-based version allowed better compiler optimizations

**Lesson**: Micro-benchmark wins ≠ end-to-end improvements. Always benchmark the complete pipeline. This is the fourth optimization (after P2.6, P2.8, P3 in YAML) where micro-benchmarks showed gains but end-to-end showed regressions.

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

5. **Micro-benchmark-driven optimization**
   - BP unrolling: +34% micro, -19% end-to-end
   - YAML P2.6/P2.8/P3: Similar micro wins, end-to-end regressions
   - Always test full pipeline, not isolated components

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

**Success Rate**: 12/19 attempted optimizations (63%)

**Key Insight**: The most successful optimizations were algorithmic (cumulative index: 627x, RangeMin: 40x) rather than micro-optimizations. SIMD acceleration works best when:
1. Targeting actual bottlenecks
2. Compute-bound workloads
3. Simple, regular patterns that do not fight the compiler

**The best optimization is often choosing the right algorithm, not the fanciest SIMD instructions.**

---

## Recent Optimizations

### ✅ Balanced Parentheses Unrolled Lookup (January 2026)

**Status**: Implemented and deployed in [src/trees/bp.rs](../../src/trees/bp.rs)

**Technique**: Fully unrolled lookup table access for `word_min_excess` computation.

**Benchmark Results** (Apple M1 Max):

| Size | Before | After | Improvement |
|------|--------|-------|-------------|
| 10K nodes | 2.41 µs | 2.00 µs | **17%** |
| 100K nodes | 20.70 µs | 17.07 µs | **18%** |
| 1M nodes | 207.21 µs | 171.96 µs | **17%** |

**Key insight**: Loop unrolling and batched lookups provide better instruction scheduling and cache locality, without needing SIMD intrinsics.

### ✅ NEON 256-byte Popcount Unrolling (January 2026)

**Status**: Implemented and deployed in [src/bits/popcount.rs](../../src/bits/popcount.rs)

**Technique**: Process 256 bytes (4 × 64 bytes) per iteration instead of 64 bytes, enabling better instruction-level parallelism on superscalar CPUs.

**Benchmark Results** (AWS Graviton 4 / Neoverse-V2):

| Metric | Before (64B) | After (256B) | Improvement |
|--------|--------------|--------------|-------------|
| 10M bits raw popcount | baseline | -15.5% time | **15.5% faster** |
| 1M bits raw popcount | baseline | -10.8% time | **10.8% faster** |
| 1M bits construction | baseline | -4.0% time | **4.0% faster** |
| 10M bits construction | baseline | -2.3% time | **2.3% faster** |

**Key insight**: The 4 independent 64-byte operations can execute in parallel on superscalar CPUs before their results are summed. This amortizes horizontal reduction overhead and improves instruction scheduling.

### ✅ NEON VMINV for L1 Index Building (January 2026)

**Status**: Implemented and deployed in [src/trees/bp.rs](../../src/trees/bp.rs)

**Technique**: Use NEON SIMD to process 8 words at a time during L1 index building:
1. Load min_excess values (i8 → i16) and word_excess values (i16)
2. Compute prefix sums using parallel prefix pattern (3 shuffle+add steps)
3. Add prefix sums to min_excess values
4. Use `vminvq_s16` to find block minimum in one instruction

**Benchmark Results** (AWS Graviton 4 / Neoverse-V2):

| Size | Before | After | Improvement |
|------|--------|-------|-------------|
| 10K nodes | 5.4 µs | 2.07 µs | **2.6x faster (-62%)** |
| 100K nodes | 52.7 µs | 19.1 µs | **2.8x faster (-64%)** |
| 1M nodes | 527 µs | 185 µs | **2.8x faster (-65%)** |

**Key insight**: The VMINV instruction finds the minimum across a vector in a single operation, eliminating the scalar min-finding loop. Combined with SIMD prefix sums, this dramatically accelerates the L1 index construction which aggregates per-word statistics into per-block statistics.

### ✅ NEON VMINV for L2 Index Building (January 2026)

**Status**: Implemented and deployed in [src/trees/bp.rs](../../src/trees/bp.rs)

**Technique**: Same SIMD pattern as L1, but for L2 index building (processing L1 entries instead of L0).
Added for code consistency and benefit at large data sizes.

**Benchmark Results** (AWS Graviton 4 / Neoverse-V2):

| Size | Nodes | L2 Entries | Scalar | SIMD | Improvement |
|------|-------|------------|--------|------|-------------|
| ~2.5MB | 10M | ~10K | 1.91ms | 1.91ms | ~0% (noise) |
| ~25MB | 100M | ~100K | 19.70ms | 19.44ms | **1.3%** |
| ~125MB | 500M | ~488K | 153.78ms | 148.73ms | **3.4%** |

**Key insight**: L2 SIMD benefit scales with data size. At 500M nodes, L2 has ~488K entries
(comparable to L1 at ~1M nodes), making SIMD worthwhile. No regression at smaller sizes,
so kept for consistency with L1.

### ⚠️ SSE4.1 PHMINPOSUW for L1/L2 Index Building (January 2026)

**Status**: Implemented and deployed in [src/trees/bp.rs](../../src/trees/bp.rs) — marginal benefit

**Technique**: x86_64 equivalent of NEON VMINV using SSE4.1's `PHMINPOSUW` (`_mm_minpos_epu16`).
The key challenge is that PHMINPOSUW only works on **unsigned** 16-bit values, while min_excess can be negative.

**Bias Trick Solution**:
```rust
// Bias by 0x8000 to convert signed [-32768, 32767] to unsigned [0, 65535]
let bias = _mm_set1_epi16(i16::MIN);
let biased = _mm_add_epi16(values, bias);
let minpos = _mm_minpos_epu16(biased);
let biased_min = _mm_extract_epi16(minpos, 0) as i16;
let result = biased_min.wrapping_add(i16::MIN);  // Unbias
```

**Benchmark Results** (AMD Ryzen 9 7950X, Zen 4):

| Size | Scalar | SSE4.1 | Improvement |
|------|--------|--------|-------------|
| 10K nodes | 1.97 µs | 1.98 µs | ~0% (neutral) |
| 100K nodes | 18.96 µs | 18.97 µs | ~0% (neutral) |
| 1M nodes | 188.0 µs | 197.2 µs | **-5% (regression)** |
| 10M nodes | 3.15 ms | 3.05 ms | **+3%** |
| 100M nodes | 31.98 ms | 31.36 ms | **+2%** |
| 500M nodes | 253.8 ms | 250.6 ms | **+1%** |

**Why SSE4.1 shows much less improvement than NEON (2.8x)**:

| Factor | ARM NEON | x86 SSE4.1 |
|--------|----------|------------|
| Horizontal min | `vminvq_s16` (signed direct) | `_mm_minpos_epu16` (unsigned only) |
| Horizontal sum | `vaddvq_s16` (single instruction) | Multiple shifts + adds |
| Bias overhead | None needed | +2 ops per block |

**Key insight**: ARM's `vminvq_s16` handles signed values directly. SSE4.1's unsigned-only
PHMINPOSUW requires bias/unbias operations that add significant overhead, negating most benefit
at typical data sizes. Only at 10M+ nodes is the improvement measurable.

**Future opportunity**: AVX-512 provides `_mm512_reduce_min_epi16` which handles signed 16-bit
values directly and processes 32 lanes. This could provide better results on CPUs with efficient
AVX-512 execution (though memory bandwidth may still limit gains for this workload).

### ✅ P12-A Build Regression Mitigation (January 2026)

**Status**: Implemented and deployed — [issue #72](https://github.com/rust-works/succinctly/issues/72)

**Problem**: P12 compact bitmap encoding introduced 11-24% `yaml_bench` regression from bitmap construction overhead.

**Technique**: Three build-path optimisations to reduce allocation and scanning costs:

1. **A1 — Inline zero-filling**: Eliminated temporary `Vec<u32>` allocation in `EndPositions::build()`. Zero-filling now happens inline during bitmap construction, saving one O(N) alloc+copy+dealloc per build.

2. **A2 — Combined monotonicity check**: Merged the separate O(N) monotonicity scan into the bitmap construction loop. `CompactEndPositions::try_build()` now checks monotonicity inline and returns `None` on violation, eliminating one full pass over the positions array.

3. **A4 — Lazy newline index**: Changed `YamlIndex.newlines` from eager `BitVec` to `OnceCell<BitVec>`. The newline index is only used by `yq-locate` CLI, not by parsing or queries. Removes a full O(N) text scan from every `build()` call.

**Benchmark Results** (Apple M1 Max, `yaml_bench`):

| Category        | Improvement     | Notes                              |
|-----------------|-----------------|-------------------------------------|
| simple_kv       | -11% to -22%   | Core key-value parsing              |
| nested          | -15% to -24%   | Deeply nested structures            |
| sequences       | -9% to -15%    | Array-like YAML                     |
| quoted strings  | -20% to -37%   | Double/single quoted values         |
| long strings    | -44% to -85%   | Largest gains (newline scan removed) |
| large files     | -18% to -21%   | 100KB-1MB files                     |
| block scalars   | -42% to -57%   | Literal/folded blocks               |
| anchors         | -12% to -16%   | Anchor/alias workloads              |

**Key insight**: Build-path allocations and eager scans that aren't needed by the hot path (queries, yq evaluation) should be deferred or eliminated. The lazy newline index has the largest impact on newline-heavy content because it removes an entire O(N) pass.

**Files**: [src/yaml/index.rs](../../src/yaml/index.rs), [src/yaml/end_positions.rs](../../src/yaml/end_positions.rs)

---

## Future Optimization Opportunities

Based on analysis of ARM NEON instructions for indexing and data structures (January 2026):

### ✅ ~~Medium Priority: Popcount Loop Unrolling~~ — DEPLOYED

**Status**: Implemented January 2026. See [Recent Optimizations](#-neon-256-byte-popcount-unrolling-january-2026) above.

**Result**: 10-15% faster (exceeded 5-8% expectation)

### Low Priority: Anchor Nibble Tables

**Current state**: [src/yaml/simd/neon.rs](../../src/yaml/simd/neon.rs) uses 10 CMEQ + 9 ORR for anchor terminators.

**Opportunity**: Replace with 2 TBL + 1 AND (nibble table approach like JSON).

**⚠️ Warning**: Similar approach was rejected for YAML char classification (-12-15% penalty). Only implement if micro-benchmark shows 2x+ improvement for 10-character set.

**Risk**: High - needs validation before implementation

### Reference: Applicable NEON Instructions

| Instruction | Use Case | Status in Codebase |
|-------------|----------|-------------------|
| CNT (`vcntq_u8`) | Popcount | ✅ Deployed |
| CMEQ (`vceqq_*`) | Char classification | ✅ Deployed (JSON/YAML/DSV) |
| TBL (`vqtbl1q_u8`) | Nibble lookup | ✅ Deployed (JSON only) |
| PMULL (`vmull_p64`) | Prefix XOR | ✅ Deployed (DSV, +25%) |
| ADDV (`vaddvq_*`) | Horizontal sum | ✅ Deployed |
| **Unrolled lookup** | BP min excess | ✅ **Deployed (BP, +17%)** |
| **256B loop unroll** | Popcount ILP | ✅ **Deployed (popcount, +10-15%)** |
| **MINV** (`vminvq_s16`) | L1/L2 block minimum | ✅ **Deployed (BP L1: +2.8x, L2: +1-3% at scale)** |
| **Vector CLZ** | First match position | ❌ Not yet used (BP opportunity) |

See [docs/plan/sve2-optimizations.md](../plan/sve2-optimizations.md#future-neon-optimization-opportunities) for detailed analysis.

---

*Last Updated: 2026-01-28*
