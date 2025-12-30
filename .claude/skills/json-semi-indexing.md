# JSON Semi-Indexing and Succinct Data Structures

## JSON Semi-Index Structure

The semi-index produces two bit vectors:
- **Interest Bits (IB)**: Marks structurally interesting positions (opens, leaves)
- **Balanced Parentheses (BP)**: Encodes the tree structure for navigation

### Cursors

**Simple Cursor**: 3 states (InJson, InString, InEscape)
- Only tracks string boundaries for proper quote handling

**Standard Cursor**: 4 states (InJson, InString, InEscape, InValue)
- Treats primitive values (numbers, booleans, null) as leaves
- Values emit BP=10 (open+close) making them leaf nodes

### State Machine Outputs (Phi)

```
Open (O):   IB=1, BP=1   - Opening bracket { or [
Close (C):  IB=0, BP=0   - Closing bracket } or ]
Leaf (L):   IB=1, BP=10  - String/value start (treated as leaf node)
None (-):   (nothing)    - Whitespace, delimiters, string content
```

## SIMD JSON Semi-Indexing

### Architecture Pattern

Both NEON (ARM) and SSE2 (x86) use the same 16-byte chunk processing:

1. `classify_chars()` - Vectorized character classification returning bitmasks
2. `process_chunk_standard()` - Serial state machine over 16 bytes
3. `build_semi_index_standard()` - Main loop with SIMD chunks + scalar tail

### Key Intrinsics

**ARM NEON**:
```rust
vceqq_u8(chunk, splat)      // Equality comparison
vcleq_u8(chunk, max)        // Unsigned less-than-or-equal
vandq_u8(a, b)              // Bitwise AND
vorrq_u8(a, b)              // Bitwise OR
```

Movemask emulation (NEON lacks native movemask):
```rust
vshrn_n_u16(vreinterpretq_u16_u8(mask), 4)  // Pack to 4-bit nibbles
vget_lane_u64(vreinterpret_u64_u8(...), 0)  // Extract as u64
```

**x86 SSE2**:
```rust
_mm_cmpeq_epi8(chunk, splat)  // Equality comparison
_mm_movemask_epi8(cmp)        // Extract MSBs as u16 bitmask
_mm_min_epu8(a, b)            // For unsigned LE: min(a,b) == a
```

Unsigned LE comparison (SSE2 lacks unsigned compare):
```rust
unsafe fn unsigned_le(a: __m128i, b: __m128i) -> __m128i {
    let min_ab = _mm_min_epu8(a, b);
    _mm_cmpeq_epi8(min_ab, a)  // a <= b iff min(a,b) == a
}
```

### Multi-Architecture Support

```rust
// src/json/simd/mod.rs
#[cfg(target_arch = "aarch64")]
pub mod neon;

#[cfg(target_arch = "x86_64")]
pub mod x86;

#[cfg(target_arch = "aarch64")]
pub use neon::build_semi_index_standard;

#[cfg(target_arch = "x86_64")]
pub use x86::build_semi_index_standard;

// Fallback for other platforms
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub use super::standard::build_semi_index as build_semi_index_standard;
```

## Balanced Parentheses Operations

### Core Operations

- `find_close(p)`: Find matching close for open at position p
- `find_open(p)`: Find matching open for close at position p
- `excess(p)`: Count of opens minus closes up to position p
- `enclose(p)`: Find the nearest enclosing open
- `depth(p)`: Nesting depth at position p (same as excess)

### Word-Level Operations

`find_close_in_word(word, p)`: Find matching close within a single 64-bit word
- Uses excess tracking with `count_ones` on masked portions
- Returns `None` if match extends beyond word boundary

`find_unmatched_close_in_word(word)`: Find first unmatched close
- Important for multi-word find_close acceleration

### MinExcess for Acceleration

Pre-computed minimum excess within blocks enables skipping:
- If block's min_excess >= current_excess, the match isn't in that block
- L0 index: per-word min_excess and cumulative excess
- L1 index: per-64-word-block summary for large structures

## BitWriter Pattern

Efficient bit vector construction:
```rust
pub struct BitWriter {
    words: Vec<u64>,
    current: u64,
    bit_pos: u32,  // 0-63 position in current word
}

impl BitWriter {
    pub fn write(&mut self, value: bool) { ... }
    pub fn write_bits(&mut self, bits: u64, count: u32) { ... }
    pub fn finish(self) -> Vec<u64> { ... }
}
```

For BP output where leaves need "10" (open+close pair):
```rust
bp.write_bits(0b01, 2);  // Note: LSB first, so 0b01 = "10" in reading order
```

## Testing Patterns

### SIMD vs Scalar Comparison
Always verify SIMD produces identical results to scalar:
```rust
fn compare_results(json: &[u8]) {
    let scalar = standard::build_semi_index(json);
    let simd = simd::build_semi_index_standard(json);
    assert_eq!(scalar.ib, simd.ib);
    assert_eq!(scalar.bp, simd.bp);
    assert_eq!(scalar.state, simd.state);
}
```

### Boundary Testing
Test at SIMD chunk boundaries (multiples of 16):
- Escapes spanning chunk boundaries
- State transitions at position 15/16
- Inputs of length 15, 16, 17, 31, 32, 33, etc.

### Cross-Verification
Verify accelerated operations match naive implementations:
```rust
fn naive_find_close(bp: &BalancedParens, p: usize) -> Option<usize> {
    let mut excess = 1i64;
    for i in (p + 1)..bp.len() {
        excess += if bp.is_open(i) { 1 } else { -1 };
        if excess == 0 { return Some(i); }
    }
    None
}
```
