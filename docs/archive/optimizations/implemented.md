# Implemented Optimizations

This document catalogs all optimization techniques currently implemented in the succinctly library.

**Last Updated:** January 2026

---

## 1. Bit Vector Operations

### 1.1 Three-Level Rank Directory (Poppy-style)

**File:** [src/bits/rank.rs](../../../src/bits/rank.rs)

The rank directory enables O(1) rank queries using a hierarchical structure:

| Level | Granularity        | Storage   | Purpose                               |
|-------|--------------------|-----------|---------------------------------------|
| L0    | 2^32 bits          | u64       | Absolute rank (only for >4GB vectors) |
| L1    | 512 bits (8 words) | u32       | Cumulative rank per block             |
| L2    | Within block       | 7 x 9-bit | Offsets within each 512-bit block     |

**Memory overhead:** ~3% of bitvector size

**How it works:**
```rust
// Query: rank1(position)
let block = position / 512;
let word_in_block = (position % 512) / 64;
let bit_in_word = position % 64;

let rank = l0[block >> 16]           // Absolute checkpoint
         + l1[block]                  // Block cumulative
         + l2_offset[word_in_block]   // Within-block offset
         + popcount(word & mask);     // Partial word
```

### 1.2 Cache-Aligned L1+L2 Storage

**File:** [src/bits/rank.rs](../../../src/bits/rank.rs)

The rank directory uses 64-byte cache-line aligned allocation:

```rust
// Custom allocation with cache-line alignment
let layout = Layout::from_size_align(size, 64).unwrap();
let ptr = alloc::alloc::alloc(layout);
```

**Benefits:**
- 4 entries (u128 each = 16 bytes) fit per 64-byte cache line
- Eliminates cache line crossing penalties
- Measured improvement: 3-4% on rank queries

### 1.3 Sampled Select Index

**File:** [src/bits/select.rs](../../../src/bits/select.rs)

Select queries use a sampled index for O(log n) acceleration:

| Sample Rate   | Memory Overhead | Speed        |
|---------------|-----------------|--------------|
| 64            | ~12.5%          | Fastest      |
| 256 (default) | ~3%             | Balanced     |
| 512           | ~1.5%           | Most compact |

**Algorithm:**
1. Binary search the sample index to find approximate word
2. Linear scan from sample point to exact bit position

### 1.4 Popcount Strategies

**File:** [src/bits/popcount.rs](../../../src/bits/popcount.rs)

Three mutually exclusive implementations selected via feature flags:

| Feature             | Technique                       | Best For                          |
|---------------------|---------------------------------|-----------------------------------|
| (default)           | Rust `count_ones()`             | Auto-vectorizes on most platforms |
| `simd`              | Explicit NEON/POPCNT intrinsics | Maximum throughput                |
| `portable-popcount` | SWAR bitwise algorithm          | Portability testing               |

**NEON bulk popcount (64 bytes at a time):**
```rust
let counts = vcntq_u8(chunk);           // Per-byte popcount
let pairs = vpaddlq_u8(counts);         // Pairwise add to u16
let quads = vpaddlq_u16(pairs);         // Pairwise add to u32
total += vaddvq_u32(quads);             // Horizontal sum
```

### 1.5 Broadword Select-in-Word

**File:** [src/util/broadword.rs](../../../src/util/broadword.rs)

Finding the k-th set bit within a single u64 word:

```rust
// CTZ loop approach - O(k) but fast on modern CPUs
pub fn select_in_word(word: u64, k: u32) -> u32 {
    let mut val = word;
    for _ in 0..k {
        val &= val - 1;  // Clear lowest set bit
    }
    val.trailing_zeros()
}
```

Uses TZCNT instruction on modern x86/ARM, which is single-cycle.

---

## 2. Balanced Parentheses (BP) Operations

### 2.1 Byte Lookup Tables

**File:** [src/trees/bp.rs](../../../src/trees/bp.rs)

Precomputed tables enable ~20x faster word-level excess computation:

```rust
/// Minimum excess seen while scanning each byte value
const BYTE_MIN_EXCESS: [i8; 256] = [...];

/// Total excess (opens - closes) for each byte value
const BYTE_TOTAL_EXCESS: [i8; 256] = [...];
```

**Usage:** Process 8 bits at once instead of bit-by-bit scanning.

### 2.2 RangeMin Hierarchical Structure

**File:** [src/trees/bp.rs](../../../src/trees/bp.rs)

Enables O(1) `find_close` via 3-level excess tracking:

| Level | Granularity | Storage              | Purpose               |
|-------|-------------|----------------------|-----------------------|
| L0    | Per word    | (i8 min, i16 total)  | Word-level excess     |
| L1    | 32 words    | (i16 min, i16 total) | Block aggregate       |
| L2    | 1024 words  | (i16 min, i16 total) | Super-block aggregate |

**Memory overhead:** ~6-7% of BP size

**State machine acceleration:** Skips entire L0/L1/L2 blocks when minimum excess cannot reach zero.

### 2.3 BP Rank Index

Reuses the same Poppy-style 3-level hierarchy as bitvector rank for O(1) rank queries on the BP sequence.

---

## 3. JSON SIMD Parsing

### 3.1 Runtime SIMD Dispatch

**Files:** [src/json/simd/](../../../src/json/simd/)

Automatic selection of best available instruction set:

| Level  | Width   | Bytes/Iter | Availability | File     |
|--------|---------|------------|--------------|----------|
| AVX2   | 256-bit | 32         | ~95% (2013+) | avx2.rs  |
| SSE4.2 | 128-bit | 16         | ~90% (2008+) | sse42.rs |
| SSE2   | 128-bit | 16         | 100%         | x86.rs   |
| NEON   | 128-bit | 16         | 100% (ARM64) | neon.rs  |

**Dispatch logic:**
```rust
if is_x86_feature_detected!("avx2") {
    avx2::build_semi_index_standard(bytes)
} else if is_x86_feature_detected!("sse4.2") {
    sse42::build_semi_index_standard(bytes)
} else {
    x86::build_semi_index_standard(bytes)  // SSE2 baseline
}
```

### 3.2 Parallel Character Classification

All bytes in a SIMD register classified simultaneously:

```rust
// AVX2: 32 bytes at once
let eq_quote = _mm256_cmpeq_epi8(chunk, quote_vec);
let eq_open_brace = _mm256_cmpeq_epi8(chunk, open_brace_vec);
// ... combine into masks
let structural_mask = _mm256_movemask_epi8(combined) as u32;
```

### 3.3 NEON Nibble Lookup Tables (simdjson technique)

**File:** [src/json/simd/neon.rs](../../../src/json/simd/neon.rs)

Character classification using nibble lookup replaces ~12 comparisons + ~8 ORs with just 2 lookups + 1 AND:

```rust
// Nibble tables encode character classes in bit flags
const LO_NIBBLE_TABLE: [u8; 16] = [
    0x00, 0x00, 0x08, 0x00,  // 0-3: bit 3 for quote (lo=2)
    0x00, 0x00, 0x00, 0x00,  // 4-7
    0x00, 0x00, 0x04, 0x01,  // 8-B: colon (A), opens (B)
    0x14, 0x02, 0x00, 0x00,  // C-F: comma/backslash (C), closes (D)
];
const HI_NIBBLE_TABLE: [u8; 16] = [
    0x00, 0x00, 0x0C, 0x04,  // hi=2: comma,quote; hi=3: colon
    0x00, 0x13, 0x00, 0x03,  // hi=5: []\; hi=7: {}
    // ...
];

// Classification: split byte into nibbles, lookup both, AND results
let lo_nibble = vandq_u8(chunk, vdupq_n_u8(0x0F));
let hi_nibble = vshrq_n_u8::<4>(chunk);
let lo_result = vqtbl1q_u8(lo_table, lo_nibble);  // NEON table lookup
let hi_result = vqtbl1q_u8(hi_table, hi_nibble);
let classified = vandq_u8(lo_result, hi_result);   // AND gives final flags
```

**Why it works:** Each ASCII character is uniquely identified by (lo_nibble, hi_nibble). The tables are designed so the AND of both lookups produces the correct class bits.

**Performance:** 2 `vqtbl1q_u8` + 1 `vandq_u8` vs 8+ `vceqq_u8` + 4+ `vorrq_u8` in the comparison approach.

**Benchmark Results (Apple M1):**

| Pattern       | Size | NEON (nibble)       | Scalar              | Speedup   |
|---------------|------|---------------------|---------------------|-----------|
| comprehensive | 1MB  | 1.62 ms (500 MiB/s) | 2.04 ms (395 MiB/s) | **1.26x** |
| comprehensive | 10MB | 15.9 ms (503 MiB/s) | 20.1 ms (398 MiB/s) | **1.26x** |
| strings       | 1MB  | 481 µs (1.78 GiB/s) | 1.46 ms (601 MiB/s) | **3.0x**  |

The 3x speedup on string-heavy JSON comes from the combination of nibble lookup + fast-path string scanning.

### 3.4 NEON Custom Movemask (Optimized)

**File:** [src/json/simd/neon.rs](../../../src/json/simd/neon.rs)

ARM NEON lacks a direct movemask instruction. Uses multiplication trick instead of slow variable shifts:

```rust
unsafe fn neon_movemask(v: uint8x16_t) -> u16 {
    // Shift right by 7 to get 0 or 1 in each byte
    let high_bits = vshrq_n_u8::<7>(v);

    // Extract as two u64 values
    let low_u64 = vgetq_lane_u64::<0>(vreinterpretq_u64_u8(high_bits));
    let high_u64 = vgetq_lane_u64::<1>(vreinterpretq_u64_u8(high_bits));

    // Multiplication trick: magic number positions each byte at the right bit
    const MAGIC: u64 = 0x0102040810204080;
    let low_packed = (low_u64.wrapping_mul(MAGIC) >> 56) as u8;
    let high_packed = (high_u64.wrapping_mul(MAGIC) >> 56) as u8;

    (low_packed as u16) | ((high_packed as u16) << 8)
}
```

**Why:** Variable shifts (`vshlq_u8` with vector amounts) are slow on M1/M2, while fixed shifts and scalar multiplications are fast.

### 3.5 Fast-Path String Scanning

**File:** [src/json/simd/neon.rs](../../../src/json/simd/neon.rs)

When inside a string, skip directly to next quote or backslash:

```rust
// Combined mask for quick scanning
string_special: quotes | backslashes,

// In State::InString:
let special_remaining = class.string_special & remaining_mask;
if special_remaining == 0 {
    // No quotes/backslashes - batch write zeros
    ib.write_zeros(len - i);
    return State::InString;
}
// Jump directly to next special character
let next_special = special_remaining.trailing_zeros() as usize;
```

**Measured improvement:** 2.5-8x speedup on string-heavy JSON.

---

## 4. BitWriter Batch Operations

### 4.1 Batch Zero Writing

**File:** [src/json/bit_writer.rs](../../../src/json/bit_writer.rs)

Write multiple zero bits without per-bit iteration:

```rust
pub fn write_zeros(&mut self, count: usize) {
    // Fill current word if mid-word
    if self.bit_position > 0 {
        let space = 64 - self.bit_position as usize;
        if count < space {
            self.bit_position += count as u32;
            return;
        }
        self.words.push(self.current_word);
        remaining -= space;
        self.bit_position = 0;
    }

    // Push full words of zeros
    let full_words = remaining / 64;
    for _ in 0..full_words {
        self.words.push(0);
    }

    // Handle leftover bits
    self.bit_position = (remaining % 64) as u32;
}
```

### 4.2 Batch Bit Writing

**File:** [src/json/bit_writer.rs](../../../src/json/bit_writer.rs)

Write up to 64 bits in a single operation:

```rust
pub fn write_bits(&mut self, bits: u64, count: usize) {
    let space_in_word = 64 - self.bit_position as usize;

    if count <= space_in_word {
        // All bits fit in current word
        let mask = (1u64 << count) - 1;
        self.current_word |= (bits & mask) << self.bit_position;
        self.bit_position += count as u32;
    } else {
        // Spans two words - handle boundary
        // ...
    }
}
```

**Note:** Best for 8+ bits. For 2 bits, individual writes are faster due to inlining.

---

## 5. JSON Query Optimization

### 5.1 Cumulative Index for O(log n) Select

**File:** [src/json/light.rs](../../../src/json/light.rs)

The critical optimization that achieved 627x speedup:

**Problem:** Original `ib_select1()` was O(n) linear scan per call. With O(n) results, total complexity was O(n²).

**Solution:** Cumulative popcount index enables binary search:

```rust
// Build cumulative index during construction
fn build_ib_rank(words: &[u64]) -> Vec<u32> {
    let mut rank = Vec::with_capacity(words.len() + 1);
    let mut cumulative: u32 = 0;
    rank.push(0);
    for &word in words {
        cumulative += word.count_ones();
        rank.push(cumulative);
    }
    rank
}

// Binary search for select
fn ib_select1(&self, k: usize) -> Option<usize> {
    let k32 = k as u32;
    let mut lo = 0;
    let mut hi = self.ib_words.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if self.ib_rank[mid + 1] <= k32 { lo = mid + 1; }
        else { hi = mid; }
    }
    // Scan within word for exact position
    // ...
}
```

**Result:** `.unicode[]` on 10MB file: 2.76s → 4.4ms (627x faster, now 5x faster than jq)

### 5.2 Fast Random Access for Array Indexing (get_fast)

**Files:** [src/json/light.rs](../../../src/json/light.rs), [src/jq/eval.rs](../../../src/jq/eval.rs)

**Problem:** Array indexing via `JsonElements::get(n)` was calling `value()` (which triggers `text_position()` and IB select) for every intermediate element. Accessing element 100 required 101 IB select operations.

**Solution:** Added `get_fast()` method that navigates using only BP operations (`next_sibling()`) for intermediate elements:

```rust
/// Get element by index (fast path for random access).
pub fn get_fast(&self, index: usize) -> Option<StandardJson<'a, W>> {
    let mut cursor = self.element_cursor?;

    // Navigate using O(1) BP operations only
    for _ in 0..index {
        cursor = cursor.next_sibling()?;
    }

    // Only call value() (which uses text_position/ib_select) at the target
    Some(cursor.value())
}
```

**Complexity comparison:**
- `get()`: O(n) IB select operations
- `get_fast()`: O(n) BP `next_sibling` operations + O(log N) IB select for target

**Benchmark results (10MB JSON, 60K element array):**

| Index | get()   | get_fast() | Speedup |
|-------|---------|------------|---------|
| 10    | 535 ns  | 115 ns     | 4.7x    |
| 50    | 2.51 µs | 351 ns     | 7.2x    |
| 100   | 5.04 µs | 637 ns     | 7.9x    |
| 1000  | 48 µs   | 5.7 µs     | 8.4x    |
| 10000 | 506 µs  | 58 µs      | 8.7x    |

**Impact on jq evaluator:**
- Updated `get_element_at_index()` to use `get_fast()` for all array index operations (`.[n]`, `.field[n]`)
- No regressions detected in iteration benchmarks (which still use `uncons()`)

---

## 6. Zero-Copy Output

### 6.1 Raw Bytes Access

For CLI output, avoid string allocation by accessing raw bytes directly:

```rust
// Zero-copy for strings and numbers
StandardJson::String(s) => out.write_all(s.raw_bytes())?,
StandardJson::Number(n) => out.write_all(n.raw_bytes())?,

// Buffered I/O reduces syscall overhead
let mut out = BufWriter::new(stdout.lock());
```

---

## 7. Testing Infrastructure

### 7.1 Cross-SIMD-Level Validation

**File:** [tests/simd_level_tests.rs](../../../tests/simd_level_tests.rs)

Runtime dispatch only tests the highest available SIMD level on modern hardware. Explicit tests ensure all levels work correctly:

```rust
// Force-test each SIMD level explicitly
#[test]
fn test_sse2_matches_scalar() {
    let result_sse2 = simd::x86::build_semi_index_standard(json);
    let result_scalar = standard::build_semi_index(json);
    assert_eq!(result_sse2.ib, result_scalar.ib);
}
```

---

## Summary: Performance Impact

| Optimization        | Technique              | Complexity          | Impact                      |
|---------------------|------------------------|---------------------|-----------------------------|
| Rank directory      | 3-level Poppy          | O(1)                | Enables fast navigation     |
| Cache alignment     | 64-byte aligned alloc  | -                   | 3-4% rank speedup           |
| Select index        | Sampled positions      | O(log n)            | Enables fast select         |
| BP RangeMin         | 3-level excess index   | O(1) find_close     | ~11x vs linear scan         |
| BP byte lookup      | BYTE_FIND_CLOSE tables | O(1) per byte       | ~4x vs bit-by-bit           |
| JSON SIMD           | Multi-level dispatch   | O(n)                | 2-4x vs scalar              |
| NEON nibble lookup  | vqtbl1q_u8 tables      | O(n)                | Replaces 12+ ops with 3     |
| NEON movemask       | Multiplication trick   | O(1)                | Avoids slow variable shifts |
| String fast-path    | Batch zero writing     | O(n)                | 2.5-8x on strings           |
| Cumulative index    | Binary search select   | O(log n)            | 627x on queries             |
| Exponential search  | Galloping from hint    | O(log d)            | 3.3x for sequential         |
| Dual select methods | Binary + exponential   | O(log n) / O(log d) | 39% faster random access    |

---

### 5.2 Exponential Search for Sequential Select

**File:** [src/json/light.rs](../../../src/json/light.rs)

When iterating through JSON elements (`.users[]`, `.items[].name`), select queries access sequential positions. Exponential search (galloping) exploits this locality:

```rust
/// Perform select1 with a hint for the starting word index.
///
/// Uses exponential search (galloping) from the hint, which is optimal for
/// sequential access patterns. When iterating through elements, the next
/// select is typically near the previous one, so starting from the hint
/// gives O(log d) where d is the distance, instead of O(log n).
fn ib_select1_from(&self, k: usize, hint: usize) -> Option<usize> {
    // Clamp hint to valid range
    let hint = hint.min(n.saturating_sub(1));

    // Check direction from hint
    if ib_rank[hint + 1] <= k {
        // Gallop forward: 1, 2, 4, 8, ... until overshoot
        let mut bound = 1;
        while hint + bound < n && ib_rank[hint + bound + 1] <= k {
            bound *= 2;
        }
        // Binary search within [hint + bound/2, hint + bound]
    } else {
        // Gallop backward similarly
    }
}

// Hint is estimated from rank / 8 (typical JSON density)
let hint = rank / 8;
self.index.ib_select1_from(rank, hint)
```

**Results:**

| Benchmark                      | Binary Search | Exponential Search | Improvement     |
|--------------------------------|---------------|--------------------|-----------------|
| Sequential select (microbench) | 335 µs        | 102 µs             | **3.3x faster** |
| count_all_leaves (8MB)         | 106 ms        | 92 ms              | **13% faster**  |
| extract_all_strings (8MB)      | 131 ms        | 117 ms             | **11% faster**  |

**Dual Select Methods (January 2026):**

To address the 37% random access regression, both methods are now exposed:

| Method                     | Use Case          | Complexity | Best For                          |
|----------------------------|-------------------|------------|-----------------------------------|
| `ib_select1(k)`            | Random access     | O(log n)   | `.[42]`, slicing, indexed lookups |
| `ib_select1_from(k, hint)` | Sequential access | O(log d)   | `.[]`, iteration, traversal       |

**Benchmark results (10K elements):**

| Benchmark                | Time    | Throughput   |
|--------------------------|---------|--------------|
| Sequential (exponential) | 108 µs  | 92.2 Melem/s |
| Sequential (binary)      | 340 µs  | 29.4 Melem/s |
| Random (exponential)     | 1080 µs | 9.3 Melem/s  |
| Random (binary)          | 779 µs  | 12.8 Melem/s |

**Key findings:**
- Sequential: Exponential search is **3.1x faster** (108 µs vs 340 µs)
- Random: Binary search is **39% faster** (779 µs vs 1080 µs)
- End-to-end JSON benchmarks show **no regression** from adding dual methods

Callers should choose the appropriate method based on access pattern:
- For iteration (`.[]`, tree traversal): use `ib_select1_from` with hint
- For indexed access (`.[42]`, slicing): use `ib_select1`

---

### 5.3 BP find_close Byte-Level Lookup Tables

**File:** [src/trees/bp.rs](../../../src/trees/bp.rs)

The `BalancedParens::find_close` operation uses byte-level lookup tables for fast excess tracking, achieving ~11x speedup over bit-by-bit scanning.

**Lookup tables (compile-time generated):**

```rust
/// Minimum excess within each byte value (0-255)
const BYTE_MIN_EXCESS: [i8; 256] = { /* precomputed */ };

/// Total excess (opens - closes) for each byte value
const BYTE_TOTAL_EXCESS: [i8; 256] = { /* precomputed */ };

/// Find position where excess drops to 0 within a byte
/// Index: [byte_value][initial_excess-1] → bit position (0-7) or 8 if not found
const BYTE_FIND_CLOSE: [[u8; 16]; 256] = { /* precomputed */ };
```

**Algorithm (`find_close_in_word_fast`):**

1. For each full byte in the word:
   - Check if `excess + BYTE_MIN_EXCESS[byte]` can reach 0
   - If yes, use `BYTE_FIND_CLOSE[byte][excess-1]` for O(1) match position
   - If no, skip entire byte: `excess += BYTE_TOTAL_EXCESS[byte]`
2. Handle partial bytes with bit-by-bit fallback

**Benchmark Results:**

| Implementation        | 10K elements | 100K elements | 1M elements |
|-----------------------|--------------|---------------|-------------|
| RangeMin (byte-level) | 11.4 µs      | 12.2 µs       | 11.3 µs     |
| Linear (bit-by-bit)   | 15.4 µs      | 49.5 µs       | 130.9 µs    |

**Speedup:** ~11.6x for 1M elements (130.9 µs → 11.3 µs)

The RangeMin implementation combines the hierarchical skip structure (L0/L1/L2 blocks) with byte-level scanning for O(1) `find_close` operations.

---

## Not Implemented (See opportunities.md)

The following optimizations are documented but not yet implemented:

- AVX2 escape sequence preprocessing

## Recently Implemented

The following were marked as "not implemented" but are now complete:

- ✅ NEON nibble lookup tables (`vqtbl1q_u8`) - implemented in [src/json/simd/neon.rs](../../../src/json/simd/neon.rs)
- ✅ Optimized movemask for NEON - uses multiplication trick instead of slow variable shifts
- ✅ Separate binary search select for random access patterns - `ib_select1()` method
