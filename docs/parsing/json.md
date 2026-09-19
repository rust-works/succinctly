# JSON Parsing

[Home](../../) > [Docs](../) > [Parsing](./) > JSON

This document describes how the succinctly library parses and indexes JSON documents using semi-indexing techniques.

## Overview

Unlike traditional JSON parsers that build a DOM tree, succinctly creates a **semi-index**: a compact bit-vector representation that enables O(1) navigation without materializing the parse tree.

| Component                | Purpose                   | Size           |
|--------------------------|---------------------------|----------------|
| Interest Bits (IB)       | Mark structural positions | ~0.5% of input |
| Balanced Parentheses (BP)| Encode tree structure     | ~0.5% of input |
| Rank/Select indices      | Enable O(1) queries       | ~2-3% of input |

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

| Operation     | Implementation                              |
|---------------|---------------------------------------------|
| First child   | `BP: find next 1 after current position`    |
| Next sibling  | `BP: find_close(current) + 1`               |
| Parent        | `BP: find matching open before current close`|
| Text position | `IB: select1(BP.rank1(bp_pos))`             |

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

| State        | Description            | Transitions                             |
|--------------|------------------------|-----------------------------------------|
| **InJson**   | Outside strings/values | `"` → InString, digit/letter → InValue  |
| **InString** | Inside quoted string   | `"` → InJson, `\` → InEscape            |
| **InEscape** | After backslash        | any → InString                          |
| **InValue**  | Inside unquoted value  | whitespace/delimiter → InJson           |

### Output (Phi Values)

Each byte produces 0-3 output bits:

| Character     | State    | IB | BP         |
|---------------|----------|----|-----------:|
| `{` or `[`    | InJson   |  1 | open       |
| `}` or `]`    | InJson   |  0 | close      |
| `"` (opening) | InJson   |  1 | open+close |
| `"` (closing) | InString |  0 | -          |
| digit (first) | InJson   |  1 | open+close |
| other         | any      |  0 | -          |

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

Each bit plane of the AND matches exactly a Cartesian product
{lo nibbles} × {hi nibbles}, so every character class gets one bit plane per
product it decomposes into; sharing a plane over-matches boundary bytes and
diverges from the other backends on invalid JSON (#186).

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

### LineIndex

Line/column to offset conversion, backed by Elias-Fano-encoded line starts so the index scales with
the number of lines rather than the size of the text:

```rust
use succinctly::text::LineIndex;

let text = b"line1\nline2\nline3";
let index = LineIndex::build(text);

// Line/column are 1-indexed
assert_eq!(index.to_offset(2, 1), Some(6));  // Start of line 2
assert_eq!(index.to_line_column(6), (2, 1)); // Reverse lookup
```

Handles all line ending conventions: Unix (LF), Windows (CRLF), and classic Mac (CR).

`JsonIndex` and `YamlIndex` build one lazily on the first `to_line_column`/`to_offset` call, so a jq
query that never asks for a position never pays for it. See
[optimizations/line-index.md](../optimizations/line-index.md) and [ADR-0012](../adrs/adr-0012.md).

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

| File                        | Purpose                        |
|-----------------------------|--------------------------------|
| `src/json/mod.rs`           | Module exports                 |
| `src/json/pfsm_tables.rs`   | TRANSITION_TABLE, PHI_TABLE    |
| `src/json/pfsm_optimized.rs`| Single-pass PFSM processor     |
| `src/json/standard.rs`      | 4-state cursor algorithm       |
| `src/json/light.rs`         | JsonIndex, JsonCursor APIs     |
| `src/json/locate.rs`        | Path location (jq-locate CLI)  |
| `src/json/bit_writer.rs`    | BitWriter implementation       |
| `src/json/simd/avx2.rs`     | AVX2 classification            |
| `src/json/simd/neon.rs`     | NEON nibble lookup             |
| `src/trees/bp.rs`           | Balanced parentheses operations|

---

## Performance

### Parsing Throughput

| Platform | CPU                       | Method      | Throughput |
|----------|---------------------------|-------------|------------|
| x86_64   | AMD Ryzen 9 7950X (Zen 4) | PFSM + BMI2 | ~880 MiB/s |
| x86_64   | AMD Ryzen 9 7950X (Zen 4) | AVX2        | ~730 MiB/s |
| ARM64    | Apple M1 Max              | NEON        | ~570 MiB/s |

### Navigation

| Operation            | Complexity       |
|----------------------|------------------|
| First child          | O(1)             |
| Next sibling         | O(1) amortized   |
| Random field access  | O(log n)         |
| Sequential iteration | O(1) per element |

---

### Canonical Compact Echo (`-c`, #2608)

For a compact (`-c`), unsorted, default-convention (`JsonConvention::JqCompat`) render, the `// #2608:` block in `JsonCursor::stream_json` (`src/json/light.rs`) first tries to echo the node's source span verbatim instead of re-rendering it through `stream_json_pretty`. A span is echoed only once a strict single-pass scan, `canonical_compact_jq_span_end` (built from `scan_canonical_value`, `scan_json_string_span`, and `scan_canonical_object`), certifies that it is *exactly* what the node-by-node re-render would emit — no whitespace, every number already in jq's canonical spelling (`is_jq_canonical_number`), every string already in jq's escape table's canonical form (no `\/`, no uppercase hex or `\u00XX` escape for a codepoint that has a short-form escape or is otherwise printable, raw DEL rejected), and no repeated key in any object — and `core::str::from_utf8` then rules on the span's UTF-8 validity in one vectorised pass. Any failure of either half falls through to the unchanged re-render, so bailing is always safe: the scan doubles as the structural validation the re-render otherwise supplies (a stray comma, missing colon, or bareword has no legal token at the position the grammar expects one).

The scan finds the value's own end, so the echo stops exactly at the node — the file's trailing newline, a following top-level document, and `.[]`'s next sibling are never included — and a `debug_assert_eq!` against `text_range` pins that on every accepted span through the 2,000-document fuzz sweep (`is_canonical_compact_jq_span_fuzz_2608`). `JsonIndex::build` does not itself validate UTF-8 (only the CLI's `utf8_lossy_document` substitutes invalid input before indexing), which is why the UTF-8 half must fall through rather than error. `-a`/`--ascii-output` never reaches this method at all — its callers route around `stream_json` — and `--preserve-input` keeps its own, older verbatim path (the branch immediately above the `#2608` block).

Safety argument and tests: `canonical_compact_jq_span_end`'s own doc comment, plus `is_canonical_compact_jq_span_agrees_with_rerender_2608`, `is_canonical_compact_jq_span_fuzz_2608`, `canonical_echo_stops_at_the_value_end_2608` (`src/json/light.rs`) and `test_canonical_compact_echo_declines_non_canonical_spans_2608`, `test_canonical_echo_stops_at_the_root_value_2608`, `test_invalid_utf8_in_string_survives_the_echo_gate_2608`, `test_ascii_output_still_escapes_through_new_echo_gate_2608` (`tests/jq_cli_tests.rs`).

**Why it pays.** Callgrind on the 7950X, `-c .` on a canonical 7,085,880-byte `{"data":[records]}` document, before this work: the re-render spends 58.5% of its instructions walking the semi-index (`text_position`/select 19.8%, BP `find_close`/`uncons` 27.2%, the duplicate-key census 10.1% — `collapsed_fields_checked` walks every object field once to census keys and the printer walks it again — gap checks 1.4%), 21.3% on index build and input read, 9.0% on per-string scanning plus `from_utf8` re-validation, 7.1% on the inlined render driver, and only 3.8% on `String` appends and copies. Any design that keeps the walk can remove at most ~7.9%. The echo removes the walk entirely: in the accepting profile, `stream_json_pretty`, `find_close`, `text_position`, `census`, and `key_hash_of` are absent.

**Gate cost per input byte** (7950X, callgrind exclusive, same document):

| Component                                                                             | PR #2919 as reviewed | After dropping the `text_range` rescan | Final (UTF-8 ruled on once) |
|---------------------------------------------------------------------------------------|----------------------|----------------------------------------|-----------------------------|
| `JsonCursor::text_range` linear rescan for the closing bracket (inside `raw_bytes()`) | 9.23                 | 0                                      | 0                           |
| `scan_canonical_value`                                                                | 12.36                | 12.36                                  | 12.32                       |
| `scan_json_string_span`                                                               | 11.17                | 11.17                                  | 10.54                       |
| `core::str::from_utf8` on the accepted span                                           | 0.44                 | 0.44                                   | 0.44                        |
| copy out                                                                              | 0.02                 | 0.02                                   | 0.02                        |
| **total**                                                                             | **33.2**             | **24.0**                               | **23.3**                    |

The echo itself is free; everything the gate costs is spent deciding whether it may echo. The first draft called `raw_bytes()`, whose `text_range` walks the whole container a second time just to find the closing bracket the scan is about to reach anyway — 28% of the gate. Moving UTF-8 validation out of the scan's string arm (it decoded every byte ≥ 0x80 through `decode_code_point`) bought only 0.63 Ir/byte on this near-ASCII corpus; the byte-at-a-time loop, not the decode, is the string arm's cost.

**Results**, interleaved wall-clock vs `main` (no gate), min / median, 9 reps, control floor ±2–4% (measured 2026-09-19 on `terminus` = AMD Ryzen 9 7950X and `johns-mac-mini` = Apple M4 Pro):

| Row                                                      | 7950X             | M4 Pro            |
|----------------------------------------------------------|-------------------|-------------------|
| `-c .` data 10 MB (canonical)                            | −68.4% / −68.3%   | −71.6% / −71.7%   |
| `-c .data` data 10 MB                                    | −68.2% / −68.0%   | −71.3% / −71.3%   |
| `-c .` arrays 10 MB                                      | −71.7% / −71.6%   | −73.7% / −73.7%   |
| `-c .` wide 2 MB (one object, 136k keys)                 | −49.1% / −48.8%   | −47.9% / −48.1%   |
| `-c .` users 2 MB                                        | −64.3% / −63.6%   | −60.1% / −60.4%   |
| `-c .` late-fail twin (duplicate key in the last record) | **+7.7% / +7.7%** | **+8.8% / +9.5%** |
| `-c .` early-fail twin (`1e2` in the first record)       | +0.9% / +0.5%     | +2.4% / +1.6%     |
| pretty `.` (never reaches the gate)                      | +0.3% / +1.1%     | +0.9% / +0.9%     |
| `-c --preserve-input .`                                  | −1.1% / −2.2%     | 0.0% / 0.0%       |

The late-fail twin is the precheck-cost control CLAUDE.md's O6/#1514 lesson calls for: the whole scan is charged and the whole re-render still runs afterward. It started at +12.7% (7950X) / +13.8% (M4 Pro) with the reviewed PR, and the two changes above (dropping the `text_range` rescan, moving UTF-8 validation out of the string scan) took it to +7.7% / +8.8%; the residual gate (23.3 Ir/byte against the re-render's ~253 Ir/byte on that row) predicts +9.2%, which matches. A document that fails early — the common non-canonical case, e.g. pretty-printed input fails at its first whitespace — pays only the early-fail row's +1–2%.

**Rejected: SIMD-skipping a string's plain bytes.** A jq-table `define_escape_scanner!` instantiation (`"`, `\`, `< 0x20`, DEL) plus #2963's 8-byte word probe cut `scan_json_string_span` from 10.54 to 7.02 Ir/byte (−33% on that arm, −4.8% Ir on the whole run on the 7950X), and *regressed wall-clock on both boxes*: data 10 MB +8.5%/+9.4% (M4 Pro, reproduced twice), +2.9%/+9.0% (7950X); users 2 MB +3.5–5.6% / +0.4–3.4%; the late-fail row it existed to help got worse (+1.5–3.0%); and the pretty row moved +3.0%/+4.4% on the 7950X on identical instruction counts. The strings in these fixtures are 4–12 bytes, below the width where a NEON/AVX compare→movemask→GPR round trip beats a short scalar loop (#2963), with a code-layout band on top — the two mechanisms were never separated with a holdout build. Reverted (commit `c74f11a66`, not on this branch).

**Follow-ups (reported, not built):** on a numbers-only document (`arrays` 10 MB, 6,291,458 bytes) `scan_canonical_value` is 48.0 Ir/byte and 42% of the whole run, all in the inlined `nested_number_span` + `is_jq_canonical_number` — an arrays-shaped late failure would pay far more than the +7.7% above. On `wide` 2 MB the >16-key `KeyHashes::insert` tier costs 5.4 Ir/byte, 18% of the gate, while the ≤16-key pairwise fingerprint scan is invisible on record-shaped documents.

---

## Optimisation Techniques Used

| Technique            | Document                                                       | Application             |
|----------------------|----------------------------------------------------------------|-------------------------|
| Lookup tables        | [lookup-tables.md](../optimizations/lookup-tables.md)          | PFSM state machine      |
| SIMD classification  | [simd.md](../optimizations/simd.md)                            | Character detection     |
| Nibble lookup        | [lookup-tables.md](../optimizations/lookup-tables.md)          | NEON classification     |
| Hierarchical indices | [hierarchical-structures.md](../optimizations/hierarchical-structures.md) | Rank/select for BP |
| Branchless masking   | [branchless.md](../optimizations/branchless.md)                | SIMD result extraction  |
| Lazy evaluation      | [zero-copy.md](../optimizations/zero-copy.md)                  | Defer value decoding    |
| Exponential search   | [access-patterns.md](../optimizations/access-patterns.md)      | Sequential select hints |

---

## See Also

- [JsonIndex wiki page](json-index.md) — concept overview, dependencies, and academic references

## References

- Langdale, G. & Lemire, D. "Parsing Gigabytes of JSON per Second" (2019)
- Mison: A Fast JSON Parser for Data Analytics (Microsoft Research)
- simdjson: https://github.com/simdjson/simdjson
