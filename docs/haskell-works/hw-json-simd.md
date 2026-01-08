# hw-json-simd Analysis

## Project Overview

**hw-json-simd** is a Haskell library and command-line tool for generating semi-indexes on JSON files using SIMD (Single Instruction, Multiple Data) CPU instructions. The project implements the algorithms described in the academic paper ["Semi-Indexing Semi-Structured Data in Tiny Space"](http://www.di.unipi.it/~ottavian/files/semi_index_cikm.pdf) by Giuseppe Ottaviano and Roberto Grossi.

### Purpose

The library creates two semi-index files for any given JSON document:
- **IB (Interesting Bits) index**: A bitvector marking positions of structural JSON characters (delimiters, brackets, braces)
- **BP (Balanced Parentheses) index**: A bitvector encoding the nesting structure of the JSON document

These indexes enable efficient navigation and querying of large JSON files without fully parsing them, using succinct data structures that consume very little additional space relative to the original document.

### Version and License

- **Version**: 0.1.1.2
- **License**: BSD-3-Clause
- **Author**: John Ky (newhoggy@gmail.com)
- **Repository**: https://github.com/haskell-works/hw-json-simd

## Code Structure

### Directory Layout

```
hw-json-simd/
├── src/
│   └── HaskellWorks/Data/Json/Simd/
│       ├── Capabilities.hs              # CPU capability detection
│       ├── Index/
│       │   ├── Simple.hs                # Simple indexing API (Section 5 of paper)
│       │   └── Standard.hs              # Standard indexing API (Section 4 of paper)
│       └── Internal/
│           ├── Foreign.chs              # FFI bindings (c2hs)
│           ├── List.hs                  # Utility functions
│           └── Index/
│               ├── Simple.hs            # Simple indexing internals
│               └── Standard.hs          # Standard indexing internals
├── cbits/
│   ├── simd.h                           # C header with function declarations
│   ├── simd.c                           # Capability detection functions
│   ├── simd-spliced.c                   # Simple indexing SIMD implementation
│   ├── simd-state.c                     # Standard indexing state machine
│   ├── simd-phi-table-32.c              # Output function lookup table
│   ├── simd-transition-table-32.c       # State transition lookup table
│   ├── intrinsics.h                     # SIMD intrinsics includes
│   ├── debug.h                          # Debug printing utilities
│   └── main.c                           # Standalone C executable
├── app/
│   ├── Main.hs                          # CLI entry point
│   └── App/
│       ├── Commands.hs                  # Command routing
│       ├── Commands/
│       │   ├── CreateIndex.hs           # Index creation command
│       │   ├── Capabilities.hs          # CPU capabilities command
│       │   └── Types.hs                 # Option types
│       └── Lens.hs                      # Lens definitions
└── test/
    └── Spec.hs                          # Test suite (placeholder)
```

### Key Modules

#### 1. HaskellWorks.Data.Json.Simd.Capabilities

Exposes runtime CPU capability detection:
- `avx_2 :: Bool` - AVX2 instruction set support
- `sse_4_2 :: Bool` - SSE 4.2 instruction set support
- `bmi_2 :: Bool` - BMI2 instruction set support

All three capabilities are required for the indexing functions to be enabled.

#### 2. HaskellWorks.Data.Json.Simd.Index.Simple

Implements the "simple" indexing method from Section 5 of the semi-indexing paper:

```haskell
makeSimpleJsonIbBps :: LBS.ByteString -> Either String [(BS.ByteString, BS.ByteString)]
makeSimpleJsonIbBpsUnsafe :: LBS.ByteString -> [(BS.ByteString, BS.ByteString)]
enabledMakeSimpleJsonIbBps :: Bool
```

This method uses a splicing technique that:
1. Summarizes input into character class bitmasks using SIMD
2. Handles escape sequences and quote tracking
3. Generates interesting-bits (IB) and balanced-parentheses (BP) indexes

#### 3. HaskellWorks.Data.Json.Simd.Index.Standard

Implements the "standard" indexing method from Section 4 of the paper:

```haskell
makeStandardJsonIbBps :: LBS.ByteString -> Either String [(BS.ByteString, BS.ByteString)]
makeStandardJsonIbBpsUnsafe :: LBS.ByteString -> [(BS.ByteString, BS.ByteString)]
enabledMakeStandardJsonIbBps :: Bool
```

This method uses a state-machine approach with:
- Precomputed transition tables
- Precomputed output (phi) tables
- SIMD-accelerated parallel state lookups

#### 4. HaskellWorks.Data.Json.Simd.Internal.Foreign

C2HS bindings file that provides the Haskell-C interface:
- Type definitions: `UInt8`, `UInt32`, `UInt64`, `Size`
- Capability functions: `enabled_avx_2`, `enabled_sse_4_2`, `enabled_bmi_2`
- Simple method functions: `processChunk`, `initBpState`, `writeBpChunk`, `writeBpChunkFinal`
- Standard method functions: `smProcessChunk`, `smMakeIbOpClChunks`, `smWriteBpChunk`, `smWriteBpChunkFinal`

## Haskell Language Features

### FFI (Foreign Function Interface)

The project makes extensive use of Haskell's FFI capabilities:

1. **c2hs Preprocessor**: The `.chs` file format is used for automatic generation of FFI bindings:
   ```haskell
   {#call unsafe hw_json_simd_process_chunk as c_hw_json_simd_process_chunk#}
   ```

2. **Foreign Pointer Management**: Uses `ForeignPtr` for memory safety:
   ```haskell
   fptr <- F.mallocForeignPtrBytes (6 * n)
   ```

3. **Low-level Memory Operations**: Direct pointer arithmetic via `Foreign.Ptr`:
   ```haskell
   workBuffersA = ptr `F.plusPtr` n
   ```

### Unsafe Operations

The code uses several unsafe operations for performance:

1. **unsafePerformIO**: For top-level capability detection
   ```haskell
   avx_2 = U.unsafePerformIO F.enabled_avx_2 /= 0
   ```

2. **unsafeInterleaveIO**: For lazy streaming of index chunks
   ```haskell
   IO.unsafeInterleaveIO $ go wb ws (LBS.toChunks lbs)
   ```

3. **unsafeLocalState**: For pure-looking functions that perform IO
   ```haskell
   makeIbs lbs = F.unsafeLocalState $ do
   ```

4. **unsafeIOToST**: For embedding IO in ST monad computations
   ```haskell
   fmap fromIntegral . ST.unsafeIOToST $ do
   ```

### Advanced Type System Features

1. **GADTs (Generalized Algebraic Data Types)**:
   ```haskell
   data Step where
     Step :: (forall s. BpState -> DVSM.MVector s Word64 -> ST s Int)
          -> Int
          -> Step
   ```

2. **RankNTypes**: Higher-rank polymorphism for ST monad encapsulation

3. **ScopedTypeVariables**: For explicit type annotations in local bindings

4. **Template Haskell**: For automatic lens generation via `makeFields`

### Lazy ByteString Processing

The library processes input as lazy ByteStrings chunked for streaming:
```haskell
contents <- LBS.resegmentPadded 512 <$> LBS.hGetContents hIn
```

## Computer Science Techniques

### 1. Semi-Indexing (Succinct Data Structures)

The fundamental technique from the Ottaviano-Grossi paper. Semi-indexes provide:

- **Space Efficiency**: O(n) bits additional space for an n-byte document
- **Query Support**: Constant-time rank and select operations
- **Lazy Parsing**: Navigate JSON structure without fully parsing

The semi-index consists of:

1. **Interesting-Bits (IB)**: A bitvector where bit i is set iff position i contains a structural character (`:`, `,`, `[`, `]`, `{`, `}`)

2. **Balanced Parentheses (BP)**: A bitvector encoding the nesting structure:
   - Each `{` or `[` is encoded as `1` (open parenthesis)
   - Each `}` or `]` is encoded as `0` (close parenthesis)
   - Delimiters `:` and `,` are encoded as `10` (matched pair)

### 2. SIMD Character Classification

The `hw_json_simd_summarise` function uses SIMD to classify 32 bytes at once:

**AVX2 Implementation** (256-bit vectors):
```c
__m256i v_in_data = *(__m256i *)buffer;
__m256i v_bytes_of_comma = _mm256_cmpeq_epi8(v_in_data, _mm256_set1_epi8(','));
// ... similar for other characters
uint32_t mask_comma = (uint32_t)_mm256_movemask_epi8(v_bytes_of_comma);
```

**SSE 4.2 Implementation** (128-bit vectors):
```c
_mm_cmpestrm(*(__m128i*)":,", 2, v_in_data_0, 16, _SIDD_CMP_EQUAL_ANY | _SIDD_BIT_MASK)
```

This produces bitmasks for:
- `mask_d`: Delimiter characters (`:`, `,`)
- `mask_a`: Opening brackets (`{`, `[`)
- `mask_z`: Closing brackets (`}`, `]`)
- `mask_q`: Quote characters (`"`)
- `mask_b`: Backslash characters (`\`)

### 3. Escape Sequence Handling

A clever precomputed lookup table handles escape sequences:

```c
uint8_t hw_json_simd_escape_mask[2][256]
```

This 2x256 table encodes:
- Whether backslashes at each position are escaping or escaped
- Indexed by `[parity of trailing ones from previous byte][current byte value]`
- Uses `_lzcnt_u64` (leading zero count) to track consecutive backslashes

### 4. Quote Tracking with Bitwise Addition

The algorithm tracks quoted vs. unquoted regions using parallel bit manipulation:

```c
// Identify opening and closing quotes
uint64_t qas = _pdep_u64(0x5555555555555555 << ((*quote_odds_carry) & 1), w64_bits_of_q_word);
uint64_t qzs = _pdep_u64(0x5555555555555555 << ((*quote_evens_carry) & 1), w64_bits_of_q_word);

// Create quote mask using carry-propagating addition
uint64_t quote_mask = hw_json_simd_bitwise_add(qas, ~qzs, quote_mask_carry);
```

The `_pdep_u64` (parallel bit deposit) instruction from BMI2 is used to:
- Distribute alternating bits to quote positions
- Determine which quotes are opening vs. closing based on parity

### 5. State Machine Approach (Standard Method)

The standard method uses a finite state machine with:

**States**:
- State 0: Outside string (initial)
- State 1: Inside string
- State 2: After backslash in string
- State 3: Value context

**Transition Table** (`simd-transition-table-32.c`):
256 entries mapping (state, input_byte) -> new_state, stored as packed 32-bit values for SIMD lookup.

**Output Table** (`simd-phi-table-32.c`):
256 entries mapping (state, input_byte) -> output_bits, encoding:
- Bit 5: Is interesting bit
- Bit 6: Is opening bracket
- Bit 7: Is closing bracket

**SIMD State Processing**:
```c
__m128i s = _mm_set_epi64x(0, *inout_state);
for (size_t i = 0; i < in_length; i += 1) {
  __m128i p = _mm_shuffle_epi8(_mm_set1_epi32(hw_json_simd_phi_table_32[w]), s);
  out_phi_buffer[i] = _mm_extract_epi32(p, 0);
  s = _mm_shuffle_epi8(_mm_set1_epi32(hw_json_simd_transition_table_32[w]), s);
}
```

Uses `_mm_shuffle_epi8` for parallel table lookup across all 4 states simultaneously.

### 6. Bit Extraction and Deposit (BMI2)

The project makes heavy use of BMI2 instructions:

**PEXT (Parallel Bit Extract)**:
```c
uint64_t ext_d = _pext_u64(~(w64_a | w64_z), w64_ib);
```
Extracts bits at positions indicated by mask into contiguous low bits.

**PDEP (Parallel Bit Deposit)**:
```c
_pdep_u64(remainder_bits_a, 0x5555555555555555)
```
Deposits contiguous bits to positions indicated by mask.

These are used to:
- Extract only the structural characters from the input
- Interleave open/close parentheses into the BP index

### 7. Popcount for Variable-Length Output

Since the number of interesting bits per 64-byte chunk varies, the code uses popcount to track how many bits to output:

```c
size_t pc_ib = __builtin_popcountll(w64_ib);
if (remainder_len + pc_ib >= 64) {
  // Output full word
}
```

## Rust Porting Considerations

### Feasibility Assessment

**High Feasibility**: This project is well-suited for Rust porting because:

1. **Clear FFI Boundary**: The core algorithm is already in C, making the Rust implementation straightforward
2. **SIMD Support**: Rust has excellent SIMD support via `std::arch` and the `simd` crate
3. **Safe Abstractions**: Rust can provide memory-safe wrappers around the unsafe SIMD operations
4. **No GC Dependency**: The algorithm doesn't rely on Haskell's runtime or lazy evaluation semantically

### Rust SIMD Intrinsics

Rust provides equivalent intrinsics through `std::arch`:

| C/Intel Intrinsic | Rust Equivalent |
|-------------------|-----------------|
| `_mm256_cmpeq_epi8` | `std::arch::x86_64::_mm256_cmpeq_epi8` |
| `_mm256_movemask_epi8` | `std::arch::x86_64::_mm256_movemask_epi8` |
| `_mm_shuffle_epi8` | `std::arch::x86_64::_mm_shuffle_epi8` |
| `_pext_u64` | `std::arch::x86_64::_pext_u64` |
| `_pdep_u64` | `std::arch::x86_64::_pdep_u64` |
| `_lzcnt_u64` | `std::arch::x86_64::_lzcnt_u64` |
| `__builtin_popcountll` | `u64::count_ones()` |

Example Rust translation of the summarize function:

```rust
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[target_feature(enable = "avx2")]
unsafe fn summarize(buffer: &[u8; 32]) -> (u32, u32, u32, u32, u32) {
    let v_in_data = _mm256_loadu_si256(buffer.as_ptr() as *const __m256i);

    let v_comma = _mm256_cmpeq_epi8(v_in_data, _mm256_set1_epi8(b',' as i8));
    let v_colon = _mm256_cmpeq_epi8(v_in_data, _mm256_set1_epi8(b':' as i8));
    // ... etc

    let mask_d = (_mm256_movemask_epi8(v_comma) | _mm256_movemask_epi8(v_colon)) as u32;
    // ... etc

    (mask_d, mask_a, mask_z, mask_q, mask_b)
}
```

### Comparison to simd-json

The [simd-json](https://github.com/simd-lite/simd-json) Rust crate takes a different approach:

| Aspect        | hw-json-simd                   | simd-json                |
|---------------|--------------------------------|--------------------------|
| **Purpose**   | Create navigable semi-index    | Full JSON parsing        |
| **Output**    | IB + BP bitvectors             | Parsed DOM or tape       |
| **Algorithm** | Ottaviano-Grossi semi-indexing | Langdale-Lemire simdjson |
| **Memory**    | O(n/8) bytes additional        | O(n) bytes for DOM       |
| **Use Case**  | Query-time navigation          | Complete deserialization |

**Key Differences**:

1. **simd-json** implements the simdjson algorithm which:
   - Validates JSON syntax
   - Builds a complete parse tree (DOM) or tape representation
   - Optimizes for full document parsing speed

2. **hw-json-simd** implements semi-indexing which:
   - Does NOT validate JSON syntax
   - Builds a compressed navigation structure
   - Optimizes for space-efficient partial document access
   - Enables rank/select queries for JSON navigation

**Complementary Use**: These libraries serve different purposes:
- Use simd-json when you need to fully parse and access all JSON data
- Use semi-indexing when you need to query specific paths in large JSON files without full parsing

### Recommended Rust Implementation Strategy

1. **Phase 1: Core SIMD Functions**
   - Port `hw_json_simd_summarise` using `std::arch`
   - Implement escape mask lookup table
   - Port quote tracking logic with `_pdep_u64`/`_pext_u64`

2. **Phase 2: State Machine Method**
   - Create static transition and phi tables
   - Implement SIMD state machine using `_mm_shuffle_epi8`
   - Port IB/OP/CL chunk generation

3. **Phase 3: BP Generation**
   - Implement bit accumulator for variable-length output
   - Port `write_bp_chunk` and `write_bp_chunk_final`

4. **Phase 4: High-Level API**
   - Create streaming iterator over (IB, BP) chunks
   - Implement runtime CPU feature detection
   - Provide fallback scalar implementations

5. **Phase 5: Integration**
   - Create bitvector rank/select structures (from hw-rankselect concepts)
   - Implement JSON navigation using the semi-index

### Dependencies for Rust Port

```toml
[dependencies]
# For explicit SIMD control
std-simd = "0.1"  # or use std::arch directly

# For runtime CPU feature detection
raw-cpuid = "10"

# For efficient bitvectors
bitvec = "1"

# For memory-mapped file support
memmap2 = "0.5"
```

### Architecture Considerations

1. **ARM Support**: The current C code only supports x86 (SSE4.2/AVX2). For ARM:
   - Use NEON intrinsics via `std::arch::aarch64`
   - The `vceqq_u8` instruction is equivalent to `_mm_cmpeq_epi8`
   - No direct BMI2 equivalent; requires emulation or different algorithm

2. **Portable SIMD**: Consider using Rust's `portable_simd` feature (nightly) for cross-platform support

3. **Fallback Path**: Implement scalar versions for platforms without SIMD support

## Summary

hw-json-simd is a sophisticated implementation of SIMD-accelerated semi-indexing for JSON documents. It combines:

- Academic research (Ottaviano-Grossi semi-indexing)
- Modern CPU instructions (AVX2, SSE4.2, BMI2)
- Haskell's FFI for integration with the broader hw-json ecosystem
- Streaming/lazy processing for large documents

A Rust port would be highly feasible and could potentially offer:
- Better performance through more direct SIMD control
- Memory safety without runtime overhead
- Integration with the Rust ecosystem (serde, etc.)
- Cross-platform SIMD via portable_simd

The main challenge would be implementing the rank/select operations on the resulting bitvectors to enable actual JSON navigation, which would require porting concepts from hw-rankselect-base as well.
