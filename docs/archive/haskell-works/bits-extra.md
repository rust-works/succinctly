# bits-extra

A Haskell library providing efficient bitwise operations, specifically the BMI2 instruction set operations PDEP (Parallel Deposit) and PEXT (Parallel Extract).

## Project Overview

**Repository**: https://github.com/haskell-works/bits-extra
**License**: BSD-3-Clause
**Author**: John Ky
**Version**: 0.0.2.3

### Purpose

The `bits-extra` library exposes support for BMI2 CPU instructions on x86-based CPUs, specifically:

- **PDEP** (Parallel Deposit): Deposits bits from a source at locations specified by a mask
- **PEXT** (Parallel Extract): Extracts bits from a source at locations specified by a mask

These operations are essential for high-performance bit manipulation in applications such as **succinct data structures**, which are a key component of the haskell-works ecosystem.

### Key Features

1. **Hardware Acceleration**: When compiled with the `bmi2` flag on GHC 8.4.1+, uses native CPU instructions
2. **Software Fallback**: Provides emulated implementations for CPUs without BMI2 support
3. **Multiple Word Sizes**: Supports Word8, Word16, Word32, Word64, and Word types
4. **Runtime Detection**: Exposes `fastPdepEnabled` and `fastPextEnabled` flags to check if hardware acceleration is active

## Code Structure

```
bits-extra/
├── src/
│   └── Data/
│       └── Bits/
│           ├── BitSize.hs       # Type class for getting bit sizes
│           ├── Pdep.hs          # Main PDEP interface (type class)
│           ├── Pdep/
│           │   ├── Prim.hs      # Primitive operations (hardware or fallback)
│           │   └── Slow.hs      # Emulated software implementation
│           ├── Pext.hs          # Main PEXT interface (type class)
│           └── Pext/
│               ├── Prim.hs      # Primitive operations (hardware or fallback)
│               └── Slow.hs      # Emulated software implementation
├── test/
│   ├── Spec.hs                  # HSpec test discovery
│   └── Data/Bits/
│       ├── PdepSpec.hs          # Property-based tests for PDEP
│       └── PextSpec.hs          # Property-based tests for PEXT
└── bench/
    └── Main.hs                  # Criterion benchmarks
```

## Key Modules

### Data.Bits.Pdep

Defines the `Pdep` type class with a single method:

```haskell
class Pdep a where
  pdep :: a -> a -> a
```

**Operation**: `pdep src mask` copies the lower-order bits from `src` to the 1-bit positions in `mask`. All 0-bit positions in the result are cleared.

**Example**:
```
src  = 0b00001111 (bits to deposit)
mask = 0b10101010 (where to deposit)
result = 0b10101010 (deposited at mask positions)
```

### Data.Bits.Pext

Defines the `Pext` type class with a single method:

```haskell
class Pext a where
  pext :: a -> a -> a
```

**Operation**: `pext src mask` extracts bits from `src` at positions where `mask` has 1-bits, then packs them into contiguous low-order bits.

**Example**:
```
src  = 0b10101010 (source bits)
mask = 0b11110000 (which bits to extract)
result = 0b00001010 (extracted bits, packed right)
```

### Data.Bits.BitSize

A utility type class for computing bit sizes of various types:

```haskell
class BitSize a where
  bitSize :: a -> Int
  bitCount :: a -> Word64
```

Instances cover primitive types (Word8-Word64, Int8-Int64) and containers (lists, vectors).

## Haskell Language Features

### Type Classes

The library uses Haskell type classes extensively for polymorphism:

```haskell
class Pdep a where
  pdep :: a -> a -> a

instance Pdep Word64 where
  pdep = P.primPdep64
  {-# INLINE pdep #-}
```

This enables the same `pdep` function to work with multiple word sizes.

### CPP (C Preprocessor)

Conditional compilation selects between hardware and software implementations:

```haskell
#if MIN_VERSION_base(4,11,0) && defined(BMI2_ENABLED)
primPdep64 (W64# src#) (W64# mask#) = W64# (pdep64# src# mask#)
#else
primPdep64 = slowPdep
#endif
```

### MagicHash and Unboxed Types

For accessing GHC primops (primitive operations):

```haskell
{-# LANGUAGE MagicHash #-}

import GHC.Prim
import GHC.Word

primPdep64 (W64# src#) (W64# mask#) = W64# (pdep64# src# mask#)
```

The `#` suffix indicates unboxed types, allowing direct access to CPU instructions.

### INLINE Pragmas

Extensive use of `{-# INLINE #-}` ensures these small functions are inlined for optimal performance:

```haskell
instance Pdep Word64 where
  pdep = P.primPdep64
  {-# INLINE pdep #-}
```

### Bang Patterns

Strict evaluation for performance:

```haskell
{-# LANGUAGE BangPatterns #-}

let !(W# src#) = fromIntegral src in ...
```

### Property-Based Testing

Uses Hedgehog for property-based testing, verifying that hardware and software implementations produce identical results:

```haskell
it "Word64" $ require $ property $ do
  a <- forAll $ G.word64 R.constantBounded
  b <- forAll $ G.word64 R.constantBounded
  pdep a b === slowPdep a b
```

## Computer Science Techniques

### BMI2 Instruction Set

The library wraps Intel's BMI2 (Bit Manipulation Instruction Set 2) instructions:

#### PDEP (Parallel Deposit)

**CPU Instruction**: `PDEP r64, r64, r/m64`

**Algorithm**: Takes bits from the source operand and deposits them at the positions indicated by the mask operand.

**Pseudocode**:
```
result = 0
src_bit_idx = 0
for i in 0..63:
    if mask[i] == 1:
        result[i] = src[src_bit_idx]
        src_bit_idx++
return result
```

**Software Implementation** (`slowPdep64'`):
```haskell
slowPdep64' :: Word64 -> Word64 -> Word64 -> Word64
slowPdep64' result src mask = if lowest /= 0
  then slowPdep64' newResult (src `shiftR` 1) (mask .&. complement lowest)
  else result
  where lowest    = (-mask) .&. mask  -- isolate lowest set bit
        newResult = result .|. (lsb src .&. lowest)
```

The trick `(-mask) .&. mask` isolates the lowest set bit in the mask using two's complement arithmetic.

#### PEXT (Parallel Extract)

**CPU Instruction**: `PEXT r64, r64, r/m64`

**Algorithm**: Extracts bits from the source at positions where the mask has 1-bits and packs them into contiguous low-order bits.

**Pseudocode**:
```
result = 0
result_bit_idx = 0
for i in 0..63:
    if mask[i] == 1:
        result[result_bit_idx] = src[i]
        result_bit_idx++
return result
```

**Software Implementation** (`slowPext64'`):
```haskell
slowPext64' :: Word64 -> Int -> Int -> Word64 -> Word64 -> Word64
slowPext64' result offset index src mask = if index /= 64
  then if maskBit /= 0
          then slowPext64' nextResult (offset + 1) (index + 1) src mask
          else slowPext64' result      offset      (index + 1) src mask
  else result
  where srcBit     = (src  `shiftR` index) .&. 1
        maskBit    = (mask `shiftR` index) .&. 1
        nextResult = result .|. (srcBit `shiftL` offset)
```

### Applications in Succinct Data Structures

PDEP and PEXT are fundamental operations for:

1. **Rank/Select Operations**: Efficiently counting and locating bits in bitvectors
2. **Wavelet Trees**: Navigating hierarchical bit structures
3. **FM-Index**: Text indexing and pattern matching
4. **Compressed Representations**: Encoding sparse data efficiently

### Performance Characteristics

From the README benchmarks on Intel Core i7:

| Operation                   | Hardware (BMI2) | Software (Emulated) | Speedup |
|-----------------------------|-----------------|---------------------|---------|
| pdep64                      | 4.9 ns          | 10.5 ns             | ~2x     |
| pext64                      | 5.2 ns          | 73 ns               | ~14x    |
| pext vector (1024 elements) | 1.6 us          | 72 us.              | ~45x    |

PEXT shows dramatically better performance with hardware support due to its more complex software emulation.

## Rust Porting Considerations

### Feasibility: HIGH

This is an excellent candidate for Rust porting due to:

1. **Pure bit manipulation**: No monadic state, IO, or complex Haskell features
2. **Direct hardware mapping**: BMI2 intrinsics have direct Rust equivalents
3. **Simple interface**: Just two main operations with clear semantics
4. **Well-tested**: Property tests define exact behavior

### Rust Intrinsics

Rust provides BMI2 intrinsics in `std::arch::x86_64`:

```rust
use std::arch::x86_64::{_pdep_u64, _pext_u64, _pdep_u32, _pext_u32};

// Hardware-accelerated PDEP
#[target_feature(enable = "bmi2")]
unsafe fn pdep64(src: u64, mask: u64) -> u64 {
    _pdep_u64(src, mask)
}

// Hardware-accelerated PEXT
#[target_feature(enable = "bmi2")]
unsafe fn pext64(src: u64, mask: u64) -> u64 {
    _pext_u64(src, mask)
}
```

### Suggested Rust Implementation

```rust
/// Parallel deposit bits from src into positions indicated by mask
pub fn pdep(src: u64, mask: u64) -> u64 {
    #[cfg(target_feature = "bmi2")]
    unsafe {
        std::arch::x86_64::_pdep_u64(src, mask)
    }

    #[cfg(not(target_feature = "bmi2"))]
    {
        pdep_slow(src, mask)
    }
}

/// Software fallback for PDEP
fn pdep_slow(mut src: u64, mut mask: u64) -> u64 {
    let mut result = 0u64;
    while mask != 0 {
        let lowest = mask & mask.wrapping_neg(); // isolate lowest bit
        if src & 1 != 0 {
            result |= lowest;
        }
        src >>= 1;
        mask &= !lowest;
    }
    result
}

/// Parallel extract bits from src at positions indicated by mask
pub fn pext(src: u64, mask: u64) -> u64 {
    #[cfg(target_feature = "bmi2")]
    unsafe {
        std::arch::x86_64::_pext_u64(src, mask)
    }

    #[cfg(not(target_feature = "bmi2"))]
    {
        pext_slow(src, mask)
    }
}

/// Software fallback for PEXT
fn pext_slow(src: u64, mask: u64) -> u64 {
    let mut result = 0u64;
    let mut offset = 0;
    for i in 0..64 {
        if (mask >> i) & 1 != 0 {
            if (src >> i) & 1 != 0 {
                result |= 1 << offset;
            }
            offset += 1;
        }
    }
    result
}
```

### Runtime Feature Detection

Rust supports runtime CPU feature detection:

```rust
pub fn pdep(src: u64, mask: u64) -> u64 {
    if is_x86_feature_detected!("bmi2") {
        unsafe { std::arch::x86_64::_pdep_u64(src, mask) }
    } else {
        pdep_slow(src, mask)
    }
}

/// Check if hardware PDEP is available
pub fn fast_pdep_enabled() -> bool {
    is_x86_feature_detected!("bmi2")
}
```

### Trait-Based API (Mirroring Haskell)

```rust
pub trait Pdep {
    fn pdep(self, mask: Self) -> Self;
}

pub trait Pext {
    fn pext(self, mask: Self) -> Self;
}

impl Pdep for u64 {
    fn pdep(self, mask: u64) -> u64 {
        pdep64(self, mask)
    }
}

impl Pext for u64 {
    fn pext(self, mask: u64) -> u64 {
        pext64(self, mask)
    }
}

// Implement for u8, u16, u32 via conversion to u64/u32
```

### Key Differences from Haskell

| Aspect                  | Haskell            | Rust                     |
|-------------------------|--------------------|--------------------------|
| Polymorphism            | Type classes       | Traits                   |
| Conditional compilation | CPP macros         | `cfg` attributes         |
| Unsafe code             | Hidden in GHC.Prim | Explicit `unsafe` blocks |
| Feature detection       | Compile-time flag  | Runtime or compile-time  |
| Inlining                | INLINE pragma      | `#[inline]` attribute    |

### Crate Structure

```
bits-extra/
├── Cargo.toml
├── src/
│   ├── lib.rs        # Re-exports
│   ├── pdep.rs       # PDEP trait and implementations
│   ├── pext.rs       # PEXT trait and implementations
│   └── bitsize.rs    # BitSize trait (optional)
├── benches/
│   └── benchmark.rs  # Criterion benchmarks
└── tests/
    └── properties.rs # Property tests with proptest
```

### Dependencies

```toml
[dependencies]

[dev-dependencies]
criterion = "0.5"
proptest = "1.0"

[features]
default = []
# Force software fallback for testing
no-bmi2 = []
```

### Testing Strategy

Use proptest for property-based testing:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn pdep_matches_slow(src: u64, mask: u64) {
        prop_assert_eq!(pdep(src, mask), pdep_slow(src, mask));
    }

    #[test]
    fn pext_matches_slow(src: u64, mask: u64) {
        prop_assert_eq!(pext(src, mask), pext_slow(src, mask));
    }
}
```

## Summary

The `bits-extra` library is a focused, well-designed Haskell package that:

1. Provides essential BMI2 bit manipulation operations (PDEP/PEXT)
2. Uses clean type class abstractions for polymorphic interfaces
3. Implements efficient software fallbacks when hardware support is unavailable
4. Is thoroughly tested with property-based testing

**Rust porting is highly feasible** and would benefit from:
- Direct access to BMI2 intrinsics via `std::arch`
- Runtime feature detection with `is_x86_feature_detected!`
- Zero-cost abstractions for trait-based polymorphism
- The same performance characteristics (or better) as the Haskell version

This library would be a natural foundation for porting the larger haskell-works succinct data structure ecosystem to Rust.
