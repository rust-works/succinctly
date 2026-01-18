# Lookup Table Techniques

Lookup tables trade memory for computation, replacing complex calculations with simple array indexing. This technique is particularly effective when the input domain is small and bounded.

## Overview

| Table Type          | Size       | Use Case                    | Speedup       |
|---------------------|------------|-----------------------------|---------------|
| Byte lookup (256)   | 256 bytes  | Per-byte operations         | 10-50x        |
| Nibble lookup (16)  | 16 bytes   | SIMD character class        | 2-6x          |
| State machine (256) | 256+ bytes | Parser transitions          | 40-77%        |
| 2D lookup           | N×M        | Multi-parameter queries     | Variable      |

---

## Byte-Level Lookup Tables

### Principle

For any function `f: u8 -> T`, precompute all 256 results:

```rust
const TABLE: [T; 256] = compute_all_results();

fn f(byte: u8) -> T {
    TABLE[byte as usize]
}
```

**Cost**: 256 × sizeof(T) bytes
**Benefit**: Single array access instead of computation

### Popcount Table

Count bits in a byte:

```rust
const POPCOUNT_TABLE: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = (i as u8).count_ones() as u8;
        i += 1;
    }
    table
};

fn byte_popcount(b: u8) -> u8 {
    POPCOUNT_TABLE[b as usize]
}
```

**Note**: Modern CPUs have POPCNT instruction, so this is mainly for portable code.

### Select-in-Byte Table

Find position of k-th set bit within a byte:

```rust
// src/util/table.rs - 2048 entries (256 bytes × 8 positions)
const SELECT_IN_BYTE_TABLE: [u8; 2048] = /* precomputed */;

fn select_in_byte(byte: u8, k: u8) -> u8 {
    SELECT_IN_BYTE_TABLE[(byte as usize) * 8 + k as usize]
}
```

**Usage in succinctly**: Final step of broadword select algorithm.

---

## Balanced Parentheses Tables

Tree navigation requires computing properties of parentheses sequences. Precomputing these for all byte values enables O(1) per-byte queries.

### Min Excess Table

**Problem**: Find minimum prefix sum (excess = opens - closes) within a byte.

```rust
// src/trees/bp.rs
const BYTE_MIN_EXCESS: [i8; 256] = /* precomputed */;

// For byte 0b10110100 (opens at 2,4,5,7; closes at 0,1,3,6):
// Prefix sums: -1,-2,-1,-2,-1,0,-1,0
// Min excess = -2
```

**Application**: Finding matching close parenthesis - scan bytes until min_excess <= target.

### Total Excess Table

**Problem**: Net change in depth for a byte.

```rust
const BYTE_TOTAL_EXCESS: [i8; 256] = /* precomputed */;

// Total = popcount(byte) - (8 - popcount(byte))
//       = 2 * popcount(byte) - 8
```

### Find-Close Table

**Problem**: Find bit position where excess drops to target within a byte.

```rust
// src/trees/bp.rs - 4096 entries (256 bytes × 16 excess values)
const BYTE_FIND_CLOSE: [[u8; 16]; 256] = /* precomputed */;

fn find_close_in_byte(byte: u8, initial_excess: u8) -> Option<u8> {
    let result = BYTE_FIND_CLOSE[byte as usize][initial_excess as usize];
    if result < 8 { Some(result) } else { None }
}
```

**Result**: 11x speedup (130.9 µs → 11.3 µs for 1M elements)

### Prior Art

- **Jacobson (1989)**: Original succinct tree representation
- **Munro & Raman (2001)**: Succinct encoding of balanced parentheses
- **Navarro & Sadakane (2014)**: Survey of succinct data structures

---

## SIMD Nibble Lookup Tables

Modern SIMD provides table lookup instructions that operate on all vector lanes in parallel.

### NEON vqtbl1q_u8

Performs 16 parallel lookups from a 16-entry table:

```rust
// src/json/simd/neon.rs
use core::arch::aarch64::vqtbl1q_u8;

fn classify_chars_nibble(chars: uint8x16_t) -> uint8x16_t {
    // Split each byte into high and low nibbles
    let lo_nibble = vandq_u8(chars, vdupq_n_u8(0x0F));
    let hi_nibble = vshrq_n_u8(chars, 4);

    // Lookup tables for character classification
    let lo_table = vld1q_u8(LO_NIBBLE_TABLE.as_ptr());
    let hi_table = vld1q_u8(HI_NIBBLE_TABLE.as_ptr());

    // Parallel lookups
    let lo_result = vqtbl1q_u8(lo_table, lo_nibble);
    let hi_result = vqtbl1q_u8(hi_table, hi_nibble);

    // Combine: character matches if both nibbles agree
    vandq_u8(lo_result, hi_result)
}
```

**Benefit**: Replaces 13 comparison operations with 2 lookups + 1 AND.
**Result**: 2-6% speedup in JSON parsing.

### AVX2 vpshufb

Similar technique on x86:

```rust
use core::arch::x86_64::_mm256_shuffle_epi8;

// _mm256_shuffle_epi8 does 32 parallel 4-bit lookups
let result = _mm256_shuffle_epi8(table, indices);
```

### Prior Art

- **Lemire (2017)**: "Parsing Gigabytes of JSON per Second"
- **Langdale & Lemire (2019)**: simdjson nibble-based classification

---

## State Machine Tables

Finite state machines benefit enormously from table-driven transitions.

### PFSM (Parallel Finite State Machine)

**Problem**: Parse JSON with a state machine that tracks string/non-string state.

```rust
// src/json/pfsm_tables.rs

// Transition table: byte × state → next_state (packed)
const TRANSITION_TABLE: [u32; 256] = /* precomputed */;

// Output table: byte × state → output_bits (packed)
const PHI_TABLE: [u32; 256] = /* precomputed */;

fn process_byte(byte: u8, state: u8) -> (u8, u8) {
    let packed_trans = TRANSITION_TABLE[byte as usize];
    let packed_phi = PHI_TABLE[byte as usize];

    let next_state = ((packed_trans >> (state * 8)) & 0xFF) as u8;
    let output = ((packed_phi >> (state * 8)) & 0xFF) as u8;

    (next_state, output)
}
```

**Packing**: Each table entry contains 4 states worth of data in 32 bits.

**Result**: 33-77% faster than scalar JSON parsing.

### Classic DFA Table

For parsers with more states:

```rust
const TRANSITIONS: [[State; 256]; NUM_STATES] = /* precomputed */;
const ACTIONS: [[Action; 256]; NUM_STATES] = /* precomputed */;

fn step(state: State, byte: u8) -> (State, Action) {
    (
        TRANSITIONS[state as usize][byte as usize],
        ACTIONS[state as usize][byte as usize]
    )
}
```

### Prior Art

- **Aho & Corasick (1975)**: String matching with finite automata
- **Mytkowicz et al. (2014)**: "Data-Parallel Finite-State Machines"
- **Langdale & Lemire (2019)**: PFSM for JSON parsing

---

## Two-Dimensional Lookup

When function depends on two bounded parameters.

### Example: Find-Close with Excess

```rust
// Find close paren position for byte with given initial excess
const FIND_CLOSE: [[u8; MAX_EXCESS]; 256] = /* precomputed */;

fn find_close(byte: u8, excess: u8) -> u8 {
    FIND_CLOSE[byte as usize][excess as usize]
}
```

**Trade-off**: Memory grows as O(N×M), but lookup is O(1).

---

## Table Generation

### Compile-Time (const fn)

Rust allows computing tables at compile time:

```rust
const fn generate_popcount_table() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut count = 0;
        let mut n = i;
        while n > 0 {
            count += (n & 1) as u8;
            n >>= 1;
        }
        table[i] = count;
        i += 1;
    }
    table
}

const POPCOUNT_TABLE: [u8; 256] = generate_popcount_table();
```

**Benefit**: No runtime initialization, tables in read-only memory.

### Build Script (build.rs)

For complex tables, generate code at build time:

```rust
// build.rs
fn main() {
    let table = compute_complex_table();
    let code = format!("const TABLE: [u64; {}] = {:?};", table.len(), table);
    std::fs::write("src/generated_table.rs", code).unwrap();
}
```

---

## Usage in Succinctly

| Table                   | Size    | Location              | Purpose                    |
|-------------------------|---------|-----------------------|----------------------------|
| SELECT_IN_BYTE_TABLE    | 2 KB    | `util/table.rs`       | Select-in-word final step  |
| BYTE_MIN_EXCESS         | 256 B   | `trees/bp.rs`         | BP min excess per byte     |
| BYTE_TOTAL_EXCESS       | 256 B   | `trees/bp.rs`         | BP total excess per byte   |
| BYTE_FIND_CLOSE         | 4 KB    | `trees/bp.rs`         | BP find-close per byte     |
| TRANSITION_TABLE        | 1 KB    | `json/pfsm_tables.rs` | PFSM state transitions     |
| PHI_TABLE               | 1 KB    | `json/pfsm_tables.rs` | PFSM output generation     |
| Nibble tables (NEON)    | 32 B    | `json/simd/neon.rs`   | Character classification   |

**Total**: ~9 KB of lookup tables

---

## Key Lessons

1. **256 entries is magic**: Byte-indexed tables are extremely cache-friendly
2. **Trade memory for computation**: Tables pay off when accessed frequently
3. **Combine with SIMD**: Nibble lookup tables enable parallel classification
4. **Pack multiple results**: Store all state transitions in single lookup
5. **Const tables**: Generate at compile time to avoid initialization cost

---

## References

- Knuth, D. E. "The Art of Computer Programming, Volume 3: Sorting and Searching"
- Warren, H. S. "Hacker's Delight" - Chapter on lookup tables
- Lemire, D. & Langdale, G. "Parsing Gigabytes of JSON per Second" (2019)
- Intel "Optimization Reference Manual" - Table-driven optimization patterns
