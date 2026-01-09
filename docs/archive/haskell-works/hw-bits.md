# hw-bits: Bit Manipulation Library Analysis

## Project Overview

**hw-bits** is a Haskell library providing facilities for manipulating bits. It is part of the larger `haskell-works` ecosystem of libraries designed for building succinct data structures.

- **Repository**: https://github.com/haskell-works/hw-bits
- **Version**: 0.7.2.2
- **License**: BSD-3-Clause
- **Author**: John Ky
- **Category**: Data, Bit

### Purpose

The library provides:
1. Bit-level operations on primitive word types (Word8, Word16, Word32, Word64)
2. Vector-level bit operations for efficient bulk processing
3. Broadword computing primitives for SIMD-like parallel bit manipulation
4. Bit string parsing and display utilities
5. Population count (popcount) implementations
6. Logarithm base 2 computation using De Bruijn sequences
7. A mutable bit writer for constructing bit vectors

### Dependencies

Key dependencies include:
- `bitvec`: Provides unboxed bit vectors
- `bytestring`: For byte string processing
- `vector`: Boxed, storable, and unboxed vectors
- `hw-prim`: Primitive operations (AtIndex, Positioning types)
- `hw-int`: Integer widening operations
- `hw-string-parse`: String parsing utilities

## Code Structure and Key Modules

```
src/HaskellWorks/Data/Bits/
+-- Bits.hs                      # Main re-export module
+-- AllExcess/
|   +-- AllExcess.hs             # Re-exports
|   +-- AllExcess0.hs            # Count of 0s minus 1s
|   +-- AllExcess1.hs            # Count of 1s minus 0s
+-- BitLength.hs                 # Bit length computation
+-- BitParse.hs                  # Parsing bit strings to values
+-- BitRead.hs                   # High-level bit string reading
+-- BitShow.hs                   # Display values as bit strings
+-- BitShown.hs                  # Newtype wrapper for BitShow
+-- BitWise.hs                   # Core bitwise operations
+-- Broadword/
|   +-- Broadword.hs             # Re-exports (deprecated)
|   +-- Type.hs                  # Broadword newtype
|   +-- Word8.hs                 # Broadword ops for Word8
|   +-- Word16.hs                # Broadword ops for Word16
|   +-- Word32.hs                # Broadword ops for Word32
|   +-- Word64.hs                # Broadword ops for Word64
+-- ElemFixedBitSize.hs          # Fixed bit size for container elements
+-- FixedBitSize.hs              # Fixed bit size type class
+-- FromBitTextByteString.hs     # Parse bit text to vectors
+-- LoBitsSized.hs               # Mask with N low bits set
+-- Log2.hs                      # Fast log2 using De Bruijn
+-- PopCount/
|   +-- PopCount.hs              # Re-exports
|   +-- PopCount0.hs             # Count zero bits
|   +-- PopCount1.hs             # Count one bits
+-- Types/
|   +-- Broadword.hs             # Broadword newtype wrapper
|   +-- Builtin.hs               # Builtin ops newtype wrapper
+-- Word.hs                      # Word concat/split operations
+-- Word64.hs                    # Word64 specific ops (lsb)
+-- Writer/
    +-- Storable.hs              # Mutable bit vector writer
```

### Module Descriptions

#### Core Bitwise Operations (`BitWise.hs`)

Defines the fundamental type classes for bit manipulation:

```haskell
class BitWise a where
  (.&.) :: a -> a -> a   -- Bitwise AND
  (.|.) :: a -> a -> a   -- Bitwise OR
  (.^.) :: a -> a -> a   -- Bitwise XOR
  comp  :: a -> a        -- Complement
  all0s :: a             -- All zeros
  all1s :: a             -- All ones

class Shift a where
  (.<.) :: a -> Count -> a   -- Shift left
  (.>.) :: a -> Count -> a   -- Shift right

class TestBit a where
  (.?.) :: a -> Position -> Bool  -- Test bit at position

class Bit a where
  bit :: Position -> a   -- Value with single bit set
```

These operators follow C-style precedence conventions and are implemented for:
- Primitive types: Bool, Int, Word8, Word16, Word32, Word64
- Vectors: `DV.Vector`, `DVS.Vector`, `DVU.Vector`
- Specialized bit vectors: `DVU.Vector Bit.Bit`, `DVU.Vector BitTS.Bit`

#### Bit Length (`BitLength.hs`)

Provides bit length computation for various types:

```haskell
class BitLength v where
  bitLength :: v -> Count
  endPosition :: v -> Position
```

#### Population Count (`PopCount/`)

Two complementary type classes:

```haskell
class PopCount0 v where
  popCount0 :: v -> Count   -- Count of 0-bits

class PopCount1 v where
  popCount1 :: v -> Count   -- Count of 1-bits
```

The library provides two implementations:
1. **Broadword**: Software-based bit counting using the classic algorithm:
   ```haskell
   popCount1 (Broadword x0) = widen64 (x3 * 0x0101010101010101) .>. 56
     where
       x1 = x0 - ((x0 .&. 0xaaaaaaaaaaaaaaaa) .>. 1)
       x2 = (x1 .&. 0x3333333333333333) + ((x1 .>. 2) .&. 0x3333333333333333)
       x3 = (x2 + (x2 .>. 4)) .&. 0x0f0f0f0f0f0f0f0f
   ```

2. **Builtin**: Uses GHC's `Data.Bits.popCount` which maps to hardware instructions (POPCNT)

#### Broadword Computing (`Broadword/`)

Implements SIMD-like operations on packed sub-words within a single machine word:

- **`h k`**: Returns a mask with the high bit set in each k-bit sub-word
- **`l k`**: Returns a mask with the low bit set in each k-bit sub-word
- **`kBitDiff k x y`**: Parallel subtraction of k-bit sub-words
- **`kBitDiffPos k x y`**: Same but bounded at 0 (no underflow)
- **`kBitDiffUnsafe k x y`**: Faster version assuming no overflow

For example, with `k=8` on a 64-bit word, you can perform 8 parallel subtractions:
```haskell
kBitDiff 8 0x0807060504030201 0x0404030302020101
-- Result: 0x0403030202010100 (each byte subtracted independently)
```

#### Logarithm (`Log2.hs`)

Fast integer log2 using De Bruijn sequence multiplication:

```haskell
log2 :: Word64 -> Int
log2 v0 =
  let v1 = v0 .|. (v0 .>.  1) in
  let v2 = v1 .|. (v1 .>.  2) in
  -- ... smear bits right
  let v6 = v5 .|. (v5 .>. 32) in
  log2_64_tab !!! fromIntegral (((v6 - (v6 .>. 1)) * 0x07EDD5E59A4E28C2) .>. 58)
```

This O(1) algorithm:
1. Smears the highest set bit rightward to fill all lower positions
2. Extracts just the highest bit
3. Multiplies by a De Bruijn constant
4. Uses the high bits as an index into a lookup table

#### Bit String I/O (`BitRead.hs`, `BitShow.hs`, `BitParse.hs`)

Enables human-readable bit string representation:

```haskell
bitShow (8 :: Word64)
-- "00010000 00000000 00000000 00000000 00000000 00000000 00000000 00000000"

bitRead "10000000 101" :: Maybe [Word8]
-- Just [1, 5]
```

Note: The library uses **little-endian bit ordering** - the least significant bit comes first in the string representation.

#### Bit Writer (`Writer/Storable.hs`)

A mutable writer for constructing bit vectors efficiently:

```haskell
data Writer s = Writer
  { vector   :: DVSM.MVector s Word64
  , position :: ST.STRef s Int
  }

unsafeWriteBit :: Writer s -> Word64 -> ST s ()
unsafeWriteBits :: Writer s -> Int -> Word64 -> ST s ()
```

This allows building bit vectors by appending bits one at a time or in groups.

#### Word Operations (`Word.hs`, `Word64.hs`)

Word splitting and concatenation for working across word boundaries:

```haskell
class WordSplit a where
  type HalfWords a
  leSplit :: a -> (HalfWords a, HalfWords a)

class WordConcat a where
  type DoubleWords a
  leConcat :: a -> a -> DoubleWords a

lsb :: Word64 -> Word64  -- Position of least significant bit
```

## Haskell Language Features Leveraged

### Type Classes and Polymorphism

The library heavily uses type classes to provide uniform interfaces across different types:

```haskell
-- Multiple parameter type classes
class TestBit a where ...

-- Type families for associated types
class ElemFixedBitSize v where
  type Elem v
  elemFixedBitSize :: v -> Count

class WordSplit a where
  type HalfWords a
  leSplit :: a -> (HalfWords a, HalfWords a)
```

### GHC Extensions Used

From the source files:
- `FlexibleInstances` - Allows instances on concrete types
- `FlexibleContexts` - Enables flexible class constraints
- `MultiParamTypeClasses` - Multiple type parameters in classes
- `TypeFamilies` - Associated types
- `ScopedTypeVariables` - Type annotations in patterns
- `DeriveGeneric`, `DeriveAnyClass`, `DeriveFunctor` - Automatic derivation
- `GeneralizedNewtypeDeriving` - Derive instances for newtypes
- `TypeApplications` - Explicit type application
- `UndecidableInstances` - Relaxed instance checking

### Newtype Wrappers

The library uses newtypes for implementation selection:

```haskell
-- Select broadword (software) popcount
newtype Broadword a = Broadword a

-- Select builtin (hardware) popcount
newtype Builtin a = Builtin a

-- Wrapper for BitShow instances
newtype BitShown a = BitShown { unBitShown :: a }
```

### INLINE Pragmas

Extensive use of `{-# INLINE #-}` pragmas ensures that bit operations are optimized:

```haskell
instance BitWise Word64 where
  (.&.) = (B..&.)
  {-# INLINE (.&.) #-}
```

### ST Monad for Mutable State

The Writer module uses the ST monad for safe mutable operations:

```haskell
newWriter :: Int -> ST s (Writer s)
unsafeWriteBit :: Writer s -> Word64 -> ST s ()
written :: Writer s -> ST s (DVSM.MVector s Word64)
```

### Storable Vectors

For performance, the library primarily uses `Data.Vector.Storable` which provides:
- Contiguous memory layout
- Zero-cost interop with C
- Cache-friendly access patterns

## Computer Science Techniques Used

### 1. Broadword Computing (SWAR - SIMD Within A Register)

The broadword technique treats a single machine word as a vector of smaller sub-words, enabling parallel operations:

```haskell
kBitDiff :: Int -> Word64 -> Word64 -> Word64
kBitDiff k x y = ((x .|. h k) - (y .&. comp (h k))) .^. ((x .^. comp y) .&. h k)
```

This performs k parallel subtractions in a single word by:
1. Setting sentinel bits to prevent carry propagation between sub-words
2. Performing arithmetic on the protected word
3. XORing to fix up the result

**Application**: Essential for succinct data structures where you need to count bits within sub-word boundaries.

### 2. De Bruijn Sequences for Log2

A De Bruijn sequence B(2,n) is a cyclic sequence where every possible n-bit pattern appears exactly once as a substring.

```haskell
log2_64_tab :: DVS.Vector Int
-- Maps De Bruijn hash positions to log2 values

log2 :: Word64 -> Int
-- Uses 0x07EDD5E59A4E28C2 as the De Bruijn constant
```

The algorithm:
1. Isolates the highest set bit
2. Multiplies by the De Bruijn constant (each bit position produces a unique hash)
3. Uses the high 6 bits as a perfect hash into the lookup table

**Complexity**: O(1) time, O(64) space

### 3. Population Count Algorithms

**Broadword popcount** (also known as "sideways addition"):
```haskell
x1 = x0 - ((x0 .&. 0xaaaaaaaaaaaaaaaa) .>. 1)  -- Count in 2-bit groups
x2 = (x1 .&. 0x33...) + ((x1 .>. 2) .&. 0x33...) -- Merge to 4-bit groups
x3 = (x2 + (x2 .>. 4)) .&. 0x0f0f...             -- Merge to 8-bit groups
result = (x3 * 0x0101...) .>. 56                  -- Sum all bytes
```

The final multiplication with `0x0101010101010101` acts as a parallel sum by placing each byte's count in overlapping positions where they add up in the high byte.

### 4. Bit Manipulation Idioms

**Least Significant Bit**:
```haskell
lsb :: Word64 -> Word64
lsb w = let r = fromIntegral (DB.countTrailingZeros w)
        in r .|. negate ((r .>. 6) .&. 0x1)
```
Returns all-1s when input is 0, otherwise the position of the lowest set bit.

**Low Bits Mask**:
```haskell
loBitsSized :: Count -> Word64
loBitsSized n = let o = fromIntegral (64 - n) in 0xFFFFFFFFFFFFFFFF .<. o .>. o
```
Creates a mask with the N lowest bits set.

### 5. Parser Combinators

The BitParse module uses monadic parser combinators:

```haskell
instance BitParse Word8 where
  bitParse1 = do
    a <- bitParse1  -- Parse 8 bits
    b <- bitParse0
    ...
    return $ (if a then 0x01 else 0) .|. ...
```

### 6. Newtype-Based Polymorphism

Using newtypes to select algorithm implementations at the type level:

```haskell
-- User selects implementation via type
popCount1 (Broadword x)  -- Uses software algorithm
popCount1 (Builtin x)    -- Uses hardware POPCNT
```

This is a zero-cost abstraction as the newtype wrapper is erased at runtime.

## Rust Porting Considerations

### Feasibility: HIGH

The hw-bits library is well-suited for porting to Rust because:
1. Operations are primarily on primitive integer types
2. No complex monadic structures except for the Writer
3. Algorithms are portable across languages
4. Rust has excellent low-level bit manipulation support

### Rust Feature Mappings

| Haskell Concept                       | Rust Equivalent                   |
|---------------------------------------|-----------------------------------|
| Type classes (`BitWise`, `PopCount1`) | Traits                            |
| Associated types (`type Elem v`)      | Associated types in traits        |
| `Word8/16/32/64`                      | `u8/u16/u32/u64`                  |
| `DVS.Vector Word64`                   | `Vec<u64>` or `Box<[u64]>`        |
| `DVU.Vector Bit.Bit`                  | `bitvec` crate or custom `BitVec` |
| Newtype wrappers                      | `struct Broadword<T>(T)`          |
| INLINE pragmas                        | `#[inline]` attributes            |
| ST monad (Writer)                     | `&mut` references                 |

### Suggested Rust Architecture

```rust
// Core traits
pub trait BitWise {
    fn band(&self, other: &Self) -> Self;
    fn bor(&self, other: &Self) -> Self;
    fn bxor(&self, other: &Self) -> Self;
    fn complement(&self) -> Self;
    fn all_zeros() -> Self;
    fn all_ones() -> Self;
}

pub trait PopCount {
    fn pop_count_0(&self) -> u64;
    fn pop_count_1(&self) -> u64;
}

// Marker types for algorithm selection
pub struct Broadword<T>(pub T);
pub struct Builtin<T>(pub T);

impl PopCount for Broadword<u64> {
    fn pop_count_1(&self) -> u64 {
        let x0 = self.0;
        let x1 = x0 - ((x0 & 0xaaaaaaaaaaaaaaaa) >> 1);
        let x2 = (x1 & 0x3333333333333333) + ((x1 >> 2) & 0x3333333333333333);
        let x3 = (x2 + (x2 >> 4)) & 0x0f0f0f0f0f0f0f0f;
        (x3.wrapping_mul(0x0101010101010101)) >> 56
    }
}

impl PopCount for Builtin<u64> {
    fn pop_count_1(&self) -> u64 {
        self.0.count_ones() as u64  // Uses hardware POPCNT
    }
}
```

### Rust Advantages

1. **No-cost abstractions**: Same as Haskell's newtypes
2. **SIMD intrinsics**: Direct access via `std::arch`
3. **Const generics**: Could simplify some type-level computations
4. **No GC**: Predictable performance for critical paths
5. **Explicit mutability**: Writer pattern maps naturally to `&mut Vec<u64>`

### Potential Rust Crate Dependencies

- `bitvec`: Sophisticated bit vector with slice semantics
- `bitflags`: For bit flag manipulation
- `num-traits`: For generic numeric operations

### Implementation Strategy

1. **Phase 1**: Core traits and primitive implementations
   - `BitWise`, `PopCount`, `BitLength`, `Log2`
   - Implementations for `u8`, `u16`, `u32`, `u64`

2. **Phase 2**: Vector operations
   - Implementations for `&[u64]`, `Vec<u64>`
   - Consider integration with `bitvec`

3. **Phase 3**: Broadword module
   - Port the `h`, `l`, `kBitDiff*` functions
   - These are critical for succinct data structure operations

4. **Phase 4**: I/O utilities
   - `BitShow`, `BitRead` equivalents
   - Consider using `std::fmt::Display` and `std::str::FromStr`

5. **Phase 5**: Bit Writer
   - Straightforward port using `&mut` and `Vec<u64>`

### Challenges

1. **Type-level tricks**: Some Haskell patterns (like `Naive` wrapper for testing) may need different approaches
2. **Operator overloading**: Rust's trait system requires explicit implementation
3. **Vector types**: Need to decide between `Vec`, slices, and `bitvec`
4. **Performance parity**: Ensure Rust achieves same performance as GHC with `-O2 -msse4.2`

### SSE4.2 Considerations

The Haskell version has an SSE4.2 flag for optimizations. In Rust:
- Use `#[cfg(target_feature = "popcnt")]` for conditional compilation
- Consider explicit SIMD with `std::arch::x86_64::_mm_popcnt_u64`
- The `popcount` intrinsic is typically auto-vectorized by LLVM

## Summary

The hw-bits library provides a comprehensive, well-designed foundation for bit manipulation in Haskell. Its modular architecture, use of type classes, and focus on performance make it an excellent candidate for porting to Rust. The core algorithms (broadword computing, De Bruijn log2, sideways addition popcount) are language-agnostic and will translate directly to Rust's trait-based polymorphism.

Key value propositions for the Rust port:
- Zero-cost abstractions via traits and generics
- Direct access to SIMD intrinsics
- Predictable memory layout and performance
- Integration with Rust's ecosystem of data structure crates

The porting effort is estimated as **moderate** - the core logic is straightforward, but achieving feature parity across all vector types and maintaining performance requires careful attention.
