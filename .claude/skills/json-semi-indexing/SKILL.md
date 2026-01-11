---
name: json-semi-indexing
description: JSON semi-indexing implementation details and patterns. Use when working on JSON parsing, semi-index construction, or cursor navigation. Triggers on terms like "JSON index", "semi-index", "interest bits", "balanced parentheses", "JSON cursor".
---

# JSON Semi-Indexing Skill

Implementation details for JSON semi-indexing using succinct data structures.

## JSON Semi-Index Structure

The semi-index produces two bit vectors:
- **Interest Bits (IB)**: Marks structurally interesting positions (opens, leaves)
- **Balanced Parentheses (BP)**: Encodes the tree structure for navigation

### State Machine Outputs (Phi)

```
Open (O):   IB=1, BP=1   - Opening bracket { or [
Close (C):  IB=0, BP=0   - Closing bracket } or ]
Leaf (L):   IB=1, BP=10  - String/value start (treated as leaf node)
None (-):   (nothing)    - Whitespace, delimiters, string content
```

### Cursors

**Simple Cursor**: 3 states (InJson, InString, InEscape)
- Only tracks string boundaries for proper quote handling

**Standard Cursor**: 4 states (InJson, InString, InEscape, InValue)
- Treats primitive values (numbers, booleans, null) as leaves
- Values emit BP=10 (open+close) making them leaf nodes

## SIMD Architecture Pattern

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

**x86 SSE2**:
```rust
_mm_cmpeq_epi8(chunk, splat)  // Equality comparison
_mm_movemask_epi8(cmp)        // Extract MSBs as u16 bitmask
_mm_min_epu8(a, b)            // For unsigned LE: min(a,b) == a
```

## Multi-Architecture Module Pattern

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

## PFSM (Table-Driven Parser)

**Default implementation** achieving 950 MiB/s throughput.

### Two-Stage Pipeline

1. **Stage 1 - State Machine**: Sequential byte-by-byte with 256-entry lookup tables
   - TRANSITION_TABLE: Maps (byte, state) -> next_state
   - PHI_TABLE: Maps (byte, state) -> output bits (IB/OP/CL)

2. **Stage 2 - Bit Extraction**: Parallel extraction of interest bits and BP
   - BMI2+AVX2 path: Process 8 phi values at once using PEXT/PDEP
   - Scalar fallback: Byte-by-byte extraction

### Why PFSM Wins

- Table lookups are cache-friendly (256 entries = 1-2KB per table)
- State machine separates control flow from bit extraction
- BMI2 PEXT/PDEP used correctly for sparse bit operations
- Amortizes table lookup overhead across the entire input
- Benefits from modern CPU branch prediction

## Balanced Parentheses Operations

### Core Operations

- `find_close(p)`: Find matching close for open at position p
- `find_open(p)`: Find matching open for close at position p
- `excess(p)`: Count of opens minus closes up to position p
- `enclose(p)`: Find the nearest enclosing open
- `depth(p)`: Nesting depth at position p

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

## Performance Reference

| Size      | succinctly            | jq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  2.4 ms  (3.9 MiB/s)  |  4.3 ms  (2.2 MiB/s)  | **1.79x**  |
| **100KB** |  4.6 ms (18.4 MiB/s)  |  8.2 ms (10.5 MiB/s)  | **1.76x**  |
| **1MB**   | 24.7 ms (32.7 MiB/s)  | 43.9 ms (18.4 MiB/s)  | **1.78x**  |
