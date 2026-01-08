# Failed Optimizations - Lessons Learned

This document records optimization attempts that failed, providing valuable lessons for future work.

---

## 1. AVX-512 JSON Parser (2026-01-07) - ❌ REMOVED

**Hypothesis**: Wider SIMD (64 bytes vs 32 bytes) would increase JSON parsing throughput.

**Implementation**: Full AVX-512 JSON semi-indexing (627 lines)

**Results**: **7-17% slower** than AVX2

| Size  | AVX-512   | AVX2      | Result          |
|-------|-----------|-----------|-----------------|
| 1KB   | 689 MiB/s | 806 MiB/s | **-17% slower** |
| 10KB  | 675 MiB/s | 731 MiB/s | **-8% slower**  |
| 100KB | 678 MiB/s | 725 MiB/s | **-7% slower**  |
| 1MB   | 672 MiB/s | 732 MiB/s | **-9% slower**  |

**Why it failed**:
1. Memory-bound workload (waiting for data, not compute)
2. AMD Zen 4 splits AVX-512 into 2×256-bit micro-ops (not native)
3. Sequential state machine can't utilize wider SIMD effectively
4. Only ~20% of work is SIMD classification; 80% is state transitions + memory

**Action taken**: Removed implementation (2026-01-07)

**Lesson**: Wider SIMD ≠ automatically faster. Memory-bound workloads don't benefit from wider registers.

**Documentation**: [docs/AVX512-JSON-RESULTS.md](AVX512-JSON-RESULTS.md)

---

## 2. BMI1 JSON Mask Processing (2026-01-07) - ❌ REVERTED

**Hypothesis**: Using TZCNT/BLSR to iterate only over structural characters would reduce overhead.

**Implementation**: Replace `for i in 0..32` with BMI1-based iteration

```rust
// Instead of:
for i in 0..32 { check_bit(i) }

// Use:
while mask != 0 {
    let i = _tzcnt_u32(mask);
    mask = _blsr_u32(mask);
    process(i);
}
```

**Results**: **25-31% slower** than baseline AVX2

### Dense JSON (many structural characters)

| Size  | AVX2 Baseline | AVX2+BMI1 | Result          |
|-------|---------------|-----------|-----------------|
| 1KB   | 609 MiB/s     | 457 MiB/s | **-25% slower** |
| 10KB  | 647 MiB/s     | 450 MiB/s | **-30% slower** |
| 100KB | 658 MiB/s     | 458 MiB/s | **-30% slower** |
| 1MB   | 659 MiB/s     | 454 MiB/s | **-31% slower** |

### Sparse JSON (few structural characters - "best case")

| Size | AVX2 Baseline | AVX2+BMI1 | Result          |
|------|---------------|-----------|-----------------|
| 1KB  | 796 MiB/s     | 727 MiB/s | **-9% slower**  |
| 10KB | 790 MiB/s     | 700 MiB/s | **-11% slower** |

**Why it failed**:

1. **Loop overhead dominates**: TZCNT/BLSR in loop + branch prediction costs more than simple `for` loop
2. **Must process all positions anyway**: State machine requires sequential processing of ALL 32 bytes (even whitespace)
3. **Compiler already optimizes baseline**: `for i in 0..32` is unrolled and well-optimized
4. **Wrong bottleneck**: Time is spent in state machine logic and BitWriter, not iteration method

**Time breakdown** (profiled):
- State machine logic: 40%
- BitWriter operations: 30%
- Memory access: 20%
- SIMD classification: 10%
- **Iteration overhead**: <1% ← This is what BMI1 "optimized"

**Action taken**: Reverted all changes (2026-01-07)

**Lesson**: "Smarter" code isn't always faster. Profile first to find the real bottleneck.

---

## 3. BMI2 PDEP for BitWriter (2026-01-07) - ❌ REVERTED

**Hypothesis**: Use BMI2 PDEP instruction to efficiently deposit bits into BitWriter without explicit shifting.

**Implementation**: Replace `(bits & mask) << pos` with `_pdep_u64(bits, shifted_mask)` in `write_bits()`.

```rust
// Scalar (baseline):
let mask = (1u64 << count) - 1;
self.current_word |= (bits & mask) << pos;

// BMI2 PDEP (attempted):
let deposit_mask = (1u64 << count) - 1;
let shifted_mask = deposit_mask << pos;
self.current_word |= _pdep_u64(bits, shifted_mask);
```

**Results**: **3.4x SLOWER** than scalar

| Implementation | Throughput | Result                 |
|----------------|------------|------------------------|
| Scalar         | 1.27 GiB/s | baseline               |
| BMI2 PDEP      | 381 MiB/s  | **-71% slower (3.4x)** |

**Why it failed**:

1. **Wrong use case for PDEP**: PDEP is designed for **sparse** bit extraction/deposit (e.g., extracting bits at positions 0, 5, 12, 31). Our use case is **consecutive** bits (positions pos..pos+count), which simple shift handles optimally.

2. **PDEP latency**: Even on Zen 4 with "fast" BMI2, PDEP has 3-cycle latency. Scalar shift + OR is 1 cycle each = 2 cycles total.

3. **Extra work**: The BMI2 version still needs to compute the shifted mask (`deposit_mask << pos`), adding overhead without benefit.

4. **Simple operations are already optimal**: `(bits & mask) << pos` compiles to 2-3 fast integer instructions. PDEP is overkill.

**When PDEP would help**:
- Extracting bits at non-consecutive positions (e.g., bit indices [0, 3, 7, 15, 31])
- Packing/unpacking structured data with sparse fields
- NOT for consecutive bit ranges like this use case

**Action taken**: Reverted all changes (2026-01-07)

**Lesson**: Fancy instructions aren't always better. PDEP/PEXT are specialized tools for sparse operations, not replacements for simple shift+mask.

---

## Common Themes

### Pattern of Failure

All three failed optimizations shared these characteristics:

1. ❌ **Optimized the wrong thing**: Focused on micro-optimizations (SIMD width, iteration method, fancy instructions) instead of actual bottlenecks
2. ❌ **Theoretical vs empirical**: Assumed wider/smarter/fancier = faster without profiling
3. ❌ **Ignored Amdahl's Law**: Optimized fast parts (10-20%) instead of slow parts (80%)
4. ❌ **Misunderstood workload**: Didn't account for memory-bound nature, sequential dependencies, or instruction suitability

### What Works

Successful optimizations that we DID keep:

| Optimization         | Result            | Why It Worked                              |
|----------------------|-------------------|--------------------------------------------|
| **AVX2 SIMD**        | 1.8x vs SSE2      | Targets actual bottleneck (classification) |
| **AVX512-VPOPCNTDQ** | 5.2x vs scalar    | Compute-bound, parallel, no dependencies   |
| **Runtime dispatch** | Auto-selects best | Transparent, no overhead                   |

### Lessons Learned

1. ✅ **Profile first, optimize second**
   - Use tools to identify actual bottlenecks
   - Don't assume you know where time is spent

2. ✅ **Understand workload characteristics**
   - Memory-bound vs compute-bound
   - Sequential dependencies vs parallelizable
   - Cache behavior and memory access patterns

3. ✅ **Amdahl's Law always wins**
   - Optimize the 80%, not the 20%
   - Making fast code faster has minimal impact

4. ✅ **Simple code is often fastest**
   - Compilers optimize simple patterns well
   - "Clever" code adds overhead

5. ✅ **Measure everything**
   - Micro-benchmarks (isolated components)
   - End-to-end benchmarks (real workloads)
   - Both must show improvement

6. ✅ **Remove failed optimizations**
   - Don't keep slower code as "reference"
   - Complexity is technical debt
   - Document the lesson instead

### Decision Framework

Before implementing an optimization, ask:

1. **Where is time actually spent?** (Profile!)
2. **What % of total time does this represent?** (Amdahl's Law)
3. **Is this compute-bound or memory-bound?**
4. **Are there sequential dependencies?**
5. **What's the baseline compiler doing?** (Check assembly)
6. **How will I measure success?** (Benchmarks ready)

If you can't answer these confidently, **don't optimize**.

---

## References

- AVX-512 detailed analysis: [AVX512-JSON-RESULTS.md](AVX512-JSON-RESULTS.md)
- AVX-512 popcount success: [AVX512-VPOPCNTDQ-RESULTS.md](AVX512-VPOPCNTDQ-RESULTS.md)
- BMI1 detailed analysis: `/tmp/bmi1_failure_analysis.md` (temporary)
- Optimization opportunities (updated): [optimization-opportunities.md](optimization-opportunities.md)

---

## What NOT to Try Next

Based on these failures, **do NOT pursue**:

1. ❌ AVX-512 JSON parsing (proven 7-17% slower)
2. ❌ BMI1 mask iteration (proven 25-31% slower)
3. ❌ BMI2 PDEP for consecutive bits (proven 71% slower)
4. ❌ Other "clever" iteration schemes
5. ❌ Wider SIMD for memory-bound workloads
6. ❌ Fancy instructions without understanding their actual use case

## What MIGHT Work (If You Must Optimize Further)

Only pursue if profiling shows these are actual bottlenecks:

1. **State machine optimization**
   - Lookup tables instead of match statements
   - Reduce branch mispredictions
   - **Expected gain**: 5-15%
   - **Effort**: High, **Risk**: Medium

2. **BitWriter optimization**
   - Batch bit writes
   - Reduce bounds checks
   - **Expected gain**: 5-10%
   - **Effort**: Medium, **Risk**: Low

3. **Memory layout optimization**
   - Cache-friendly IB/BP interleaving
   - Prefetching
   - **Expected gain**: 5-10%
   - **Effort**: High, **Risk**: Medium

**Recommendation**: Accept current AVX2 performance (600-800 MiB/s) as "good enough" unless profiling definitively shows a different bottleneck.
