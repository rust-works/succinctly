# Performance Optimization Analysis

This document identifies performance optimization opportunities in the succinctly library, prioritized by impact and implementation complexity.

**Analysis Date:** January 2026
**Codebase Size:** ~26,500 lines of Rust

---

## Executive Summary

The succinctly library is well-architected with good SIMD utilization. However, several bottlenecks exist in hot paths that could yield 10-25% overall throughput improvement with targeted optimizations.

**Top 3 Opportunities:**
1. JSON SIMD escape sequence preprocessing (15-25% JSON parsing speedup)
2. BP find_close state machine unrolling (10-20% BP query speedup)
3. Select index exponential search (8-15% select1() speedup)

---

## Tier 1: Highest Impact (10-25% improvement)

### 1. JSON SIMD Escape Sequence Cache

**Impact:** 15-25% JSON parsing speedup for strings with escapes
**Complexity:** Medium
**Estimated Lines:** 300-500

**Current Issue:**
- AVX2/SSE2 implementations in `src/json/simd/avx2.rs` and `src/json/simd/x86.rs` process every byte individually for escape detection
- The escape handling state (`InEscape`) requires sequential bit-by-bit scanning after SIMD classification
- For typical JSON with many escaped characters, the state machine re-examines bytes in multiple passes

**Opportunity:**
- Pre-compute escape mask in parallel: identify all quotes AND preceding backslashes in one SIMD pass
- Use `_mm256_alignr_epi8` to check `byte[i-1] == '\\' && byte[i]` simultaneously for all positions
- Cache escape positions in a side buffer to skip re-scanning

**Implementation Approach:**
```rust
// Conceptual approach - shift and compare in parallel
let prev_bytes = _mm256_alignr_epi8(chunk, prev_chunk, 15);
let is_backslash = _mm256_cmpeq_epi8(prev_bytes, backslash_vec);
let escaped_quotes = _mm256_and_si256(is_quote, is_backslash);
```

---

### 2. BP find_close State Machine Unrolling

**Impact:** 10-20% BP query speedup
**Complexity:** Medium
**Estimated Lines:** 250-400

**Current Issue:**
- `find_close_from()` in `src/bp.rs` (lines 927-1078) uses a 7-state machine:
  - `ScanBit`, `CheckL0`, `CheckL1`, `CheckL2`, `FromL0`, `FromL1`, `FromL2`
- State transitions on every bit check create branch mispredictions
- Typical paths: `ScanBit` → `FromL0` → `CheckL0` → back to `ScanBit`
- Excess tracking involves i32 arithmetic and repeated comparisons per bit

**Opportunity:**
- **Unroll inner loop:** Process 4-8 bits at a time using precomputed excess deltas from lookup tables (similar to existing `word_min_excess`)
- **Reduce state transitions:** Combine `ScanBit` and `FromL0` into single state
- **SIMD within register:** Use `_lzcnt_u64` to skip consecutive zeros
- **Hot path specialization:** Fast path for distances < 64 bits that skips L1/L2 checks

**Implementation Approach:**
```rust
// Lookup table for 4-bit chunks: (min_excess_delta, total_excess_delta, match_position_if_any)
const NIBBLE_EXCESS: [(i8, i8, Option<u8>); 16] = /* precomputed */;

// Process 4 bits at a time
let nibble = (word >> bit_pos) & 0xF;
let (min_delta, total_delta, match_pos) = NIBBLE_EXCESS[nibble as usize];
if excess + min_delta <= 0 {
    // Match in this nibble - fine-grained scan
} else {
    excess += total_delta;
    bit_pos += 4;
}
```

---

### 3. Select Index Exponential Search

**Impact:** 8-15% select1() speedup
**Complexity:** Low-Medium
**Estimated Lines:** 150-250

**Current Issue:**
- `select1()` in `src/bitvec.rs` (lines 223-251) uses sampled index to jump, then linear word scan
- Default `sample_rate=256` means average 128 words scanned per select query
- Linear scan is cache-unfriendly (poor prefetching)
- Each word lookup requires full `popcount_word()` call

**Opportunity:**
- **Exponential search prelude:** After jumping to sample point, use exponential jumps (1→2→4→8 words) to locate target word
- **Reduce popcount calls:** Track cumulative counts during scan
- **Prefetch guidance:** Prefetch 2-3 words ahead to warm L1 cache
- **Batch select:** For consecutive selects (common in iteration), batch up to 8 queries

**Implementation Approach:**
```rust
// Exponential search after sample jump
let mut step = 1;
let mut pos = sample_pos;
while pos + step < words.len() && cumulative + word_popcount(pos + step) < k {
    cumulative += word_popcount(pos + step);
    pos += step;
    step *= 2;
}
// Binary search within [pos, pos + step]
```

---

## Tier 2: Medium Impact (5-10% improvement)

### 4. Popcount SIMD Batching

**Impact:** 4-8% overall throughput
**Complexity:** Low
**Estimated Lines:** 100-200

**Current Issue:**
- `popcount_words()` in `src/popcount.rs` uses loop of `word.count_ones()`
- Relies on LLVM auto-vectorization (not guaranteed)
- NEON/x86 branches optimized but portable fallback weak

**Opportunity:**
- Explicit AVX2/NEON `popcount_words_simd()` processing 8 words at once
- Process in cache-line-sized chunks (8 words = 64 bytes)

---

### 5. JsonIndex Lazy ib_rank Construction

**Impact:** 2-5% index build time, 10-15% memory for simple queries
**Complexity:** Low
**Estimated Lines:** 100-150

**Current Issue:**
- `build_ib_rank()` in `src/json/light.rs` always computes cumulative rank
- For simple navigation (finding root), rank data is never used

**Opportunity:**
- Lazy construction with `OnceCell<Vec<u32>>`
- Build rank index only on first access

---

### 6. BP Batch Query API

**Impact:** 6-10% for tree traversal workloads
**Complexity:** Medium
**Estimated Lines:** 300-500

**Current Issue:**
- Calling `bp.find_close()` 1000 times means 1000 independent state machine instances
- No sharing of intermediate excess computations

**Opportunity:**
- Batch multiple `find_close()` calls sharing excess computation
- Cache path excess values for tree navigation patterns

---

## Tier 3: Quick Wins (Low effort, measurable gain)

### 7. Cache-Align RankDirectory ✓ IMPLEMENTED

**Impact:** 3-4% rank query improvement (measured)
**Status:** Completed

The `RankDirectory` L1+L2 data is now allocated with 64-byte cache-line alignment using custom allocation via `alloc::alloc::alloc` with `Layout::from_size_align(size, 64)`.

---

### 8. BitWriter Batch Methods

**Impact:** 2-5% JSON parsing
**Complexity:** Low
**Estimated Lines:** ~80

**Implementation:**
```rust
impl BitWriter {
    /// Write up to 64 bits in one operation
    pub fn write_bits(&mut self, bits: u64, count: usize) {
        // Single word boundary check instead of per-bit
    }
}
```

---

### 9. Broadword select_in_word Hybrid

**Impact:** 2-4% select query latency
**Complexity:** Low
**Estimated Lines:** ~70

**Current Issue:**
- Loop with bit-by-bit extraction has data dependency per iteration

**Implementation:**
```rust
pub fn select_in_word(word: u64, k: u32) -> u32 {
    if k < 4 {
        // CTZ loop for small k (latency-sensitive)
        let mut val = word;
        for _ in 0..k {
            val &= val - 1;
        }
        val.trailing_zeros()
    } else {
        // Broadword algorithm for larger k
        select_in_word_broadword(word, k)
    }
}
```

---

## Memory/Cache Issues

### Issue 1: BalancedParens Allocation Overhead

**Current:** 8 separate `Vec` allocations
- `l0_min_excess`, `l0_word_excess`
- `l1_min_excess`, `l1_block_excess`
- `l2_min_excess`, `l2_block_excess`
- `rank_l1`, `rank_l2`

**Opportunity:** Consolidate into single allocation with computed offsets

### Issue 2: RankDirectory Cache Line Crossing ✓ RESOLVED

**Previous:** L1 and L2 in separate allocations, queries cross cache lines

**Solution:** L1+L2 data now uses 64-byte aligned allocation (see Item 7)

### Issue 3: BitWriter Flush Frequency

**Current:** Word-by-word flushing causes L1 cache pressure

**Opportunity:** Buffer multiple words before flush

---

## SIMD Utilization Gaps

| Gap | Current State | Opportunity |
|-----|--------------|-------------|
| AVX2 escape handling | Sequential after classification | Pre-compute escape masks in parallel |
| NEON for BP operations | Not used | Port key BP operations to NEON |
| BMI2 PDEP/PEXT | Disabled (AMD Zen 1/2 slow) | Enable with CPU detection |

---

## Implementation Roadmap

### Phase 1: Quick Wins (~150 lines remaining)

1. ~~Cache-align RankDirectory~~ ✓ **DONE** (3-4% improvement measured)
2. BitWriter batch methods
3. Broadword select hybrid

**Expected Impact:** 3-8% overall improvement

### Phase 2: Medium Effort (~400 lines, 3-5 days)

5. Popcount SIMD batching
6. Select exponential search

**Expected Impact:** Additional 5-10% improvement

### Phase 3: Higher Effort (~600 lines, 1-2 weeks)

7. BP find_close unrolling
8. JSON SIMD escape cache

**Expected Impact:** Additional 10-20% improvement

---

## Benchmarking Notes

Key benchmarks to measure impact:

```bash
# BP operations
cargo bench --bench balanced_parens

# JSON parsing throughput
cargo bench --bench json_pipeline

# Rank/select operations
cargo bench --bench rank_select

# Full comparison with jq
cd bench-compare && cargo bench
```

---

## Not Recommended

| Optimization | Reason |
|-------------|--------|
| Multi-threading JSON | Very high complexity (2000+ lines), only helps files >10MB |
| Select0 index | Low demand for 0-bit queries in JSON workloads |
| Full BMI2 dependency | AMD Zen 1/2 have 18-cycle microcode PDEP/PEXT |
| BP rank_l1 u32→u16 | Real JSON easily exceeds 65K structural elements (10MB file has ~1.5M); u16 would silently overflow |

---

## References

- Sadakane & Navarro, "Fully-Functional Succinct Trees", SODA 2010
- haskell-works hw-json and hw-balancedparens implementations
- Intel Intrinsics Guide for AVX2/BMI2 instructions
