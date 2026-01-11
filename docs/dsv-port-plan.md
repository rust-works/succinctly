# DSV Port Plan: High-Performance CSV/DSV Parsing for Succinctly

## Executive Summary

Port the core algorithms from `hw-dsv` to create a high-performance DSV (delimiter-separated values) parser for succinctly. The approach uses succinct semi-indexing to achieve O(1) field access with minimal memory overhead (~3-4%).

**Key insight from hw-dsv**: Instead of parsing CSV into a data structure, create two bit vectors (markers, newlines) that enable fast navigation via rank/select operations.

## Architecture Overview

```
Input CSV → Character Classification → Quote-Aware Filtering → DsvIndex
                                                                   ↓
                                                            DsvCursor
                                                                   ↓
                                              O(1) rank, O(log n) select
                                                                   ↓
                                                     Field/Row Navigation
```

## What We Can Reuse Directly

| Component              | Location              | Reuse Level |
|------------------------|-----------------------|-------------|
| BitVec + rank/select   | `src/bits/bitvec.rs`  | 100%        |
| BitWriter              | `src/json/bit_writer.rs` | 100%     |
| Binary serialization   | `src/binary.rs`       | 100%        |
| RankSelect trait       | `src/lib.rs`          | 100%        |
| Config pattern         | `src/lib.rs`          | 90%         |
| Cursor design pattern  | `src/json/light.rs`   | 85%         |

## Module Structure

```
src/dsv/
├── mod.rs              # Public API exports
├── index.rs            # DsvIndex - the semi-index structure
├── cursor.rs           # DsvCursor - navigation and field access
├── parser.rs           # Quote-aware indexing algorithm (core port)
├── record.rs           # DsvRecord, DsvField - lazy value types
└── config.rs           # DsvConfig - delimiter, quote char, etc.
```

## Implementation Plan

### Phase 1: Core Data Structures

#### 1.1 DsvConfig

```rust
// src/dsv/config.rs

/// Configuration for DSV parsing
#[derive(Debug, Clone)]
pub struct DsvConfig {
    /// Field delimiter (default: b',')
    pub delimiter: u8,
    /// Quote character (default: b'"')
    pub quote_char: u8,
    /// Record delimiter (default: b'\n')
    pub newline: u8,
    /// Select sample rate for BitVec (default: 256)
    pub select_sample_rate: u32,
}

impl Default for DsvConfig {
    fn default() -> Self {
        Self {
            delimiter: b',',
            quote_char: b'"',
            newline: b'\n',
            select_sample_rate: 256,
        }
    }
}

impl DsvConfig {
    pub fn csv() -> Self { Self::default() }
    pub fn tsv() -> Self { Self { delimiter: b'\t', ..Self::default() } }
    pub fn psv() -> Self { Self { delimiter: b'|', ..Self::default() } }
}
```

#### 1.2 DsvIndex

```rust
// src/dsv/index.rs

use crate::bits::BitVec;

/// Semi-index for DSV data enabling fast field/row navigation
pub struct DsvIndex {
    /// Bit vector marking field delimiter positions (filtered by quote state)
    pub markers: BitVec,
    /// Bit vector marking newline positions (filtered by quote state)
    pub newlines: BitVec,
    /// Total byte length of indexed text
    pub text_len: usize,
}

impl DsvIndex {
    /// Number of fields (delimiters + newlines)
    pub fn field_count(&self) -> usize {
        self.markers.count_ones()
    }

    /// Number of rows
    pub fn row_count(&self) -> usize {
        self.newlines.count_ones()
    }
}
```

### Phase 2: Quote-Aware Indexing Algorithm (Core Port)

This is the heart of hw-dsv - the algorithm that masks out delimiters inside quoted fields.

#### 2.1 Character Classification

```rust
// src/dsv/parser.rs

/// Replicate a byte across all 8 positions in a u64
#[inline]
fn fill_word64(b: u8) -> u64 {
    0x0101_0101_0101_0101_u64 * (b as u64)
}

/// Compare each byte in word against target, return mask of matches
/// Each bit in result corresponds to a byte position (0-7)
#[inline]
fn cmp_eq_word8s(target: u8, word: u64) -> u8 {
    let pattern = fill_word64(target);
    let xor = word ^ pattern;

    // Detect zero bytes using the classic trick:
    // A byte is zero iff (byte - 0x01) & ~byte & 0x80 is set
    let lo = fill_word64(0x01);
    let hi = fill_word64(0x80);

    let zero_mask = (xor.wrapping_sub(lo)) & !xor & hi;

    // Extract one bit per byte position
    ((zero_mask >> 7) & 0x01)
        | ((zero_mask >> 14) & 0x02)
        | ((zero_mask >> 21) & 0x04)
        | ((zero_mask >> 28) & 0x08)
        | ((zero_mask >> 35) & 0x10)
        | ((zero_mask >> 42) & 0x20)
        | ((zero_mask >> 49) & 0x40)
        | ((zero_mask >> 56) & 0x80)
}
```

#### 2.2 Quote-Aware Chunk Indexing (The Core Algorithm)

```rust
// src/dsv/parser.rs

/// Odd positions mask for PDEP: 0101...
const ODDS_MASK: u64 = 0x5555_5555_5555_5555;

/// Process a 64-bit chunk with quote-aware filtering
///
/// # Arguments
/// * `qq_carry` - Quote parity carry from previous chunk (0 or 1)
/// * `delims` - Bits set at delimiter positions
/// * `newlines` - Bits set at newline positions
/// * `quotes` - Bits set at quote positions
///
/// # Returns
/// * `(filtered_markers, filtered_newlines, new_carry)`
#[inline]
fn index_chunk(
    qq_carry: u64,
    delims: u64,
    newlines: u64,
    quotes: u64,
) -> (u64, u64, u64) {
    // The algorithm determines which positions are "outside quotes"
    // by tracking quote parity with carry propagation.
    //
    // Key insight: Between paired quotes, all bits should be masked.
    // We use PDEP to scatter quote bits and addition for parity tracking.

    #[cfg(target_feature = "bmi2")]
    {
        use core::arch::x86_64::_pdep_u64;

        // Scatter quotes into alternating positions based on carry
        let enters = unsafe { _pdep_u64(quotes, ODDS_MASK << (qq_carry & 1)) };
        let leaves = unsafe { _pdep_u64(quotes, ODDS_MASK << ((qq_carry ^ 1) & 1)) };

        // Compute quote mask via carry-propagating addition
        let comp_leaves = !leaves;
        let (quote_mask, carry_out) = enters.overflowing_add(comp_leaves.wrapping_add(qq_carry));

        // Positions outside quotes have quote_mask bit set
        let outside_quotes = quote_mask;

        // Filter: keep only delimiters/newlines outside quotes
        let filtered_markers = (delims | newlines) & outside_quotes;
        let filtered_newlines = newlines & outside_quotes;

        let new_carry = if carry_out || quote_mask < (enters | comp_leaves | qq_carry) { 1 } else { 0 };

        (filtered_markers, filtered_newlines, new_carry)
    }

    #[cfg(not(target_feature = "bmi2"))]
    {
        // Portable fallback using popcount-based parity tracking
        index_chunk_portable(qq_carry, delims, newlines, quotes)
    }
}

/// Portable fallback without BMI2 PDEP
fn index_chunk_portable(
    qq_carry: u64,
    delims: u64,
    newlines: u64,
    quotes: u64,
) -> (u64, u64, u64) {
    // Track quote state bit-by-bit using prefix XOR (popcount parity)
    // This is slower but works on all architectures

    let mut quote_mask: u64 = 0;
    let mut in_quote = qq_carry != 0;

    for i in 0..64 {
        let bit = 1u64 << i;
        if quotes & bit != 0 {
            in_quote = !in_quote;
        }
        if !in_quote {
            quote_mask |= bit;
        }
    }

    let filtered_markers = (delims | newlines) & quote_mask;
    let filtered_newlines = newlines & quote_mask;
    let new_carry = if in_quote { 1 } else { 0 };

    (filtered_markers, filtered_newlines, new_carry)
}
```

#### 2.3 Full Index Builder

```rust
// src/dsv/parser.rs

use crate::bits::{BitVec, BitWriter};
use super::config::DsvConfig;
use super::index::DsvIndex;

/// Build a DsvIndex from input text
pub fn build_index(text: &[u8], config: &DsvConfig) -> DsvIndex {
    let num_words = (text.len() + 63) / 64;

    let mut markers_writer = BitWriter::with_capacity(num_words);
    let mut newlines_writer = BitWriter::with_capacity(num_words);
    let mut qq_carry: u64 = 0;

    // Process 8 bytes (64 bits) at a time
    let chunks = text.chunks(8);
    let mut remaining = text.len();

    for chunk in chunks {
        // Pad last chunk if needed
        let mut word = [0u8; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        let word = u64::from_le_bytes(word);

        // Classify bytes
        let delims = cmp_eq_word8s(config.delimiter, word) as u64;
        let newlines = cmp_eq_word8s(config.newline, word) as u64;
        let quotes = cmp_eq_word8s(config.quote_char, word) as u64;

        // Expand byte masks to bit masks (1 bit per byte → 8 bits)
        let delims = expand_byte_mask(delims);
        let newlines = expand_byte_mask(newlines);
        let quotes = expand_byte_mask(quotes);

        // Apply quote-aware filtering
        let (filtered_markers, filtered_newlines, new_carry) =
            index_chunk(qq_carry, delims, newlines, quotes);

        qq_carry = new_carry;

        // Write to bit vectors (only valid bits for last chunk)
        let valid_bits = remaining.min(8);
        let mask = if valid_bits == 8 { !0u64 } else { (1u64 << valid_bits) - 1 };

        markers_writer.write_bits(filtered_markers & mask, valid_bits);
        newlines_writer.write_bits(filtered_newlines & mask, valid_bits);

        remaining = remaining.saturating_sub(8);
    }

    DsvIndex {
        markers: BitVec::new(markers_writer.into_words(), config.select_sample_rate),
        newlines: BitVec::new(newlines_writer.into_words(), config.select_sample_rate),
        text_len: text.len(),
    }
}

/// Expand a byte mask (1 bit per byte) to a full mask (8 bits per byte, but we use 1)
/// Input: bits 0-7 indicate which bytes matched
/// Output: bit at position i*8 is set if byte i matched
#[inline]
fn expand_byte_mask(byte_mask: u64) -> u64 {
    // For simplicity, we work at byte granularity
    // bit i in input → bit i in output (each bit represents one byte position)
    byte_mask
}
```

### Phase 3: Cursor and Navigation

#### 3.1 DsvCursor

```rust
// src/dsv/cursor.rs

use super::index::DsvIndex;
use crate::bits::RankSelect;

/// Lightweight cursor for navigating DSV data
#[derive(Clone, Copy)]
pub struct DsvCursor<'a> {
    text: &'a [u8],
    index: &'a DsvIndex,
    /// Current position in text (byte offset)
    position: usize,
    /// Hint for accelerating sequential access
    marker_hint: usize,
    newline_hint: usize,
}

impl<'a> DsvCursor<'a> {
    pub fn new(text: &'a [u8], index: &'a DsvIndex) -> Self {
        Self {
            text,
            index,
            position: 0,
            marker_hint: 0,
            newline_hint: 0,
        }
    }

    /// Current byte position in text
    pub fn position(&self) -> usize {
        self.position
    }

    /// Are we at end of data?
    pub fn at_end(&self) -> bool {
        self.position >= self.text.len()
    }

    /// Move to the next field delimiter
    pub fn next_field(&mut self) -> bool {
        if self.at_end() {
            return false;
        }

        // Find rank of current position in markers
        let current_rank = self.index.markers.rank1(self.position);

        // Find position of next marker
        if let Some(next_pos) = self.index.markers.select1(current_rank + 1) {
            if next_pos < self.text.len() {
                self.position = next_pos + 1; // Move past the delimiter
                self.marker_hint = current_rank + 1;
                return true;
            }
        }

        // No more fields, move to end
        self.position = self.text.len();
        false
    }

    /// Move to the start of the next row
    pub fn next_row(&mut self) -> bool {
        if self.at_end() {
            return false;
        }

        // Find rank of current position in newlines
        let current_rank = self.index.newlines.rank1(self.position);

        // Find position of next newline
        if let Some(next_pos) = self.index.newlines.select1(current_rank + 1) {
            if next_pos < self.text.len() {
                self.position = next_pos + 1; // Move past the newline
                self.newline_hint = current_rank + 1;
                return true;
            }
        }

        // No more rows
        self.position = self.text.len();
        false
    }

    /// Move to the start of row `n` (0-indexed)
    pub fn goto_row(&mut self, n: usize) -> bool {
        if n == 0 {
            self.position = 0;
            return true;
        }

        // Row n starts after newline (n-1)
        if let Some(newline_pos) = self.index.newlines.select1(n) {
            if newline_pos < self.text.len() {
                self.position = newline_pos + 1;
                self.newline_hint = n;
                return true;
            }
        }

        false
    }

    /// Get field at current position (up to next delimiter or newline)
    pub fn current_field(&self) -> &'a [u8] {
        if self.at_end() {
            return &[];
        }

        let start = self.position;

        // Find next marker (delimiter or newline)
        let current_rank = self.index.markers.rank1(start);
        let end = self.index.markers
            .select1(current_rank + 1)
            .unwrap_or(self.text.len());

        &self.text[start..end]
    }

    /// Get current field as a string (handles quote stripping)
    pub fn current_field_str(&self) -> Result<&'a str, std::str::Utf8Error> {
        let field = self.current_field();
        let field = strip_quotes(field);
        std::str::from_utf8(field)
    }
}

/// Strip surrounding quotes from a field if present
fn strip_quotes(field: &[u8]) -> &[u8] {
    if field.len() >= 2 && field[0] == b'"' && field[field.len() - 1] == b'"' {
        &field[1..field.len() - 1]
    } else {
        field
    }
}
```

#### 3.2 Row and Field Iterators

```rust
// src/dsv/cursor.rs (continued)

/// Iterator over rows in DSV data
pub struct DsvRows<'a> {
    cursor: DsvCursor<'a>,
    started: bool,
}

impl<'a> DsvRows<'a> {
    pub fn new(text: &'a [u8], index: &'a DsvIndex) -> Self {
        Self {
            cursor: DsvCursor::new(text, index),
            started: false,
        }
    }
}

impl<'a> Iterator for DsvRows<'a> {
    type Item = DsvRow<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.started {
            self.started = true;
            if self.cursor.at_end() {
                return None;
            }
        } else if !self.cursor.next_row() {
            return None;
        }

        Some(DsvRow {
            cursor: self.cursor,
        })
    }
}

/// A single row in DSV data
#[derive(Clone, Copy)]
pub struct DsvRow<'a> {
    cursor: DsvCursor<'a>,
}

impl<'a> DsvRow<'a> {
    /// Iterate over fields in this row
    pub fn fields(&self) -> DsvFields<'a> {
        DsvFields {
            cursor: self.cursor,
            started: false,
        }
    }

    /// Get field at column index (0-indexed)
    pub fn get(&self, column: usize) -> Option<&'a [u8]> {
        let mut cursor = self.cursor;
        for _ in 0..column {
            if !cursor.next_field() {
                return None;
            }
        }
        Some(cursor.current_field())
    }
}

/// Iterator over fields in a row
pub struct DsvFields<'a> {
    cursor: DsvCursor<'a>,
    started: bool,
}

impl<'a> Iterator for DsvFields<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if !self.started {
            self.started = true;
        } else if !self.cursor.next_field() {
            return None;
        }

        // Stop at end of row (newline)
        let field = self.cursor.current_field();
        if field.is_empty() && self.cursor.at_end() {
            return None;
        }

        Some(field)
    }
}
```

### Phase 4: Public API

```rust
// src/dsv/mod.rs

mod config;
mod index;
mod cursor;
mod parser;

pub use config::DsvConfig;
pub use index::DsvIndex;
pub use cursor::{DsvCursor, DsvRow, DsvRows, DsvFields};
pub use parser::build_index;

/// Parse DSV data and return an indexed structure
pub fn parse(text: &[u8]) -> Dsv {
    parse_with_config(text, &DsvConfig::default())
}

/// Parse DSV data with custom configuration
pub fn parse_with_config(text: &[u8], config: &DsvConfig) -> Dsv {
    let index = build_index(text, config);
    Dsv {
        text: text.to_vec(),
        index,
    }
}

/// Owned DSV data with index
pub struct Dsv {
    text: Vec<u8>,
    index: DsvIndex,
}

impl Dsv {
    /// Create a cursor for navigation
    pub fn cursor(&self) -> DsvCursor {
        DsvCursor::new(&self.text, &self.index)
    }

    /// Iterate over rows
    pub fn rows(&self) -> DsvRows {
        DsvRows::new(&self.text, &self.index)
    }

    /// Number of rows
    pub fn row_count(&self) -> usize {
        self.index.row_count()
    }

    /// Get a specific row by index
    pub fn row(&self, n: usize) -> Option<DsvRow> {
        let mut cursor = self.cursor();
        if cursor.goto_row(n) {
            Some(DsvRow { cursor })
        } else {
            None
        }
    }
}

/// Borrowed DSV reference (for memory-mapped files)
pub struct DsvRef<'a> {
    text: &'a [u8],
    index: &'a DsvIndex,
}

impl<'a> DsvRef<'a> {
    pub fn new(text: &'a [u8], index: &'a DsvIndex) -> Self {
        Self { text, index }
    }

    pub fn cursor(&self) -> DsvCursor<'a> {
        DsvCursor::new(self.text, self.index)
    }

    pub fn rows(&self) -> DsvRows<'a> {
        DsvRows::new(self.text, self.index)
    }
}
```

### Phase 5: Integration

#### 5.1 Update lib.rs

```rust
// Add to src/lib.rs

pub mod dsv;

// Re-export main types
pub use dsv::{Dsv, DsvConfig, DsvCursor, DsvIndex};
```

#### 5.2 CLI Commands (Optional)

```rust
// src/bin/succinctly/dsv.rs

/// DSV subcommands for the CLI
#[derive(clap::Subcommand)]
pub enum DsvCommands {
    /// Parse CSV and extract columns
    Query {
        /// Input file
        #[arg(short, long)]
        input: PathBuf,

        /// Column indices to extract (1-indexed)
        #[arg(short = 'k', long)]
        columns: Vec<usize>,

        /// Field delimiter
        #[arg(short, long, default_value = ",")]
        delimiter: char,

        /// Output delimiter
        #[arg(short = 'e', long, default_value = ",")]
        output_delimiter: char,
    },

    /// Count rows and fields
    Count {
        #[arg(short, long)]
        input: PathBuf,

        #[arg(short, long, default_value = ",")]
        delimiter: char,
    },
}
```

## Testing Strategy

### Unit Tests

```rust
// tests/dsv_tests.rs

#[test]
fn test_simple_csv() {
    let csv = b"a,b,c\n1,2,3\n";
    let dsv = succinctly::dsv::parse(csv);

    assert_eq!(dsv.row_count(), 2);

    let row0: Vec<_> = dsv.row(0).unwrap().fields().collect();
    assert_eq!(row0, vec![b"a", b"b", b"c"]);
}

#[test]
fn test_quoted_fields() {
    let csv = b"\"hello, world\",b\n";
    let dsv = succinctly::dsv::parse(csv);

    let row: Vec<_> = dsv.row(0).unwrap().fields().collect();
    assert_eq!(row.len(), 2);
    assert_eq!(row[0], b"\"hello, world\"");
}

#[test]
fn test_quoted_newlines() {
    let csv = b"\"line1\nline2\",b\n";
    let dsv = succinctly::dsv::parse(csv);

    assert_eq!(dsv.row_count(), 1); // Only one logical row
}

#[test]
fn test_escaped_quotes() {
    let csv = b"\"say \"\"hello\"\"\",b\n";
    let dsv = succinctly::dsv::parse(csv);

    let field = dsv.row(0).unwrap().get(0).unwrap();
    // Field contains the raw bytes including quotes
}
```

### Property Tests

```rust
// tests/dsv_property_tests.rs

use proptest::prelude::*;

proptest! {
    #[test]
    fn roundtrip_simple_csv(rows: Vec<Vec<String>>) {
        // Generate CSV from rows
        let csv = rows.iter()
            .map(|r| r.join(","))
            .collect::<Vec<_>>()
            .join("\n");

        let dsv = succinctly::dsv::parse(csv.as_bytes());

        // Verify row count matches
        prop_assert_eq!(dsv.row_count(), rows.len());
    }
}
```

## Benchmarks

```rust
// benches/dsv_bench.rs

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

fn bench_csv_parsing(c: &mut Criterion) {
    let csv = generate_csv(10_000, 10); // 10k rows, 10 columns

    let mut group = c.benchmark_group("dsv_parsing");
    group.throughput(Throughput::Bytes(csv.len() as u64));

    group.bench_function("succinctly", |b| {
        b.iter(|| succinctly::dsv::parse(&csv))
    });

    // Compare with csv crate
    group.bench_function("csv_crate", |b| {
        b.iter(|| {
            let mut rdr = csv::Reader::from_reader(&csv[..]);
            for result in rdr.records() {
                let _ = result.unwrap();
            }
        })
    });

    group.finish();
}
```

## Implementation Order

1. **Phase 1** (~1 day): Core data structures
   - [ ] `DsvConfig`
   - [ ] `DsvIndex`
   - [ ] Basic module structure

2. **Phase 2** (~2-3 days): Quote-aware indexing algorithm
   - [ ] `cmp_eq_word8s` byte comparison
   - [ ] `index_chunk` with BMI2 PDEP
   - [ ] `index_chunk_portable` fallback
   - [ ] `build_index` main function

3. **Phase 3** (~2 days): Cursor and navigation
   - [ ] `DsvCursor` with next_field/next_row
   - [ ] `DsvRow`, `DsvRows` iterators
   - [ ] `DsvFields` iterator
   - [ ] Quote stripping utilities

4. **Phase 4** (~1 day): Public API and integration
   - [ ] `Dsv` owned type
   - [ ] `DsvRef` borrowed type
   - [ ] Update `lib.rs` exports

5. **Phase 5** (~1 day): Testing
   - [ ] Unit tests for edge cases
   - [ ] Property tests
   - [ ] Integration tests

6. **Phase 6** (~1 day): CLI and benchmarks
   - [ ] CLI commands
   - [ ] Criterion benchmarks
   - [ ] Performance comparison with csv crate

## BMI2 Optimization Details

The hw-dsv library achieves ~10x speedup on x86_64 using two BMI2 instructions: **PDEP** and **PEXT**.
These are critical for quote-aware parsing performance.

### Current Implementation Status

| Optimization | hw-dsv | succinctly | Status |
|--------------|--------|------------|--------|
| `toggle64` with PDEP | Yes | Yes | **Ported** |
| `testWord8s` with PEXT | Yes | No | Not needed (AVX2 movemask suffices) |
| `prefix_xor` fallback | No | Yes | Implemented (ARM fallback) |
| AVX2 character matching | Yes | Yes | Ported |
| NEON character matching | No | Yes | Added |
| Runtime BMI2 detection | Yes | Yes | Ported |

### BMI2 Instructions Used by hw-dsv

#### 1. PDEP (Parallel Bit Deposit) - Critical for Quote Masking

**Function:** `toggle64` in `hw-dsv/src/HaskellWorks/Data/Dsv/Internal/Broadword.hs`

```haskell
toggle64 :: Word64 -> Word64 -> Word64
toggle64 carry w =
  let c = carry .&. 0x1
      addend = pdep (0x5555555555555555 .<. c) w
  in ((addend .<. 1) .|. c) + comp w
```

**Purpose:** Generate a mask that toggles between "inside quotes" and "outside quotes".

**How it works:**
1. Input `w` is a bitmask where `1` = quote position
2. `0x5555555555555555` = alternating `01010101...` pattern
3. PDEP places the alternating pattern at quote positions
4. Addition creates carry propagation that toggles regions
5. Result: `1` = outside quotes, `0` = inside quotes

**Performance:**
- With PDEP: ~3 cycles
- Without PDEP: ~50-100 operations (must iterate through quote positions)

**Rust implementation:**

```rust
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "bmi2")]
unsafe fn toggle64_bmi2(carry: u64, w: u64) -> u64 {
    use std::arch::x86_64::_pdep_u64;
    let c = carry & 0x1;
    let addend = _pdep_u64(0x5555555555555555u64 << c, w);
    ((addend << 1) | c).wrapping_add(!w)
}
```

#### 2. PEXT (Parallel Bit Extract) - For Byte Compression

**Function:** `testWord8s` in `hw-dsv/src/HaskellWorks/Data/Dsv/Internal/Bits.hs`

```haskell
testWord8s :: Word64 -> Word64
testWord8s w = let w8s = w
                   w4s = w8s .|. (w8s .>. 4)
                   w2s = w4s .|. (w4s .>. 2)
                   w1s = w2s .|. (w2s .>. 1)
               in pext w1s 0x0101010101010101
```

**Purpose:** Compress 8 bytes of SIMD comparison results into 8 bits.

**How it works:**
1. Takes 8 bytes where each is 0x00 or 0xFF (from SIMD comparison)
2. OR-reduces each byte to its LSB
3. Uses PEXT to extract the 8 LSBs into positions 0-7

**Performance:**
- With PEXT: 1 cycle
- Without PEXT: ~15-20 operations

**Rust implementation:**

```rust
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "bmi2")]
unsafe fn test_word8s_bmi2(w: u64) -> u64 {
    use std::arch::x86_64::_pext_u64;
    let w4s = w | (w >> 4);
    let w2s = w4s | (w4s >> 2);
    let w1s = w2s | (w2s >> 1);
    _pext_u64(w1s, 0x0101010101010101)
}
```

#### 3. indexCsvChunk - Quote Entry/Exit Tracking

**Location:** `hw-dsv/src/HaskellWorks/Data/Dsv/Internal/Vector.hs`

```haskell
let enters = pdep (oddsMask .<. (0x1 .&.      pc)) qq
let leaves = pdep (oddsMask .<. (0x1 .&. comp pc)) qq
```

**Purpose:** Track where quotes are entered and exited for proper state management.

### Performance Impact

**Measured on 7GB CSV file (hw-dsv benchmarks):**

| Configuration | Throughput | Speedup |
|--------------|-----------|---------|
| Broadword (no BMI2) | 16.3 MiB/s | 1x |
| BMI2 enabled | 165 MiB/s | **10.1x** |
| BMI2 + AVX2 | 181 MiB/s | **11.1x** |

**Current succinctly performance:** 10-28 MiB/s (using `prefix_xor`)

### Why BMI2 Matters

1. **Single-cycle operations** - PDEP/PEXT execute in 1 cycle on Intel Haswell+ and AMD Zen+
2. **Eliminates branches** - Bit manipulation replaces conditional logic
3. **Reduces register pressure** - One instruction vs. many temporaries
4. **Enables elegant algorithms** - `toggle64` would be very complex without PDEP

### ARM/Apple Silicon Limitation

ARM has **no equivalent** to PDEP/PEXT:
- SVE2 has BDEP/BEXT but operates on vectors only
- Apple Silicon (M1-M4) doesn't implement SVE/SVE2
- Must use software emulation (`prefix_xor` approach)

Expected ARM performance: 25-40 MiB/s (better than broadword due to NEON SIMD)

### Implementation Plan for BMI2

1. **Add runtime detection:**
```rust
pub fn build_index(text: &[u8], config: &DsvConfig) -> DsvIndex {
    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("bmi2") && is_x86_feature_detected!("avx2") {
        return unsafe { build_index_bmi2_avx2(text, config) };
    }
    // Fall back to current implementation
    build_index_simd(text, config)
}
```

2. **Implement BMI2 path:**
   - Replace `prefix_xor` with `toggle64_bmi2`
   - Use PEXT for byte mask compression where applicable
   - Keep existing SIMD character matching

3. **Benchmark both paths:**
   - Verify 5-10x improvement on x86_64 with BMI2
   - Ensure no regression on ARM

## Key Differences from hw-dsv

| Aspect              | hw-dsv                    | This Port                      |
|---------------------|---------------------------|--------------------------------|
| Streaming           | Chunked lazy processing   | Full file in memory (simpler)  |
| Index storage       | List of Vec<u64>          | Single BitVec with rank/select |
| SIMD                | AVX2 + BMI2               | AVX2 + BMI2 (x86) / NEON (ARM) |
| BMI2 PDEP           | Yes (toggle64)            | Yes (toggle64_bmi2)            |
| Quote handling      | Same algorithm            | Same algorithm                 |
| Memory model        | Haskell lazy              | Rust owned/borrowed            |

## Performance Expectations

Based on hw-dsv benchmarks:
- **With BMI2** (x86_64): ~165 MiB/s expected
- **With AVX2 only** (x86_64): ~20-46 MiB/s (prefix_xor fallback)
- **With NEON** (ARM): ~10-28 MiB/s (prefix_xor, no BMI2 equivalent)
- **Memory overhead**: ~3-4% of input size

Succinctly's existing BitVec infrastructure should provide comparable performance to hw-dsv's rank/select operations.

## Implementation Files

| File | Purpose |
|------|---------|
| `src/dsv/simd/bmi2.rs` | BMI2 + AVX2 implementation with toggle64_bmi2 |
| `src/dsv/simd/avx2.rs` | AVX2 implementation with prefix_xor fallback |
| `src/dsv/simd/sse2.rs` | SSE2 implementation for older x86_64 |
| `src/dsv/simd/neon.rs` | ARM NEON implementation with prefix_xor |
| `src/dsv/simd/mod.rs` | Runtime dispatch (BMI2 > AVX2 > SSE2) |

## Open Questions

1. **Escape handling**: Should we support backslash escapes in addition to doubled quotes?
2. **Header detection**: Auto-detect header row?
3. **Type inference**: Parse numeric fields automatically?
4. **Error handling**: How to report malformed CSV (unclosed quotes, etc.)?

## References

- [hw-dsv source](../../haskell-works/hw-dsv)
- [Semi-Indexing Semi-Structured Data in Tiny Space](https://arxiv.org/abs/1104.4892) (Ottaviano et al.)
- Succinctly's existing JSON semi-indexing in `src/json/`
