# hw-prim - Primitive Functions and Data Types

## Project Overview

**hw-prim** is a foundational Haskell library that provides primitive functions and data types for building high-performance succinct data structures. It serves as the foundation layer for the haskell-works ecosystem, providing unified abstractions over various container types (ByteString, Vector, lists) with a focus on bit-level and word-level operations.

- **Version**: 0.6.3.2
- **License**: BSD-3-Clause
- **Author**: John Ky
- **Repository**: https://github.com/haskell-works/hw-prim
- **GHC Compatibility**: 8.8.4 through 9.12.1

### Purpose

The library addresses several key needs in succinct data structure implementations:

1. **Unified Container Abstractions**: Provides type classes that abstract over lists, ByteStrings, boxed vectors, and storable vectors
2. **High-Performance Primitives**: Offers branchless comparison operations and memory-mapped file access
3. **Zero-Copy Conversions**: Enables efficient reinterpretation of byte data as different word sizes
4. **Lazy I/O Support**: Provides interleaved monadic operations for streaming large files
5. **Tree Cursor Navigation**: Defines interfaces for tree-structured data navigation

---

## Code Structure and Key Modules

### Source Directory Layout

```
src/HaskellWorks/
├── Control/
│   └── Monad/
│       └── Lazy.hs              # Lazy/interleaved monadic operations
├── Data/
│   ├── AtIndex.hs               # Position-based indexing
│   ├── Branchless.hs            # Branchless comparison primitives
│   ├── ByteString.hs            # ByteString utilities (mmap, chunking, padding)
│   ├── ByteString/
│   │   ├── Builder.hs           # ByteString builder utilities
│   │   └── Lazy.hs              # Lazy ByteString utilities
│   ├── Char.hs                  # Character utilities
│   ├── Char/IsChar.hs           # Character type class
│   ├── Concat.hs                # Concatenation type class
│   ├── Cons.hs                  # List-like cons operations
│   ├── Container.hs             # Container type family (Elem association)
│   ├── Decode.hs                # Decoding type class
│   ├── Drop.hs                  # Drop elements from containers
│   ├── Empty.hs                 # Empty container creation
│   ├── Filter.hs                # Filtering operations
│   ├── Foldable.hs              # Extended foldable utilities
│   ├── FromByteString.hs        # ByteString conversion
│   ├── FromForeignRegion.hs     # Memory-mapped region conversion
│   ├── FromString.hs            # String conversion
│   ├── Generate.hs              # Vector generation
│   ├── Head.hs                  # Head element access
│   ├── Length.hs                # Length and end position
│   ├── Naive.hs                 # Naive implementation wrapper
│   ├── Null.hs                  # Null checking
│   ├── Ops.hs                   # Binary operations
│   ├── Positioning.hs           # Position and Count type aliases
│   ├── Product.hs               # Product type (:*:)
│   ├── Search.hs                # Binary search
│   ├── Sign.hs                  # Signed type conversion
│   ├── Snoc.hs                  # Append to end operations
│   ├── Take.hs                  # Take elements from containers
│   ├── TreeCursor.hs            # Tree navigation interface
│   ├── Uncons.hs                # Deconstruct from head
│   ├── Unsign.hs                # Unsigned type conversion
│   ├── Unsnoc.hs                # Deconstruct from tail
│   ├── Word.hs                  # Word-level operations
│   └── Vector/
│       ├── AsVector8.hs         # Convert to Vector Word8
│       ├── AsVector8s.hs        # Convert to [Vector Word8]
│       ├── AsVector8ns.hs       # Convert to [Vector Word8] (normalized)
│       ├── AsVector64.hs        # Convert to Vector Word64
│       ├── AsVector64s.hs       # Convert to [Vector Word64]
│       ├── AsVector64ns.hs      # Convert to [Vector Word64] (normalized)
│       ├── BoxedVectorLike.hs   # Boxed vector operations
│       ├── Storable.hs          # Storable vector utilities
│       └── StorableVectorLike.hs # Storable vector operations
└── Foreign.hs                   # Foreign pointer utilities (aligned allocation)
```

### Key Modules Analysis

#### 1. Container and Element Type Association (`Container.hs`)

```haskell
class Container a where
  type Elem a

instance Container BS.ByteString where
  type Elem BS.ByteString = Word8

instance Container (DVS.Vector Word64) where
  type Elem (DVS.Vector Word64) = Word64
```

This uses **associated type families** to define the element type for each container, enabling generic algorithms that work across different container types.

#### 2. Positioning Types (`Positioning.hs`)

```haskell
type Count = Word64    -- Unsigned count
type Position = Int64  -- Signed position (can be negative for bounds checking)
```

These type aliases provide semantic clarity and consistent sizing across the library.

#### 3. Indexed Access (`AtIndex.hs`)

```haskell
class Length v => AtIndex v where
  (!!!) :: v -> Position -> Elem v
  atIndex :: v -> Position -> Elem v
```

Features conditional bounds checking via CPP:
- Default: Uses `unsafeIndex` for maximum performance
- With `-DBOUNDS_CHECKING_ENABLED`: Uses safe indexing

Also provides:
- `atIndexOr`: Safe access with default value
- `atIndexOrBeforeOrAfter`: Clamped access with boundary defaults
- `atIndexOrBeforeOrLast`: Access clamped to last element

#### 4. Branchless Comparisons (`Branchless.hs`)

```haskell
ltWord64 :: Word64 -> Word64 -> Word64
ltWord64 (W64# a#) (W64# b#) = fromIntegral (I# (ltWord64# a# b#))
```

Uses GHC primitives (`GHC.Prim`) to perform comparisons that return 0 or 1 without branching. This is critical for SIMD-friendly code and cache-efficient algorithms in succinct data structures.

Operations provided:
- `ltWord{8,16,32,64}` - Less than
- `leWord{8,16,32,64}` - Less than or equal
- `gtWord{8,16,32,64}` - Greater than
- `geWord{8,16,32,64}` - Greater than or equal

#### 5. Memory Mapping (`FromForeignRegion.hs`, `ByteString.hs`)

```haskell
type ForeignRegion = (ForeignPtr Word8, Int, Int)

class FromForeignRegion a where
  fromForeignRegion :: ForeignRegion -> a

mmapFromForeignRegion :: FromForeignRegion a => FilePath -> IO a
```

Enables zero-copy memory mapping of files directly into various vector types.

#### 6. Vector Conversion (`AsVector64.hs`, `AsVector8.hs`)

```haskell
class AsVector64 a where
  asVector64 :: a -> DVS.Vector Word64

instance AsVector64 BS.ByteString where
  asVector64 bs = ... -- Zero-copy with padding if needed
```

Provides zero-copy reinterpretation of byte data as 64-bit words, with automatic padding for alignment.

#### 7. Chunked Processing (`AsVector64s.hs`, `ByteString.hs`)

```haskell
class AsVector64s a where
  asVector64s :: Int -> a -> [DVS.Vector Word64]
```

Functions like `rechunk`, `resegment`, and `resegmentPadded` handle:
- Splitting data into fixed-size chunks
- Proper alignment for word-level processing
- Padding final chunks to maintain alignment

#### 8. Tree Cursor Interface (`TreeCursor.hs`)

```haskell
class TreeCursor k where
  firstChild  :: k -> Maybe k
  nextSibling :: k -> Maybe k
  parent      :: k -> Maybe k
  depth       :: k -> Maybe Count
  subtreeSize :: k -> Maybe Count
```

Defines the navigation interface for tree-structured data (used by higher-level succinct tree implementations).

#### 9. Lazy Monadic Operations (`Control/Monad/Lazy.hs`)

```haskell
interleaveSequenceIO :: [IO a] -> IO [a]
interleaveUnfoldrM :: MonadUnliftIO m => (b -> m (Maybe (a, b))) -> b -> m [a]
interleaveTraverseM :: MonadUnliftIO m => (a -> m b) -> [a] -> m [b]
```

Uses `unsafeInterleaveIO` to enable lazy streaming of I/O operations - crucial for processing files larger than memory.

#### 10. Stateful Vector Construction (`Vector/Storable.hs`)

```haskell
constructSI :: Storable a => Int -> (Int -> s -> (s, a)) -> s -> (s, DVS.Vector a)
construct2N :: ... -> (DVS.Vector b, DVS.Vector c)
construct64UnzipN :: Int -> [(BS.ByteString, BS.ByteString)] -> (DVS.Vector Word64, DVS.Vector Word64)
```

Provides efficient in-place construction of vectors with state threading, avoiding intermediate allocations.

---

## Haskell Language Features Leveraged

### 1. Type Classes with Associated Type Families

```haskell
class Container a where
  type Elem a
```

Used extensively to associate element types with container types, enabling generic programming over heterogeneous container types.

### 2. Multi-Parameter Type Classes

```haskell
class Container v => Cons v where
  cons :: Elem v -> v -> v
```

Combined with `FlexibleInstances` to create expressive type class hierarchies.

### 3. CPP Preprocessor Directives

```haskell
#if MIN_VERSION_base(4,16,0)
ltWord8 (W8# a#) (W8# b#) = fromIntegral (I# (ltWord8# a# b#))
#else
ltWord8 (W8# a#) (W8# b#) = fromIntegral (I8# (ltWord# a# b#))
#endif
```

Heavy use of CPP for:
- GHC version compatibility
- Optional bounds checking (`BOUNDS_CHECKING_ENABLED`)

### 4. GHC Primitives (MagicHash)

```haskell
{-# LANGUAGE MagicHash #-}
import GHC.Prim
import GHC.Word (Word64 (..))

ltWord64 (W64# a#) (W64# b#) = fromIntegral (I# (ltWord64# a# b#))
```

Direct access to GHC's primitive operations for maximum performance.

### 5. Strict Evaluation Controls

```haskell
{-# LANGUAGE BangPatterns #-}

mmap filepath = do
  (fptr :: ForeignPtr Word8, offset, size) <- IO.mmapFileForeignPtr filepath IO.ReadOnly Nothing
  let !bs = BSI.fromForeignPtr (castForeignPtr fptr) offset size
  return bs
```

Bang patterns and `NFData` constraints ensure proper evaluation.

### 6. Rank-N Types

```haskell
construct2N :: (Storable b, Storable c)
  => Int
  -> (forall s. a -> DVSM.MVector s b -> ST s Int)
  -> ...
```

Used for ST monad actions that work with mutable vectors.

### 7. Type Operators

```haskell
{-# LANGUAGE TypeOperators #-}

infixr :*:
data (:*:) a b = (:*:) a b
```

Custom product type for paired data.

### 8. INLINE Pragmas

Nearly every function is marked `{-# INLINE #-}` to ensure optimal code generation through specialization.

### 9. MonadUnliftIO for Safe Lazy I/O

```haskell
interleaveUnfoldrM :: MonadUnliftIO m => (b -> m (Maybe (a, b))) -> b -> m [a]
```

Uses `unliftio-core` for safe interleaving of effects.

---

## Computer Science Techniques Used

### 1. Branchless Programming

The `Branchless` module implements comparison operations that avoid CPU branch misprediction penalties:

```haskell
ltWord64 :: Word64 -> Word64 -> Word64
-- Returns 1 if a < b, else 0, without branching
```

This technique is essential for:
- SIMD vectorization
- Predictable performance in tight loops
- Succinct data structure rank/select operations

### 2. Memory Mapping (mmap)

```haskell
mmap :: FilePath -> IO BS.ByteString
mmap filepath = do
  (fptr, offset, size) <- IO.mmapFileForeignPtr filepath IO.ReadOnly Nothing
  return $ BSI.fromForeignPtr (castForeignPtr fptr) offset size
```

Benefits:
- Zero-copy file access
- Lazy loading (pages loaded on demand by OS)
- Shared memory between processes

### 3. Zero-Copy Type Punning

```haskell
asVector64 :: BS.ByteString -> DVS.Vector Word64
asVector64 bs = case BSI.toForeignPtr bs of
  (fptr, start, offset) -> DVS.unsafeCast (DVS.unsafeFromForeignPtr fptr start offset)
```

Reinterprets byte arrays as arrays of larger words without copying, using:
- `unsafeCast` for vector type conversion
- Direct foreign pointer manipulation
- Proper alignment handling

### 4. Lazy Streaming with Effects

```haskell
interleaveSequenceIO :: [IO a] -> IO [a]
interleaveSequenceIO (fa:fas) = do
  a <- fa
  as <- IO.unsafeInterleaveIO (interleaveSequenceIO fas)
  return (a:as)
```

Enables processing of infinite or very large sequences in constant memory.

### 5. Binary Search

```haskell
binarySearch :: (Ord a, Integral n) => a -> (n -> a) -> n -> n -> n
binarySearch w f p q = if p + 1 >= q
  then p
  else let m = (p + q) `div` 2 in
    if w <= f m
      then binarySearch w f p m
      else binarySearch w f m q
```

Used for rank queries on bit vectors in higher-level libraries.

### 6. Chunked/Segmented Processing

```haskell
resegment :: Int -> [BS.ByteString] -> [BS.ByteString]
rechunkPadded :: Int -> [BS.ByteString] -> [BS.ByteString]
```

Enables:
- Cache-friendly processing of large datasets
- Word-aligned operations for bit manipulation
- Parallel processing opportunities

### 7. Succinct Data Structure Primitives

The library provides the foundation for succinct data structures:
- **Bit vectors**: Word-aligned storage and access
- **Tree cursors**: Navigation without explicit pointers
- **Rank/Select primitives**: Through branchless comparisons

### 8. Aligned Memory Allocation

```haskell
mallocForeignPtrBytesWithAlignedPtr :: Storable a => Int -> Int -> IO (ForeignPtr a, Ptr a)
mallocForeignPtrBytesWithAlignedPtr alignment n = do
  fptr <- F.mallocForeignPtrBytes (n + alignment)
  let alignedPtr = alignPtr (F.unsafeForeignPtrToPtr fptr) alignment
  return (fptr, alignedPtr)
```

Ensures SIMD-compatible memory alignment.

---

## Rust Porting Considerations

### Feasibility Assessment

**Overall: HIGH FEASIBILITY**

The library is fundamentally about:
1. Container abstractions (traits in Rust)
2. Low-level memory operations (Rust excels here)
3. Performance optimizations (Rust's forte)

### Rust Feature Mapping

#### 1. Type Classes to Traits

Haskell:
```haskell
class Container a where
  type Elem a

class Container v => Length v where
  length :: v -> Count
```

Rust:
```rust
trait Container {
    type Elem;
}

trait Length: Container {
    fn length(&self) -> u64;

    fn end(&self) -> i64 {
        self.length() as i64
    }
}
```

#### 2. Associated Type Families

Haskell:
```haskell
class Container a where
  type Elem a
```

Rust:
```rust
trait Container {
    type Elem;
}

impl Container for Vec<u8> {
    type Elem = u8;
}
```

#### 3. Branchless Operations

Haskell:
```haskell
ltWord64 :: Word64 -> Word64 -> Word64
```

Rust:
```rust
#[inline]
pub fn lt_word64(a: u64, b: u64) -> u64 {
    (a < b) as u64
}
// Or for guaranteed branchless:
pub fn lt_word64_branchless(a: u64, b: u64) -> u64 {
    ((a.wrapping_sub(b)) >> 63) & 1
}
```

#### 4. Memory Mapping

Haskell uses `mmap` package:
```haskell
mmap :: FilePath -> IO BS.ByteString
```

Rust using `memmap2`:
```rust
use memmap2::Mmap;

pub fn mmap_file(path: &Path) -> io::Result<Mmap> {
    let file = File::open(path)?;
    unsafe { Mmap::map(&file) }
}
```

#### 5. Zero-Copy Conversions

Haskell:
```haskell
asVector64 :: BS.ByteString -> DVS.Vector Word64
```

Rust:
```rust
use bytemuck::{cast_slice, Pod};

pub fn as_u64_slice(bytes: &[u8]) -> &[u64] {
    // Requires proper alignment
    bytemuck::cast_slice(bytes)
}

// Or safer with alignment check:
pub fn as_u64_vec(bytes: &[u8]) -> Vec<u64> {
    bytes.chunks(8)
        .map(|chunk| u64::from_le_bytes(chunk.try_into().unwrap_or([0; 8])))
        .collect()
}
```

#### 6. Lazy Streaming

Haskell:
```haskell
interleaveSequenceIO :: [IO a] -> IO [a]
```

Rust iterators provide lazy evaluation naturally:
```rust
pub fn lazy_sequence<I, T, F>(items: I, f: F) -> impl Iterator<Item = T>
where
    I: Iterator,
    F: Fn(I::Item) -> T,
{
    items.map(f)
}
```

For async:
```rust
use futures::stream::{self, StreamExt};

pub fn async_sequence<T>(items: Vec<impl Future<Output = T>>) -> impl Stream<Item = T> {
    stream::iter(items).then(|f| f)
}
```

#### 7. Chunked Processing

Haskell:
```haskell
rechunk :: Int -> [BS.ByteString] -> [BS.ByteString]
```

Rust:
```rust
pub fn rechunk(size: usize, chunks: impl Iterator<Item = Vec<u8>>) -> impl Iterator<Item = Vec<u8>> {
    // Use itertools or custom implementation
    chunks.flat_map(|c| c.into_iter())
        .collect::<Vec<_>>()
        .chunks(size)
        .map(|c| c.to_vec())
        .collect::<Vec<_>>()
        .into_iter()
}
```

#### 8. Conditional Bounds Checking

Haskell CPP:
```haskell
#if !defined(BOUNDS_CHECKING_ENABLED)
  (!!!) v i = DVS.unsafeIndex v (fromIntegral i)
#else
  (!!!) v i = v DVS.! fromIntegral i
#endif
```

Rust features:
```rust
#[cfg(feature = "unsafe-indexing")]
pub fn at_index<T>(v: &[T], i: usize) -> &T {
    unsafe { v.get_unchecked(i) }
}

#[cfg(not(feature = "unsafe-indexing"))]
pub fn at_index<T>(v: &[T], i: usize) -> &T {
    &v[i]
}
```

### Suggested Rust Crate Structure

```
hw-prim/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── container.rs      # Container trait + Elem associated type
│   ├── positioning.rs    # Count, Position types
│   ├── at_index.rs       # AtIndex trait
│   ├── branchless.rs     # Branchless comparisons
│   ├── length.rs         # Length trait
│   ├── take_drop.rs      # Take, Drop traits
│   ├── cons_snoc.rs      # Cons, Snoc, Uncons, Unsnoc
│   ├── mmap.rs           # Memory mapping utilities
│   ├── vector/
│   │   ├── mod.rs
│   │   ├── as_u64.rs     # AsVector64 equivalent
│   │   ├── as_u8.rs      # AsVector8 equivalent
│   │   └── storable.rs   # Vector construction utilities
│   ├── bytestring.rs     # Chunking, padding utilities
│   ├── tree_cursor.rs    # TreeCursor trait
│   ├── search.rs         # Binary search
│   └── foreign.rs        # Aligned allocation
```

### Key Advantages in Rust

1. **Zero-cost abstractions**: Traits with static dispatch
2. **Fearless concurrency**: Safe parallel processing of chunks
3. **SIMD intrinsics**: `std::simd` (nightly) or `packed_simd` crate
4. **No GC pauses**: Deterministic memory management
5. **Better unsafe ergonomics**: Clear unsafe boundaries
6. **Cargo features**: Cleaner conditional compilation than CPP

### Challenges in Rust

1. **No lazy evaluation by default**: Need explicit iterators/streams
2. **No HKT**: Some abstractions require workarounds
3. **Orphan rules**: May need newtype wrappers for external type implementations
4. **Associated type limitations**: Less flexible than Haskell type families

### Recommended Rust Dependencies

```toml
[dependencies]
memmap2 = "0.9"           # Memory mapping
bytemuck = "1.14"         # Safe type punning
bytes = "1.5"             # ByteString equivalent
thiserror = "1.0"         # Error handling

[dev-dependencies]
proptest = "1.4"          # Property-based testing (like Hedgehog)
criterion = "0.5"         # Benchmarking
```

---

## Summary

hw-prim is a carefully designed primitive library that provides:

1. **Unified abstractions** over diverse container types
2. **High-performance primitives** using GHC's low-level facilities
3. **Memory-efficient operations** through zero-copy and mmap
4. **Streaming support** for processing large datasets

The library is highly portable to Rust, with most concepts mapping directly to Rust traits and types. The main architectural decisions (type-class hierarchy, branchless operations, memory mapping) translate well to Rust idioms, often with performance improvements due to Rust's zero-cost abstractions and lack of garbage collection overhead.

For a Rust port, focus on:
1. Defining the trait hierarchy (Container -> Length -> AtIndex)
2. Implementing branchless primitives using Rust's integer operations
3. Using `memmap2` for memory mapping
4. Leveraging `bytemuck` for safe type punning
5. Using Rust iterators for lazy/streaming operations
