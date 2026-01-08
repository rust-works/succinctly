# hw-simd Analysis

## Project Overview

**hw-simd** is a Haskell library that provides SIMD (Single Instruction, Multiple Data) operations for high-performance bit manipulation and byte comparison operations. The library is part of the haskell-works ecosystem and is designed to accelerate data processing tasks, particularly for succinct data structures and JSON parsing.

- **Version**: 0.1.2.2
- **Author**: John Ky (newhoggy@gmail.com)
- **License**: BSD-3-Clause
- **Repository**: https://github.com/haskell-works/hw-simd

### Purpose

The library provides two implementation strategies:
1. **AVX2 Implementation**: Uses Intel AVX2 SIMD instructions for high performance on x86_64 CPUs
2. **Stock Implementation**: Pure Haskell fallback using broadword programming techniques

The primary use cases are:
- Fast byte comparison (finding specific characters in large byte arrays)
- Bulk bitwise operations (AND, OR, XOR, NOT, ANDNOT)
- Support for succinct data structure operations in the hw-* ecosystem

### Platform Support

The library explicitly does **not** support ARM/AArch64 architectures (enforced via a `base < 0` constraint making it impossible to build on those platforms). It targets x86_64 with:
- SSE4.2 support (enabled by default)
- AVX2 support (optional flag)
- BMI2 support (optional flag)

## Code Structure

```
hw-simd/
├── cbits/                    # C source files with SIMD intrinsics
│   ├── simd.h               # Header with function declarations
│   ├── simd_avx2.c          # AVX2 implementations
│   ├── simd_sse2.c          # SSE2 implementations
│   ├── simd_debug.c         # Debug utilities
│   └── simd_debug.h         # Debug header
├── src/HaskellWorks/Data/Simd/
│   ├── Capabilities.hs      # CPU feature detection
│   ├── ChunkString.hs       # Chunked ByteString wrapper
│   ├── Comparison.hs        # Unified comparison API
│   ├── Comparison/
│   │   ├── Avx2.hs          # AVX2 byte comparison
│   │   └── Stock.hs         # Pure Haskell comparison
│   ├── Logical.hs           # Unified logical operations API
│   ├── Logical/
│   │   ├── Avx2.hs          # AVX2 logical operations
│   │   └── Stock.hs         # Pure Haskell logical operations
│   └── Internal/
│       ├── Bits.hs          # Bit manipulation utilities
│       ├── Broadword.hs     # Broadword programming helpers
│       ├── ChunkString.hs   # ChunkString internals
│       ├── Foreign.chs      # c2hs FFI bindings
│       └── Marshal.hs       # Memory marshaling utilities
├── test/                     # Test suite
└── bench/                    # Criterion benchmarks
```

## Key Modules

### Comparison Module (`Comparison.hs`, `Comparison/Avx2.hs`, `Comparison/Stock.hs`)

The core functionality is `cmpEqWord8s` which performs parallel byte comparison:

```haskell
class CmpEqWord8s a where
  cmpEqWord8s :: Word8 -> a -> a
```

Given a byte value and an input vector, it produces a bit vector where each bit indicates whether the corresponding input byte matches the target.

**AVX2 Implementation** (`Comparison/Avx2.hs`):
- Uses the C function `avx2_cmpeq8` via FFI
- Processes 32 bytes at a time using 256-bit AVX2 registers
- Uses `_mm256_cmpeq_epi8` to compare bytes in parallel
- Uses `_mm256_movemask_epi8` to extract comparison results into a 32-bit mask

**Stock Implementation** (`Comparison/Stock.hs`):
- Uses broadword programming techniques
- Processes 8 bytes (Word64) at a time
- Implements the `testWord8s` algorithm using XOR and bit manipulation

### Logical Operations Module (`Logical.hs`, `Logical/Avx2.hs`, `Logical/Stock.hs`)

Provides bulk bitwise operations on vectors:

```haskell
class XorBits a where xorBits :: a -> a -> a
class OrBits a where orBits :: a -> a -> a
class AndBits a where andBits :: a -> a -> a
class AndNotBits a where andNotBits :: a -> a -> a
class NotBits a where notBits :: a -> a
```

**AVX2 Implementation**:
- Uses `_mm256_xor_si256`, `_mm256_or_si256`, `_mm256_and_si256`, `_mm256_andnot_si256`
- Processes 32 bytes per iteration

**Stock Implementation**:
- Uses Haskell's `Bits` typeclass operations
- Processes Word64 at a time

### Internal Modules

**`Internal.Broadword`**: Implements broadword programming primitives:
- `fillWord64`: Broadcasts a byte to all 8 positions in a Word64 (multiply by `0x0101010101010101`)
- `toggle64`: Bit toggle operation using PDEP instruction

**`Internal.Bits`**: Provides:
- `testWord8s`: Tests if any byte in a Word64 is zero (used for equality comparison)
- `zipOr`, `zipAnd`: Vector-wise boolean operations

**`Internal.Foreign`**: c2hs-generated FFI bindings to the C SIMD functions.

**`Internal.ChunkString`**: A `ChunkString` type for working with chunked ByteStrings aligned to 32KB boundaries (minus 64 bytes for cache line alignment).

## C Implementation Details (cbits/)

### simd_avx2.c

The AVX2 C implementation provides:

1. **`avx2_cmpeq8`**: Byte comparison
   ```c
   void avx2_cmpeq8(uint8_t byte, uint8_t *target, size_t target_length, uint8_t *source) {
     __m256i v_comparand = _mm256_set1_epi8(byte);
     for (i = 0; i < target_length * 2; ++i) {
       __m256i v_data_a = *(__m256i *)(source + (i * 32));
       __m256i v_results_a = _mm256_cmpeq_epi8(v_data_a, v_comparand);
       uint32_t mask = (uint32_t)_mm256_movemask_epi8(v_results_a);
       target32[i] = mask;
     }
   }
   ```

2. **`avx2_cmpeq8_para`**: Parallel comparison against multiple target bytes (useful for finding multiple characters simultaneously)

3. **Logical operations**: `avx2_and_bits`, `avx2_or_bits`, `avx2_xor_bits`, `avx2_not_bits`, `avx2_and_not_bits`

### simd_sse2.c

Contains SSE2 fallback for `cmpeq8` using 128-bit registers:
```c
void sse_cmpeq8(uint8_t byte, uint64_t *target, size_t target_length, uint8_t *source) {
  __m128i v_comparand = _mm_set1_epi8(byte);
  // ... processes 16 bytes at a time
}
```

## Haskell Language Features Leveraged

### FFI (Foreign Function Interface)

The project uses c2hs for generating FFI bindings:

```haskell
-- Internal/Foreign.chs
{#call unsafe avx2_cmpeq8 as c_cmpeq8#} byte target targetLength source
```

Key FFI patterns:
- `Foreign.ForeignPtr`: Safe pointer management with automatic cleanup
- `Foreign.Marshal.Unsafe.unsafeLocalState`: Allows treating impure FFI calls as pure
- `DVS.unsafeFromForeignPtr`: Zero-copy vector construction from foreign memory

### Type Classes

Extensive use of type classes for polymorphism:
- `CmpEqWord8s`, `XorBits`, `OrBits`, `AndBits`, `NotBits`, `AndNotBits`
- `ToChunkString`, `ToLazyByteString`, `ToByteStrings`

### Type Families

Associated type families for flexible return types:
```haskell
class CmpEqWord8s a where
  type Target a
  cmpEqWord8s :: Word8 -> a -> Target a
```

### CPP (Conditional Compilation)

Used extensively for compile-time feature detection:
```haskell
{-# LANGUAGE CPP #-}
#if defined(AVX2_ENABLED)
avx2Enabled = True
#else
avx2Enabled = False
#endif
```

### INLINE Pragmas

Aggressive inlining for performance:
```haskell
{-# INLINE cmpEqWord8s #-}
{-# INLINE fillWord64 #-}
```

### GHC Extensions
- `FlexibleInstances`: For instances on specific type applications
- `ScopedTypeVariables`: For type annotations in expressions
- `MultiWayIf`: For cleaner conditional dispatch

## Computer Science Techniques

### SIMD (Single Instruction Multiple Data)

The core technique - processing multiple data elements in parallel:

1. **AVX2 (256-bit vectors)**:
   - Processes 32 bytes simultaneously
   - Uses intrinsics like `_mm256_cmpeq_epi8`, `_mm256_movemask_epi8`

2. **SSE2 (128-bit vectors)**:
   - Processes 16 bytes simultaneously
   - Uses `_mm_cmpeq_epi8`, `_mm_movemask_epi8`

### Broadword Programming

A technique for simulating SIMD in software using wide integer operations:

**Byte Broadcasting**:
```haskell
fillWord64 w = 0x0101010101010101 * fromIntegral w
```
This broadcasts a single byte to all 8 positions in a 64-bit word.

**Parallel Byte Testing** (`testWord8s`):
```haskell
testWord8s w = let w8s = w
                   w4s = w8s .|. (w8s .>. 4)
                   w2s = w4s .|. (w4s .>. 2)
                   w1s = w2s .|. (w2s .>. 1)
               in  pext w1s 0x0101010101010101
```
This collapses each byte to a single bit, testing if any bit in the byte is set.

### BMI2 Bit Manipulation Instructions

Uses `PEXT` (Parallel Extract) and `PDEP` (Parallel Deposit):
- `pext`: Extracts bits at positions specified by a mask
- `pdep`: Deposits bits at positions specified by a mask

These are used for efficient bit gathering/scattering operations in the broadword algorithms.

### Bit Vector Compression

The `cmpEqWord8s` operation compresses data by 8x:
- Input: N bytes of data
- Output: N/8 bytes (N bits) indicating matches

### Memory Alignment

The code is careful about memory alignment for SIMD operations:
- ChunkString uses 32KB - 64 byte chunks (cache-line aligned)
- AVX2 operations process 32-byte aligned blocks

## Rust Porting Considerations

### Feasibility

Porting hw-simd to Rust is **highly feasible** and would likely result in cleaner, safer code:

1. **Rust has first-class SIMD support** via `std::arch` and portable SIMD (nightly)
2. **No FFI overhead** - SIMD intrinsics compile directly to native instructions
3. **Strong type safety** for pointer operations without `unsafe` proliferation
4. **Feature flags** work similarly to Haskell's cabal flags

### Rust SIMD Intrinsics

Direct mappings for the C intrinsics used:

| C/Haskell Function        | Rust Equivalent           |
|---------------------------|---------------------------|
| `_mm256_set1_epi8(b)`     | `_mm256_set1_epi8(b)`     |
| `_mm256_cmpeq_epi8(a, b)` | `_mm256_cmpeq_epi8(a, b)` |
| `_mm256_movemask_epi8(a)` | `_mm256_movemask_epi8(a)` |
| `_mm256_and_si256(a, b)`  | `_mm256_and_si256(a, b)`  |
| `_mm256_or_si256(a, b)`   | `_mm256_or_si256(a, b)`   |
| `_mm256_xor_si256(a, b)`  | `_mm256_xor_si256(a, b)`  |

### Rust Implementation Strategy

```rust
use std::arch::x86_64::*;

#[cfg(target_feature = "avx2")]
pub fn cmpeq8(byte: u8, source: &[u8]) -> Vec<u8> {
    unsafe {
        let comparand = _mm256_set1_epi8(byte as i8);
        let mut result = Vec::with_capacity(source.len() / 8);

        for chunk in source.chunks_exact(32) {
            let data = _mm256_loadu_si256(chunk.as_ptr() as *const __m256i);
            let cmp_result = _mm256_cmpeq_epi8(data, comparand);
            let mask = _mm256_movemask_epi8(cmp_result) as u32;
            result.extend_from_slice(&mask.to_le_bytes());
        }
        result
    }
}
```

### Key Porting Considerations

1. **Target Feature Detection**:
   ```rust
   #[cfg(target_feature = "avx2")]
   fn avx2_impl() { ... }

   #[cfg(not(target_feature = "avx2"))]
   fn fallback_impl() { ... }
   ```

2. **Runtime Detection** (for dynamic dispatch):
   ```rust
   if is_x86_feature_detected!("avx2") {
       unsafe { avx2_impl(data) }
   } else {
       fallback_impl(data)
   }
   ```

3. **Portable SIMD** (nightly, but safer):
   ```rust
   #![feature(portable_simd)]
   use std::simd::*;

   fn cmpeq8(byte: u8, data: &[u8]) -> Vec<u8> {
       let pattern = u8x32::splat(byte);
       // ...
   }
   ```

4. **Memory Safety**: Rust's borrow checker prevents the aliasing issues that the Haskell code guards against with careful ForeignPtr management.

5. **Zero-Copy Operations**: Use `&[u8]` slices instead of ByteString, `Vec<u64>` instead of Storable vectors.

### Advantages of Rust Port

- **No FFI Boundary**: Direct SIMD intrinsic calls without C intermediary
- **Compile-Time Safety**: SIMD operations marked `unsafe` but contained
- **Better Tooling**: cargo features map well to cabal flags
- **ARM Support**: Could add NEON implementations for ARM64
- **Portable SIMD**: Future-proof with abstract SIMD types

### Challenges

- **Type Classes**: Would need traits with generic associated types (GATs) for equivalent polymorphism
- **Lazy Evaluation**: The ChunkString lazy streaming would need explicit iterators
- **Broadword Operations**: Direct port, but test carefully for bit ordering

## Dependencies

The library depends on several hw-* ecosystem packages:
- `hw-bits`: Bit manipulation primitives
- `hw-prim`: Primitive operations and ByteString utilities
- `hw-rankselect`: Rank and select operations for succinct structures
- `hw-rankselect-base`: Base types for rank/select
- `bits-extra`: Additional bit manipulation (PEXT, PDEP)

## Summary

hw-simd is a well-designed SIMD library that provides:
- High-performance byte comparison using AVX2/SSE2
- Bulk bitwise operations on vectors
- Clean fallback to broadword techniques when SIMD is unavailable

The library is an excellent candidate for Rust porting due to:
- Direct availability of equivalent SIMD intrinsics in `std::arch`
- Cleaner memory management without FFI
- Potential for portable SIMD abstractions
- Strong ecosystem support for SIMD in Rust

A Rust port would eliminate the C dependency, potentially add ARM support, and benefit from Rust's safety guarantees around pointer operations.
