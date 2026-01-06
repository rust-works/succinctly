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

## ARM NEON Optimizations (M1/M2/M3/M4)

This section documents ARM-specific optimization opportunities for Apple Silicon and other ARMv8+ processors.

**Analysis Date:** January 2026
**Target:** Apple M1-M4, AWS Graviton, Ampere Altra

---

### Current Implementation

The NEON implementation in `src/json/simd/neon.rs` uses:
- Individual byte comparisons (`vceqq_u8`) for 8+ character types
- Custom `neon_movemask()` using 6 instructions
- Range checks for alphanumeric detection (`vcgeq_u8`, `vcleq_u8`)

**Current instruction count for `classify_chars()`:** ~35-40 NEON instructions

---

### NEON-1: Nibble Lookup Tables (`vqtbl1q_u8`)

**Impact:** 20-40% character classification speedup
**Complexity:** Medium
**Estimated Lines:** 150-200

**Current Issue:**
The `classify_chars()` function (lines 73-151) performs ~12 individual comparisons plus ~8 ORs to detect structural characters:

```rust
// Current: 8 comparisons + 4 ORs = ~12 instructions for structural
let eq_quote = vceqq_u8(chunk, v_quote);
let eq_backslash = vceqq_u8(chunk, v_backslash);
let eq_open_brace = vceqq_u8(chunk, v_open_brace);
// ... 5 more comparisons
let opens = vorrq_u8(eq_open_brace, eq_open_bracket);
// ... 3 more ORs
```

**Opportunity:**
The [simdjson approach](https://arxiv.org/html/1902.08318v7) uses nibble-based lookup tables to classify all characters with just 2 lookups + 1 AND:

```rust
// Optimized: 2 lookups + 1 AND + 2 shifts = ~5 instructions
unsafe fn classify_structural_nibble(chunk: uint8x16_t) -> uint8x16_t {
    // Split each byte into high/low nibbles
    let lo_nibble = vandq_u8(chunk, vdupq_n_u8(0x0F));
    let hi_nibble = vshrq_n_u8(chunk, 4);

    // Lookup tables from simdjson paper
    // Low nibble table: encodes which low nibbles could be structural
    let lo_table: [u8; 16] = [16, 0, 0, 0, 0, 0, 0, 0, 0, 8, 10, 4, 1, 12, 0, 0];
    // High nibble table: encodes which high nibbles could be structural
    let hi_table: [u8; 16] = [8, 0, 17, 2, 0, 4, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0];

    let lo_lookup = vqtbl1q_u8(vld1q_u8(lo_table.as_ptr()), lo_nibble);
    let hi_lookup = vqtbl1q_u8(vld1q_u8(hi_table.as_ptr()), hi_nibble);

    // Non-zero where character is structural
    vandq_u8(lo_lookup, hi_lookup)
}
```

**How it works:**
- Each ASCII character has a unique (high_nibble, low_nibble) pair
- The tables are crafted so `lo_table[lo] & hi_table[hi] != 0` only for target characters
- Different bits in the result indicate different character classes

**Character classification with nibble lookup:**

| Character | Hex  | Hi | Lo | lo_table[Lo] | hi_table[Hi] | AND result |
|-----------|------|----|----|--------------|--------------|------------|
| `{`       | 0x7B | 7  | B  | 4            | 4            | 4 (match)  |
| `}`       | 0x7D | 7  | D  | 12           | 4            | 4 (match)  |
| `[`       | 0x5B | 5  | B  | 4            | 4            | 4 (match)  |
| `]`       | 0x5D | 5  | D  | 12           | 4            | 4 (match)  |
| `:`       | 0x3A | 3  | A  | 10           | 2            | 2 (match)  |
| `,`       | 0x2C | 2  | C  | 1            | 17           | 1 (match)  |
| `"`       | 0x22 | 2  | 2  | 0            | 17           | 0 (separate check needed) |
| `a`       | 0x61 | 6  | 1  | 0            | 0            | 0 (no match) |

**Implementation notes:**
- Quote (`"`) and backslash (`\`) still need separate checks (escape handling)
- Can combine nibble result with separate quote/backslash masks
- Tables can be tuned for specific character sets

**References:**
- [simdjson paper Section 4.1](https://arxiv.org/html/1902.08318v7)
- [Daniel Lemire: Locating identifiers with NEON](https://lemire.me/blog/2023/09/04/locating-identifiers-quickly-arm-neon-edition/)

---

### NEON-2: Optimized Movemask

**Impact:** 10-20% per-chunk speedup
**Complexity:** Low
**Estimated Lines:** 50-80

**Current Issue:**
The `neon_movemask()` function (lines 27-48) uses 6 instructions:

```rust
// Current: ~6 instructions
let high_bits = vshrq_n_u8::<7>(v);
let shifted = vshlq_u8(high_bits, shifts);  // Variable shift (slow)
let low_sum = vaddv_u8(vget_low_u8(shifted));
let high_sum = vaddv_u8(vget_high_u8(shifted));
low_sum | (high_sum << 8)
```

**Opportunity:**
Use narrowing instructions and direct register transfer:

```rust
// Optimized: ~4 instructions
unsafe fn neon_movemask_fast(v: uint8x16_t) -> u16 {
    // Narrow 16-bit lanes, taking high byte of each pair
    let paired = vpaddq_u8(v, v);  // Pairwise add (brings high bits together)
    let narrowed = vshrn_n_u16(vreinterpretq_u16_u8(paired), 4);

    // Alternative using SHRN + bit manipulation
    let high_bits = vshrq_n_u8::<7>(v);
    // Use polynomial multiply or dedicated sequence
    // ...
}
```

**Alternative using `vsri` (shift right and insert):**

```rust
unsafe fn neon_movemask_vsri(v: uint8x16_t) -> u16 {
    // Pack high bits using shift-right-insert
    let v1 = vsriq_n_u8(v, v, 7);
    // Continue packing...
}
```

**M1-specific consideration:**
The [simdjson M1 discussion](https://github.com/simdjson/simdjson/discussions/1658) notes M1 Firestorm can issue 4 NEON ops/cycle. The current variable shift (`vshlq_u8` with vector shift amounts) may be slower than fixed shifts.

---

### NEON-3: Batch Movemask Extraction

**Impact:** 5-10% per-chunk speedup
**Complexity:** Low
**Estimated Lines:** 80-120

**Current Issue:**
`classify_chars()` calls `neon_movemask()` 6 times (lines 139-148):

```rust
let quotes = neon_movemask(eq_quote);
let backslashes = neon_movemask(eq_backslash);
let opens = neon_movemask(opens);
let closes = neon_movemask(closes);
let delims = neon_movemask(delims);
let value_chars = neon_movemask(value_chars);
```

**Opportunity:**
Interleave the movemask operations to better utilize M1's 4-wide NEON dispatch:

```rust
// Process pairs of masks in parallel
// M1 can execute multiple independent NEON ops per cycle
unsafe fn batch_movemask_4(
    v0: uint8x16_t, v1: uint8x16_t,
    v2: uint8x16_t, v3: uint8x16_t
) -> (u16, u16, u16, u16) {
    // Interleaved operations for better ILP
    let h0 = vshrq_n_u8::<7>(v0);
    let h1 = vshrq_n_u8::<7>(v1);
    let h2 = vshrq_n_u8::<7>(v2);
    let h3 = vshrq_n_u8::<7>(v3);
    // Continue with interleaved shifts and adds...
}
```

---

### NEON-4: `vtstq_u8` for Bit Testing

**Impact:** Minor (2-5%)
**Complexity:** Very Low
**Estimated Lines:** ~20

**Current Issue:**
When checking if classification bits are set, the code uses comparison results directly.

**Opportunity:**
Use `vtstq_u8` (vector test bits) for more efficient "any bits set" checks:

```rust
// Instead of checking result == 0xFF
let classified = vtstq_u8(lo_result, hi_result);  // Non-zero if any matching bits
```

This is particularly useful with nibble lookup where different bits indicate different classes.

---

### NEON-5: SIMD Popcount for Rank Operations

**Impact:** 5-10% for bulk rank operations
**Complexity:** Low
**Estimated Lines:** 60-100

**Current Issue:**
Rank operations use scalar `count_ones()`:

```rust
cumulative += word.count_ones();
```

**Opportunity:**
Use NEON `vcntq_u8` for bulk popcount:

```rust
unsafe fn popcount_words_neon(words: &[u64]) -> u64 {
    let mut total = 0u64;
    let mut i = 0;

    // Process 2 words (16 bytes) at a time
    while i + 2 <= words.len() {
        let chunk = vld1q_u8(words[i..].as_ptr() as *const u8);
        let counts = vcntq_u8(chunk);  // Popcount per byte
        total += vaddvq_u8(counts) as u64;  // Horizontal sum
        i += 2;
    }

    // Handle remaining
    for w in &words[i..] {
        total += w.count_ones() as u64;
    }
    total
}
```

**Note:** For single-word popcount, scalar `count_ones()` compiles to efficient `CNT` instruction on ARM. SIMD is only beneficial for bulk operations (8+ words).

---

### Apple Silicon Specific: M1-M4 Considerations

#### What's Available (All M-series)

| Instruction | Use Case | Notes |
|-------------|----------|-------|
| `vqtbl1q_u8` | Nibble lookup | Single 16-byte table |
| `vqtbl2q_u8` | Extended lookup | Two 16-byte tables (32 entries) |
| `vqtbl4q_u8` | Full byte map | Four tables (64 entries) |
| `vcntq_u8` | Popcount | Per-byte population count |
| `vaddvq_u8` | Horizontal add | Sum all bytes |
| `vpaddq_u8` | Pairwise add | Adjacent byte pairs |
| `vshrn_n_u16` | Narrow | 16-bit to 8-bit with shift |

#### M4-Specific (Not Useful for JSON)

| Feature | Status | Why Not Useful |
|---------|--------|----------------|
| SME (Matrix Extension) | Available | Designed for ML matrix ops, not byte streams |
| Streaming SVE | Available | Mode transitions zero registers; too costly |
| SVE2 MATCH | Unknown | May not be in M4's streaming subset |

#### Performance Characteristics

From [simdjson M1 benchmarks](https://github.com/simdjson/simdjson/discussions/1658):
- M1 Firestorm: 4 NEON instructions/cycle
- NEON latency: 2-4 cycles for most ops
- `vqtbl1q_u8`: 2 cycle latency, 1/cycle throughput
- Memory bandwidth: 60+ GB/s (not the bottleneck)

---

### Implementation Priority for ARM

| Priority | Optimization | Expected Gain | Effort |
|----------|-------------|---------------|--------|
| 1 | Nibble lookup tables | 20-40% classify | Medium |
| 2 | Batch movemask | 5-10% per chunk | Low |
| 3 | Optimized movemask | 10-20% per chunk | Low |
| 4 | SIMD popcount bulk | 5-10% rank ops | Low |
| 5 | vtstq_u8 bit test | 2-5% | Very Low |

**Recommended order:** Start with nibble lookup (biggest win), then batch movemask (easy), then optimized movemask.

---

### Benchmarking ARM Optimizations

```bash
# Run on Apple Silicon
cargo bench --bench json_simd -- neon

# Compare with scalar baseline
cargo bench --bench json_simd -- scalar

# Profile with Instruments
xcrun xctrace record --template 'CPU Profiler' --launch -- \
    ./target/release/succinctly json query '.users[]' large.json
```

---

## References

- Sadakane & Navarro, "Fully-Functional Succinct Trees", SODA 2010
- haskell-works hw-json and hw-balancedparens implementations
- Intel Intrinsics Guide for AVX2/BMI2 instructions
- [simdjson paper (Langdale & Lemire)](https://arxiv.org/html/1902.08318v7) - Nibble lookup technique
- [simdjson M1 optimization discussion](https://github.com/simdjson/simdjson/discussions/1658)
- [Daniel Lemire: Locating identifiers with NEON](https://lemire.me/blog/2023/09/04/locating-identifiers-quickly-arm-neon-edition/)
- [ARM NEON Intrinsics Reference](https://developer.arm.com/architectures/instruction-sets/intrinsics/)
- [Apple M4 SME exploration](https://github.com/tzakharko/m4-sme-exploration)
