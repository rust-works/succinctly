# DSV (CSV/TSV) Parsing

This document describes how the succinctly library parses and indexes delimiter-separated value (DSV) files, including CSV and TSV formats.

## Overview

The DSV parser creates a **semi-index** that marks field and row boundaries without materializing the data. This enables streaming access with minimal memory overhead.

| Component | Purpose | Size |
|-----------|---------|------|
| Markers | Delimiter and newline positions | ~1 bit/byte |
| Newlines | Row boundary positions | ~1 bit/64 bytes |
| Rank indices | Enable O(1) field lookup | ~3-4% of input |

**Total overhead**: ~3-4% of input size vs 6-40x for materialized parsers.

---

## Index Structure

### Two Bit Vectors

```rust
struct DsvIndexLightweight {
    markers: Vec<u64>,       // Bits at delimiter/newline positions
    markers_rank: Vec<u32>,  // Cumulative popcount per word
    newlines: Vec<u64>,      // Bits at newline positions only
    newlines_rank: Vec<u32>, // Cumulative popcount per word
    text_len: usize,
}
```

### Marker Bits

Set to 1 at delimiter (`,`) and newline (`\n`) positions **outside quotes**:

```csv
"hello, world",name,42
             ^    ^  ^
             13   18 21  (markers set here)
```

The comma inside quotes (position 6) is **not** marked.

### Newline Bits

Subset of markers - only newline positions:

```csv
row1,data\n
row2,data\n
         ^
         10, 21  (newlines set here)
```

---

## Quote Handling

The key challenge is tracking whether a position is inside or outside quotes, especially across chunk boundaries.

### Quote State Logic

```
Position:  0123456789...
Text:      "hello, world",name
Quotes:    1            1
In-quote:  .111111111111.....

Inside quotes: mask out delimiters
Outside quotes: mark delimiters
```

### Three Implementation Approaches

| Method | Platform | Speed | Technique |
|--------|----------|-------|-----------|
| toggle64_bmi2 | x86 BMI2 | 10x baseline | PDEP + carry propagation |
| prefix_xor | AVX2/SSE/NEON | baseline | Parallel prefix XOR |
| Scalar | All | ~35x slower | Byte-by-byte loop |

---

## Prefix XOR Algorithm

Computes cumulative XOR to track quote parity:

```rust
fn prefix_xor(mut y: u64) -> u64 {
    y ^= y << 1;   // XOR adjacent pairs
    y ^= y << 2;   // XOR adjacent quads
    y ^= y << 4;   // XOR adjacent bytes
    y ^= y << 8;   // XOR adjacent u16s
    y ^= y << 16;  // XOR adjacent u32s
    y ^= y << 32;  // XOR halves
    y
}
```

**How it works**:

```
quote_mask:  0b00100100  (quotes at positions 2 and 5)
             After << 1:  0b01001000
             After XOR:   0b01101100
             ... (continue doubling)
Result:      0b00111100  (positions 2,3,4,5 are "inside")
```

After `prefix_xor`, bit `i` is 1 if there's an odd number of quotes before position `i`.

**Application**:
```rust
let in_quote_mask = prefix_xor(quote_bits) ^ carry_from_previous_chunk;
let valid_delims = delim_bits & !in_quote_mask;
```

See [parallel-prefix.md](../optimisations/parallel-prefix.md) for technique details.

---

## BMI2 PDEP Carry Propagation

A 10x faster alternative using hardware PDEP instruction:

```rust
unsafe fn toggle64_bmi2(quote_mask: u64, carry: u64) -> (u64, u64) {
    const ODDS_MASK: u64 = 0x5555_5555_5555_5555;  // 0101...

    // Scatter alternating pattern to quote positions
    let c = carry & 0x1;
    let addend = _pdep_u64(ODDS_MASK << c, quote_mask);

    // Use addition for carry propagation
    let comp_mask = !quote_mask;
    let shifted = (addend << 1) | c;
    let (result, overflow) = shifted.overflowing_add(comp_mask);

    (result ^ comp_mask, overflow as u64)
}
```

### How It Works

1. **PDEP scatters bits**: Places alternating 0/1 pattern at quote positions
2. **Addition propagates carries**: Fills regions between quotes
3. **Result**: 1-bits where outside quotes, 0-bits where inside

```
quote_mask:     0b00100100  (quotes at 2, 5)
PDEP result:    0b00100000  (alternating pattern at quote positions)
After shift:    0b01000000
After add:      Result fills between quotes automatically
```

**Why it's 10x faster**:
- PDEP is single-cycle on modern CPUs
- Addition handles carry propagation without loops
- No sequential dependency chain like prefix_xor

See [bit-manipulation.md](../optimisations/bit-manipulation.md) for PDEP/PEXT details.

---

## SIMD Character Detection

### AVX2 (64 bytes/iteration)

```rust
unsafe fn process_chunk_64(chunk: &[u8; 64]) -> (u64, u64, u64) {
    // Load two 32-byte vectors
    let lo = _mm256_loadu_si256(chunk[0..32].as_ptr() as *const __m256i);
    let hi = _mm256_loadu_si256(chunk[32..64].as_ptr() as *const __m256i);

    // Find delimiters, quotes, newlines in parallel
    let delim_lo = _mm256_cmpeq_epi8(lo, _mm256_set1_epi8(b',' as i8));
    let quote_lo = _mm256_cmpeq_epi8(lo, _mm256_set1_epi8(b'"' as i8));
    let newline_lo = _mm256_cmpeq_epi8(lo, _mm256_set1_epi8(b'\n' as i8));
    // ... same for hi

    // Extract to 64-bit masks
    let delim_mask = (_mm256_movemask_epi8(delim_lo) as u64)
                   | ((_mm256_movemask_epi8(delim_hi) as u64) << 32);
    // ...

    (delim_mask, quote_mask, newline_mask)
}
```

### NEON (64 bytes/iteration)

```rust
unsafe fn process_chunk_64_neon(chunk: &[u8; 64]) -> (u64, u64, u64) {
    // Load four 16-byte vectors
    let v0 = vld1q_u8(chunk[0..16].as_ptr());
    let v1 = vld1q_u8(chunk[16..32].as_ptr());
    let v2 = vld1q_u8(chunk[32..48].as_ptr());
    let v3 = vld1q_u8(chunk[48..64].as_ptr());

    // Compare and extract masks using neon_movemask
    let delim = vdupq_n_u8(b',');
    let m0 = neon_movemask(vceqq_u8(v0, delim));
    // ...

    // Combine into 64-bit mask
    (delim_mask, quote_mask, newline_mask)
}
```

See [simd.md](../optimisations/simd.md) for SIMD technique details.

---

## Lightweight vs Full Index

The DSV parser uses a **lightweight index** instead of full BitVec:

### Full BitVec (Not Used)
- 3-level RankDirectory
- SelectIndex with sampling
- ~6% memory overhead
- Complex cache access patterns

### Lightweight Index (Used)
- Simple cumulative rank array
- Binary search for select
- ~3-4% memory overhead
- **5-9x faster iteration**

```rust
// Lightweight rank: O(1)
fn markers_rank1(&self, pos: usize) -> usize {
    let word_idx = pos / 64;
    let bit_idx = pos % 64;

    let cumulative = self.markers_rank[word_idx] as usize;
    let word = self.markers[word_idx];
    let mask = (1u64 << bit_idx) - 1;

    cumulative + (word & mask).count_ones() as usize
}

// Lightweight select: O(log n)
fn markers_select1(&self, k: usize) -> Option<usize> {
    // Binary search on cumulative ranks
    let word_idx = self.markers_rank
        .partition_point(|&r| (r as usize) <= k);

    // Find bit within word
    let remaining = k - self.markers_rank[word_idx - 1] as usize;
    let bit_pos = select_in_word(self.markers[word_idx - 1], remaining);

    Some((word_idx - 1) * 64 + bit_pos)
}
```

See [hierarchical-structures.md](../optimisations/hierarchical-structures.md) and [cache-memory.md](../optimisations/cache-memory.md) for why simpler structures win.

---

## Navigation API

### DsvCursor

Low-level position tracking:

```rust
let mut cursor = index.cursor();

// Move to row 5
cursor.goto_row(5);

// Iterate fields in current row
while cursor.in_current_row() {
    let field = cursor.current_field();
    process(field);
    cursor.next_field();
}
```

### DsvRow

Row-level abstraction:

```rust
let row = index.row(5)?;

// Access by column index
let name = row.get(0)?;   // First column
let age = row.get(2)?;    // Third column

// Iterate all fields
for field in row.fields() {
    process(field);
}
```

### DsvRows

Iterator over all rows:

```rust
for row in index.rows() {
    let name = row.get(0)?;
    let value = row.get(1)?;
    process(name, value);
}
```

---

## Configuration

```rust
pub struct DsvConfig {
    pub delimiter: u8,      // default: b','
    pub quote_char: u8,     // default: b'"'
    pub newline: u8,        // default: b'\n'
}

// Presets
let csv = DsvConfig::csv();   // comma delimiter
let tsv = DsvConfig::tsv();   // tab delimiter
let psv = DsvConfig::psv();   // pipe delimiter
```

---

## File Locations

| File | Purpose |
|------|---------|
| `src/dsv/mod.rs` | Module exports, Dsv/DsvRef types |
| `src/dsv/config.rs` | DsvConfig |
| `src/dsv/parser.rs` | Scalar parser (fallback) |
| `src/dsv/index_lightweight.rs` | Lightweight index implementation |
| `src/dsv/cursor.rs` | Navigation APIs |
| `src/dsv/simd/mod.rs` | Runtime dispatch |
| `src/dsv/simd/bmi2.rs` | toggle64_bmi2 (PDEP) |
| `src/dsv/simd/avx2.rs` | prefix_xor (AVX2) |
| `src/dsv/simd/neon.rs` | NEON implementation |

---

## Performance

### Parsing Throughput

| Platform | Method | Throughput |
|----------|--------|------------|
| x86_64 | BMI2 + AVX2 | 1.3-3.7 GB/s |
| x86_64 | AVX2 only | ~1.0 GB/s |
| ARM64 | NEON | ~0.8 GB/s |
| All | Scalar | ~100 MiB/s |

### Field Iteration

| Pattern | Throughput |
|---------|------------|
| Sequential (all fields) | 792-1331 MiB/s |
| Random access | 93-173 ns/field |

### Memory Comparison

| Approach | Memory (100MB CSV) |
|----------|-------------------|
| Materialized | 600MB - 4GB |
| Semi-index | ~104 MB |
| Streaming | ~3-4 MB |

---

## Optimisation Techniques Used

| Technique | Document | Application |
|-----------|----------|-------------|
| PDEP carry propagation | [bit-manipulation.md](../optimisations/bit-manipulation.md) | toggle64_bmi2 (10x faster) |
| Parallel prefix XOR | [parallel-prefix.md](../optimisations/parallel-prefix.md) | Quote state tracking |
| SIMD comparison | [simd.md](../optimisations/simd.md) | Character detection |
| Lightweight index | [cache-memory.md](../optimisations/cache-memory.md) | Simple beats complex (5-9x) |
| Cumulative rank | [hierarchical-structures.md](../optimisations/hierarchical-structures.md) | O(1) rank queries |
| Binary search select | [access-patterns.md](../optimisations/access-patterns.md) | O(log n) field lookup |

---

## Cross-Chunk Quote State

Quotes can span 64-byte chunk boundaries. The carry mechanism handles this:

```rust
let mut carry: u64 = 0;  // Start outside quotes

for chunk in data.chunks(64) {
    let (delim_mask, quote_mask, newline_mask) = classify_chunk(chunk);

    // Apply quote masking with carry from previous chunk
    let (in_quote_mask, new_carry) = toggle64_bmi2(quote_mask, carry);
    carry = new_carry;

    // Mask out delimiters inside quotes
    let valid_delims = delim_mask & !in_quote_mask;
    let valid_newlines = newline_mask & !in_quote_mask;

    // Write to index
    markers.push(valid_delims | valid_newlines);
    newlines.push(valid_newlines);
}
```

---

## References

- RFC 4180: Common Format and MIME Type for CSV Files
- hw-dsv (Haskell): https://github.com/haskell-works/hw-dsv
- [DSV Performance Benchmarks](../dsv-performance.md)
