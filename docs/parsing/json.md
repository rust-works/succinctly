# JSON Parsing

This document describes how the succinctly library parses and indexes JSON documents using semi-indexing techniques.

## Overview

Unlike traditional JSON parsers that build a DOM tree, succinctly creates a **semi-index**: a compact bit-vector representation that enables O(1) navigation without materializing the parse tree.

| Component | Purpose | Size |
|-----------|---------|------|
| Interest Bits (IB) | Mark structural positions | ~0.5% of input |
| Balanced Parentheses (BP) | Encode tree structure | ~0.5% of input |
| Rank/Select indices | Enable O(1) queries | ~2-3% of input |

**Total overhead**: ~3-4% of input size vs 10-50x for DOM parsers.

---

## Semi-Index Structure

### Interest Bits (IB)

One bit per JSON byte. Set to 1 at structurally significant positions:

- Opening brackets/braces: `[`, `{`
- String starts (first `"`)
- Value starts (first digit, letter, `-`)

```
JSON:  {"name":"Alice","age":30}
IB:    1 1     1       1    1
       {  "     "       "    3
```

### Balanced Parentheses (BP)

Encodes the tree structure using open/close bits:

- `1` = open (entering a node)
- `0` = close (leaving a node)

```
JSON:  {"name":"Alice","age":30}
BP:    1 10   10     10   10  0
       { key  value  key  value }
```

The BP sequence forms a valid balanced parentheses string where each node (object, array, or value) is represented as a matched pair.

### Navigation

With IB and BP, navigation becomes bit operations:

| Operation | Implementation |
|-----------|----------------|
| First child | `BP: find next 1 after current position` |
| Next sibling | `BP: find_close(current) + 1` |
| Parent | `BP: find matching open before current close` |
| Text position | `IB: select1(BP.rank1(bp_pos))` |

See [hierarchical-structures.md](../optimizations/hierarchical-structures.md) for rank/select implementation details.

---

## Parsing Pipeline

```
JSON bytes
    ↓
[Character Classification] ← SIMD (AVX2/SSE/NEON)
    ↓
[State Machine] ← PFSM tables or cursor
    ↓
[BitWriter] → IB bits, BP bits
    ↓
[Index Construction] → Rank/select indices
    ↓
JsonIndex (ready for queries)
```

---

## State Machine

The parser uses a 4-state finite state machine:

| State | Description | Transitions |
|-------|-------------|-------------|
| **InJson** | Outside strings/values | `"` → InString, digit/letter → InValue |
| **InString** | Inside quoted string | `"` → InJson, `\` → InEscape |
| **InEscape** | After backslash | any → InString |
| **InValue** | Inside unquoted value | whitespace/delimiter → InJson |

### Output (Phi Values)

Each byte produces 0-3 output bits:

| Character | State | IB | BP |
|-----------|-------|----|----|
| `{` or `[` | InJson | 1 | open |
| `}` or `]` | InJson | 0 | close |
| `"` (opening) | InJson | 1 | open+close |
| `"` (closing) | InString | 0 | - |
| digit (first) | InJson | 1 | open+close |
| other | any | 0 | - |

---

## PFSM (Parallel Finite State Machine)

The fastest parsing method uses precomputed lookup tables.

### Table Structure

```rust
// 256-entry tables (one per byte value)
const TRANSITION_TABLE: [u32; 256];  // (byte, state) → next_state
const PHI_TABLE: [u32; 256];         // (byte, state) → output_bits
```

Each entry packs 4 state outcomes into 32 bits (8 bits per state).

### Processing

```rust
fn process_byte(byte: u8, state: u8) -> (u8, u8) {
    let packed_trans = TRANSITION_TABLE[byte as usize];
    let packed_phi = PHI_TABLE[byte as usize];

    let next_state = ((packed_trans >> (state * 8)) & 0xFF) as u8;
    let output = ((packed_phi >> (state * 8)) & 0xFF) as u8;

    (next_state, output)
}
```

**Performance**: 40-77% faster than scalar parsing.

See [lookup-tables.md](../optimizations/lookup-tables.md) and [state-machines.md](../optimizations/state-machines.md) for technique details.

---

## SIMD Character Classification

Before the state machine runs, SIMD identifies character types in parallel.

### AVX2 (32 bytes/iteration)

```rust
unsafe fn classify_avx2(chunk: &[u8; 32]) -> Masks {
    let data = _mm256_loadu_si256(chunk.as_ptr() as *const __m256i);

    // Find specific characters
    let quotes = _mm256_cmpeq_epi8(data, _mm256_set1_epi8(b'"' as i8));
    let opens = _mm256_or_si256(
        _mm256_cmpeq_epi8(data, _mm256_set1_epi8(b'{' as i8)),
        _mm256_cmpeq_epi8(data, _mm256_set1_epi8(b'[' as i8))
    );
    // ... more character types

    // Extract to bitmasks
    Masks {
        quotes: _mm256_movemask_epi8(quotes) as u32,
        opens: _mm256_movemask_epi8(opens) as u32,
        // ...
    }
}
```

### NEON Nibble Lookup

ARM NEON uses table lookup for character classification:

```rust
unsafe fn classify_neon(chunk: &[u8; 16]) -> Masks {
    let data = vld1q_u8(chunk.as_ptr());

    // Split into nibbles
    let lo_nibble = vandq_u8(data, vdupq_n_u8(0x0F));
    let hi_nibble = vshrq_n_u8(data, 4);

    // Parallel table lookups (16 at once)
    let lo_result = vqtbl1q_u8(LO_TABLE, lo_nibble);
    let hi_result = vqtbl1q_u8(HI_TABLE, hi_nibble);

    // Combine: match if both nibble tables agree
    let result = vandq_u8(lo_result, hi_result);
    // ...
}
```

This replaces 12+ comparisons with 2 lookups + 1 AND.

See [simd.md](../optimizations/simd.md) for SIMD technique details.

---

## BitWriter

Accumulates output bits efficiently:

```rust
struct BitWriter {
    words: Vec<u64>,
    current_word: u64,
    bit_position: u32,  // 0-63
}

impl BitWriter {
    fn write_bit(&mut self, bit: bool) {
        if bit {
            self.current_word |= 1u64 << self.bit_position;
        }
        self.bit_position += 1;
        if self.bit_position == 64 {
            self.flush_word();
        }
    }

    fn write_zeros(&mut self, count: usize) {
        // Optimized: skip full words of zeros
    }
}
```

See [zero-copy.md](../optimizations/zero-copy.md) for buffer management techniques.

---

## JsonIndex API

### Construction

```rust
use succinctly::json::JsonIndex;

let json = br#"{"users":[{"name":"Alice"},{"name":"Bob"}]}"#;
let index = JsonIndex::from_json(json)?;
```

### Navigation

```rust
let cursor = index.root();

// Navigate to first user's name
let users = cursor.get("users")?;
let first_user = users.first_child()?;
let name = first_user.get("name")?;

// Get the actual value
let text = name.as_str()?;  // "Alice"
```

### Lazy Evaluation

Values are decoded only when accessed:

```rust
enum StandardJson<'a> {
    String(JsonString),   // Parsed on as_str()
    Number(JsonNumber),   // Parsed on as_i64(), as_f64()
    Object(JsonFields),   // Iterated lazily
    Array(JsonElements),  // Iterated lazily
    Bool(bool),
    Null,
}
```

---

## Position Location (jq-locate)

The `locate` module enables reverse lookup: given a byte offset or line/column position, find the jq expression that navigates to that location.

### Algorithm

1. **Position resolution**: Convert line/column to byte offset using a newline index
2. **Node lookup**: Use `ib_rank1(offset)` to find the containing structural element
3. **Path construction**: Walk up using `bp.parent()`, collecting path components

### NewlineIndex

Fast line/column to offset conversion using rank/select on newline positions:

```rust
use succinctly::json::locate::NewlineIndex;

let text = b"line1\nline2\nline3";
let index = NewlineIndex::build(text);

// Line/column are 1-indexed
assert_eq!(index.to_offset(2, 1), Some(6));  // Start of line 2
assert_eq!(index.to_line_column(6), (2, 1)); // Reverse lookup
```

Handles all line ending conventions: Unix (LF), Windows (CRLF), and classic Mac (CR).

### CLI Usage

```bash
# By byte offset (0-indexed)
succinctly jq-locate file.json --offset 42
# Output: .users[0].name

# By line/column (1-indexed)
succinctly jq-locate file.json --line 5 --column 10
# Output: .data.items[2]

# Detailed JSON output
succinctly jq-locate file.json --offset 42 --format json
# Output: {"expression": ".users[0].name", "type": "string", "byte_range": [42, 49]}
```

---

## File Locations

| File | Purpose |
|------|---------|
| `src/json/mod.rs` | Module exports |
| `src/json/pfsm_tables.rs` | TRANSITION_TABLE, PHI_TABLE |
| `src/json/pfsm_optimized.rs` | Single-pass PFSM processor |
| `src/json/standard.rs` | 4-state cursor algorithm |
| `src/json/light.rs` | JsonIndex, JsonCursor APIs |
| `src/json/locate.rs` | Path location (jq-locate CLI) |
| `src/json/bit_writer.rs` | BitWriter implementation |
| `src/json/simd/avx2.rs` | AVX2 classification |
| `src/json/simd/neon.rs` | NEON nibble lookup |
| `src/trees/bp.rs` | Balanced parentheses operations |

---

## Performance

### Parsing Throughput

| Platform | Method | Throughput |
|----------|--------|------------|
| x86_64 | PFSM + BMI2 | ~950 MiB/s |
| x86_64 | AVX2 | ~730 MiB/s |
| ARM64 | NEON | ~570 MiB/s |

### Navigation

| Operation | Complexity |
|-----------|------------|
| First child | O(1) |
| Next sibling | O(1) amortized |
| Random field access | O(log n) |
| Sequential iteration | O(1) per element |

---

## Optimisation Techniques Used

| Technique | Document | Application |
|-----------|----------|-------------|
| Lookup tables | [lookup-tables.md](../optimizations/lookup-tables.md) | PFSM state machine |
| SIMD classification | [simd.md](../optimizations/simd.md) | Character detection |
| Nibble lookup | [lookup-tables.md](../optimizations/lookup-tables.md) | NEON classification |
| Hierarchical indices | [hierarchical-structures.md](../optimizations/hierarchical-structures.md) | Rank/select for BP |
| Branchless masking | [branchless.md](../optimizations/branchless.md) | SIMD result extraction |
| Lazy evaluation | [zero-copy.md](../optimizations/zero-copy.md) | Defer value decoding |
| Exponential search | [access-patterns.md](../optimizations/access-patterns.md) | Sequential select hints |

---

## References

- Langdale, G. & Lemire, D. "Parsing Gigabytes of JSON per Second" (2019)
- Mison: A Fast JSON Parser for Data Analytics (Microsoft Research)
- simdjson: https://github.com/simdjson/simdjson
