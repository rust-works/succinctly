# DSV Performance Profiling Results

## Executive Summary

**Bottleneck identified**: Row/field iteration via rank/select operations, NOT parsing.

## Profiling Data (AMD Ryzen 9 7950X, AVX2 SIMD)

### 10MB strings pattern

| Operation                     | Time/iter | Throughput   | Bottleneck |
|-------------------------------|-----------|--------------|------------|
| **Pure parsing (index build)**| 3.9ms     | **2545 MiB/s** | ✅ FAST    |
| **Parsing + iteration**       | 75.5ms    | 132 MiB/s    | ⚠️ SLOW    |
| **+ String conversion**       | 111.8ms   | 90 MiB/s     | ⚠️ SLOWER  |

### 1MB strings pattern

| Operation                     | Time/iter | Throughput   |
|-------------------------------|-----------|--------------|
| **Pure parsing (index build)**| 0.18ms    | **5451 MiB/s** |
| **Parsing + iteration**       | 7.4ms     | 136 MiB/s    |
| **+ String conversion**       | 9.6ms     | 104 MiB/s    |

## Analysis

### What's Fast ✅
- **SIMD AVX2 parsing**: 2.5-5.5 GB/s throughput
- Index building with succinct bit vectors
- Quote masking via prefix XOR

### What's Slow ⚠️
- **Rank/select operations during iteration**: 19x slowdown (2545 → 132 MiB/s)
  - Each field access requires rank1() and select1() calls
  - For 226,556 fields in 10MB file, that's ~450K rank/select operations
  - Average cost per field: ~0.33μs (rank + select)
- **String conversion**: Additional 30% overhead
  - UTF-8 validation + String allocation
  - from_utf8_lossy() per field

### End-to-End Pipeline
The full jq pipeline (`./target/release/succinctly jq --input-dsv ',' '.' file.csv`) measures:
1. Parsing: 3.9ms (2545 MiB/s) ← **3%** of time
2. Iteration: 71.6ms (132 MiB/s slowdown) ← **64%** of time
3. String conversion: 36.3ms ← **32%** of time
4. JSON serialization + jq eval + output: remainder

**Result**: ~35-40 MiB/s end-to-end throughput

## Root Cause

The bottleneck is **not** the CSV parsing itself - our SIMD implementation is excellent.

The issue is the **cursor navigation model**:
- Each `row.fields()` iteration calls `rank1()` and `select1()` multiple times
- With O(1) rank but O(log n) select, this adds significant overhead
- Processing 226K fields × 2 operations = 452K rank/select calls

## Detailed Profiling of Rank/Select Overhead

### Microbenchmark Results

**Rank/Select operation costs** (measured on 10M bit BitVec):
- `rank1()`: **2.5ns** per operation (O(1) with Poppy directory)
- `select1()`: **26ns** per operation (O(log n) with sampled index)
- **Simulated field iteration**: 71ns per field (6 rank1 + 2 select1 calls)

**Actual DSV iteration costs** (10MB file with 226,556 fields):
- **Theoretical**: 71ns × 226,556 = 16ms expected overhead
- **Measured**: 71.6ms actual iteration time
- **Gap**: **316ns per field** vs 71ns expected = **4.5x slower than theory**

### Why is iteration 4.5x slower than expected?

Profiling revealed the cursor does **redundant operations** per field:

1. `current_field()` → **rank1** + **select1** (to find field end)
2. Check for newline → **rank1 × 2** (to detect row boundary)
3. `next_field()` → **rank1** + **select1** (to advance cursor)
4. `at_newline()` → **rank1 × 2** (to verify still in same row)

**Total per field**: ~6 rank1 calls + 2 select1 calls = 6×2.5ns + 2×26ns = **67ns** (theory)

But we're seeing **316ns** in practice, indicating:
- Cache misses on BitVec structures
- BitVec overhead (RankDirectory + SelectIndex lookups)
- Redundant position calculations

## Optimization Opportunities

### Critical Path Optimizations (Expected 4-5x speedup)

#### 1. **Eliminate redundant rank/select calls** (Highest impact)
Current code pattern:
```rust
// Called 6x per field!
let rank = self.index.markers.rank1(position);
```

Fix: Cache the rank result in the iterator state:
```rust
struct DsvFields {
    cached_rank: usize,
    // ... reuse across fields in same row
}
```

**Expected impact**: Reduce from 6 rank1 calls to 1-2 per field = **3x speedup**

#### 2. **Use lightweight index structure like JSON code** (High impact)
Current: Full `BitVec` with RankDirectory + SelectIndex
- Memory: ~6% overhead per bitvector
- Cache pressure: 3-level directory lookups

Proposed: Lightweight like `JsonIndex`:
```rust
struct DsvIndex {
    markers: Vec<u64>,           // raw bits
    markers_rank: Vec<u32>,      // cumulative popcount per word
    newlines: Vec<u64>,          // raw bits
    newlines_rank: Vec<u32>,     // cumulative popcount per word
}
```

**Expected impact**:
- Faster rank (single array lookup vs 3-level directory)
- Better cache behavior
- Faster index construction
- **Estimated 1.5-2x speedup**

#### 3. **Sequential scan for dense data** (Medium-high impact)
For rows with many fields close together, linear scan beats rank/select:

```rust
// If next delimiter is within 64 bytes, scan instead of rank/select
if estimated_distance < 64 {
    // SIMD scan for delimiter byte
} else {
    // Use rank/select
}
```

**Expected impact**: 2-3x speedup for narrow columns (< 64 bytes)

#### 4. **PDEP-optimized select** (Low-medium impact on Zen 4)
Current `select_in_word()` uses CTZ loop. Could use PDEP:
```rust
#[cfg(target_feature = "bmi2")]
unsafe fn select_in_word_pdep(word: u64, k: u32) -> u32 {
    _pdep_u64(1 << k, word).trailing_zeros()
}
```

**Expected impact**: 10-20% faster select on Intel/Zen4, slower on Zen1-3

### Medium Impact Optimizations

#### 5. **Batch field extraction** (Medium impact)
Extract all fields in a row with one pass instead of repeated rank/select:
```rust
fn extract_row_fields(&self, row_start: usize, row_end: usize) -> Vec<&[u8]> {
    // Single linear scan from row_start to row_end
    // Collect all delimiter positions
    // Slice text into fields
}
```

**Expected impact**: 2x speedup for small rows (< 20 fields)

#### 6. **Optimize String conversion** (Medium impact)
Current: `String::from_utf8_lossy()` validates UTF-8 per field
```rust
// Unsafe but faster if we trust CSV data
unsafe { String::from_utf8_unchecked(field.to_vec()) }
```

**Expected impact**: 30% faster string conversion (from 36ms → 25ms)

#### 7. **String allocation pooling** (Low-medium impact)
Reuse String buffers across field conversions:
```rust
struct StringPool {
    buffers: Vec<String>,
}
```

**Expected impact**: 10-20% reduction in allocation overhead

### Low Impact (Already Fast)

#### 8. **Further optimize parsing** (Diminishing returns)
Already at 2.5-5.5 GB/s with AVX2 SIMD. Possible improvements:
- AVX-512: Maybe 10-20% faster
- BMI2 PDEP quote masking: Tested, minimal improvement

**Expected impact**: < 5% end-to-end

## Comparison to hw-dsv

### Architecture Comparison

hw-dsv (Haskell) implementation:
- **Parsing**: Processes 512 bytes at a time (64 Word64s) vs our 64 bytes
- **Quote masking**: Uses BMI2 PDEP with broadword arithmetic for carry propagation
- **Index structure**: Uses `CsPoppy` (equivalent to our `BitVec`)
  - `CsPoppy` contains: raw bits + CsPoppyIndex0 + CsPoppyIndex1
  - Same as our: raw bits + RankDirectory + SelectIndex
- **Iteration**: Same algorithm as ours
  - `nextField` calls `rank1` + `select1` per field
  - Same redundant operations per field

### Key Finding

**hw-dsv uses the exact same rank/select infrastructure and iteration pattern as we do!**

Both implementations:
1. ✅ Build full rank/select indices during parsing
2. ✅ Use Poppy-style rank directories
3. ✅ Call rank1 + select1 for each field navigation
4. ✅ Have the same iteration bottleneck

The only differences are:
- Language (Haskell vs Rust)
- Compiler (GHC vs LLVM/rustc)
- Chunk size during parsing (512 vs 64 bytes)
- Quote masking technique (PDEP vs prefix XOR)

**Conclusion**: We're implementing the canonical hw-dsv algorithm correctly. The performance bottleneck (iteration overhead) is inherent to this approach, not a bug in our implementation.

## Recommendations

### Immediate High-Impact Optimizations (Target: 3-5x speedup)

**Priority 1**: **Eliminate redundant rank/select calls in cursor iteration**
- Current: 6 rank1 + 2 select1 per field
- Target: 1-2 rank1 + 1 select1 per field
- Implementation: Cache rank results in iterator state
- **Expected gain**: 2-3x faster iteration (132 MiB/s → 300-400 MiB/s)

**Priority 2**: **Switch to lightweight index structure like JSON**
- Current: Full `BitVec` with RankDirectory + SelectIndex (~6% overhead)
- Target: Simple `Vec<u32>` cumulative rank per word (~0.8% overhead)
- Benefits: Faster rank lookups, better cache behavior, faster index construction
- **Expected gain**: 1.5-2x faster overall (300-400 MiB/s → 500-700 MiB/s)

**Combined expected end-to-end**: 35 MiB/s → **150-280 MiB/s** (4-8x improvement)

### Future Medium-Impact Optimizations

**Priority 3**: **Hybrid scan/rank-select for dense data**
- Use SIMD linear scan when delimiter is likely within 64 bytes
- Fall back to rank/select for sparse/wide columns
- **Expected gain**: Additional 1.5-2x for narrow-column data

**Priority 4**: **Optimize string conversion**
- Use `from_utf8_unchecked` for validated CSV data
- Pool String allocations
- **Expected gain**: 20-30% reduction in string conversion overhead

### Long-term Explorations

**Priority 5**: **Alternative APIs**
- Batch row extraction (extract whole row in one pass)
- Streaming iterators that avoid repeated rank/select
- Column-oriented access patterns
