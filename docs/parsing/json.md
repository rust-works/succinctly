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

**Why it pays.** Callgrind on the 7950X, `-c .` on a canonical 7,085,880-byte `{"data":[records]}` document, before this work: the re-render spends 58.5% of its instructions walking the semi-index (`text_position`/select 19.8%, BP `find_close`/`uncons` 27.2%, the duplicate-key census 10.1% — `collapsed_fields_checked` walks every object field once to census keys and the printer walks it again — gap checks 1.4%), 21.3% on index build and input read, 9.0% on per-string scanning plus `from_utf8` re-validation, 7.1% on the inlined render driver, and only 3.8% on `String` appends and copies. Any design that keeps the walk *as it then cost* can remove at most ~7.9%; the walk itself later got cheaper (#3140 answers a leaf's `find_close` from its next bit, −12.4% x86_64 / −11.8% ARM64 instructions on perf-guard's `users_compact_latefail`, the row that pays the gate and then the re-render). The echo removes the walk entirely: in the accepting profile, `stream_json_pretty`, `find_close`, `text_position`, `census`, and `key_hash_of` are absent.

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

**Rejected again: SIMD-skipping a string's plain bytes (#3168).** #2608 tried a jq-table `define_escape_scanner!` instantiation (`"`, `\`, `< 0x20`, DEL) in `scan_json_string_span`, behind #2963's 8-byte word probe on aarch64. It cut that arm from 10.54 to 7.02 Ir/byte, yet regressed wall-clock on both boxes (data 10 MB +8.5%/+9.4% on the M4 Pro, +2.9%/+9.0% on the 7950X, and the pretty row +3.0%/+4.4% on the 7950X on identical instruction counts). The commit that added it, `c74f11a66`, was then dropped from #2608's branch without the two candidate mechanisms being separated. #3168 re-applied it on `60e74bcd9` and measured variants against a holdout:
- **Holdout:** the same tree dispatching once per string, at function entry, to the old loop (`black_box(false)`), so the scanner is compiled in and never called. A first holdout that tested inside the byte loop added +5–6% Ir and was discarded.
- **Variants:** the scanner called on every string ("prefix 0"), and a plain byte walk over each string's first 8 or 16 bytes before calling it.
- **Corpus:** 4 MB arrays of 0–3-byte ("tiny") and 4/8/16/64-byte strings, escape-dense strings (embedded JSON, tabs, quotes), records with one-character keys, and the #2608 `data`/`users` files.

All runs are `-c .`, 21 interleaved reps, with output identical everywhere:

| M4 Pro, wall (median)              | tiny  | 4 B   | 8 B    | 16 B  | 64 B   | esc-dense | short keys | data  | users |
|------------------------------------|-------|-------|--------|-------|--------|-----------|------------|-------|-------|
| control (base vs itself)           | −0.2% | +0.1% | +0.8%  | −2.6% | +0.4%  | +0.6%     | +0.2%      | +0.6% | 0.0%  |
| holdout (never calls the scanner)  | +1.8% | +1.0% | −1.0%  | +0.5% | −2.0%  | +0.3%     | +2.2%      | +1.5% | +0.1% |
| prefix 0 (scanner on every string) | +3.4% | −4.8% | −11.3% | −7.4% | −20.2% | +16.7%    | +12.0%     | +5.8% | +3.5% |
| 8-byte scalar prefix               | +0.6% | +0.3% | +2.5%  | −7.1% | −19.4% | +3.7%     | +0.9%      | −0.4% | 0.0%  |
| 16-byte scalar prefix              | +0.8% | −0.4% | −2.2%  | +4.1% | −17.3% | +1.3%     | +0.7%      | +0.7% | −0.7% |

| 7950X, Ir             | tiny   | 4 B   | 8 B   | 16 B   | 64 B   | esc-dense | short keys | data  | users |
|-----------------------|--------|-------|-------|--------|--------|-----------|------------|-------|-------|
| holdout               | +2.9%  | +2.1% | +1.5% | +0.9%  | +0.3%  | +0.6%     | +1.6%      | +1.1% | +1.1% |
| prefix 0              | +10.3% | −0.4% | −9.1% | −16.7% | −24.2% | +3.5%     | +6.6%      | −4.9% | −4.7% |
| 8-byte scalar prefix  | +6.2%  | +3.0% | +7.4% | −6.4%  | −21.6% | +3.8%     | +3.6%      | −0.3% | −0.1% |
| 16-byte scalar prefix | +6.2%  | +3.0% | +0.3% | +2.4%  | −18.9% | +2.7%     | +3.6%      | +0.5% | +0.7% |

| 7950X, wall (median) | tiny  | 4 B   | 8 B    | 64 B   | esc-dense | short keys | data  | users |
|----------------------|-------|-------|--------|--------|-----------|------------|-------|-------|
| control              | −0.8% | +1.9% | −1.2%  | −1.8%  | −3.9%     | −0.7%      | −1.0% | −1.3% |
| holdout              | −1.4% | −5.6% | −3.4%  | −6.3%  | −1.8%     | −2.7%      | −2.1% | −2.5% |
| prefix 0             | −7.6% | −4.5% | −10.2% | −18.1% | +3.1%     | +0.7%      | −6.2% | −7.3% |

- **The mechanism is real and per call.** The scanner's cost is paid on entry, once per string and again after every escape. It wins as soon as a string is long enough to amortise that (every variant: −17% to −24% at 64 bytes), and loses where entries are dense: escape-dense text, short keys, tiny strings. On the M4 Pro it is large (+17% escape-dense, +12% short keys with no prefix); on the 7950X it shows net of the holdout as about +5% (escape-dense) and +3% (short keys) wall, and in Ir as +3.5%/+6.6%.
- **#2608's own M4 Pro loss was partly the word probe, not NEON.** The build it measured resolved almost every 4-byte string in the 8-byte word probe, which LLVM vectorised (a GPR→NEON→GPR round trip). The truly prefix-free build above *wins* at 4 bytes on the M4 Pro.
- **Layout exists on x86 but is not the story.** The holdout reads −1% to −6% wall on the 7950X, and ±2% on the M4 Pro.
- **No variant is neutral-or-better on every shape on both boxes.** A scalar prefix hides the entry cost on aarch64 but keeps only the 64-byte win, and on x86 every prefix regresses tiny strings and short keys in Ir. So nothing ships. The per-byte loop stays, and the shape that would remove the per-call entry entirely (classify the span's string specials in one pass) is [#3340](https://github.com/rust-works/succinctly/issues/3340), which shipped: see the next section.

**Numbers in one pass (#3167).** On a numbers-only document (`arrays` 10 MB, 6,291,458 bytes) the gate's number arm was 48.0 Ir/byte, 42% of the whole run. It read every number twice (the greedy `nested_number_span`, then `is_jq_canonical_number` over it), and it paid an out-of-line recursive `scan_canonical_value` call for every array element and an out-of-line `is_jq_canonical_number` call per number: 33 Ir each on single-digit numbers. Now `jq_canonical_number_prefix` (the canonical rules, with `is_jq_canonical_number` defined as "the prefix parse covers the whole span") is read straight off the document, and the span is accepted when the next byte cannot extend the greedy class (`is_number_span_byte`, the class's one definition, shared with `nested_number_span` and `strict_number_end`). `scan_canonical_value` is `inline(always)` into the array and object loops, so only a container recurses; the string arm stays one out-of-line call. `scan_canonical_number_agrees_with_the_two_pass_rule_3167` pins the one-pass arm against the two-pass rule for every token of up to four class bytes followed by each of the 256 byte values, and inside an array and an object through the whole gate. Against `923a080a5`, `-c .`, 21 interleaved reps, output identical on every row:

| row                               | 7950X Ir | 7950X wall (median) | M4 Pro wall (median) |
| --------------------------------- | -------- | ------------------- | -------------------- |
| arrays 2 MB                       | −14.1%   | −6.4%               | −4.0%                |
| arrays 10 MB                      | −14.2%   | −3.9%               | −4.8%                |
| data 2 MB                         | −3.4%    | −4.8%               | −1.0%                |
| data 10 MB                        | −3.4%    | −2.0%               | −2.5%                |
| data 10 MB late-fail              | −1.0%    | −0.3%               | −0.9%                |
| data 10 MB early-fail             | +0.02%   | —                   | —                    |
| users 2 MB                        | −3.4%    | −0.1%               | −1.4%                |
| wide 2 MB                         | −5.0%    | −2.1%               | −2.1%                |
| pretty `.` arrays 10 MB (control) | +0.14%   | +1.8%               | +0.8%                |
| pretty `.` data 10 MB (control)   | +0.02%   | −0.1%               | −0.5%                |

The Ir ratio is the same at 2 MB and 10 MB, as a per-number constant-factor saving should be. `ab-cli.py --control` read −1.5%..+0.5% on the 7950X and −0.3%..+0.3% on the M4 Pro. A first build of the same change read −5%..−9% wall on the 7950X; this one, with identical Ir, reads −0.1%..−6.4%, so the 7950X's wall column carries the layout band this page's #2720 notes describe, and the Ir column is the attribution. `scripts/perf-guard.py` stays within threshold (`users_compact_identity` −3.2%, `users_compact_latefail` −0.9%). The per-object zeroing of `scan_canonical_object`'s small-tier arrays is not visible in any profile (inline stores, no `memset` call). The >16-key `KeyHashes::insert` tier was then the largest gate term on a wide object, 83 Ir per key and 6.9% of `wide`'s run; [#3333](https://github.com/rust-works/succinctly/issues/3333) is below.

**Wide objects: one exactly sized table (#3333).** After #3167 the largest remaining gate term on a wide object was the duplicate-key table: past `PAIRWISE_SPAN_SCAN_LIMIT` (16) keys `scan_canonical_object` grew a `KeyHashes` from 32 slots, so `wide` 2 MB (136k keys) spent 11.2 M of its 157 M instructions in `KeyHashes::insert` (cachegrind, 7950X). Line-level attribution split it three ways: the probe itself, the rehash on each doubling (the `hash == 0` / `while slots[at] != 0` lines of the inlined `grow` account for at least 1.6 M of it, before their loop overhead), and 16 Ir per key of prologue and epilogue because `insert` is not inlined. `scan_canonical_object_wide` now takes the object over at the 17th key, only *collects* each key's `key_hash` into a `Vec<u64>`, and at the closing `}` settles them through `KeyHashes::has_repeat`, a table sized for exactly that many keys (nothing rehashed, no growth test in the loop). It declines at `KeyHashes::SATURATING_KEYS` (786,432) keys, the width at which the grown table turned `saturated()`, so the accept/decline set is unchanged. The small-object loop keeps no wide-tier state, which is also why the wide tier is its own function. The cost: a repeat in a wide object is found at the object's end rather than at the key. The span is declined either way, but a repeat near the *front* of a wide object (the 21st key of 136k or 700k) now costs the scan of the rest of the object before the decline, **+5% instructions and +2.3% to +4.1% wall** on a path that goes on to the full re-render (a ~10x larger cost). It is the one row that regresses, measured and accepted: a duplicate key in a document wide enough for this to show is rare, and the bound is one extra scan of one object. A checkpoint that would recover it was built and measured (#4156) and rejected; see "Rejected: a checkpoint for a front-loaded repeat" below.

Against `ce09e00da`, `-c .`, 21 interleaved reps, output identical on every row (min / median wall; 7950X `Ir` from cachegrind):

| row                                     | 7950X `Ir` | 7950X wall        | M4 Pro wall       |
|-----------------------------------------|------------|-------------------|-------------------|
| `wide` 2 MB (136k keys)                 | −6.7%      | −25.4% / −24.4%   | −22.0% / −22.9%   |
| `wide` 8 MB (560k keys)                 | −6.5%      | −27.2% / −29.0%   | −30.8% / −29.3%   |
| `wide` 11.7 MB (700k keys)              | —          | −24.9% / −25.2%   | −27.5% / −27.4%   |
| array of 1,000-key objects, 2 MB        | —          | −16.8% / −17.2%   | −17.2% / −17.0%   |
| array of 100-key objects, 2 MB          | −9.8%      | −8.9% / −9.2%     | −10.3% / −10.0%   |
| array of 24-key objects, 2 MB           | −2.7%      | −5.0% / −4.6%     | −0.4% / −0.3%     |
| array of 17-key objects, 2 MB           | −1.8%      | −4.4% / −5.2%     | +1.0% / −0.1%     |
| `wide` with a repeat as the last key    | −1.4%      | −5.1% / −5.6%     | −5.9% / −6.6%     |
| `wide` 2 MB, repeat at the 21st key     | +5.2%      | +3.4% / +3.4%     | +3.5% / +3.6%     |
| `wide` 700k keys, repeat at the 21st    | +5.1%      | +2.5% / +2.3%     | +4.1% / +4.0%     |
| `wide` with a repeat in the first 16    | ≈0         | −1.7% / −0.8%     | −0.1% / +0.9%     |
| `users` 2 MB (6 keys per object)        | −0.4%      | +1.5% / +1.1%     | +1.7% / +0.4%     |
| `data` 10 MB                            | −0.4%      | +0.9% / +2.1%     | +0.6% / +1.0%     |
| `data` 10 MB late-fail twin             | −0.1%      | +0.5% / +0.5%     | +0.2% / +0.9%     |

`ab-cli.py --control` read −0.7%..+1.6% (7950X) and −0.4%..+0.5% (M4 Pro). The `Ir` column is the attribution, and it is not the whole story: the wall-clock win on `wide` (−25%) is nearly four times its `Ir` win (−6.7%). That is consistent with the growing table's doublings walking and zeroing ever larger arrays (2 MB at 136k keys) that the exactly sized table allocates once, but no build isolating that was made. Peak RSS is flat (`wide` 2 MB 13.1 MB → 13.0 MB, 700k keys 33.9 MB → 35.0 MB, `wide` 8 MB 30.8 MB → 30.9 MB).

**The last three rows are layout, not the change.** Those objects (at most 6 keys) never reach the new function, and their instruction counts fall. A holdout (the same tree with `scan_canonical_object_wide` bypassed by a `black_box(true)` to the old growing-table continuation, so both are compiled in) reads `base` → holdout −2.5%..+1.5% (min) on the 7950X and −3.7%..+1.0% on the M4 Pro, and holdout → head `users` +2.6% / +3.0% and `data` +3.6% / +3.0% on the 7950X, +1.7% / +1.4% and +1.3% / +2.2% on the M4 Pro, while `wide` and the 100/1,000-key rows read the same win against the holdout as against `base` (7950X `wide` 2 MB −24.3% / −23.9%, M4 Pro −23.8% / −22.5%). It is the band this page's #2720 and #3100 notes describe: the same `Ir` to within 0.4% on a row the change cannot reach, moving by a point or two of wall clock with the code around it.

**Rejected: sort the collected hashes instead of building a table.** The issue's second direction (the batch census sites' shape: `sort_unstable`, adjacent compare) is the architecture split #1514 recorded. Against the table above, same method:

| row                              | 7950X (min / median) | M4 Pro (min / median) |
|----------------------------------|----------------------|-----------------------|
| `wide` 2 MB                      | −7.4% / −7.6%        | +4.7% / +5.9%         |
| `wide` 8 MB                      | −9.8% / −8.8%        | +10.2% / +9.4%        |
| `wide` 11.7 MB                   | −8.4% / −8.2%        | +5.4% / +6.8%         |
| array of 1,000-key objects       | +6.2% / +6.6%        | +7.7% / +9.1%         |
| array of 100-key objects         | +5.0% / +4.3%        | +7.8% / +7.0%         |
| array of 17-key objects          | −3.9% / −4.0%        | +0.8% / +0.2%         |

**The crossover, located (#4158).** The sort wins on the 7950X from 64,000 keys per object and loses below it; on the M4 Pro it loses at every width above 17 keys. Same method, `jq -c .`, 21 interleaved reps per run, output identical on all 13 rows, cgu1 + fat LTO builds of one tree that differ only in how `scan_canonical_object_wide` settles its `Vec<u64>` (`has_repeat` against `fold_hash` + `sort_unstable` + an adjacent compare, same `SATURATING_KEYS` ceiling). Each row is an array of objects of the stated width, about 3 MB of input (the 17/100/1,000-key rows are #3333's 2 MB files, the `wide` rows its single objects). Sort against table, min / median wall, two runs each (negative: the sort is faster):

| array of objects of        | table, `has_repeat`    | 7950X min (2 runs) | 7950X med (2 runs) | M4 Pro min (2 runs) | M4 Pro med (2 runs) |
|----------------------------|------------------------|--------------------|--------------------|---------------------|---------------------|
| 17 keys                    | 32 slots (256 B)       | +1.3 / +0.5        | +1.1 / +0.9        | −0.9 / +1.8         | +0.7 / +0.7         |
| 100 keys                   | 256 slots (2 KB)       | +5.7 / +6.3        | +7.7 / +7.3        | +6.8 / +6.5         | +6.2 / +5.5         |
| 1,000 keys                 | 2,048 slots (16 KB)    | +9.1 / +9.9        | +8.3 / +9.5        | +8.9 / +7.6         | +6.6 / +7.7         |
| 2,000 keys                 | 4,096 slots (32 KB)    | +11.0 / +7.8       | +9.0 / +9.1        | +9.3 / +7.5         | +8.7 / +7.8         |
| 4,000 keys                 | 8,192 slots (64 KB)    | +10.0 / +10.5      | +9.9 / +9.9        | +9.7 / +8.6         | +8.9 / +9.5         |
| 8,000 keys                 | 16,384 slots (128 KB)  | +8.3 / +7.7        | +9.4 / +8.5        | +10.8 / +10.7       | +10.1 / +10.5       |
| 16,000 keys                | 32,768 slots (256 KB)  | +7.7 / +7.1        | +6.4 / +8.5        | +11.5 / +8.2        | +9.9 / +10.0        |
| 32,000 keys                | 65,536 slots (512 KB)  | +6.4 / +4.5        | +5.7 / +5.0        | +9.2 / +8.8         | +9.9 / +9.6         |
| 64,000 keys                | 131,072 slots (1 MB)   | −4.3 / −2.3        | −2.3 / −2.6        | +6.6 / +8.5         | +7.3 / +8.2         |
| 128,000 keys               | 262,144 slots (2 MB)   | −6.9 / −7.1        | −6.6 / −6.6        | +8.6 / +9.5         | +8.9 / +9.1         |
| `wide` 2 MB (136k keys)    | 262,144 slots (2 MB)   | −7.6 / −4.7        | −6.1 / −5.4        | +5.9 / +7.6         | +5.7 / +7.6         |
| `wide` 8 MB (560k keys)    | 1,048,576 slots (8 MB) | −8.0 / −8.2        | −7.9 / −8.6        | +7.7 / +8.4         | +8.6 / +9.6         |
| `wide` 11.7 MB (700k keys) | 1,048,576 slots (8 MB) | −6.3 / −7.2        | −7.5 / −6.6        | +6.3 / +6.8         | +6.5 / +6.0         |

`ab-cli.py --control` on the 7950X read −1.2%..+1.5% per row (min) and +0.31% median of medians; on the M4 Pro it read −1.4% on every row (−1.9%..−0.7%, 13 of 13 faster), a bias of the second copy on that box, so the M4 Pro's sort penalty is about 1.4 points larger than its column shows. Both boxes idle at the start (checked by hand; `--force` for the harness's own self-matching idle check).

The 7950X flips between 32,000 keys (+4.5% to +6.4%) and 64,000 keys (−2.3% to −4.3%), and the flip is where the table reaches 131,072 slots, the core's whole 1 MB L2 (the 512 KB table below it still fits). That fits the earlier guess: below the L2 a probe is a cache hit and the table wins; above it most probes miss and the sort's sequential passes win. The mechanism is inferred from the flip point, not isolated by a counter run. The M4 Pro never flips because its L2 (16 MB shared by the cluster) holds even the 8 MB table of the 700k-key object; that is an inference from the cache sizes, not a measurement. The sort's margin at 64,000 keys (−2.3% to −4.3%) is at the edge of the layout band; it is a clear win only from 128,000 keys (−6.6% to −7.1%).

**Decision: the table stays, no threshold.** The sort beats the table by more than the ~3% layout band only on an object of at least 128,000 keys, on the 7950X only, and loses 6% to 11% on the M4 Pro at every one of those widths. A gate would be a per-architecture constant for a shape few documents have (one object of 128k or more keys is a 2 MB object), and `target_arch` is the wrong key for it anyway: the variable that moved the result is the core's L2, which differs across x86 parts (256 KB, 1 MB and 2 MB L2 are all in circulation) and which std cannot report portably. Not measured: a Zen-class or Intel box with a different L2 (none other than the 7950X and the Apple boxes was reachable), which is what would move the 64,000-key flip point. If such a box has wide objects as its common case, `has_repeat` is the one place to add the sorted path and the sweep above is the method to price it.

Not done: seeding the table from the BP span. `scan_canonical_object` runs on bytes and recurses, so only a root-level wide object could be seeded, and it would pay a `find_close` per object for an upper bound.

Pins: `key_hashes_has_repeat_agrees_with_insert_3333` (`src/jq/document.rs`: the zero-hash fold seam, every table-rounding length, a repeat first / middle / last), `key_hashes_has_repeat_stops_at_the_saturating_width_3333` (`SATURATING_KEYS` is where `saturated()` turns true; the widest list that settles; a repeat in its last slot; the conservative answer at the ceiling), and the `canonical_object_key_tiers_2608` sweep now covers 300-key objects and a repeat as the last key against both tiers. Each was checked to fail with `has_repeat` forced false, with the closing-brace verify removed, and with `SATURATING_KEYS` off by one.

**Rejected: a checkpoint for a front-loaded repeat (#4156).** The one regressing row above (a repeat near the front of a very wide object) was priced against the cheapest fix the issue named: a single extra `KeyHashes::has_repeat` over the first `N` collected hashes, run once when the count first reaches `N`, declining on a repeat. The loop's per-key `hashes.len() >= SATURATING_KEYS` test became `>= next_stop` (`N`, then `SATURATING_KEYS`), so the no-repeat path pays no extra compare per key, only the one table build per object wider than `N`. Built at `N` = 64 and 256 beside `main` and a holdout (the 64 tree with the checkpoint compiled in and `black_box(false)` in front of the call), all cgu1 + fat LTO, on a 7950X and an M4 Pro (`Ir` from cachegrind on the 7950X only, one run per row; wall is `ab-cli.py`, `jq -c .`, 21 interleaved reps, output identical on all 17 rows of every run). Change in instructions against the holdout:

| row                                            | `N` = 64  | `N` = 256 |
|------------------------------------------------|-----------|-----------|
| array of 17-key objects                        | 0.0%      | 0.0%      |
| array of 65-key objects                        | +4.2%     | 0.0%      |
| array of 100-key objects                       | +2.4%     | 0.0%      |
| array of 257-key objects                       | +1.1%     | +4.6%     |
| array of 1,000-key objects                     | +0.2%     | +1.2%     |
| `wide` 2 MB / 8 MB (no repeat)                 | ≤ +0.001% | ≤ +0.007% |
| `wide` 2 MB, repeat at the 21st key            | −5.3%     | −5.3%     |
| `wide` 2 MB, repeat at the 64th key            | −5.3%     | −5.3%     |
| `wide` 2 MB, repeat at the 65th key            | 0.0%      | −5.3%     |
| `wide` 2 MB, repeat at the 301st / 5,001st key | 0.0%      | 0.0%      |

The win is real and is exactly the +5% #3333 gave back, but only for a repeat inside the first `N` keys; wall clock recovers 1% to 6% (7950X −1.0% to −2.8%, M4 Pro −3.2% to −5.9% on those rows), less than the `Ir` because the re-render that follows makes the run about 6x longer than the same file without the repeat. The cost lands on the no-repeat object: every object just wider than `N` pays one table build for hashes the closing settle then probes again, `Ir` +4.2% at 65 keys (`N` = 64) and +4.6% at 257 (`N` = 256), +2.4% at the 100-key row the #3333 table already used. Wall clock on those rows read inside the controls (`ab-cli.py --control` per-row medians: 7950X −0.8%..+1.0%, M4 Pro −3.1%..+0.5%; `N` = 64 on 100-key objects +0.9% / +1.1% and +2.2% / +1.2% min / median against the holdout), so wall clock alone could not have rejected it; the instruction count can, because it has no noise.

**Decision: not taken.** The issue's own bar was to close this if the cheapest checkpoint costs a no-repeat row anything outside the control floor, and `Ir` shows it does on every width just past `N`. A heap-free table (not built) could cut the roughly 1,850 instructions per checkpoint (4.2 M over the 2,267 objects of the 100-key file) but not remove them: the checkpoint is 64 more probes per object on a path that has no repeat to find, so the objection is structural, not the allocation. What the checkpoint buys is a duplicate key in the first 64 or 256 keys of an object that is thousands of keys wide, on a path that goes on to a re-render six times larger. What it charges is a few percent on ordinary wide records. A repeat after key `N` (the 301st and 5,001st rows) is not helped by either, and the issue's other candidate, checkpoints at doubling widths, adds roughly the closing settle again to every wide object, the shape that currently wins 25%, so it was not built. Not measured: widths between the rows above, or a Zen-class box with a different L2 (the boundary cost is instructions, not cache, so it should not move).

**Strings over one specials mask (#3340).** #3168 found the per-string cost of a SIMD skip is its *entry*: paid once per string and again after every escape. #3340 takes the entry out. `scan_json_string_span` now asks `SpecialMask` (`src/util/simd/specials.rs`) for the next byte it must decide on (`"`, `\`, `< 0x20`, DEL) instead of walking the string. The mask classifies one 64-byte block (NEON: four 16-byte compares reduced with `vpaddq` to one `u64`, one vector-to-GPR transfer per 64 bytes; x86_64: four SSE2 `movemask`s; scalar elsewhere) and answers every query inside that block with a shift and a `trailing_zeros`. A short string is a bit scan of a word a neighbouring string already paid for; a long one steps block to block.
- **It is lazy, not the span-wide pass the issue sketched.** The text handed to the gate runs to the end of the document even when the cursor is a small sub-value, and `arrays` has no strings at all, so an eager pass would be charged O(document) for an O(value) decision on exactly the rows with the least to gain (the O6/#1514 precheck lesson). A block is classified when a string first reaches it, starting *at that string* rather than at an aligned address.
- **The semi-index's classification cannot be reused.** `JsonIndex` keeps only the interest bits and the balanced parentheses; the build's quote and backslash masks are consumed on the way to those and not retained. Storing them would put a bitvector in every index to speed one gate.
- **Holdout:** the same tree with `scan_json_string_span` dispatching once, at entry, to the old byte loop (`black_box(true)`), so the mask is compiled in and never called. `base` is `3be2e2ec4`, the branch's merge-base; all three binaries are `cgu1`+fat LTO builds from the same toolchain on each box (the M4 Pro runs the binaries built on the dev machine, the 7950X builds its own).

`-c .`, interleaved, min / median wall change against `base`, 21 reps (the #3168 and #2608 rows) or 31 reps (the generator rows), output identical and exit 0 on every row, 2026-10-07. `hold` is the layout-and-codegen bias; the mechanism's own effect is `head` minus `hold`:

| row                    | M4 Pro hold   | M4 Pro head       | 7950X hold    | 7950X head      |
|------------------------|---------------|-------------------|---------------|-----------------|
| tiny (0-3 B strings)   | +2.0% / +2.0% | −12.4% / −12.0%   | −0.2% / −0.2% | −14.0% / −14.2% |
| s4                     | +2.6% / +2.0% | −1.5% / −1.9%     | −2.9% / −3.4% | −8.9% / −11.1%  |
| s8                     | +1.8% / +1.0% | −3.7% / −4.6%     | −4.6% / +0.4% | −14.1% / −13.5% |
| s16                    | +0.4% / +0.1% | −13.9% / −13.8%   | −5.7% / −3.8% | −19.0% / −19.1% |
| s64                    | +0.1% / +0.8% | −22.3% / −22.4%   | −5.1% / −5.3% | −24.2% / −23.3% |
| escape-dense           | +0.3% / +0.5% | −1.2% / −1.0%     | −2.4% / −2.9% | −4.4% / −4.4%   |
| short-key records      | +2.5% / +2.2% | −2.4% / −1.8%     | −2.0% / −3.0% | −10.2% / −12.0% |
| data 2 MB              | +2.7% / +0.1% | −1.4% / −1.6%     | −4.3% / −3.3% | −8.6% / −8.4%   |
| data 10 MB             | +1.7% / +2.0% | −2.5% / −3.2%     | −2.6% / −2.7% | −9.4% / −10.1%  |
| users 2 MB             | +2.6% / +1.0% | −1.7% / −1.5%     | −2.9% / −3.1% | −8.2% / −9.1%   |
| arrays 10 MB           | −0.2% / −2.0% | +0.5% / −0.4%     | −2.9% / −2.5% | −2.8% / −2.7%   |
| wide 2 MB              | +1.4% / +1.1% | **+2.4% / +1.3%** | −1.6% / −1.9% | −2.8% / −1.2%   |
| generated wide 8 MB    | +0.5% / +0.7% | **+0.9% / +1.3%** | −1.9% / −1.8% | −3.5% / −4.0%   |
| generated users 8 MB   | +4.3% / +2.5% | −3.6% / −3.2%     | −3.7% / −4.1% | −8.7% / −9.6%   |
| generated strings 8 MB | −0.3% / +5.1% | −30.8% / −31.5%   | −6.8% / −7.3% | −25.0% / −24.8% |
| generated unicode 8 MB | +0.6% / +1.4% | −12.9% / −13.7%   | −2.3% / −1.3% | −15.8% / −16.1% |
| late-fail twin         | +0.4% / +0.7% | +0.3% / −0.8%     | −0.6% / −0.9% | −2.3% / −2.5%   |
| early-fail twin        | −0.8% / −0.2% | +0.1% / −0.3%     | −0.8% / −1.4% | −0.9% / −1.4%   |
| pretty `.` (control)   | −0.3% / −0.4% | +0.4% / +0.1%     | −2.8% / −3.3% | −3.5% / −3.0%   |

`ab-cli.py --control` read −1.3%..+2.0% (min) and −2.4%..+1.4% (median) on the M4 Pro, and −1.4%..+1.2% (min) and −1.8%..+4.3% (median) on the 7950X. The table is the final tree (`a2db8360d`, after review); an earlier revision of the same code read within 1.5 points of every cell. The late-fail twin, the precheck-cost control, did not get worse: its string work is now cheaper, and the re-render that follows is untouched. The mechanism wins at every string length #3168 lost at: 0-3-byte strings (−12.6% / −14.8%), the 4-byte and 8-byte rows, the escape-dense and short-key rows that cost the SIMD skip +5% to +17%.

Instruction counts (M4 Pro `time -l` instructions retired, min of 5; 7950X cachegrind `Ir`), `head` vs `base`, with `hold` beside it: `tiny` +7.0% (hold +7.9%) and +2.3% (+3.2%); `s4` −3.9% (+6.1%) and −5.8% (+2.4%); `short keys` +3.0% (+6.6%) and −0.3% (+2.8%); `wide 2 MB` −3.3% (+2.0%) and −4.1% (+1.0%); `data` 2 MB −6.8% (+1.4%) on the 7950X; `data 10 MB` late-fail −2.1% (+1.0%) on the M4 Pro; early-fail and `arrays` unchanged to 0.3%. No row executes more instructions than its holdout, and the numbers-only `arrays` row, which never classifies a block, is the proof the laziness holds.

**One residual, recorded rather than hidden:** the two `wide` rows (one object of 136k `"kN":N` members, each key 2-8 bytes) read +2.4% / +1.3% and +0.9% / +1.3% on the M4 Pro against a holdout at +1.4% / +1.1% and +0.5% / +0.7%, while retiring 3% *fewer* instructions: about +0.5% to +1% net, inside the control floor, and −1% to −4% on the 7950X. A first revision of the tree read +2.3% / +1.0% against a holdout at +0.1% / +0.5%, so the figure moves with layout. It has the signature #2963 describes (a vector-to-GPR transfer on the critical path of a short string) rather than an instruction-count cost. Not chased: the shape is a flat object with numeric values, and the rest of the table moves the other way by 2% to 31%.

The per-element shape, `-c '.[]'` over the array files (each output element runs the gate on its own, so a 64-byte block is classified for a short element), is neutral: `head` tracks `hold` on every row on both boxes (M4 Pro min −2.1%..+2.8%, median −0.9%..+0.6%; 7950X min −6.5%..−0.6% against `hold` at −6.1%..−0.3%). The one outlier, `s16` on the M4 Pro (+2.8% min against `hold` at −0.5%), reads +0.5% / +0.3% in the median.

`string_scan_agrees_with_the_bytewise_reference_across_block_boundaries_3340` and `..._on_random_strings_3340` pin the new scan against the old byte loop (kept in the test module as an independent oracle) with the opening quote at every offset of a 64-byte block and a special of every kind planted at every offset across two boundaries; `src/util/simd/specials.rs` pins the kernel against a scalar reference for every byte value in every lane.

### Keyed Lookup into a Wide Object (#3913)

`DocumentFields::find_cursor` answers "the last member whose key is `name`" by walking every member: a later duplicate can always supersede the match, and a malformed sibling must raise whether or not it is the one asked for. A pipeline that reads one wide object many times (`.big[$k]`, or `$b[.]` for a `$b` bound to a node of the document, #2072) therefore cost time linear in the key count per lookup (3.5 s for 10,000 lookups into 10,000 keys, 350x the owned-map read).

`src/jq/key_index.rs` adds a per-object `KeyIndex`: the key node ids in document order and an open-addressed table of 32-bit hashes, built in one pass over the keys alone (no member value is decoded). The walk reports how many members it visited (`find_cursor_counted`, free: the loop already counts); a walk of 64 or more registers the object, and the *next* lookup builds the index, so a small object, or a wide one read once, costs what it always did. A probe keeps the last entry whose decoded key equals the name, then checks only the winner's own `,`/`:` delimiters through `DocumentCursor::preceding_delimiter_ok`, as the walk does.

**Every anomaly refuses the build** (a non-string key, an unpaired tail, a stray trailing comma, more than 2^21 members, or repeated keys whose runs of table slots outgrow the build's probe budget, which would make the build quadratic) and the caller runs the walk, which raises what it always raised; a refusal is remembered so it is not retried, and so is an eviction (only four indexes are kept, and a pipeline cycling over more wide objects than that would otherwise rebuild one on nearly every lookup). The memo lives inside `slot_memo`'s evaluation scope (a `document_token` is not a security boundary), keeps at most four indexes and 2^21 members, and does not exist under `no_std`. YAML keeps the walk (`head_key_cursor` defaults to `None`): merge keys, alias keys and yq's duplicate rule need their own design.

Apple M4 Pro, release, 10,000 lookups, min of 3, base `main` (`ce32db7ab`) vs this change: `$b[.]` 0.61 s -> 0.11 s (1,000 keys) and 3.46 s -> 0.09 s (10,000); `.big[$k]` 0.62 s -> 0.10 s and 3.60 s -> 0.09 s; 100,000 keys: 74.8 s -> 0.09 s. Instructions retired (min of 3, same box): `.big.k5000.v` -0.1%, `keys_unsorted` -0.1%, `to_entries` 0.0%, `.users[0].name` 0.0%, two lookups of one wide object -1.5%, three -11.1%; the two rows that do nothing but look up fields of small records, `.users[].name` and `[.users[] | select(.age > 20) | .email]`, read +0.3% to +1.4% across six builds of this source, and two builds that differ only in an `#[inline]` hint read +1.5% / +0.2% against +0.9% / +1.2% -- the layout band, not a cost that tracks the change. Wall-clock on those rows (interleaved, 41 runs, load average 16) read +1.2% / -3.5% (min) and +0.3% / +1.7% (median). The index costs about 32 bytes per key (3 MB at 100,000 keys). Not measured: x86_64.

### Indexed Reads of a Wide Array (#4035)

Reading an element of a document array (`.users[0]`, `.[5]`, `$root.nodes[.from]`) resolved the index through `DocumentElements::len_checked` first: the walk normalizes a negative index, raises yq's own out-of-range error, and is where the malformed-delimiter checks live (a missing or doubled `,`, a trailing stray one, #1677, #2261, #2594). That is free for one read and quadratic for a program that uses a document array as a lookup table, because every read walks the whole array again and then walks siblings to the element. `. as $r | .users[] | $r.users[0].id` took 73 s on the 7 MB `users` document.

`src/jq/array_index.rs` adds a per-array `ElementIndex`: the node id of every element, built by the same walk `len_checked` makes with the same checks, so the length is the number of ids and element `k` is the node `ids[k]` names. **Every anomaly refuses the build** and the caller runs the walk, which raises what it always raised (and an array whose walk raises is never registered at all: only a walk that succeeded registers one). A walk of 64 or more elements registers the array, the second length lookup walks again and counts, and the **third** builds; an element read uses an existing index but never builds one. Building on the second made an array read exactly twice slower than the two walks it replaced (+4.7% / +8.7% on `.[] | [.[3], .[4]]` over 20,000 hundred-element arrays), because an index costs one walk plus the ids. The memo lives inside the same evaluation scope as `key_index`'s (a `document_token` is not a security boundary), keeps at most four indexes and 2^21 elements, remembers a refusal and an eviction, and does not exist under `no_std`. `length`, `last`, `keys`, `has`, `.[N]`, `.[$k]`, `getpath` (a slice of it included) and the `path()` index step read through it.

The probe names a list by `DocumentElements::head_id` (the node id it stands at, with no resolution and no sibling walk) behind a one-word Bloom filter over the registered heads, so a small array read after a wide one was registered costs one thread-local read and a shift. Resolving the first element and its next sibling to name the list cost +4.9% instructions on that shape. YAML sequences take the same path: 126 base-vs-branch rows over bare `-` items, anchors and aliases, comments, tags, flow and nested sequences, block scalars and nulls, in JSON and YAML output, show no difference, and `. as $r | .users[] | $r.users[0].name` on 1 MB of YAML went from 3.43 s to 0.24 s.

Release, `ab-cli.py` interleaved, output identity gated on every row, a control run per box in the same session (every control row within -1.7%..+0.9%), base `36f8582c0` vs the shipped commit (`636ecb1f0`):

| row                                                                               | 7950X (min)       | M4 Pro (min)    |
|-----------------------------------------------------------------------------------|-------------------|-----------------|
| `. as $r \| .users[] \| $r.users[0].id`, 0.7 MB                                   | 1,114 ms -> 22 ms | 848 ms -> 17 ms |
| the same, 2.8 MB                                                                  | 17.7 s -> 84 ms   | 13.4 s -> 56 ms |
| `[.[range(0;300)]] \| length`, one 1M-element array                               | 4.31 s -> 70 ms   | 3.21 s -> 52 ms |
| `.[] \| [.[3], .[4], .[5], .[6]]`, 20,000 arrays of 100                           | -13.8%            | -17.1%          |
| `.[] \| [.[3], .[4]]` (read twice: the worst case)                                | +1.4%             | -1.3%           |
| `.users[100]`, `.users \| length`, `.users[-1]`, `last` (10 and 26 MB, read once) | -2.2%..+0.5%      | -3.0%..+1.1%    |
| `.[100]`, `length`, `last`, `.[-1]` on one 1M-element array                       | -2.7%..-1.7%      | -5.4%..-1.6%    |
| `.recs[] \| .s[0]` over 100,000 small arrays, after a wide one was registered     | -0.3%             | -1.3%           |

Growth: 0.7 MB to 2.8 MB (4x the records) multiplies the base by 16x and the change by 3.7x. Instructions (7950X cachegrind `Ir`, M4 Pro `time -l` instructions retired, min of 3), base vs shipped: every single-read row is -0.03% to +0.28%, the twice-read row +1.13% / +1.56% (registering 20,000 arrays), `[.[] | length] | add` +0.58% / +0.78%, the four-read row -17.1% / -15.4%, and the worst case for the probe -- 100,000 small arrays each read three times after one wide array was registered -- +1.57% / +2.14%. Cache misses and branch mispredicts (cachegrind with `--cache-sim=yes --branch-sim=yes`, `.users | length`, 7 MB): identical to 0.01%.

**The 7950X wall-clock on the single-read rows is placement, not cost, and moves with the build.** Five builds of near-identical source read +1.0%, +2.1%, +2.4%, +0.8% and -1.1% (median of the 14 single-read rows) at the same `Ir`, while the M4 Pro reads -1.4% to -1.5% on the four it was given; a holdout build of the same tree whose memo sits behind `black_box(false)` read +0.15% beside a `new` at +2.4% in the same session. This is the function-placement band `docs/guides/benchmarking.md` § 9 describes; it is recorded rather than chased.

Pinned by `array_index::tests` (the index agrees with the walk at every index and past the end; `build_succeeds_exactly_when_len_checked_does_4035` sweeps every arrangement of up to four members with every separator, leading and trailing delimiter the semi-index accepts; it builds on the third length lookup and never on an element read; never registers a small or malformed array; never answers a partly consumed list from the whole array's index) and `wide_document_array_reads_agree_with_jq_across_repeated_reads_4035`, whose rows are jq 1.7.1's.

### Key Census of a Wide Object (#3343)

`census` (`src/jq/document.rs`) is the key-only walk behind `length`, `keys | length` and the identity print of an object: it hashes every key, finds out whether any key repeats (jq keeps the last of a repeated key), and checks each member's `,` and `:` (#1677). After #3140 made the sibling walk cheaper it was still about a third of the identity print of perf-guard's 2 MB `wide` fixture (158,981 top-level keys). #3343 named two suspects, the #1677 delimiter scans and the 25 M-instruction `sort_unstable` of the hashes.

**The scans were not the cost; the second pass over each key was.** A build with both checks disabled saved 27 M instructions (172 per key), but the scans are two byte reads per key on a compact document. What the check paid for was `text_end()`, which rescans the key to find its closing quote, right after `key_hash_of` had scanned the same key for the raw span it hashes. `key_hash_and_end_of` takes both from the one `raw_and_escaped` scan (`DocumentValue::key_raw_unescaped_with_end`, JSON only), and `DistinctKeyCursors::next`, the `keys_unsorted` probe, uses it too. An escape, an unterminated span or a key with no raw span takes the old two-call path, so those answers cannot differ.

**The sort is replaced by a bitset prefilter for 128 to 2^21 hashes** (`repeated_hashes`). A real object almost never repeats a key, so sorting every hash only proves that nothing matched. One pass sets a bit per hash (the low bits of an already-mixed hash) and records in a second bitset every bit that was set twice; only the hashes on such a bit, roughly a tenth of them and almost all accidental bitset collisions, are sorted. The answer is exact: every occurrence of a repeated hash is on a doubly-set bit, and every other hash is alone on its own bit and so its own value. The bitsets are 8 bits per hash each, rounded up to a power of two (+0.6 MB peak RSS at 159 K keys, 0 at 763 K, +3.1 MB at 1.9 M keys). If more than a quarter of the hashes land on an already-set bit the filter is not filtering (a document that repeats most of its keys, or keys chosen to share low hash bits), so the whole list is sorted instead, bounding the worst case at the bitset pass plus the sort it was meant to avoid.

#### Results

Release build, `scripts/ab-cli.py` (interleaved, output identity gated on every row, 21 reps, 7 on the 100 MB rows), base `ce09e00da` vs this change. A control run (the base binary against itself) read -0.2%..+0.7% on the 7950X. The 7950X and M4 Pro columns below are medians of the 21 interleaved reps.

| row (7950X, median)                  | 2 MB (159 K keys) | 10 MB (763 K) | 16 MB (1.2 M) | 26 MB (1.9 M) | 100 MB (7.1 M) |
|--------------------------------------|------------------:|--------------:|--------------:|--------------:|---------------:|
| `length`                             |             -9.3% |         -9.6% |         -9.4% |         -9.2% |          -2.7% |
| `keys \| length`                     |             -9.7% |        -10.2% |         -9.5% |         -8.6% |                |
| `.`                                  |             -6.3% |         -6.2% |         -4.9% |         -5.6% |          -3.4% |
| `keys_unsorted`                      |             -2.9% |         -2.6% |         -3.3% |         -2.0% |          -2.2% |
| `to_entries` (does not use `census`) |             -0.2% |         +0.6% |         -0.3% |         +0.1% |                |

Objects too small to matter are not charged: `map(length) \| add` over equal objects of 8, 32, 128, 512, 2,048, 8,192 and 32,768 keys reads -2.4%, -3.3%, -4.5%, -6.2%, -8.2%, -9.5% and -10.6% on the 7950X, and below 128 keys the plain sort still runs (measured: with the prefilter on, 8 keys per object was +1.3% to +2.3% slower, 32 a wash). Other shapes on the 7950X: `users` `length` -5.2%, `.` -0.7%, `keys_unsorted` -5.4%; `wide-escaped-keys` `length` -7.5%, `.` -4.1%, `keys_unsorted` -1.5%.

| row (M4 Pro, median)                 | 2 MB (159 K keys) | 10 MB (763 K) | 16 MB (1.2 M) | 26 MB (1.9 M) | 100 MB (7.1 M) |
|--------------------------------------|------------------:|--------------:|--------------:|--------------:|---------------:|
| `length`                             |             -9.0% |        -12.3% |         -5.3% |         -6.3% |          -5.3% |
| `keys \| length`                     |             -9.2% |        -12.8% |         -5.4% |         -7.5% |                |
| `.`                                  |             -6.8% |         -8.3% |         -3.6% |         -4.0% |          -4.3% |
| `keys_unsorted`                      |             -1.3% |         -1.6% |         +0.1% |         +0.8% |          +1.0% |
| `to_entries` (does not use `census`) |             +0.4% |         -1.6% |         -2.1% |         -2.8% |                |

On the M4 Pro (control -0.5%..+0.5%) `map(length) \| add` over equal objects reads -1.4%, -4.8%, -6.1%, -9.5%, -10.2%, -10.4% and -11.8% at 8 to 32,768 keys; `users` `length` -0.3%, `.` -1.9%, `keys_unsorted` -0.9%; `wide-escaped-keys` `length` -7.0%, `.` -4.7%, `keys_unsorted` -1.0%. The 100 MB `keys_unsorted` row (+1.0% median, +1.8% min over 7 reps) is the one row on either box that is slower than its control's range; it is above the prefilter's gate, so what it measures is the key-end change and layout, and the 7950X reads -2.2% on the same row.

Instructions (deterministic): 7950X cachegrind `Ir` on `wide` 2 MB, `length` 245.6 M -> 215.7 M (-12.2%), `.` 459.8 M -> 429.8 M (-6.5%), `keys_unsorted` 282.6 M -> 275.7 M (-2.5%); `users` `length` 0.0%, `.` -1.0%. Apple M5 Max `time -l` instructions retired, same file: `length` 231.1 M -> 202.9 M (-12.2%), `.` 419.7 M -> 391.9 M (-6.6%), `keys_unsorted` -2.8%; 10 MB `length` -15.0%, `.` -7.8%. `scripts/perf-guard.py --check` against a base binary on the 7950X: every row inside its threshold (`wide_identity` -6.5%, inside its 10% override; `wide_keys_unsorted` -2.5%).

#### What did not ship

Each sort candidate was measured against `sort_unstable` in one binary (a temporary switch, so layout is shared), interleaved, before any was kept; the gap fast path against the same tree without it. Both boxes unless a cell says otherwise:

| candidate                                                             | 7950X                                                  | M4 Pro                                   | verdict                                                                              |
|-----------------------------------------------------------------------|--------------------------------------------------------|------------------------------------------|--------------------------------------------------------------------------------------|
| a table sized exactly from the `Vec` (#1514's question at 159 K keys) | `length` +3.8% median at 2-16 MB, +12.2% at 7.1 M keys | -2.9% median at 2-16 MB, -5.1% at 7.1 M  | architecture split: not shipped                                                      |
| in-place radix partition on the top 12 bits, then per-bucket sort     | +5.6% median at 2-16 MB, +37.9% at 7.1 M               | +8.5% median at 2-16 MB, +26.5% at 7.1 M | loses on both                                                                        |
| bitset prefilter, 16 bits per hash                                    | -2.2% median (2-16 MB)                                 | -5.6% median (2-16 MB)                   | superseded by 8 bits                                                                 |
| 8 bits per hash instead of 16                                         | a further -1.4% median                                 | not re-measured                          | shipped                                                                              |
| 32 bits per hash instead of 16                                        | +4.2% median (+8.8% at 16 MB)                          | not measured                             | larger bitsets (1 MB at 159 K keys, a 7950X core's whole L2; mechanism not isolated) |
| prefilter above 2^21 hashes (7.1 M keys, 16 bits per hash)            | +6.6%                                                  | not measured                             | gate stays at 2^21                                                                   |
| a canonical-gap fast path in `preceding_gap_ok` / `following_gap_ok`  | `length` -0.3%..+0.1% (no gain), **`.` +3.2%..+3.9%**  | not measured                             | not shipped                                                                          |

The canonical echo's duplicate-key gate (#3333, "Wide objects: one exactly sized table", earlier on this page) went the other way at its own site, where an exactly sized table beat a sort: that gate probes span fingerprints as it scans a document it is about to echo, not a `Vec` of hashes collected for a count, and it was measured on both boxes separately. Nothing here changes it.

The table is the architecture split `KeyHashes` records for #1514 reproduced at 159 K keys, a size the issue said had never been measured: a table that fits an M4 Pro's cache is a random-access loss on a 7950X's 1 MB L2. The prefilter wins on both because its probes are independent reads of a bitset that stays in cache and only a few percent of the list is left to sort. The fast-path row is the case for timing a change whose instruction count falls: 446.6 M -> 441.1 M `Ir` on the identity print (-1.2%), and +3.2%..+3.9% wall-clock.

The second suggestion in the issue, running the #1677 checks inside the identity writer's own per-field walk, is not done: it would change *when* a malformed member is reported, which `test_identity_writer_streams_or_materializes_by_gate_2720` pins. With the second quote scan gone the checks are no longer the cost it was written to remove.

Pinned by `repeated_hashes_tests` (the prefilter equals sort-then-`shared_hashes` for no repeat, two repeats, one value three times, every value twice, all equal and zero, at sizes either side of the 128 floor and either side of the 2^21 ceiling; hashes sharing every low bit; a mostly-repeated list that makes it give up), `key_hash_and_end_tests` (the helper equals the two separate calls, and takes the one-scan path only on an escape-free ASCII key) and `key_raw_unescaped_with_end_declines_what_the_two_calls_would_split_3343` (an unterminated or escaped span declines). A differential sweep of 122 documents (unique, duplicate, escaped-duplicate, non-ASCII, pretty, nine malformed-member shapes, each at 127 to 70,000 keys) across nine queries matched the base binary's stdout, stderr and exit code on all 1,098 comparisons, and `length` / `keys | length` matched jq 1.7.1 on every well-formed one; breaking the distinct count or the key-end arithmetic in a scratch copy fails it.

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
