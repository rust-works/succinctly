# Hierarchical Data Structures

Hierarchical structures enable O(1) or O(log n) queries by organizing auxiliary data at multiple granularity levels. This technique trades space for query time.

## Overview

| Structure           | Query Time | Space Overhead | Use Case              |
|---------------------|------------|----------------|-----------------------|
| Flat array          | O(n)       | 0%             | Small datasets        |
| Sampled index       | O(n/k)     | 1/k            | Infrequent queries    |
| 2-level hierarchy   | O(1)       | ~3%            | Frequent queries      |
| 3-level hierarchy   | O(1)       | ~3%            | Very large datasets   |
| RangeMin structure  | O(1)       | ~6%            | Tree nav (find_close) |
| RangeMax structure  | O(1)       | ~1.5%          | Tree nav (find_open)  |

---

## The Rank Problem

**Problem**: Given a bitvector B and position i, count 1-bits in B[0..i].

**Naive**: Scan and count - O(n) time.

### Two-Level Rank Directory

Store precomputed ranks at block boundaries:

```
Block size: 512 bits (8 words)

L1: Cumulative rank at each block boundary
    [0, 312, 627, 941, ...]

L2: Rank within block (relative to L1)
    Block 0: [0, 42, 89, 134, 178, 221, 267, 0]
    Block 1: [0, 38, 81, 125, 171, 214, 259, 0]
    ...
```

**Query algorithm**:
```rust
fn rank1(pos: u64) -> u64 {
    let block = pos / 512;
    let word_in_block = (pos % 512) / 64;
    let bit_in_word = pos % 64;

    let l1_rank = self.l1[block];
    let l2_rank = self.l2[block][word_in_block];
    let word = self.bits[pos / 64];
    let word_rank = (word & ((1 << bit_in_word) - 1)).count_ones();

    l1_rank + l2_rank + word_rank
}
```

**Complexity**: O(1) - three lookups plus one popcount.

### Three-Level Rank Directory (Poppy)

For bitvectors > 4GB, add an L0 level:

```
L0: Cumulative rank every 2^32 bits (4GB)
    Only needed for vectors > 4GB

L1: Cumulative rank every 512 bits (relative to L0)
    32 bits per entry

L2: Rank within 512-bit block
    7 × 9-bit offsets packed into upper bits of L1 entry
```

**Packing L1 + L2**:

```rust
// src/bits/rank.rs
// Pack L1 (32 bits) + L2[0..7] (7 × 9 bits) into 128 bits
fn pack_entry(l1: u32, l2: &[u16; 7]) -> u128 {
    let mut entry = l1 as u128;
    for (i, &offset) in l2.iter().enumerate() {
        entry |= (offset as u128) << (32 + i * 9);
    }
    entry
}

// Extract L2[i] from packed entry
fn extract_l2(entry: u128, i: usize) -> u16 {
    ((entry >> (32 + i * 9)) & 0x1FF) as u16
}
```

**Memory layout** (per 512 bits of data):
```
|   L1 (32 bits)   |L2[0]|L2[1]|L2[2]|L2[3]|L2[4]|L2[5]|L2[6]| pad |
|     32 bits      | 9b  | 9b  | 9b  | 9b  | 9b  | 9b  | 9b  | 33b |
Total: 128 bits per 512 bits = 25% overhead

But we only store L2 for 7 of 8 words (8th is implied), so:
Effective overhead: ~3%
```

### Prior Art

- **Jacobson (1989)**: Original rank structure
- **Clark (1996)**: Two-level directory
- **Zhou, Andersen, Kaminsky (2013)**: "Space-Efficient, High-Performance Rank & Select" (Poppy)

---

## The Select Problem

**Problem**: Given bitvector B and k, find position of k-th 1-bit.

**Naive**: Scan until k 1-bits found - O(n) time.

### Sampled Select Index

Store position of every k-th 1-bit:

```rust
struct SelectIndex {
    samples: Vec<(usize, u64)>,  // (word_index, cumulative_count)
    sample_rate: usize,          // Default: 256
}

// samples[i] = position info for (i * sample_rate)-th 1-bit
```

**Query algorithm**:
1. Binary search samples to find bracket containing k-th bit
2. Linear scan within bracket

```rust
fn select1(&self, k: u64) -> Option<u64> {
    // Binary search for sample bracket
    let sample_idx = self.samples.partition_point(|s| s.1 < k);

    // Start from sample position
    let (start_word, start_count) = self.samples[sample_idx - 1];

    // Linear scan to find exact position
    let mut count = start_count;
    for word_idx in start_word.. {
        let word = self.bits[word_idx];
        let word_pop = word.count_ones() as u64;
        if count + word_pop >= k {
            return Some(word_idx * 64 + select_in_word(word, (k - count) as u32));
        }
        count += word_pop;
    }
    None
}
```

**Complexity**: O(log n + sample_rate) per query.

### Cumulative Index for O(1) Select

For workloads with many select queries, precompute cumulative counts:

```rust
// src/json/light.rs
struct CumulativeIndex {
    cumulative: Vec<u32>,  // cumulative[i] = rank1(i * 64)
}

fn select1(&self, k: u64) -> u64 {
    // Binary search on cumulative counts - O(log n)
    let word_idx = self.cumulative.partition_point(|&c| (c as u64) < k);

    // Within word - O(1)
    let prev_count = if word_idx > 0 { self.cumulative[word_idx - 1] } else { 0 };
    let remaining = k - prev_count as u64;
    word_idx * 64 + select_in_word(self.bits[word_idx], remaining as u32)
}
```

**Result**: 627x speedup (2.76s → 4.4ms) on query workloads.

**Memory**: 4 bytes per word = 50% of bitvector size.

### Prior Art

- **Vigna (2008)**: "Broadword Implementation of Rank/Select Queries"
- **Gog et al. (2014)**: sdsl-lite library

---

## Elias-Fano Encoding

**Problem**: Store monotonically increasing integers (like `bp_to_text` positions) compactly.

**Naive**: Store as `Vec<u32>` - 4 bytes per element.

### Elias-Fano Structure

Split each integer into high and low bits:

```
For sequence with n elements, max value m:
- low_width = floor(log2(m/n))
- Low bits: dense array, n × low_width bits
- High bits: unary encoded bitvector with n set bits
```

**Example** (n=4, values=[3, 7, 12, 15], low_width=2):
```
Values:   3=0b0011  7=0b0111  12=0b1100  15=0b1111
Low bits:     11        11        00        11  → packed array
High bits: position encodes value >> low_width
           0 1 2 3    (high values: 0, 1, 3, 3)
           ↓ ↓   ↓↓
           1 1 0 1 1  (unary: one 1-bit per element at high_value + rank)
```

### Cursor-Based Access

For forward-only patterns (like tree navigation), maintain cursor state:

```rust
pub struct EliasFanoCursor<'a> {
    ef: &'a EliasFano,
    idx: usize,           // Current element index
    high_pos: usize,      // Position in high_bits bitvector
    word_idx: usize,      // Current word in high_bits
    remaining_bits: u64,  // Masked bits for scanning
}

fn advance_one(&mut self) -> Option<u32> {
    // Clear current bit, find next 1-bit - O(1) amortized
    self.remaining_bits &= self.remaining_bits - 1;
    if self.remaining_bits != 0 {
        self.high_pos = self.word_idx * 64 + self.remaining_bits.trailing_zeros();
    } else {
        // Scan to next non-zero word
        self.word_idx += 1;
        while self.ef.high_bits[self.word_idx] == 0 { self.word_idx += 1; }
        self.remaining_bits = self.ef.high_bits[self.word_idx];
        self.high_pos = self.word_idx * 64 + self.remaining_bits.trailing_zeros();
    }
    self.current()
}
```

### Benchmark Results (Apple M1 Max)

For bp_to_text-like data (positions with 10-100 byte gaps):

| Metric               | EliasFano          | Vec\<u32\>   | Comparison        |
|----------------------|--------------------|--------------|-------------------|
| **Compression**      | 88 KB (100K elems) | 390 KB       | **4.42×** smaller |
| Sequential iter      | 1.76 ns/elem       | 0.05 ns/elem | 36× slower        |
| cursor_from + 5 iter | 26.2 ns            | 1.70 ns      | 15× slower        |
| Random access        | 14.6 ns            | 0.4 ns       | 36× slower        |

**Key insight**: Cursor iteration amortizes setup cost. The 36× penalty for random
access drops to 15× when iterating 5 elements after positioning.

### SIMD Optimization Opportunity

The `select_in_word` operation (find k-th set bit) is the bottleneck:

| Platform | Technique            | Expected Speedup                |
|----------|----------------------|---------------------------------|
| x86 BMI2 | `PDEP` + `TZCNT`     | 3-4× (O(1) vs O(popcount) loop) |
| ARM NEON | Broadword with `CNT` | 1.5-2×                          |
| ARM SVE2 | `BDEP` + `CTZ`       | 3-4× (if available)             |

### Prior Art

- **Elias (1974)**: Original Elias gamma/delta codes
- **Fano (1971)**: Information-theoretic coding
- **Vigna (2013)**: "Quasi-Succinct Indices" (modern Elias-Fano with select)
- **Ottaviano & Venturini (2014)**: "Partitioned Elias-Fano Indexes" (SIGIR)

---

## RangeMin Structure (Balanced Parentheses)

**Problem**: Navigate tree encoded as balanced parentheses.

Tree operations require finding positions where cumulative excess (opens minus closes) reaches certain values.

### Hierarchical Min-Excess Index

Store minimum excess within each block:

```
Level 2: Min excess for 4096-bit superblocks
Level 1: Min excess for 512-bit blocks
Level 0: Use byte lookup tables for 8-bit chunks
```

```rust
struct RangeMinIndex {
    // L1: Min excess for each 512-bit block
    l1_min: Vec<i16>,

    // L2: Min excess for each 4096-bit superblock
    l2_min: Vec<i16>,
}
```

**Query: find_close(pos)**

Find matching close paren for open at position `pos`:

```rust
fn find_close(&self, pos: u64) -> u64 {
    let mut excess = 1;  // Start inside the open paren

    // Phase 1: Scan bytes using lookup table
    while excess > 0 {
        let byte = self.get_byte(current_pos);
        if let Some(offset) = BYTE_FIND_CLOSE[byte][excess] {
            return current_pos + offset;
        }
        excess += BYTE_TOTAL_EXCESS[byte];
        current_pos += 8;
    }

    // Phase 2: Skip blocks using L1 index
    while excess + self.l1_min[block] > 0 {
        excess += self.l1_total[block];
        block += 1;
    }

    // Phase 3: Scan within block
    // ...
}
```

**Result**: 40x speedup on tree navigation.

### SIMD-Accelerated Index Construction (January 2026)

On ARM, the L1 index building can be accelerated using NEON SIMD:

```rust
// Process 8 words at a time
unsafe fn build_l1_neon(l0_min: &[i8], l0_excess: &[i16]) -> (Vec<i16>, Vec<i16>) {
    // 1. Load 8 min_excess values (i8 → i16)
    let min_lo = vmovl_s8(vld1_s8(min_ptr));

    // 2. Load 8 word_excess values
    let excess = vld1q_s16(excess_ptr);

    // 3. Compute prefix sums using parallel prefix (3 shuffle+add steps)
    let prefix_sum = simd_prefix_sum(excess);

    // 4. Add to min values
    let adjusted = vaddq_s16(min_lo, prefix_sum);

    // 5. Find minimum with VMINV (single instruction!)
    let block_min = vminvq_s16(adjusted);
}
```

**Result**: 2.8x faster L1 index construction on ARM (Graviton 4).

The same SIMD pattern is also applied to L2 index building, providing 1-3%
improvement at large scales (100M+ nodes) with no regression at smaller sizes.

### Indexed find_open (January 2026)

The `find_close` operation uses hierarchical **min-excess** indices to skip blocks
where excess won't drop to zero. The symmetric `find_open` operation (find matching
open for a close) requires scanning **backwards**, needing **max-excess** indices.

**Problem**: `find_open` was O(d) bit-by-bit scanning, while `find_close` was O(1).

**Solution**: Add reverse max-excess indices at all three levels:

```rust
struct BalancedParens {
    // Forward scanning (find_close) - existing
    l0_min_excess: Vec<i8>,      // Per-word min excess
    l1_min_excess: Vec<i16>,     // Per-32-word block min excess
    l2_min_excess: Vec<i16>,     // Per-1024-word block min excess

    // Reverse scanning (find_open) - new
    l0_max_excess_rev: Vec<i8>,  // Per-word max excess (backwards)
    l1_max_excess_rev: Vec<i16>, // Per-32-word block max excess (backwards)
    l2_max_excess_rev: Vec<i16>, // Per-1024-word block max excess (backwards)
}
```

**State machine**: Mirrors `find_close_from` but scans backwards:

```rust
fn find_open_from(&self, start_pos: usize, target_excess: i32) -> Option<usize> {
    // States: ScanWord, CheckL0, CheckL1, CheckL2, FromL0, FromL1, FromL2

    // Skip condition (reversed):
    // - find_close: excess + block_min_excess <= 0  (might drop to zero)
    // - find_open:  excess + block_max_excess_rev >= target  (might rise to target)

    // If block can't reach target, skip entire block:
    excess += block_total_excess;
    pos = previous_block_boundary;
}
```

**Byte lookup table** for O(1) in-word scanning:

```rust
// BYTE_FIND_OPEN_REV[byte_val][target-1] = bit position where excess
// first reaches target when scanning bits 7→0, or 8 if not found
const BYTE_FIND_OPEN_REV: [[u8; 16]; 256] = { /* precomputed */ };
```

**Benchmark results** (1000 queries, ARM Graviton 4):

| Size | Indexed | Linear (O(d)) | Speedup |
|------|---------|---------------|---------|
| 10K  | 11.3 µs | 183.8 µs      | **16x** |
| 100K | 10.2 µs | 886.3 µs      | **87x** |
| 1M   | 9.3 µs  | 4,573 µs      | **492x** |

**Note on `enclose`**: The `enclose` operation (find parent) was kept as linear
scan because the parent is typically very close (1-2 bits away). The indexed
version added overhead without benefit for nearby targets.

**Space overhead**: ~1.5 bytes per word additional (3 new i8/i16 vectors).

### Prior Art

- **Munro & Raman (2001)**: "Succinct Representation of Balanced Parentheses"
- **Navarro & Sadakane (2014)**: "Fully Functional Static and Dynamic Succinct Trees"

---

## Lightweight vs Full Index

Sometimes simpler structures outperform theoretically optimal ones.

### Full BitVec Index

```
3-level RankDirectory + SelectIndex + storage
- O(1) rank, O(log n + k) select
- ~3% overhead for rank
- ~3% overhead for select
- Complex access patterns
```

### Lightweight Cumulative Index

```
Simple cumulative count array
- O(1) rank (same)
- O(log n) select
- ~4% overhead total
- Sequential access pattern
```

**DSV parsing result**: Lightweight index was **5-9x faster** for iteration:
- Full BitVec: 145-150 MiB/s
- Lightweight: 792-1331 MiB/s

**Reason**: Simpler data structure has better cache behavior.

---

## Design Principles

### Block Size Selection

Choose block size based on:
1. **Cache line size**: 64 bytes = 512 bits is natural
2. **Word size**: Should be multiple of 64 bits
3. **Offset range**: L2 offsets must fit in chosen bit width

```
512-bit blocks:
- Max L2 offset: 512 → needs 9 bits
- 7 offsets needed → 63 bits
- Fits nicely with 32-bit L1 in 128 bits
```

### Level Count

| Data Size | Levels Needed | Reason                           |
|-----------|---------------|----------------------------------|
| < 4 KB    | 0 (scan)      | Fits in L1 cache                 |
| < 4 GB    | 2 (L1+L2)     | L1 offset fits in 32 bits        |
| > 4 GB    | 3 (L0+L1+L2)  | Need L0 for absolute position    |

### Space-Time Tradeoff

```
More levels / smaller blocks:
  + Faster queries (less scanning)
  - More space overhead
  - More cache misses for auxiliary data

Fewer levels / larger blocks:
  + Less space overhead
  + Better cache locality
  - Slower queries (more scanning)
```

---

## Usage in Succinctly

| Structure           | Location              | Purpose                    | Overhead |
|---------------------|-----------------------|----------------------------|----------|
| 3-level RankDir     | `bits/rank.rs`        | O(1) rank queries          | ~3%      |
| SelectIndex         | `bits/select.rs`      | Accelerated select         | ~3%      |
| EliasFano           | `bits/elias_fano.rs`  | Compressed monotone ints   | ~23%*    |
| CumulativeIndex     | `json/light.rs`       | Fast IB select             | ~4%      |
| RangeMinIndex       | `trees/bp.rs`         | O(1) find_close            | ~6%      |
| RangeMaxRevIndex    | `trees/bp.rs`         | O(1) find_open (16-492x)   | ~1.5%    |
| LightweightIndex    | `dsv/index_*.rs`      | DSV iteration              | ~0.8%    |

*EliasFano overhead is relative to the data being encoded, not to a bitvector. It provides 4.4× compression vs Vec<u32> for bp_to_text-like data.

---

## Key Lessons

1. **O(1) is achievable**: Hierarchical structures give constant-time queries
2. **Space overhead is small**: 3-6% for practical implementations
3. **Simpler can be faster**: Cache effects dominate for moderate sizes
4. **Match structure to access pattern**: Random vs sequential needs different designs
5. **Pack auxiliary data**: Bit-packing reduces memory traffic
6. **Symmetric operations need symmetric indices**: Forward scan uses min, backward uses max

---

## References

- Jacobson, G. "Space-efficient Static Trees and Graphs" (1989)
- Clark, D. "Compact Pat Trees" (1996)
- Munro, J.I. & Raman, V. "Succinct Representation of Balanced Parentheses" (2001)
- Zhou, D., Andersen, D.G., Kaminsky, M. "Space-Efficient, High-Performance Rank & Select" (2013)
- Navarro, G. & Sadakane, K. "Fully Functional Static and Dynamic Succinct Trees" (2014)
- Gog, S. et al. "From Theory to Practice: Plug and Play with Succinct Data Structures" (2014)
