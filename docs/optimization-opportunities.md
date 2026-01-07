# Optimization Opportunities for Succinctly

**System**: AMD Ryzen 9 7950X (Zen 4)
**Date**: 2026-01-06
**Current Status**: Production code uses SSE2 baseline, leaving ~50-80% performance on the table

---

## Executive Summary

This document identifies optimization opportunities for the Succinctly library based on available CPU features and current implementation analysis. The Ryzen 9 7950X provides extensive SIMD capabilities (AVX-512, AVX2, BMI2) that are currently underutilized.

### Quick Wins (High ROI, Low Effort)

1. **Enable runtime dispatch in production** - 1.5-1.8x speedup, 5 lines of code
2. **AVX-512 JSON parser** - 2x SIMD width, ~30% additional speedup over AVX2
3. **AVX512-VPOPCNTDQ for rank** - 8x parallel popcount for rank operations

---

## 1. Enable Runtime CPU Feature Detection (CRITICAL)

### Current State
- **Production**: Hardcoded to SSE2 (2001 technology, 16 bytes/iteration)
- **Tests only**: Runtime dispatch to AVX2/SSE4.2/SSE2
- **Location**: [src/json/simd/mod.rs:80-86](../src/json/simd/mod.rs)

### Problem
```rust
// Lines 80-86: Production code
#[cfg(all(target_arch = "x86_64", not(test)))]
pub use x86::build_semi_index_simple;  // ← Always SSE2!
```

### Impact
- **Performance left on table**: 50-80% slower than possible
- **Dispatch overhead**: 0.2 nanoseconds (negligible for any realistic workload)
- **Benefit/Cost ratio**: 750:1 for 1KB JSON files

### Solution
Change conditional from `test` to `feature = "std"`:

```rust
// Enable runtime dispatch when std is available
#[cfg(all(target_arch = "x86_64", feature = "std"))]
pub fn build_semi_index_standard(json: &[u8]) -> crate::json::standard::SemiIndex {
    if is_x86_feature_detected!("avx2") {
        avx2::build_semi_index_standard(json)
    } else if is_x86_feature_detected!("sse4.2") {
        sse42::build_semi_index_standard(json)
    } else {
        x86::build_semi_index_standard(json)
    }
}

// Fallback for no_std: default to SSE2
#[cfg(all(target_arch = "x86_64", not(feature = "std")))]
pub use x86::build_semi_index_standard;
```

### Expected Improvement
- **Throughput**: 1.5-1.8x for JSON parsing on systems with AVX2
- **Compatibility**: Maintains no_std support
- **Lines changed**: ~10

**Priority**: ⭐⭐⭐⭐⭐ CRITICAL
**Effort**: 🔧 Trivial (5 minutes)
**Risk**: None (existing code, just changing `cfg` gates)

---

## 2. AVX-512 JSON Semi-Indexing (HIGH IMPACT)

### Current State
- Maximum SIMD width: 32 bytes (AVX2)
- Available on CPU: 64 bytes (AVX-512F/BW/DQ)

### Opportunity
Implement AVX-512 variant of JSON parser to process **64 bytes per iteration**.

### CPU Feature Availability
```
✅ avx512f       - Foundation (512-bit registers)
✅ avx512bw      - Byte/Word operations
✅ avx512dq      - Doubleword/Quadword operations
✅ avx512vl      - Vector length extensions
✅ avx512vbmi    - Vector Byte Manipulation
✅ avx512vbmi2   - Enhanced byte manipulation
✅ avx512bitalg  - Bit algorithms
```

### Implementation Approach

**New file**: `src/json/simd/avx512.rs`

```rust
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// Classify 64 bytes at once using AVX-512
#[inline]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn classify_chars_avx512(chunk: __m512i) -> CharClass512 {
    // Create comparison vectors
    let v_quote = _mm512_set1_epi8(b'"' as i8);
    let v_backslash = _mm512_set1_epi8(b'\\' as i8);
    // ... etc

    // Compare and get mask (returns __mmask64, not movemask!)
    let quotes = _mm512_cmpeq_epi8_mask(chunk, v_quote);
    let backslashes = _mm512_cmpeq_epi8_mask(chunk, v_backslash);
    // ...

    CharClass512 { quotes, backslashes, opens, closes, delims, value_chars }
}

struct CharClass512 {
    quotes: u64,      // Direct 64-bit mask
    backslashes: u64,
    opens: u64,
    closes: u64,
    delims: u64,
    value_chars: u64,
}
```

**Key differences from AVX2**:
- `__m512i` instead of `__m256i`
- `_mm512_cmpeq_epi8_mask` returns `u64` mask directly (no `movemask` needed!)
- Process 64 bytes per loop iteration
- Use `_mm512_loadu_si512` for unaligned loads

### Runtime Dispatch Update
```rust
#[cfg(all(target_arch = "x86_64", feature = "std"))]
pub fn build_semi_index_standard(json: &[u8]) -> SemiIndex {
    if is_x86_feature_detected!("avx512bw") {
        avx512::build_semi_index_standard(json)  // NEW!
    } else if is_x86_feature_detected!("avx2") {
        avx2::build_semi_index_standard(json)
    } else if is_x86_feature_detected!("sse4.2") {
        sse42::build_semi_index_standard(json)
    } else {
        x86::build_semi_index_standard(json)
    }
}
```

### Expected Improvement
- **Throughput**: 1.3-1.5x over AVX2 (not 2x due to state machine bottleneck)
- **Best case**: 2.5-2.7x over SSE2
- **Memory bandwidth**: May become bottleneck at ~15+ GB/s

### Considerations
- **Frequency scaling**: AVX-512 may reduce CPU frequency on some chips
  - Ryzen 7950X handles this well (good AVX-512 implementation)
- **Code size**: Adds ~500 lines
- **Testing**: Requires AVX-512 capable CPU

**Priority**: ⭐⭐⭐⭐ HIGH
**Effort**: 🔧🔧 Moderate (4-6 hours, copy/adapt from AVX2)
**Risk**: Low (additive, doesn't break existing code)

---

## 3. AVX-512 VPOPCNTDQ for Rank Operations (HIGH IMPACT)

### Current State
[src/popcount.rs](../src/popcount.rs) uses:
- **Default**: `word.count_ones()` (auto-vectorized by LLVM)
- **SIMD feature**: NEON on ARM, scalar on x86_64
- **Portable**: Bitwise algorithm

### Opportunity
Use `_mm512_popcnt_epi64` to count **8 u64 words in parallel**.

### Implementation

**Update**: `src/popcount.rs`

```rust
/// AVX-512 VPOPCNTDQ implementation (Zen 4, Ice Lake+)
#[cfg(all(
    feature = "simd",
    target_arch = "x86_64",
    not(feature = "portable-popcount")
))]
#[inline]
fn popcount_words_avx512vpopcntdq(words: &[u64]) -> u32 {
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;

    if words.is_empty() {
        return 0;
    }

    let mut total = 0u32;
    let mut offset = 0;

    // Process 8 words (512 bits) at a time
    while offset + 8 <= words.len() {
        unsafe {
            let ptr = words.as_ptr().add(offset) as *const __m512i;
            let v = _mm512_loadu_si512(ptr);
            let counts = _mm512_popcnt_epi64(v);
            // Sum the 8 popcounts
            total += _mm512_reduce_add_epi64(counts) as u32;
        }
        offset += 8;
    }

    // Handle remaining words (< 8)
    for &word in &words[offset..] {
        total += word.count_ones();
    }

    total
}

/// Auto-detect and dispatch
#[cfg(all(
    feature = "simd",
    target_arch = "x86_64",
    not(feature = "portable-popcount")
))]
#[inline]
fn popcount_words_x86(words: &[u64]) -> u32 {
    #[cfg(feature = "std")]
    {
        if is_x86_feature_detected!("avx512vpopcntdq") {
            return popcount_words_avx512vpopcntdq(words);
        }
    }

    // Fallback: use count_ones() which compiles to POPCNT
    let mut total = 0u32;
    for &word in words {
        total += word.count_ones();
    }
    total
}
```

### Expected Improvement
- **Throughput**: 2-4x for large word arrays (rank queries)
- **Use cases**:
  - Rank operations on large bitvectors
  - Select index construction
  - Balanced parentheses excess calculation

### Considerations
- **Feature detection**: Requires `avx512vpopcntdq` (available on Zen 4, Ice Lake+)
- **Unaligned loads**: Use `_mm512_loadu_si512` (no alignment required)
- **Short arrays**: Overhead matters for < 64 words, use scalar

**Priority**: ⭐⭐⭐⭐ HIGH
**Effort**: 🔧🔧 Moderate (2-3 hours)
**Risk**: Low (additive optimization)

---

## 4. BMI2 Integration for Efficient Bit Packing (MEDIUM IMPACT)

### Current State
- [src/json/simd/bmi2.rs](../src/json/simd/bmi2.rs) provides utilities but not integrated
- Zen 4 has **fast BMI2** (3-cycle PDEP/PEXT, not 18-cycle like Zen 1/2)

### Opportunity
Use `PDEP` (Parallel Bits Deposit) to efficiently pack bitmasks into bitvectors.

### Problem It Solves
Currently, `BitWriter` writes bits one at a time:
```rust
// Current approach (scalar)
for i in 0..32 {
    if (mask >> i) & 1 != 0 {
        writer.write_1();
    } else {
        writer.write_0();
    }
}
```

### BMI2 Solution
```rust
#[cfg(target_feature = "bmi2")]
use core::arch::x86_64::_pdep_u64;

/// Deposit bits from mask into positions
#[inline]
#[target_feature(enable = "bmi2")]
unsafe fn deposit_bits(src: u64, mask: u64) -> u64 {
    _pdep_u64(src, mask)
}

/// Fast bulk bit writing using PDEP
impl BitWriter {
    #[inline]
    pub fn write_mask_u32(&mut self, mask: u32, count: u32) {
        // Use PDEP to pack mask bits efficiently
        #[cfg(all(target_arch = "x86_64", feature = "std"))]
        {
            if is_x86_feature_detected!("bmi2") {
                unsafe { self.write_mask_u32_pdep(mask, count) }
                return;
            }
        }

        // Fallback
        self.write_mask_u32_scalar(mask, count)
    }

    #[target_feature(enable = "bmi2")]
    unsafe fn write_mask_u32_pdep(&mut self, mask: u32, count: u32) {
        // Implementation using PDEP
        // ...
    }
}
```

### Expected Improvement
- **Throughput**: 10-30% for JSON parsing (reduces state machine overhead)
- **Best for**: Dense structural characters (many brackets, commas)

### Considerations
- **CPU support**: Zen 3+ and Intel Haswell+ (fast), Zen 1/2 (slow!)
- **Detection**: Check `is_x86_feature_detected!("bmi2")` + CPU generation
- **Complexity**: Need to handle partial words carefully

**Priority**: ⭐⭐⭐ MEDIUM
**Effort**: 🔧🔧🔧 Moderate-High (6-8 hours)
**Risk**: Medium (needs careful testing, CPU-specific behavior)

---

## 5. SIMD Prefix Sum for State Machine (HIGH COMPLEXITY, x86 ONLY)

### Current State
[src/json/simd/avx2.rs:134-218](../src/json/simd/avx2.rs) processes classified bytes **serially**:

```rust
fn process_chunk_standard(...) -> State {
    for i in 0..bytes.len().min(32) {  // ← Scalar bottleneck!
        let bit = 1u32 << i;
        let is_quote = (class.quotes & bit) != 0;
        // ... state machine logic
    }
}
```

### Opportunity
Use **SIMD prefix sums** to compute state transitions in parallel, similar to simdjson.

### Approach
1. **Classify**: Already SIMD (✓)
2. **Compute quote spans**: Use SIMD to find string boundaries
3. **Mask operations**: Apply masks to exclude quoted regions
4. **Parallel counting**: Use prefix sums for excess/balance

### Implementation Sketch
```rust
// Use carry-less multiplication for prefix operations
#[target_feature(enable = "avx2")]
unsafe fn compute_quote_mask(quotes: u32) -> u32 {
    // XOR prefix to find inside/outside quotes
    let mut mask = quotes;
    mask ^= mask << 1;
    mask ^= mask << 2;
    mask ^= mask << 4;
    mask ^= mask << 8;
    mask ^= mask << 16;
    mask
}

// Apply mask to structural characters
fn filter_structural_chars(chars: u32, quote_mask: u32) -> u32 {
    chars & !quote_mask  // Remove quoted characters
}
```

### Expected Improvement
- **Throughput**: 1.5-2x over current SIMD (removes scalar bottleneck)
- **Combined with AVX-512**: Could achieve 3-4x over SSE2

### Challenges
- **Complexity**: State machine logic is non-trivial
- **Escape handling**: Backslashes complicate quote detection
- **Testing**: Requires extensive validation

### Platform Considerations

**x86 (AVX2/AVX-512)**: Recommended target
- 256-bit vectors (4 x u64) or 512-bit (8 x u64)
- Good amortization of setup overhead
- `PCLMULQDQ` for carry-less multiply (useful for quote masking)

**ARM (NEON)**: NOT RECOMMENDED
- Only 128-bit vectors (2 x u64) - less amortization
- No dedicated prefix sum instruction
- Apple Silicon already excellent at scalar execution
- Previous investigation (R1. NEON Batched Popcount) showed SIMD was **25% slower** than scalar due to extraction overhead and sequential dependencies
- The bottleneck is the `cumulative += count` dependency chain, which is inherently serial regardless of architecture

### References
- simdjson: https://github.com/simdjson/simdjson
- "Parsing Gigabytes of JSON per Second" paper

**Priority**: ⭐⭐⭐⭐ HIGH (but complex, x86 only)
**Effort**: 🔧🔧🔧🔧🔧 Very High (20-40 hours, research + implementation)
**Risk**: High (complex algorithm, hard to debug)

---

## 6. Balanced Parentheses SIMD Optimization (LOW PRIORITY - MOSTLY IMPLEMENTED)

### Current State

**Already optimized:**
- `BalancedParens::find_close` uses `find_close_in_word_fast` with byte-level lookup tables
- Byte lookup tables (`BYTE_MIN_EXCESS`, `BYTE_TOTAL_EXCESS`, `BYTE_FIND_CLOSE`) enable O(1) matching within bytes
- RangeMin hierarchical structure (L0/L1/L2) enables O(1) block skipping
- **Result:** ~11x faster than linear scan (11.3 µs vs 130.9 µs for 1M elements)

**Not optimized (low priority):**
- Free function `find_unmatched_close_in_word` still uses bit-by-bit loop
- `find_open` and `enclose` use bit-by-bit backward scanning
- These are rarely used in JSON processing (JSON uses `BalancedParens` struct)

### Remaining Opportunity

SIMD multi-word prefix sum could batch excess computation across 4-8 words. However:
- The hierarchical RangeMin structure already skips entire L0/L1/L2 blocks
- Word-level scanning uses byte lookup tables (8x fewer iterations than bit-by-bit)
- ROI is limited since the hot path is already well-optimized

### Potential Improvement (if pursued)
```rust
// Batch process 4 words: compute which word contains the match
// Then use existing byte-level scan within that word
```

**Priority**: ⭐ LOW (main path already optimized)
**Effort**: 🔧🔧🔧 Moderate-High (8-12 hours)
**Risk**: Medium (diminishing returns)

---

## 6.5. NEON JSON Parser Tuning (LOW PRIORITY)

### Current State

**File:** [src/json/simd/neon.rs](../src/json/simd/neon.rs)

The NEON JSON parser is already implemented and working:

| Benchmark (10MB comprehensive) | Throughput | Relative |
|-------------------------------|------------|----------|
| NEON (32-byte) | 560 MiB/s | 1.48x |
| NEON (16-byte, old) | 505 MiB/s | 1.30x |
| Scalar | 388 MiB/s | 1.00x |

**Current speedup: 48% over scalar** - Improved from 30% after 32-byte optimization (Option C).

### Implementation Details

The parser has three main components:

1. **`neon_movemask`** (lines 78-106): Converts NEON comparison results to u16 bitmask
   - Uses multiplication trick: `(u64.wrapping_mul(0x0102040810204080) >> 56)`
   - Requires 2 lane extractions (`vgetq_lane_u64`) per 16 bytes

2. **`classify_chars`** (lines 132-207): Classifies 16 bytes using nibble lookup
   - Uses `vqtbl1q_u8` for nibble-based character classification (simdjson technique)
   - 6 `neon_movemask` calls to extract individual class masks
   - Additional range checks for alphanumeric characters

3. **`process_chunk_standard`** (lines 309-425): Serial state machine
   - Processes classified bits one at a time
   - Fast path for strings: batch-writes zeros when no quotes/backslashes

### Potential Optimization Opportunities

#### A. Reduce `neon_movemask` calls ❌ REJECTED

**Status:** Investigated January 2026 - No improvement found

**Approaches Tried:**
1. **Batched extraction** (`neon_movemask_pair`, `neon_movemask_triple`): Group multiple vectors and extract together to improve ILP
2. **Fully inlined extraction**: Inline all 12 lane extractions (6 vectors × 2 halves) in a single code block
3. **Compute `string_special` in NEON domain**: OR the vectors before extraction instead of ORing u16 results

**Benchmark Results:**
- All approaches showed **no measurable improvement** (within noise)
- Some showed slight **regressions** (1-2%) on string-heavy patterns

**Analysis:**
The M1/M2/M3 out-of-order execution engine already schedules independent `neon_movemask` calls efficiently. The lane extractions (`vgetq_lane_u64`) and multiplications are independent operations that the CPU can execute in parallel automatically. Manual batching adds code complexity without benefit.

**Why It Failed:**
1. Apple Silicon has excellent out-of-order execution
2. The 6 movemask calls are already independent (no data dependencies)
3. Function call overhead is negligible (inlined anyway)
4. The multiplications (12 total) are fast on M1's multiply units

**Estimated gain**: ~~5-10%~~ **0% (not effective)**
**Effort**: 🔧🔧 Low-Medium (2-4 hours) ❌

#### B. Optimize value character detection ✅ IMPLEMENTED

**Status:** Completed January 2026 - Small but measurable improvement

**Previous Implementation:**
Used 13 NEON operations: 3 range checks (sub + compare each) + 3 equality checks + 4 ORs:
```rust
let lowercase = vcleq_u8(vsubq_u8(chunk, vdupq_n_u8(b'a')), vdupq_n_u8(25));
let uppercase = vcleq_u8(vsubq_u8(chunk, vdupq_n_u8(b'A')), vdupq_n_u8(25));
let digit = vcleq_u8(vsubq_u8(chunk, vdupq_n_u8(b'0')), vdupq_n_u8(9));
// ... + 3 equality checks + 4 ORs
```

**New Implementation:**
Uses nibble lookup tables (6 operations: 2 table loads + 2 lookups + AND + compare):
```rust
let value_lo_result = vqtbl1q_u8(value_lo_table, lo_nibble);
let value_hi_result = vqtbl1q_u8(value_hi_table, hi_nibble);
let value_classified = vandq_u8(value_lo_result, value_hi_result);
let value_chars = vmvnq_u8(vceqq_u8(value_classified, zero));
```

**Benchmark Results:**

| Pattern | Improvement | New Throughput |
|---------|-------------|----------------|
| comprehensive | +2% | 571 MiB/s |
| nested | +6% | 3.70 GiB/s |
| strings | +5% | 2.94 GiB/s |
| literals | +3% | 305 MiB/s |
| numbers | +2% | 351 MiB/s |
| mixed | +1% | 394 MiB/s |
| arrays | +1% | 316 MiB/s |

**Analysis:**
- Modest but consistent improvement (1-6%) across all patterns
- No regressions detected
- The lookup table reuses the already-computed `lo_nibble` and `hi_nibble` from structural classification

**Overall NEON vs Scalar improvement (comprehensive 10MB):**
- Before Option B: 560 MiB/s (1.48x over scalar)
- After Option B: 571 MiB/s (1.52x over scalar)

**Actual gain**: 2% overall, up to 6% on some patterns
**Effort**: 🔧🔧 Low-Medium (2-4 hours) ✅

#### C. Process 32 bytes at a time ✅ IMPLEMENTED

**Status:** Completed January 2026

**Implementation:**
- Added `CharClass32` struct combining two 16-byte classifications into u32 masks
- Added `process_chunk_standard_32()` for 32-byte state machine processing
- Main loop now processes 32-byte chunks, with 16-byte remainder handling

**Benchmark Results (10MB files):**

| Pattern | Improvement | Throughput |
|---------|-------------|------------|
| nested | **-41.4%** time | 3.47 GiB/s |
| strings | **-37.3%** time | 2.83 GiB/s |
| unicode | **-14.5%** time | 1.70 GiB/s |
| comprehensive | **-9.1%** time | 560 MiB/s |
| mixed | **-7.5%** time | 389 MiB/s |
| numbers | **-5.4%** time | 342 MiB/s |
| users | **-5.0%** time | 681 MiB/s |
| arrays | **-4.6%** time | 311 MiB/s |
| literals | **-2.0%** time | 290 MiB/s |
| pathological | no change | 412 MiB/s |

**Analysis:**
- String-heavy patterns benefit most (up to 41% improvement for nested, 37% for strings)
- This is because the InString fast-path can now skip up to 32 consecutive string characters
- Structural-heavy patterns (arrays, literals) see smaller but still positive gains (2-5%)
- No regressions detected across any pattern

**Overall NEON vs Scalar improvement (comprehensive 10MB):**
- Before Option C: 505 MiB/s (1.30x over scalar)
- After Option C: 560 MiB/s (1.48x over scalar)

**Actual gain**: 10% overall, up to 41% on string-heavy workloads
**Effort**: 🔧🔧🔧 Medium (4-6 hours) ✅

#### D. Prefetching
Add software prefetch hints for upcoming chunks.

**Estimated gain**: 0-10% (M1 has good hardware prefetching)
**Effort**: 🔧 Low (1-2 hours)

### Why Limited Gains Expected

1. **State machine is inherently serial**: The quote/escape tracking cannot be parallelized
2. **Apple Silicon excels at scalar**: M1/M2/M3 have wide superscalar execution, reducing SIMD advantage
3. **Memory bandwidth**: At 505 MiB/s, we're processing ~32 cycles per byte on a 3.2GHz M1 - not memory bound
4. **Previous NEON investigation failed**: R1 showed 25% slowdown when trying to batch popcount operations

### Recommendation

With Option C implemented, NEON is now **48% faster than scalar** (up from 30%). Further optimization has diminishing returns:
- Option A (reduce movemask calls): ❌ **Rejected** - no improvement found
- Option B (value char detection): May yield 5-15%
- Option D (prefetching): Likely minimal gain on M1/M2/M3

The remaining options are **low priority** because:
- Current 48% speedup is substantial
- Option A was investigated and showed no benefit
- Risk of regression on different Apple Silicon generations
- x86 optimizations (AVX-512) have higher ROI

If pursued, try **Option B (value char detection)** next, though likelihood of success is low given Option A results.

**Priority**: ⭐ LOW (already 48% faster than scalar after Option C)
**Effort**: 🔧🔧 Low-Medium (4-8 hours for additional improvement)
**Risk**: Medium (may regress, limited upside)

---

## 7. No-std Runtime Dispatch (LOW PRIORITY)

### Current State
Runtime dispatch only works with `std` because `is_x86_feature_detected!` requires it.

### Opportunity
Use `cpuid` crate to enable runtime dispatch in `no_std` environments.

### Implementation
```rust
// Add dependency
[dependencies]
cpufeatures = { version = "0.2", optional = true }

// In code
#[cfg(all(target_arch = "x86_64", not(feature = "std")))]
pub fn build_semi_index_standard(json: &[u8]) -> SemiIndex {
    #[cfg(feature = "cpufeatures")]
    {
        // Manual feature detection for no_std
        let token = cpufeatures::new!(cpuid_amd64, "avx2", "sse4.2");
        if token.get() {
            // ... dispatch based on features
        }
    }

    // Fallback: SSE2
    x86::build_semi_index_standard(json)
}
```

### Expected Improvement
- **Compatibility**: Enables optimizations in embedded/no_std contexts
- **Performance**: Same as std runtime dispatch

**Priority**: ⭐ LOW (niche use case)
**Effort**: 🔧🔧 Moderate (4-6 hours)
**Risk**: Low (external crate dependency)

---

## 8. Compile-Time Feature Selection (ALTERNATIVE)

### Current Workaround
Instead of runtime dispatch, enable features at compile time:

```bash
# Option 1: Enable all CPU features
RUSTFLAGS="-C target-cpu=native" cargo build --release

# Option 2: Enable specific features
RUSTFLAGS="-C target-feature=+avx2,+bmi2" cargo build --release
```

### When to Use
- **Known hardware**: Embedded systems, dedicated servers
- **Maximum performance**: Eliminates all dispatch overhead
- **Binary size**: Smaller than multi-path dispatch code

### Trade-offs
- **Portability**: Binary only works on CPUs with those features
- **Distribution**: Need separate binaries per CPU generation

**Priority**: N/A (user choice)
**Effort**: None (configuration change)
**Risk**: None (standard Rust practice)

---

## 9. Dual Select Methods for Access Pattern Optimization ✅ IMPLEMENTED

**Status:** Completed January 2026

### Implementation
Added `ib_select1(k)` method using pure binary search alongside `ib_select1_from(k, hint)`:

```rust
impl JsonIndex {
    /// Fast for random access (indexed lookup like .[42])
    /// Uses pure binary search - O(log n)
    pub fn ib_select1(&self, k: usize) -> Option<usize>;

    /// Fast for sequential access (iteration like .users[])
    /// Uses exponential search from hint - O(log d)
    pub fn ib_select1_from(&self, k: usize, hint: usize) -> Option<usize>;
}
```

### Benchmark Results

| Benchmark | Time | Throughput |
|-----------|------|------------|
| Sequential (exponential) | 108 µs | 92.2 Melem/s |
| Sequential (binary) | 340 µs | 29.4 Melem/s |
| Random (exponential) | 1080 µs | 9.3 Melem/s |
| Random (binary) | 779 µs | **12.8 Melem/s** |

- Sequential: Exponential search **3.1x faster**
- Random: Binary search **39% faster** (recovers the regression)
- End-to-end JSON benchmarks: **No regression**

See [implemented-optimizations.md](implemented-optimizations.md#52-exponential-search-for-sequential-select) for details.

---

## Implementation Roadmap

### Phase 1: Quick Wins (Week 1)
1. ✅ Enable runtime dispatch in production (1 hour)
2. ✅ Add benchmark suite for dispatch overhead (2 hours)
3. ✅ Document CPU features and optimization guide (2 hours)

### Phase 2: AVX-512 Foundation (Week 2)
1. Implement AVX-512 JSON parser (8 hours)
2. Add AVX-512 popcount (4 hours)
3. Comprehensive testing across SIMD levels (4 hours)

### Phase 3: Advanced Optimizations (Week 3-4)
1. BMI2 integration for bit packing (8 hours)
2. SIMD prefix sum research + prototype (16 hours, x86 only - not recommended for ARM)
3. ~~BP SIMD optimization~~ ✓ Already implemented (byte-level lookup tables)

### Phase 4: Polish (Week 5)
1. Benchmarking and tuning (8 hours)
2. Documentation and examples (4 hours)
3. CI/CD for multi-target testing (4 hours)

---

## Benchmarking Guide

### Current Performance (SSE2 Baseline)
```bash
cargo bench --bench json_simd
```

### With Runtime Dispatch Enabled
```bash
# Edit src/json/simd/mod.rs to enable dispatch
cargo bench --bench json_simd

# Expected improvement: 1.5-1.8x on AVX2 systems
```

### With Compile-Time Features
```bash
# Native CPU
RUSTFLAGS="-C target-cpu=native" cargo bench --bench json_simd

# Expected improvement: 1.5-2x (no dispatch overhead)
```

### Verify CPU Features Used
```bash
# Check what LLVM thinks your CPU supports
rustc --print target-cpus
rustc --print target-features

# Inspect generated assembly
cargo rustc --release -- --emit asm
# Look for vpopcnt, vpmov, etc. in .s files
```

---

## Hardware Feature Summary

### AMD Ryzen 9 7950X (Zen 4) - Available Features

| Feature | Available | Notes |
|---------|-----------|-------|
| SSE2 | ✅ | Universal baseline (2001) |
| SSE4.2 | ✅ | PCMPISTRI string search (2008) |
| AVX2 | ✅ | 32-byte SIMD (2013) |
| BMI2 | ✅ | **Fast** on Zen 3+ (3-cycle PDEP/PEXT) |
| AVX-512F | ✅ | Foundation, 64-byte SIMD |
| AVX-512BW | ✅ | Byte/word operations |
| AVX-512DQ | ✅ | Doubleword/quadword ops |
| AVX-512VL | ✅ | Vector length extensions |
| AVX-512VPOPCNTDQ | ✅ | **8x parallel popcount** |
| AVX-512VBMI | ✅ | Vector byte manipulation |
| AVX-512VBMI2 | ✅ | Enhanced byte manipulation |
| AVX-512BITALG | ✅ | Bit algorithms |
| LZCNT | ✅ | Leading zero count |
| POPCNT | ✅ | Population count |
| VAES | ✅ | Vector AES (not used here) |

### Performance Expectations by Optimization Level

| Level | SIMD Width | Est. Throughput | Relative | Status |
|-------|------------|-----------------|----------|--------|
| SSE2 | 16 bytes | 5 GB/s | 1.0x | Current (prod) |
| AVX2 | 32 bytes | 9 GB/s | 1.8x | Available (tests) |
| AVX-512 | 64 bytes | 12 GB/s | 2.4x | Proposed |
| AVX-512 + prefix sum | 64 bytes | 18 GB/s | 3.6x | Future |

*Note: Throughput estimates for JSON semi-indexing on typical structured data*

---

## Testing Strategy

### SIMD Level Validation
Current test: [tests/simd_level_tests.rs](../tests/simd_level_tests.rs)

```bash
# Test all SIMD implementations produce identical results
cargo test --test simd_level_tests

# Explicitly test each level
cargo test --test simd_level_tests -- --nocapture
```

### Performance Regression Testing
```bash
# Baseline
git checkout main
cargo bench --bench json_simd > baseline.txt

# After optimization
git checkout feature-branch
cargo bench --bench json_simd > optimized.txt

# Compare
cargo install cargo-benchcmp
cargo benchcmp baseline.txt optimized.txt
```

### Property-Based Testing
```bash
# Ensure correctness across random inputs
cargo test --test property_tests
cargo test --test properties
```

---

## References

### Papers
- Langdale & Lemire, "Parsing Gigabytes of JSON per Second" (2019)
- Sadakane & Navarro, "Fully-Functional Succinct Trees", SODA 2010

### Implementations
- simdjson: https://github.com/simdjson/simdjson
- Rust stdarch: https://github.com/rust-lang/stdarch
- Ryzen 7950X optimization guide: https://www.amd.com/en/support/tech-docs

### Rust Resources
- `is_x86_feature_detected!` docs: https://doc.rust-lang.org/std/arch/macro.is_x86_feature_detected.html
- Target features: https://rust-lang.github.io/packed_simd/perf-guide/target-feature/rustflags.html
- SIMD performance guide: https://rust-lang.github.io/packed_simd/perf-guide/

---

## Rejected Optimizations

This section documents optimizations that were investigated but found to be ineffective or counterproductive.

### R1. NEON Batched Popcount for Cumulative Index (REJECTED)

**Date Investigated**: January 2026

**Hypothesis**: Use NEON `vcntq_u8` to batch popcount 8 words at once, then build prefix sum.

**Implementation Approach**:
```rust
#[cfg(target_arch = "aarch64")]
fn build_ib_rank_neon(words: &[u64]) -> Vec<u32> {
    use std::arch::aarch64::*;

    // Process 8 words (64 bytes) at a time using NEON
    while i + 8 <= words.len() {
        unsafe {
            // Load and popcount 64 bytes
            let v0 = vld1q_u8(ptr);  // ...
            let c0 = vcntq_u8(v0);   // Per-byte popcount
            // Sum to per-word counts via vpaddlq_*
            let octo0 = vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(c0)));

            // Extract 8 individual word counts
            let counts: [u32; 8] = [
                vgetq_lane_u64(octo0, 0) as u32,  // 14 lane extractions!
                // ...
            ];

            // Still need sequential prefix sum
            for c in counts {
                cumulative += c;
                rank.push(cumulative);
            }
        }
    }
}
```

**Benchmark Results** (Apple M1, 1M words = 8MB):

| Implementation | Time | Relative |
|----------------|------|----------|
| Scalar (`count_ones()`) | 542 µs | 1.00x |
| NEON batched | 722 µs | **0.75x (25% slower!)** |

**Why It Failed**:
1. **Extraction overhead**: 14 `vgetq_lane` calls per 8 words to extract individual counts
2. **Sequential dependency**: Prefix sum still requires cumulative += count for each word
3. **LLVM optimization**: `count_ones()` already compiles to efficient `CNT` instruction on ARM64
4. **Memory pressure**: Additional register shuffling for lane extraction

**Lesson Learned**: SIMD batching helps for *total* sums but not for *prefix* sums where each output depends on all previous values. The bottleneck is the data dependency chain, not the popcount operation itself.

**Alternative Considered**: Parallel prefix sum using SIMD, but:
- Adds significant complexity
- Would require 64 words minimum to amortize overhead
- The cumulative index is only built once during construction

**Conclusion**: Keep the simple scalar implementation in `build_ib_rank()`. LLVM's auto-vectorization is already near-optimal for this use case.

---

## Conclusion

The Ryzen 9 7950X provides exceptional SIMD capabilities that are currently underutilized. The most impactful optimizations are:

1. **Enable runtime dispatch** (critical, trivial effort)
2. **AVX-512 implementation** (high impact, moderate effort)
3. **AVX-512 VPOPCNTDQ** (high impact for rank operations)

These three changes alone could yield **2.5-3x performance improvement** for typical JSON parsing workloads.

The current `#[cfg(test)]` gate on runtime dispatch is the primary bottleneck - fixing this is the highest priority action item.
