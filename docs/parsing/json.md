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
- **No variant is neutral-or-better on every shape on both boxes.** A scalar prefix hides the entry cost on aarch64 but keeps only the 64-byte win, and on x86 every prefix regresses tiny strings and short keys in Ir. So nothing ships. The per-byte loop stays, and the shape that would remove the per-call entry entirely (classify the span's string specials in one pass) is [#3340](https://github.com/rust-works/succinctly/issues/3340).

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

The Ir ratio is the same at 2 MB and 10 MB, as a per-number constant-factor saving should be. `ab-cli.py --control` read −1.5%..+0.5% on the 7950X and −0.3%..+0.3% on the M4 Pro. A first build of the same change read −5%..−9% wall on the 7950X; this one, with identical Ir, reads −0.1%..−6.4%, so the 7950X's wall column carries the layout band this page's #2720 notes describe, and the Ir column is the attribution. `scripts/perf-guard.py` stays within threshold (`users_compact_identity` −3.2%, `users_compact_latefail` −0.9%). The per-object zeroing of `scan_canonical_object`'s small-tier arrays is not visible in any profile (inline stores, no `memset` call). The >16-key `KeyHashes::insert` tier is now the largest gate term on a wide object, 83 Ir per key and 6.9% of `wide`'s run, and is [#3333](https://github.com/rust-works/succinctly/issues/3333).

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
