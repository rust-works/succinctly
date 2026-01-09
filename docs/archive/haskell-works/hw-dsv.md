# hw-dsv: High-Performance DSV Parser Using Succinct Data Structures

## Project Overview

**Repository**: https://github.com/haskell-works/hw-dsv
**Version**: 0.4.1.2
**Author**: John Ky (newhoggy@gmail.com)
**License**: BSD-3-Clause

hw-dsv is described as an "unbelievably fast streaming DSV file parser that reads based on succinct data structures." The library achieves remarkable performance improvements (10x+) over traditional parsing approaches by leveraging:

1. **Succinct Data Structures** - Compact bit-level representations for indexing
2. **SIMD Instructions** - AVX2 and BMI2 CPU intrinsics for parallel byte processing
3. **Rank/Select Operations** - O(1) navigation through the parsed structure

### Performance Claims

From the README benchmarks on a 7GB CSV file (2.9 GHz Intel Core i7):

| Configuration             | Throughput | Time |
|---------------------------|------------|------|
| BMI2 disabled (broadword) | 16.3 MiB/s | 7:25 |
| BMI2 only                 | 165 MiB/s  | 0:43 |
| BMI2 + AVX2               | 181 MiB/s  | 0:39 |

This represents an **11x speedup** when using BMI2+AVX2 over the broadword fallback.

---

## Code Structure and Key Modules

### Directory Layout

```
hw-dsv/
├── src/HaskellWorks/Data/Dsv/
│   ├── Internal/
│   │   ├── Bits.hs           # Bit manipulation utilities (pext, zipOr, etc.)
│   │   ├── Broadword.hs      # Broadword techniques for SWAR operations
│   │   ├── Char.hs           # Character constants (Word8)
│   │   ├── Char/Word64.hs    # 64-bit repeated character patterns
│   │   └── Vector.hs         # Core indexing algorithm (indexCsvChunk)
│   ├── Lazy/
│   │   ├── Cursor.hs         # Main lazy cursor API
│   │   ├── Cursor/Internal.hs # Navigation (nextField, nextRow, etc.)
│   │   ├── Cursor/Type.hs    # DsvCursor data type
│   │   ├── Cursor/Lazy.hs    # Lazy ByteString extraction
│   │   └── Cursor/Strict.hs  # Strict ByteString extraction
│   └── Strict/
│       ├── Cursor.hs         # Strict cursor with CsPoppy
│       ├── Cursor/Type.hs    # Parameterized DsvCursor type
│       ├── Cursor/Internal.hs # Optimized index building
│       └── Cursor/Internal/Reference.hs # Reference implementation
├── app/                      # CLI application
├── bench/                    # Criterion benchmarks
├── test/                     # HSpec + Hedgehog tests
└── weigh/                    # Memory profiling
```

### Core Data Types

#### Lazy DsvCursor (`Lazy/Cursor/Type.hs`)

```haskell
data DsvCursor = DsvCursor
  { dsvCursorText     :: !LBS.ByteString        -- Original CSV data
  , dsvCursorMarkers  :: ![DVS.Vector Word64]   -- Bit vectors marking delimiters/newlines
  , dsvCursorNewlines :: ![DVS.Vector Word64]   -- Bit vectors marking newlines only
  , dsvCursorPosition :: !Word64                -- Current position in bits
  }
```

#### Strict DsvCursor (`Strict/Cursor/Type.hs`)

```haskell
data DsvCursor t s = DsvCursor
  { dsvCursorDelimiter :: Elem t
  , dsvCursorText      :: !t                    -- Parameterized text type
  , dsvCursorMarkers   :: !s                    -- Parameterized succinct index type
  , dsvCursorNewlines  :: !s                    -- (e.g., CsPoppy for rank/select)
  , dsvCursorPosition  :: !Word64
  }
```

The strict cursor uses `CsPoppy` from `hw-rankselect` for O(1) rank and select operations.

---

## Key Algorithms and Computer Science Techniques

### 1. Semi-Index Construction (Two-Pass Approach)

The parser uses a **semi-index** approach where:

1. **First Pass**: Build bit vectors identifying all delimiter positions, newline positions, and quote positions
2. **Navigation**: Use rank/select operations to jump directly to fields/rows

This is fundamentally different from traditional byte-by-byte parsing.

### 2. SIMD-Accelerated Character Comparison

In `Lazy/Cursor.hs`, the `makeCursor` function uses SIMD comparison:

```haskell
makeCursor :: Word8 -> LBS.ByteString -> DsvCursor
makeCursor delimiter lbs = DsvCursor {...}
  where ws  = asVector64s 64 lbs                        -- Convert to Word64 vectors
        ibq = DVS.cmpEqWord8s C.doubleQuote <$> ws      -- SIMD compare for quotes
        ibn = DVS.cmpEqWord8s C.newline     <$> ws      -- SIMD compare for newlines
        ibd = DVS.cmpEqWord8s delimiter     <$> ws      -- SIMD compare for delimiter
        (ib, nls) = makeIndexes ibd ibn ibq
```

The `cmpEqWord8s` function (from `hw-simd`) uses AVX2 instructions to compare 32 bytes simultaneously against a target character, producing a 32-bit mask.

### 3. Quote State Tracking with Broadword Techniques

The quote-handling algorithm in `Internal/Vector.hs` is particularly elegant:

```haskell
indexCsvChunk :: Count -> Word64 -> DVS.Vector Word64 -> DVS.Vector Word64
              -> DVS.Vector Word64 -> (DVS.Vector Word64, DVS.Vector Word64, Word64, Word64)
indexCsvChunk qqCount qqCarry mks nls qqs = runST $ do
  -- ...
  let enters = pdep (oddsMask .<. (0x1 .&.      pc)) qq   -- Quote opens
  let leaves = pdep (oddsMask .<. (0x1 .&. comp pc)) qq   -- Quote closes

  let compLeaves    = comp leaves
  let preQuoteMask  = enters + compLeaves
  let quoteMask     = preQuoteMask + carry
  let newCarry      = quoteMask `ltWord` (enters .|. compLeaves .|. carry)
```

Key insight: This uses **parallel prefix sum** (via addition with carry propagation) to track whether each position is inside or outside quotes, all in parallel within a 64-bit word.

The `oddsMask = 0x5555555555555555` pattern places markers at alternating bit positions, which after `pdep` (parallel bit deposit) creates a proper mask for quote boundaries.

### 4. toggle64 - Quote State Machine

The `toggle64` function in `Internal/Broadword.hs` implements a bit-level toggle:

```haskell
toggle64 :: Word64 -> Word64 -> Word64
toggle64 carry w =
  let c = carry .&. 0x1
  in  let addend = pdep (0x5555555555555555 .<. c) w
      in  ((addend .<. 1) .|. c) + comp w
```

This computes a mask where bits are set in regions **outside** quotes. The carry tracks whether we started inside a quote from the previous 64-bit chunk.

### 5. Rank and Select Operations

Navigation uses O(1) rank/select from `hw-rankselect`:

```haskell
nextField :: DsvCursor -> DsvCursor
nextField cursor = cursor { dsvCursorPosition = newPos }
  where currentRank = rank1   (dsvCursorMarkers cursor) (dsvCursorPosition cursor)
        newPos      = select1 (dsvCursorMarkers cursor) (currentRank + 1) - 1
```

- **rank1(bv, i)**: Count set bits in positions [0, i]
- **select1(bv, n)**: Find position of the n-th set bit

These are O(1) operations using the CsPoppy data structure (a compact, cache-friendly rank/select implementation).

### 6. BMI2 Instructions Used

The library leverages these Intel BMI2 instructions:

- **PDEP (Parallel Bit Deposit)**: Deposits bits from source according to mask
- **PEXT (Parallel Bit Extract)**: Extracts bits from source according to mask

Example from `Internal/Bits.hs`:

```haskell
testWord8s :: Word64 -> Word64
testWord8s w = let w8s = w
                   w4s = w8s .|. (w8s .>. 4)
                   w2s = w4s .|. (w4s .>. 2)
                   w1s = w2s .|. (w2s .>. 1)
               in  pext w1s 0x0101010101010101
```

This collapses each byte to a single bit indicating if any bit was set, useful for detecting character matches.

---

## Haskell Language Features Leveraged

### Type System Features

1. **Type Classes with Multiple Parameters**
   ```haskell
   class Rank1 v => ...
   class Select1 v => ...
   ```

2. **Parameterized Types**
   ```haskell
   data DsvCursor t s = DsvCursor { ... }
   -- t = text type (ByteString), s = succinct index type (CsPoppy)
   ```

3. **Storable Vectors** for zero-copy memory mapping
   ```haskell
   DVS.Vector Word64  -- Contiguous, unboxed storage
   ```

### Performance Optimizations

1. **Strict Fields** with bang patterns
   ```haskell
   { dsvCursorText :: !LBS.ByteString, ... }
   ```

2. **INLINE Pragmas** everywhere
   ```haskell
   {-# INLINE makeCursor #-}
   {-# INLINE nextField #-}
   ```

3. **ST Monad** for safe in-place mutation
   ```haskell
   indexCsvChunk = runST $ do
     tmks <- DVSM.unsafeNew len
     -- ... mutable operations ...
     rmks <- DVS.unsafeFreeze tmks
   ```

4. **CPP Conditionals** for GHC version compatibility
   ```haskell
   #if MIN_VERSION_base(4,17,2)
   ltWord (W64# a#) (W64# b#) = fromIntegral (I# (ltWord64# a# b#))
   #else
   ltWord (W64# a#) (W64# b#) = fromIntegral (I64# (ltWord# a# b#))
   #endif
   ```

5. **Memory Mapping** for large files
   ```haskell
   mmapCursor :: Word8 -> Bool -> FilePath -> IO (DsvCursor BS.ByteString CsPoppy)
   ```

### Streaming via Lazy Data Structures

The lazy cursor uses `[DVS.Vector Word64]` (list of vectors) to enable streaming:
- Each 512-byte chunk is processed independently
- The `trim` function discards processed chunks:
  ```haskell
  trim c = if dsvCursorPosition c >= 512
    then trim c { dsvCursorText = LBS.drop 512 (dsvCursorText c)
                , dsvCursorMarkers = drop 1 (dsvCursorMarkers c)
                , ... }
    else c
  ```

---

## Dependencies (Key Libraries)

| Library | Purpose |
|---------|---------|
| `hw-rankselect` | Rank/select data structures (CsPoppy) |
| `hw-rankselect-base` | Base type classes for rank/select |
| `hw-simd` | SIMD-accelerated byte comparison |
| `hw-bits` | Bit manipulation utilities |
| `hw-prim` | Low-level primitives |
| `bits-extra` | PDEP/PEXT implementations |

---

## Rust Porting Considerations

### Feasibility Assessment: HIGH

Porting hw-dsv to Rust is highly feasible and potentially advantageous:

#### Advantages of Rust Port

1. **Native SIMD Support**
   - Rust's `std::arch` provides direct access to AVX2, BMI2 intrinsics
   - `packed_simd2` or `wide` crates for portable SIMD
   - No need for FFI like Haskell's `hw-simd`

2. **Zero-Cost Abstractions**
   - Rust's ownership model enables safe zero-copy parsing
   - No GC pauses during large file processing
   - Predictable memory layout

3. **Better Tooling for Low-Level Operations**
   - `#[target_feature(enable = "avx2")]` for function-level SIMD
   - `cfg(target_feature)` for compile-time feature detection
   - Runtime feature detection with `is_x86_feature_detected!`

4. **Memory Mapping**
   - `memmap2` crate provides excellent mmap support
   - Direct slice access to mapped memory

#### Key Porting Challenges

1. **Rank/Select Implementation**
   - Need to port CsPoppy or use existing Rust succinct libraries
   - Options: `succinct`, `bio` crates (limited), or custom implementation
   - CsPoppy is ~500 lines of Haskell; straightforward to port

2. **Streaming Architecture**
   - Haskell's lazy lists are implicit streaming
   - Rust needs explicit iterators or async streams
   - Consider `Iterator` trait for row-by-row access

3. **Quote State Machine**
   - The `toggle64` algorithm ports directly
   - Bit manipulation is idiomatic in Rust

#### Comparison to Existing Rust CSV Libraries

| Library | Approach | Performance | Streaming | Quote Handling |
|---------|----------|-------------|-----------|----------------|
| `csv` (BurntSushi) | Byte-by-byte DFA | Good | Yes | Full RFC 4180 |
| `csv-core` | Zero-alloc core | Better | Yes | Full RFC 4180 |
| `polars` | Arrow-based | Excellent | Limited | Full |
| **hw-dsv port** | Succinct + SIMD | Excellent | Yes | Basic |

**Key differentiator**: hw-dsv's approach would be unique in the Rust ecosystem. While `csv` is well-optimized, the succinct index approach offers:

- **O(1) random access** to any field/row (after index construction)
- **Parallel index construction** (SIMD)
- **Lower memory overhead** for navigation (bits vs pointers)

#### Suggested Rust Architecture

```rust
pub struct DsvCursor<'a> {
    text: &'a [u8],                    // Memory-mapped or owned
    markers: Vec<u64>,                 // Bit vector for delimiters
    newlines: Vec<u64>,                // Bit vector for newlines
    poppy: CsPoppy,                    // Rank/select index
    position: u64,
}

impl<'a> DsvCursor<'a> {
    pub fn new(delimiter: u8, data: &'a [u8]) -> Self {
        // SIMD-accelerated index construction
    }

    pub fn next_field(&mut self) -> Option<&'a [u8]> {
        // O(1) using rank/select
    }
}

impl<'a> Iterator for RowIterator<'a> {
    type Item = Row<'a>;
    // ...
}
```

#### SIMD Implementation Sketch

```rust
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[target_feature(enable = "avx2")]
unsafe fn cmp_eq_bytes(chunk: &[u8; 32], needle: u8) -> u32 {
    let data = _mm256_loadu_si256(chunk.as_ptr() as *const __m256i);
    let cmp = _mm256_set1_epi8(needle as i8);
    let mask = _mm256_cmpeq_epi8(data, cmp);
    _mm256_movemask_epi8(mask) as u32
}
```

### Recommended Porting Strategy

1. **Phase 1: Core Algorithms**
   - Port `toggle64` and quote state machine
   - Implement basic bit vector operations
   - Unit test against Haskell reference

2. **Phase 2: Index Construction**
   - SIMD character comparison
   - Index building with/without SIMD fallback

3. **Phase 3: Rank/Select**
   - Port CsPoppy or adapt existing Rust implementation
   - Verify O(1) performance

4. **Phase 4: API Design**
   - Iterator-based row access
   - Memory-mapped file support
   - Parallel processing with rayon

5. **Phase 5: Benchmarking**
   - Compare against `csv` crate
   - Profile memory usage
   - Test on multi-GB files

---

## Summary

hw-dsv represents a sophisticated application of succinct data structures to CSV parsing. The key innovations are:

1. **Semi-indexing**: Build a compact bit-level index instead of parsing byte-by-byte
2. **SIMD acceleration**: Process 32+ bytes simultaneously for character detection
3. **Rank/select navigation**: O(1) access to any field without scanning

A Rust port would be valuable for the ecosystem, offering a unique approach complementary to existing CSV libraries. The core algorithms (broadword operations, toggle64, rank/select) translate well to Rust, and the language's native SIMD support could potentially improve upon the Haskell implementation's performance.

---

## References

- [hw-dsv GitHub Repository](https://github.com/haskell-works/hw-dsv)
- [Succinct Data Structures (Wikipedia)](https://en.wikipedia.org/wiki/Succinct_data_structure)
- [Intel BMI2 Instructions](https://en.wikipedia.org/wiki/X86_Bit_manipulation_instruction_set#BMI2)
- [Semi-indexing Semi-structured Data in Tiny Space](https://www.di.ens.fr/~mauborgne/publi/sem_index.pdf)
- [CsPoppy: Cache-Succinct Rank/Select](https://www.cs.cmu.edu/~dga/papers/zhou-sea2013.pdf)
