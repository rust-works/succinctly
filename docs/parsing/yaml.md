# YAML Parsing: Feasibility Analysis

This document investigates whether succinctly's semi-indexing techniques can be applied to YAML parsing.

## Executive Summary

**Conclusion**: YAML semi-indexing is theoretically possible but significantly more complex than JSON due to context-sensitive grammar and character ambiguity.

| Aspect | JSON | YAML |
|--------|------|------|
| Character disambiguation | Trivial | Requires oracle |
| SIMD parallelization | Full (simdjson: 2+ GB/s) | Limited (rapidyaml: ~150 MB/s) |
| Extra index storage | None | 1-2 bits per structural position |
| Implementation complexity | Moderate | High |

---

## Architecture: Oracle + Index Model

YAML parsing can be split into two phases:

```
┌─────────────────────────────────────────┐
│            YAML Source                  │
│  users:                                 │
│    - name: Alice                        │
│      age: 30                            │
└─────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────┐
│     Oracle (context-aware parser)       │
│  - Tracks indentation levels            │
│  - Resolves character ambiguity         │
│  - Emits IB/OP/CL bits                  │
│  - Emits type bits (TY)                 │
└─────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────┐
│         Semi-Index (IB, BP, TY)         │
│  - Interest bits mark structural bytes  │
│  - BP encodes tree via virtual brackets │
│  - Type bits disambiguate container kind│
└─────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────┐
│     Oracle-free Navigation API          │
│  - parent(), first_child(), sibling()   │
│  - locate_offset() → ".users[0].name"   │
│  - All O(1) or O(log n)                 │
└─────────────────────────────────────────┘
```

The oracle runs once (sequentially), then all subsequent queries use the index without re-parsing.

---

## Virtual Bracket Insertion

YAML uses indentation to denote structure. The oracle converts this to virtual brackets for BP tree construction.

### Example

```yaml
users:
  - name: Alice
    age: 30
  - name: Bob
```

Oracle emits virtual structure:
```
{ users: [ { name: Alice, age: 30 }, { name: Bob } ] }
         ↑ ↑                      ↑  ↑           ↑ ↑
         │ │                      │  │           │ └─ dedent: close mapping
         │ │                      │  └─ indent: open mapping (2nd item)
         │ │                      └─ dedent: close mapping (1st item)
         │ └─ indent: open mapping (1st item)
         └─ "-" sequence item: open sequence
```

The virtual brackets don't exist in the source - they're recorded in the BP index at positions where indentation changes occur.

### Indentation Rules

| Indentation Change | Virtual Bracket |
|-------------------|-----------------|
| Increase | Open `(` - entering new container |
| Decrease | Close `)` - returning to parent |
| Same level | Sibling - no bracket |
| Sequence `-` | Open item within sequence |

---

## Character Ambiguity Analysis

Unlike JSON where each structural character has unambiguous meaning, YAML characters require context.

### 1. Colon `:` - Most Ambiguous

| Context | Meaning | Example |
|---------|---------|---------|
| After key, before value | Mapping indicator | `name: Alice` |
| In flow context with space | Mapping indicator | `{a: 1}` |
| Inside unquoted scalar | Literal character | `time: 12:30:00` |
| Inside quoted string | Literal character | `"a:b"` |
| In URL | Literal character | `url: http://x.com` |

**Resolution**: Oracle must track whether we're in a scalar context and whether `:` is followed by whitespace or end-of-line.

### 2. Hyphen `-` - Sequence vs Scalar

| Context | Meaning | Example |
|---------|---------|---------|
| At line start + space | Sequence item | `- item` |
| In flow sequence | Not structural | `[a, b, c]` |
| Negative number | Literal | `value: -5` |
| Date component | Literal | `date: 2024-01-15` |
| In string | Literal | `name: foo-bar` |
| Block scalar indicator | Chomping modifier | `\|-` or `>-` |

**Resolution**: Oracle checks line position, preceding whitespace, and current block context.

### 3. Question Mark `?` - Explicit Key

| Context | Meaning | Example |
|---------|---------|---------|
| At line start + space | Explicit key indicator | `? complex key` |
| Inside scalar | Literal | `query: what?` |

**Resolution**: Same as `-`, check line position and whitespace.

### 4. Hash `#` - Comment

| Context | Meaning | Example |
|---------|---------|---------|
| After whitespace | Comment start | `value # comment` |
| Inside quoted string | Literal | `"#hashtag"` |
| Inside unquoted (no prior space) | Literal | `color: #ff0000` |

**Resolution**: Oracle tracks quote state and preceding whitespace.

### 5. Flow Delimiters `[` `]` `{` `}` `,`

| Context | Meaning | Example |
|---------|---------|---------|
| Flow context | Structural | `items: [1, 2, 3]` |
| Inside quoted string | Literal | `"array[0]"` |
| Inside unquoted scalar | Depends | Implementation-specific |

**Resolution**: Oracle tracks flow nesting depth and quote state.

### 6. Block Scalar Indicators `|` `>`

| Context | Meaning | Example |
|---------|---------|---------|
| After key, alone | Block scalar start | `text: \|` |
| With modifier | Block + chomping | `text: \|+` |
| Inside scalar | Literal | `cmd: a \| b` |

**Resolution**: Oracle checks position relative to `:` and line structure.

### 7. Anchors/Aliases `&` `*`

| Context | Meaning | Example |
|---------|---------|---------|
| Before node | Anchor/alias | `&anchor value` |
| Inside scalar | Literal | `expr: a & b` |

**Resolution**: Oracle checks preceding characters.

### 8. Newline + Indentation

| Context | Meaning | Example |
|---------|---------|---------|
| Normal context | Structure boundary | Child/sibling/parent |
| Inside block scalar | Content continuation | Multi-line string |
| Inside flow context | Ignored | `{a: 1,`<br>`b: 2}` |

**Resolution**: Oracle tracks block scalar state and flow depth.

---

## Additional Index Storage Required

Unlike JSON where BP + IB are sufficient, YAML requires extra type information.

### Option 1: Type Bit Vector (TY)

One extra bit per structural position:

| TY[k] | Meaning |
|-------|---------|
| 0 | Mapping (object-like) |
| 1 | Sequence (array-like) |

**Overhead**: +1 bit per structural position (~0.1% of input)

### Option 2: Two-Bit Type Field

| Code | Meaning |
|------|---------|
| 00 | Block mapping |
| 01 | Block sequence |
| 10 | Flow mapping |
| 11 | Flow sequence |

**Overhead**: +2 bits per structural position (~0.2% of input)

### Why This Is Needed

In JSON, looking up `source[ib_select1(k)]` tells you unambiguously:
- `{` → object open
- `[` → array open
- `"` → string start

In YAML, looking at the byte is ambiguous:
- `n` could start `name:` (mapping key) or `null` (scalar) or `- name:` (sequence item)
- `-` could be sequence indicator or part of a value

The TY bits resolve this without rescanning context.

---

## Comparison: JSON vs YAML Semi-Indexing

| Component | JSON | YAML |
|-----------|------|------|
| **IB (Interest Bits)** | Marks `{`, `}`, `[`, `]`, value starts | Marks indent changes, `-`, `:`, key starts |
| **BP (Balanced Parens)** | Direct from brackets | Virtual from indentation |
| **TY (Type Bits)** | Not needed | Mapping vs Sequence per position |
| **ib_select1(k)** | Returns byte offset | Returns byte offset |
| **bp[k]** | Open/close | Open/close |
| **source[ib_select1(k)]** | Unambiguous character | Ambiguous - needs TY[k] |

---

## Performance Expectations

### Oracle (Index Building)

| Parser | Language | Throughput |
|--------|----------|------------|
| rapidyaml | C++ | ~150 MB/s (YAML), ~450 MB/s (JSON mode) |
| libyaml | C | ~114 MB/s |
| VYaml | C# | 6x faster than YamlDotNet |
| simdjson | C++ | 2-3 GB/s (JSON only) |

The oracle would likely achieve ~100-200 MB/s, similar to rapidyaml.

### Index Queries (Post-Oracle)

| Operation | Complexity | Notes |
|-----------|------------|-------|
| `parent(k)` | O(1) amortized | Same as JSON |
| `first_child(k)` | O(1) | Same as JSON |
| `next_sibling(k)` | O(1) | Same as JSON |
| `locate_offset(n)` | O(depth) | Same as JSON |
| `is_sequence(k)` | O(1) | Requires TY lookup |

---

## Why YAML Can't Match JSON Performance

### 1. Context Sensitivity

From the YAML 1.2 spec:
> "Context sensitivity is the cause of most of the complexity of the YAML specification."

JSON structural characters can be identified without context. YAML's cannot.

### 2. Unlimited Lookahead

You can't know if you're starting a mapping until you see `:`, which may be arbitrarily far ahead:

```yaml
this is a very long unquoted key that continues: value
```

This breaks the "structural indexing first" approach that makes simdjson fast.

### 3. Indentation as Structure

Whitespace defines nesting. This requires:
- Line-by-line processing (can't skip ahead)
- Stack-based indentation tracking (inherently sequential)
- Special handling for block scalars where indentation is content

### 4. Specification Complexity

| Metric | JSON | YAML 1.2 |
|--------|------|----------|
| Spec word count | 1,969 | 23,449 |
| String syntaxes | 1 | 63 (!) |
| Test suite cases | ~300 | ~3,000+ |

---

## Implementation Strategy

If implementing YAML semi-indexing:

### Phase 1: Use Existing Parser as Oracle

1. Wrap rapidyaml or libyaml
2. Walk their event stream
3. Emit IB, BP, TY bits at appropriate positions
4. Build rank/select indices

**Benefit**: Leverage battle-tested YAML parsing. Focus on index construction.

### Phase 2: Restricted YAML Subset

Many config files use only:
- Block style (no `{` or `[`)
- Simple scalars (no multiline, no anchors)
- Consistent 2-space indentation

A "YAML-lite" oracle could be much simpler and faster.

### Phase 3: Custom Oracle (Optional)

Full YAML 1.2 parser optimized for index emission:
- Table-driven state machine (like PFSM)
- SIMD for newline/whitespace detection
- Streaming indentation tracker

---

## Potential Optimizations

### SIMD-Accelerated Components

| Component | Technique |
|-----------|-----------|
| Newline detection | SIMD compare + movemask |
| Whitespace counting | SIMD compare + popcount |
| Quote boundary detection | Same as JSON |
| Comment detection | SIMD search for `#` after whitespace |

### Speculative Parsing

Assume common patterns, fall back on mismatch:
- 2-space indentation
- No flow style mixing
- No anchors/aliases
- No multiline strings

---

## Conclusion

YAML semi-indexing is feasible with these caveats:

1. **Oracle is sequential**: Can't parallelize like simdjson (~150 MB/s vs 2+ GB/s)
2. **Extra index storage**: Need TY bits for type disambiguation
3. **Implementation complexity**: Much higher than JSON due to grammar
4. **Value proposition**: Strong for repeated queries on same document

### Recommended Approach

For a YAML query tool similar to `jq`:

1. Use rapidyaml as the oracle (proven, fast, correct)
2. Build IB/BP/TY indices from event stream
3. Reuse existing BP navigation code unchanged
4. Add TY-aware path construction for `locate_offset()`

The semi-index enables fast repeated queries without re-parsing, even though initial indexing is slower than JSON.

---

## Outstanding Implementation Considerations

These considerations should be addressed before implementation begins.

### 1. Anchors and Aliases (`&` / `*`)

**Decision**: Index anchors and aliases as structural elements without expansion.

Anchors create DAG structures (not trees):

```yaml
defaults: &defaults
  timeout: 30
production:
  <<: *defaults
  host: prod.example.com
```

**Approach**:
- Record anchor definitions (`&name`) as structural positions in IB
- Record alias references (`*name`) as leaf nodes pointing to anchor positions
- Store anchor name → BP position mapping in auxiliary structure
- Navigation follows the physical structure (no automatic expansion)

**Trade-offs**:

| Approach | Pros | Cons |
|----------|------|------|
| No expansion (chosen) | Preserves source structure, low memory, accurate `locate_offset()` | Higher-level code must resolve aliases |
| Expand during indexing | Simpler queries, tree structure | Memory overhead, lose source fidelity |
| Reject anchors | Simplest implementation | Incompatible with many real YAML files |

The cursor API will expose `is_anchor()`, `is_alias()`, and `resolve_alias()` methods. Expansion can be implemented at a higher level if needed.

### 2. Block Scalar Boundaries

Block scalars (`|` and `>`) require special handling:

```yaml
script: |
  echo "hello"
  echo "world"
next_key: value
```

The oracle must track:
- Block scalar indicator position
- Indentation level that ends the scalar
- Chomping modifier (`+`, `-`, or default)

**Trade-offs for close bracket placement**:

| Placement | Pros | Cons |
|-----------|------|------|
| At last content line | Byte range matches visible content | Requires lookahead to determine end |
| At first dedent | Natural parsing boundary | May include trailing newlines |
| At indicator position (degenerate) | Simple | Loses content boundary info |

**Decision**: Place close bracket at the byte position of the last content character (before chomping). This gives accurate byte ranges for `locate_offset()` and content extraction.

### 3. Multi-Document Streams

**Decision**: Treat multi-document streams as an implicit array of documents.

YAML supports multiple documents in one file:

```yaml
---
doc1: value
---
doc2: value
...
```

**Approach**:
- Wrap all documents in a virtual root sequence
- Each `---` marker opens a new array element
- Single-document files (no `---`) are wrapped in a 1-element array for consistency
- `...` (document end) closes current element

**Trade-offs**:

| Approach | Pros | Cons |
|----------|------|------|
| Array wrapper (chosen) | Uniform API, consistent navigation | Extra root node, paths start with `[0]` |
| Separate indices | Independent documents | Multiple index structures, complex API |
| Reject multi-doc | Simple | Incompatible with many YAML files |

The array wrapper approach means `.` always refers to the document array, and `.[0]` is the first document. This matches common tools like `yq`.

### 4. Key Extraction

JSON keys are quoted strings with defined boundaries. YAML keys can be:
- Unquoted: `name: value`
- Quoted: `"name": value` or `'name': value`
- Multi-line: `? long key\n  continues`
- Complex: `[1, 2]: value`

**Proposed Solutions**:

| Key Type | Extraction Strategy |
|----------|---------------------|
| Unquoted simple | Scan from IB position to `:` (minus trailing whitespace) |
| Double-quoted | Same as JSON: scan for closing `"`, handle escapes |
| Single-quoted | Scan for closing `'`, handle `''` escape |
| Explicit (`?`) | IB marks `?`, scan to next `:` at same indent |
| Complex | Store key byte range in auxiliary structure |

**Index Augmentation**:

For efficient key extraction, store key end positions:
- Add `KE` (Key End) bit vector marking the byte after each key
- Alternatively, store key lengths in a separate array
- Trade-off: ~0.1% extra overhead vs O(n) scan per key lookup

**Recommendation**: For Phase 1, use linear scan (keys are typically short). Add KE index in Phase 2 if profiling shows key extraction is a bottleneck.

### 5. YAML Version

**Decision**: Target YAML 1.2 exclusively.

| Feature | YAML 1.1 | YAML 1.2 (chosen) |
|---------|----------|-------------------|
| Boolean literals | `yes`, `no`, `on`, `off`, `y`, `n` | Only `true`, `false` |
| Octal numbers | `010` = 8 | `0o10` = 8 |
| Sexagesimal | `1:30:00` = 5400 | Literal string |
| JSON compatibility | Partial | Full superset |

**Rationale**:
- YAML 1.2 is cleaner and more predictable
- Full JSON compatibility (every valid JSON is valid YAML 1.2)
- Avoids "Norway problem" (`NO` → `false` in 1.1)
- Modern tools (GitHub Actions, Kubernetes) increasingly use 1.2 semantics

**Note**: Value type interpretation (is `true` a boolean or string?) is handled at a higher level than the cursor API. The semi-index is concerned only with structure, not semantics.

### 6. Tag Resolution

**Decision**: The cursor API is type-agnostic. Tags are recorded structurally but interpretation is deferred.

YAML tags affect interpretation:

```yaml
date: 2024-01-15      # Might be Date or String
explicit: !!str 2024-01-15  # Definitely String
```

**Cursor API Scope**:

The semi-index cursor provides:
- `raw_value() -> &[u8]` - Raw bytes of scalar
- `has_tag() -> bool` - Whether explicit tag present
- `tag() -> Option<&str>` - The tag if present (`!!str`, `!custom`, etc.)

The cursor does **not** provide:
- `as_bool()`, `as_int()`, `as_date()` - Type coercion
- Automatic type inference based on YAML 1.2 core schema

**Rationale**:

| Concern | Cursor API | Higher-level API |
|---------|------------|------------------|
| Structure navigation | ✓ | ✓ |
| Byte ranges | ✓ | ✓ |
| Raw value access | ✓ | ✓ |
| Tag presence | ✓ | ✓ |
| Type coercion | ✗ | ✓ |
| Schema validation | ✗ | ✓ |

This separation keeps the index small and fast. A higher-level `YamlValue` type can implement schema-aware parsing on top of the cursor.

### 7. Testing Strategy

**Decision**: Comprehensive testing using YAML 1.2 test suite with property-based testing.

**Test Sources**:

| Source | Cases | Usage |
|--------|-------|-------|
| YAML Test Suite | ~300 YAML 1.2 cases | Conformance testing |
| Property tests | Generated | Round-trip verification |
| Fuzzing | Random | Robustness |
| Real-world files | Curated | Practical coverage |

**YAML Test Suite Integration**:
- Use only YAML 1.2 compatible tests (skip 1.1-specific edge cases)
- Test categories: structure parsing, anchors/aliases, block scalars, flow style, multi-document
- Verify: correct BP tree structure, accurate byte ranges, `locate_offset()` paths

**Property-Based Testing**:
```
∀ yaml ∈ ValidYAML1.2:
  let index = build_index(yaml)
  let cursor = index.root()
  traverse(cursor) reconstructs original structure
  locate_offset(any_position) returns valid path
```

**Fuzzing Targets**:
- Deeply nested structures (stack overflow)
- Malformed UTF-8 sequences
- Truncated input
- Mismatched indentation

### 8. Error Handling

**Decision**: Strict mode only. Reject malformed YAML with clear error messages.

**Rationale**:
- Consistent with JSON implementation behavior
- Lenient parsing creates ambiguous index structures
- Users can pre-validate with external tools if needed
- Simpler implementation, fewer edge cases

**Error Reporting**:
```rust
pub enum YamlError {
    InvalidIndentation { line: usize, expected: usize, found: usize },
    UnexpectedCharacter { offset: usize, char: char, context: &'static str },
    UnclosedQuote { start_offset: usize, quote_type: char },
    InvalidEscape { offset: usize, sequence: String },
    TabIndentation { line: usize },  // YAML forbids tabs for indentation
    // ...
}
```

Errors include byte offset and line number for IDE integration.

### 9. Performance Targets

| Component | Target | Rationale |
|-----------|--------|-----------|
| Oracle (indexing) | 100-200 MB/s | Match rapidyaml |
| Queries | O(1) | Match JSON implementation |
| Memory overhead | <5% of input | IB + BP + TY |

### 10. Phased Implementation Plan

Implementation proceeds in phases, each building on the previous and adding YAML features incrementally.

---

#### Phase 1: YAML-lite Core

**Scope**: Minimal viable YAML parser covering common config file patterns.

**Features**:
- Block mappings and sequences only (no flow style `{}` `[]`)
- Simple scalars (unquoted, double-quoted, single-quoted)
- Comments (ignored)
- Single document only

**Not Supported**:
- Flow style collections
- Block scalars (`|`, `>`)
- Anchors/aliases
- Explicit tags
- Multi-document streams
- Explicit keys (`?`)

**Deliverables**:
- `YamlIndex::build(yaml: &[u8]) -> Result<YamlIndex, YamlError>`
- `YamlCursor` with `first_child()`, `next_sibling()`, `parent()`
- `locate_offset(offset) -> Option<String>`
- Basic test suite (~50 cases)

**Coverage**: Estimated 70% of real-world config files (Kubernetes, GitHub Actions, Docker Compose basics).

---

#### Phase 2: Flow Style + Block Scalars

**Scope**: Full YAML 1.2 structural support (excluding anchors).

**New Features**:
- Flow mappings: `{key: value, ...}`
- Flow sequences: `[item, ...]`
- Block literal scalars: `|`, `|+`, `|-`
- Block folded scalars: `>`, `>+`, `>-`
- Explicit keys: `? key`

**Index Changes**:
- TY bits distinguish block vs flow containers
- Block scalar byte ranges tracked accurately

**Deliverables**:
- Extended cursor API for flow/block distinction
- Block scalar content extraction (respecting chomping)
- YAML Test Suite integration (~150 cases)

**Coverage**: Estimated 95% of real-world YAML files.

---

#### Phase 3: Anchors and Aliases

**Scope**: Full anchor/alias support with DAG navigation.

**New Features**:
- Anchor definitions: `&anchor_name`
- Alias references: `*anchor_name`
- Merge keys: `<<: *alias`

**Index Changes**:
- Anchor registry: `HashMap<String, BpPosition>`
- Alias nodes stored as structural positions
- `is_anchor()`, `is_alias()`, `resolve_alias()` methods

**Deliverables**:
- Anchor/alias cursor methods
- Optional expansion helper for tree-like traversal
- Test cases for complex anchor patterns

---

#### Phase 4: Multi-Document Streams

**Scope**: Support for multi-document YAML files.

**New Features**:
- Document start marker: `---`
- Document end marker: `...`
- Implicit root array containing documents

**Index Changes**:
- Virtual root sequence wrapping all documents
- Document boundary positions in IB

**Deliverables**:
- Multi-document test cases
- `document_count()` API
- Per-document iteration

---

#### Phase 5: Performance Optimization

**Scope**: SIMD acceleration and memory optimization to match or exceed rapidyaml (~150 MB/s).

See [High-Performance Pure Rust Oracle](#high-performance-pure-rust-oracle) section below for detailed techniques.

**Targets**:
- Oracle throughput: 150+ MB/s
- Memory overhead: <4% of input

**Deliverables**:
- Benchmark suite comparing to rapidyaml
- Platform-specific optimizations (x86_64, aarch64)

---

#### Phase Summary

| Phase | Features | Estimated Coverage | Complexity |
|-------|----------|-------------------|------------|
| 1 | Block style, simple scalars | 70% | Low |
| 2 | Flow style, block scalars | 95% | Medium |
| 3 | Anchors/aliases | 99% | Medium |
| 4 | Multi-document | 100% | Low |
| 5 | SIMD optimization | 100% | High |

Each phase is independently useful and can be released separately.

---

## Implemented: SIMD Optimizations (Phase 1.5)

The YAML parser includes SIMD-accelerated operations for hot paths in parsing.

### Implementation

Located in `src/yaml/simd/`:
- `mod.rs` - Platform dispatch and scalar fallbacks
- `neon.rs` - ARM NEON implementation (16 bytes/iteration)
- `x86.rs` - SSE2/AVX2 implementation (16-32 bytes/iteration)

### Functions

```rust
/// Find next `"` or `\` in double-quoted strings
pub fn find_quote_or_escape(input: &[u8], start: usize, end: usize) -> Option<usize>;

/// Find next `'` in single-quoted strings
pub fn find_single_quote(input: &[u8], start: usize, end: usize) -> Option<usize>;

/// Count leading spaces (indentation) from position
pub fn count_leading_spaces(input: &[u8], start: usize) -> usize;
```

### Platform-Specific Details

#### ARM64 (NEON)

Uses the multiplication trick for movemask (no native `movemask` on ARM):

```rust
const MAGIC: u64 = 0x0102040810204080;
let low_packed = (low_u64.wrapping_mul(MAGIC) >> 56) as u8;
let high_packed = (high_u64.wrapping_mul(MAGIC) >> 56) as u8;
(low_packed as u16) | ((high_packed as u16) << 8)
```

#### x86_64 (SSE2/AVX2)

- Runtime feature detection for AVX2
- SSE2 baseline (16 bytes/iteration)
- AVX2 fast path (32 bytes/iteration) when available

### Benchmark Results

#### Apple M1 Max (ARM64 NEON)

##### String Scanning

| Benchmark              | Scalar         | SIMD           | Improvement    |
|------------------------|----------------|----------------|----------------|
| yaml/quoted/double/10  | 1.57µs         | 1.44µs         | **8-9% faster**|
| yaml/quoted/double/100 | 7.57µs         | 6.97µs         | **8-9% faster**|
| yaml/quoted/double/1000| 67.1µs (688 MiB/s) | 63.0µs (738 MiB/s) | **7.3% throughput** |

##### Indentation Scanning

End-to-end yq identity filter benchmarks after adding SIMD indentation:

| Pattern       | Size  | Before       | After        | Improvement     |
|---------------|-------|--------------|--------------|-----------------|
| comprehensive | 1KB   | 4.15 ms      | 3.58 ms      | **+14-24% faster** |
| comprehensive | 10KB  | 4.91 ms      | 4.23 ms      | **+14-18% faster** |
| comprehensive | 100KB | 11.40 ms     | 10.73 ms     | **+5-8% faster**   |
| users         | 1KB   | 3.91 ms      | 3.43 ms      | **+12-16% faster** |
| users         | 10KB  | 4.76 ms      | 4.26 ms      | **+10-14% faster** |

#### AMD Ryzen 9 7950X (x86_64 AVX2/AVX-512) - Baseline (2026-01-17)

**Platform:** AMD Ryzen 9 7950X 16-Core (AVX-512, BMI2, POPCNT)
**OS:** Linux 6.6.87 WSL2
**Compiler:** rustc with `-C target-cpu=native`

##### Simple Key-Value Pairs
| Count  | Time      | Throughput   |
|--------|-----------|--------------|
| 10     | 702 ns    | 176 MiB/s    |
| 100    | 3.72 µs   | 379 MiB/s    |
| 1,000  | 34.98 µs  | 457 MiB/s    |
| 10,000 | 364 µs    | 491 MiB/s    |

##### Nested Structures
| Depth | Width | Time      | Throughput   |
|-------|-------|-----------|--------------|
| 3     | 3     | 3.31 µs   | 300 MiB/s    |
| 5     | 2     | 4.98 µs   | 340 MiB/s    |
| 10    | 2     | 196.6 µs  | 427 MiB/s    |
| 3     | 5     | 12.54 µs  | 342 MiB/s    |

##### Sequences
| Items  | Time     | Throughput   |
|--------|----------|--------------|
| 10     | 678 ns   | 112 MiB/s    |
| 100    | 3.28 µs  | 258 MiB/s    |
| 1,000  | 31.51 µs | 284 MiB/s    |
| 10,000 | 309 µs   | 290 MiB/s    |

##### Quoted Strings (Regular)
| Count | Type   | Time     | Throughput   |
|-------|--------|----------|--------------|
| 10    | Double | 712 ns   | 163 MiB/s    |
| 10    | Single | 698 ns   | 166 MiB/s    |
| 100   | Double | 4.56 µs  | 331 MiB/s    |
| 100   | Single | 4.41 µs  | 343 MiB/s    |
| 1,000 | Double | 42.1 µs  | 361 MiB/s    |
| 1,000 | Single | 41.3 µs  | 368 MiB/s    |

##### Long Quoted Strings (100 strings each)
| String Length | Type   | Time      | Throughput   |
|---------------|--------|-----------|--------------|
| 64 bytes      | Double | 5.39 µs   | 1.27 GiB/s   |
| 64 bytes      | Single | 5.41 µs   | 1.27 GiB/s   |
| 256 bytes     | Double | 11.56 µs  | 2.14 GiB/s   |
| 256 bytes     | Single | 11.32 µs  | 2.19 GiB/s   |
| 1024 bytes    | Double | 34.24 µs  | 2.81 GiB/s   |
| 1024 bytes    | Single | 34.27 µs  | 2.81 GiB/s   |
| 4096 bytes    | Double | 128.8 µs  | 2.97 GiB/s   |
| 4096 bytes    | Single | 128.5 µs  | 2.98 GiB/s   |

##### Large Files
| Size   | Time      | Throughput   |
|--------|-----------|--------------|
| 1 KB   | 2.79 µs   | 342 MiB/s    |
| 10 KB  | 21.3 µs   | 448 MiB/s    |
| 100 KB | 194.3 µs  | 491 MiB/s    |

**Summary:** Baseline throughput ranges from 176-491 MiB/s for structured data, with string scanning achieving 2.8-3.0 GiB/s. This establishes a performance baseline before AVX2/AVX-512 optimizations.

---

## x86_64 Optimization Implementation Plan

This section details planned SIMD and other optimizations for x86_64 (AMD Ryzen 9 7950X and similar), based on proven techniques from the JSON parser.

### Current SIMD Implementation (Baseline)

The YAML parser currently uses basic SIMD operations in [`src/yaml/simd/x86.rs`](../../src/yaml/simd/x86.rs:1):

| Function | SSE2 | AVX2 | Usage | Speedup |
|----------|------|------|-------|---------|
| `find_quote_or_escape` | ✓ | ✓ | Double-quoted string scanning | ~8-9% |
| `find_single_quote` | ✓ | ✓ | Single-quoted string scanning | ~8-9% |
| `count_leading_spaces` | ✓ | ✓ | Indentation counting | ~14-24% |

**Throughput:** 16 bytes/iteration (SSE2), 32 bytes/iteration (AVX2)

### JSON Parser Techniques (Proven on x86_64)

Analysis of [`src/json/simd/`](../../src/json/simd/) reveals these techniques:

#### 1. **Multi-Character Classification** ([x86.rs](../../src/json/simd/x86.rs:44-123), [avx2.rs](../../src/json/simd/avx2.rs:44-129))

JSON classifies 6 character classes in parallel per 16/32-byte chunk:
- Quotes (`"`)
- Backslashes (`\`)
- Opens (`{`, `[`)
- Closes (`}`, `]`)
- Delimiters (`,`, `:`)
- Value characters (alphanumeric, `.`, `-`, `+`)

**Technique:** Single SIMD load + multiple comparisons + movemask → 6 bitmasks

```rust
unsafe fn classify_chars(chunk: __m256i) -> CharClass {
    let eq_quote = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'"' as i8));
    let eq_backslash = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'\\' as i8));
    // ... more comparisons
    CharClass {
        quotes: _mm256_movemask_epi8(eq_quote) as u32,
        backslashes: _mm256_movemask_epi8(eq_backslash) as u32,
        // ... 4 more masks
    }
}
```

**Benefit:** 1 load + 6-8 vector ops = bulk classification

#### 2. **PFSM Tables** ([pfsm_tables.rs](../../src/json/pfsm_tables.rs:1-150))

Pre-computed state transition tables eliminate branching:

```rust
pub const TRANSITION_TABLE: [u32; 256];  // (byte, state) → next_state
pub const PHI_TABLE: [u32; 256];         // (byte, state) → output_bits (IB/BP)
```

**Access pattern:**
```rust
let trans = TRANSITION_TABLE[byte as usize];
let next_state = (trans >> (state * 8)) & 0xFF;
let phi = PHI_TABLE[byte as usize];
let output_bits = (phi >> (state * 8)) & 0x07;
```

**Benefit:** Branch-free state machine, predictable memory access

#### 3. **BMI2 Bit Manipulation** ([bmi2.rs](../../src/json/simd/bmi2.rs:1-200))

PDEP/PEXT for efficient bitmask manipulation:
- **PEXT:** Extract bits matching mask positions
- **PDEP:** Deposit bits to mask positions

**Warning:** AMD Zen 1/2 have 18-cycle PDEP/PEXT (microcode). Only use on:
- Intel Haswell+ (3 cycles)
- AMD Zen 3+ (3 cycles)

Current system (Ryzen 9 7950X = Zen 4) is **fast path eligible**.

#### 4. **Range-Based Character Detection**

Efficient alphanumeric detection using unsigned comparison trick:

```rust
// Check if c >= 'a' && c <= 'z'
let v_a = _mm256_set1_epi8(b'a' as i8);
let range = _mm256_set1_epi8((b'z' - b'a') as i8);
let sub_a = _mm256_sub_epi8(chunk, v_a);
let lowercase = _mm256_cmpeq_epi8(_mm256_min_epu8(sub_a, range), sub_a);
```

**Benefit:** Single comparison for range instead of 2 comparisons

---

### Proposed Optimizations for YAML (x86_64)

Based on baseline benchmarks and JSON techniques, here are high-value optimizations:

#### **Optimization 1: Multi-Character Classification**

**Target:** Core parsing loop in [`src/yaml/parser.rs`](../../src/yaml/parser.rs:666-1647)

**Approach:** Classify YAML structural characters in bulk, similar to JSON.

**YAML Structural Characters:**
| Character(s) | Meaning | Detection |
|-------------|---------|-----------|
| `:` + space | Mapping separator | Compare + AND with shifted space mask |
| `-` + space | Sequence item | Compare + AND with shifted space mask |
| `#` | Comment start | Compare |
| `"`, `'` | Quote delimiters | Compare (already done) |
| `\n` | Line boundary | Compare |
| ` ` (space) | Indentation/whitespace | Compare (already done) |
| `{`, `}`, `[`, `]` | Flow style | Compare |
| `\|`, `>` | Block scalars | Compare |
| `&`, `*` | Anchors/aliases | Compare |

**Implementation:**

```rust
#[derive(Debug, Clone, Copy)]
struct YamlCharClass {
    newlines: u32,       // \n
    colons: u32,         // :
    hyphens: u32,        // -
    spaces: u32,         // space (for indentation + context detection)
    quotes_double: u32,  // "
    quotes_single: u32,  // '
    backslashes: u32,    // \
    flow_open: u32,      // { or [
    flow_close: u32,     // } or ]
    block_scalar: u32,   // | or >
    anchors: u32,        // & or *
    hash: u32,           // # (comments)
}

#[target_feature(enable = "avx2")]
unsafe fn classify_yaml_chars_avx2(chunk: __m256i) -> YamlCharClass {
    // Similar to JSON classify_chars, but for YAML characters
    // ~12 comparisons, 1 load = still faster than byte-by-byte
}
```

**Context-Sensitive Detection:**
- `: ` pattern: `colon_mask & (space_mask << 1)` = positions where `:` followed by space
- `- ` pattern: `hyphen_mask & (space_mask << 1)` = sequence items

**Expected Improvement:** 10-20% faster parsing (reduces branches in hot loops)

**Files to Modify:**
- `src/yaml/simd/x86.rs` - Add `classify_yaml_chars_*` functions
- `src/yaml/parser.rs` - Use classification in parsing loops (lines 666-1647)

---

#### **Optimization 2: Newline Index with SIMD**

**Target:** Indentation tracking (currently line-by-line)

**Approach:** Pre-scan entire document for newlines using SIMD, build rank/select index.

**Algorithm:**

1. **SIMD Newline Detection** (32 bytes/iter with AVX2):
```rust
#[target_feature(enable = "avx2")]
unsafe fn find_newlines_avx2(input: &[u8]) -> Vec<u64> {
    let newline_vec = _mm256_set1_epi8(b'\n' as i8);
    let mut newline_bits = vec![0u64; input.len().div_ceil(64)];

    for (chunk_idx, chunk) in input.chunks(32).enumerate() {
        let data = _mm256_loadu_si256(chunk.as_ptr() as *const __m256i);
        let matches = _mm256_cmpeq_epi8(data, newline_vec);
        let mask = _mm256_movemask_epi8(matches) as u32;

        // Deposit mask into newline_bits at appropriate word/bit position
        // (details omitted for brevity)
    }
    newline_bits
}
```

2. **Build Lightweight Rank Index:**
```rust
pub struct NewlineIndex {
    newlines: Vec<u64>,         // Bit vector of newline positions
    cumulative_rank: Vec<u32>,  // Rank every 512 bits (8 words)
}
```

3. **O(1) Line Number Lookup:**
```rust
fn line_at_offset(&self, offset: usize) -> usize {
    // rank1(offset) = line number
    let word_idx = offset / 64;
    let base_rank = self.cumulative_rank[word_idx / 8];
    let residual = ...; // popcount within word
    base_rank + residual
}
```

**Expected Improvement:**
- Newline pre-scan: ~3-5% overhead
- Line lookups: O(n) → O(1) (useful for `locate_offset()`)
- Net: ~2-5% faster overall (amortized over large files)

**Files to Modify:**
- New file: `src/yaml/newline_index.rs`
- `src/yaml/parser.rs` - Use newline index for line tracking
- `src/yaml/locate.rs` - Use for O(1) line lookups

---

#### **Optimization 3: YFSM Tables (YAML Finite State Machine)**

**Target:** Replace branchy state machine with table lookups (like JSON PFSM)

**Challenge:** YAML has more states than JSON due to context sensitivity.

**JSON States (4):**
- InJson, InString, InEscape, InValue

**YAML States (8-10 needed):**
- Block, InKey, InValue, InString, InEscape, InSingle, FlowMap, FlowSeq, InBlockScalar, InComment

**Table Size:**
- Transition: `256 * 10 bytes = 2.5 KB` (fits in L1 cache)
- Phi output: `256 * 10 bytes = 2.5 KB`

**Challenge:** YAML's indentation sensitivity requires external stack state (indent levels).

**Hybrid Approach:**
1. **YFSM handles string/escape states** (table-driven, branch-free)
2. **Indentation stack remains** (inherently sequential, can't be table-ized)
3. **Structural characters use YFSM** for IB/BP emission

**Implementation:**

```rust
pub const YAML_TRANSITION_TABLE: [u64; 256];  // 8 states packed in u64
pub const YAML_PHI_TABLE: [u64; 256];         // Output bits per state

#[inline]
fn yfsm_step(byte: u8, state: YfsmState) -> (YfsmState, u8) {
    let trans = YAML_TRANSITION_TABLE[byte as usize];
    let next_state = ((trans >> (state as u32 * 8)) & 0xFF) as u8;

    let phi = YAML_PHI_TABLE[byte as usize];
    let output = ((phi >> (state as u32 * 8)) & 0xFF) as u8;

    (next_state.into(), output)
}
```

**Expected Improvement:** 15-25% faster (proven from JSON: PFSM gave 33-77% speedup)

**Files to Create/Modify:**
- New: `src/yaml/yfsm_tables.rs` - Pre-computed tables
- New: `src/bin/generate_yfsm_tables.rs` - Table generator
- `src/yaml/parser.rs` - Use YFSM in place of manual state transitions

---

#### **Optimization 4: Speculative Inline Parsing**

**Target:** Common YAML patterns (e.g., `key: value`)

**Approach:** Fast-path for simple key-value pairs without full state machine.

**Pattern Detection (SIMD-assisted):**

```rust
#[target_feature(enable = "avx2")]
unsafe fn is_simple_kv_line(line: &[u8]) -> Option<(usize, usize)> {
    // Find colon position using SIMD
    let colon_vec = _mm256_set1_epi8(b':' as i8);
    // ... (similar to find_quote_or_escape)

    // Verify: colon followed by space, no quotes before colon
    if colon_found && line[colon_pos + 1] == b' ' {
        return Some((indent, colon_pos));
    }
    None
}
```

**Fast Path:**
- Detect `  key: value` pattern
- Skip full parser, directly emit IB/BP bits
- Fall back to full parser for complex cases

**Expected Improvement:** 10-15% on config files (many simple KV pairs)

**Files to Modify:**
- `src/yaml/parser.rs` - Add speculative parse path before full parse

---

#### **Optimization 5: AVX-512 Exploration (Optional)**

**Platform:** AMD Ryzen 9 7950X has AVX-512 support (F, DQ, BW, VL, VBMI, VBMI2, VNNI).

**Approach:** Process 64 bytes/iteration instead of 32.

**Functions to AVX-512-ize:**
- `classify_yaml_chars_avx512` - 64-byte classification
- `find_newlines_avx512` - 64-byte newline scan
- `count_leading_spaces_avx512` - 64-byte space counting

**Warning from JSON experience:**
> "Wider SIMD != automatically faster (AVX-512 JSON was 10% slower than AVX2)"
> — [docs/optimisations/README.md](../optimisations/README.md)

**Reasons AVX-512 can be slower:**
- Frequency throttling (CPU reduces clock speed with AVX-512)
- Cache alignment issues
- Overhead of extracting 64-bit masks

**Recommendation:**
- Implement AVX-512 variants
- Benchmark against AVX2
- Only use if >5% faster
- Provide runtime dispatch

**Files to Create:**
- `src/yaml/simd/avx512.rs` - AVX-512 implementations (if beneficial)

---

### Implementation Priority

| Priority | Optimization | Expected Gain | Complexity | Dependencies |
|----------|--------------|---------------|------------|--------------|
| **P0** | Multi-Character Classification | 10-20% | Medium | None |
| **P0** | Enhance existing SIMD (quote/space) | 5-10% | Low | None |
| **P1** | YFSM Tables | 15-25% | High | P0 (classification) |
| **P2** | Newline Index | 2-5% | Medium | None |
| **P2** | Speculative Inline Parsing | 10-15% | Medium | P0 (classification) |
| **P3** | AVX-512 variants | 0-10% (uncertain) | Medium | P0, P1 |

**Total Expected Improvement:** 40-60% faster parsing (conservative estimate)

**Target:** 300-700 MiB/s (from current 176-491 MiB/s baseline)

---

### Benchmarking Strategy

After each optimization:

1. **Run micro-benchmarks:**
```bash
cargo bench --bench yaml_bench
```

2. **Compare against baseline:**
```bash
# Save results to dated file
cargo bench --bench yaml_bench > .ai/scratch/yaml_bench_after_P0_opt.txt
```

3. **Update docs with results** in this section

4. **Profile hot paths:**
```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release --example yaml_profile
./target/release/examples/yaml_profile data/bench/yaml/100kb.yaml
```

---

### Hardware-Specific Notes

**AMD Ryzen 9 7950X (Zen 4):**
- ✓ AVX2: 32 bytes/cycle (use this as primary path)
- ✓ AVX-512: 64 bytes/cycle (benchmark carefully)
- ✓ BMI2: 3-cycle PDEP/PEXT (Zen 3+, usable)
- ✓ POPCNT: 1-cycle (use liberally)
- L1 cache: 512 KB (16×32 KB) - tables fit easily
- L2 cache: 16 MB (16×1 MB) - excellent for working set
- L3 cache: 32 MB - shared, use for large buffers

**Optimization Guideline:**
- Favor AVX2 over AVX-512 unless proven faster
- Use BMI2 where appropriate (fast on Zen 4)
- Keep hot tables <32 KB for L1 residency
- Align buffers to 32-byte boundaries for AVX2

---

### References

**Implemented JSON Optimizations:**
- [SIMD x86 module](../../src/json/simd/x86.rs)
- [AVX2 module](../../src/json/simd/avx2.rs)
- [PFSM tables](../../src/json/pfsm_tables.rs)
- [BMI2 utilities](../../src/json/simd/bmi2.rs)

**Optimization Documentation:**
- [SIMD techniques](../optimisations/simd.md)
- [Bit manipulation](../optimisations/bit-manipulation.md)
- [State machines](../optimisations/state-machines.md)
- [Optimization overview](../optimisations/README.md)

### Integration with Parser

#### String Scanning

The `parse_double_quoted()` and `parse_single_quoted()` functions use SIMD to skip to the next interesting character:

```rust
fn parse_double_quoted(&mut self) -> Result<usize, YamlError> {
    self.advance(); // Skip opening quote
    loop {
        // SIMD fast-path: find next quote or backslash
        if let Some(offset) = simd::find_quote_or_escape(self.input, self.pos, self.input.len()) {
            self.advance_by(offset);
            match self.peek() {
                Some(b'"') => { self.advance(); return Ok(self.pos - start); }
                Some(b'\\') => { /* handle escape */ }
                _ => { self.advance(); }
            }
        } else {
            return Err(YamlError::UnclosedQuote { ... });
        }
    }
}
```

#### Indentation Counting

The `count_indent()` function uses SIMD to count leading spaces at line starts:

```rust
fn count_indent(&self) -> Result<usize, YamlError> {
    // SIMD-accelerated space counting (16 bytes/iteration on ARM, 32 on AVX2)
    let count = simd::count_leading_spaces(self.input, self.pos);

    // Check for tab at the position after spaces (YAML error handling)
    let next_pos = self.pos + count;
    if next_pos < self.input.len() && self.input[next_pos] == b'\t' {
        if count == 0 {
            return Err(YamlError::TabIndentation { ... });
        }
    }
    Ok(count)
}
```

---

## High-Performance Pure Rust Oracle

This section details how to achieve rapidyaml-level performance (~150 MB/s) in pure Rust by combining rapidyaml's architectural insights with succinctly's proven optimization techniques.

### Design Philosophy: Why This Can Work

rapidyaml achieves ~150 MB/s without SIMD through:
1. **Zero-copy parsing** - String views into source buffer
2. **Cache-friendly layout** - Flat arrays with index-based linking
3. **Static polymorphism** - No virtual functions
4. **Algorithmic efficiency** - O(1) operations where possible

succinctly has demonstrated:
1. **PFSM tables** - 33-77% speedup over naive parsing
2. **SIMD character detection** - 1.8x speedup (NEON nibble lookup)
3. **Lightweight indices** - 5-9x faster than complex structures
4. **Byte-level lookup tables** - 11x speedup for tree navigation

**Combining both**: We can potentially exceed rapidyaml by adding SIMD to an already cache-optimized design.

---

### Technique 1: YFSM (YAML Finite State Machine)

Extend the proven PFSM approach from JSON to YAML's richer state space.

**JSON PFSM (4 states)**:
```
InJson → InString → InEscape → InValue
```

**YAML YFSM (8 states)**:
```
Block    → InKey    → InValue  → InString
InEscape → InSingle → FlowMap  → FlowSeq
```

**Table Structure** (expanded from JSON):

```rust
// 256 entries × 8 bytes = 2 KB per table (still L1 cache resident)
pub const YAML_TRANSITION_TABLE: [u64; 256];  // 8 states packed
pub const YAML_PHI_TABLE: [u64; 256];         // Output bits per state

// State extraction: same principle as JSON
let packed = YAML_TRANSITION_TABLE[byte as usize];
let next_state = ((packed >> (state * 8)) & 0xFF) as u8;
```

**Phi Output Encoding** (4 bits per state):
```
Bit 0: IB (Interest Bit) - structural position
Bit 1: BP Open - entering container
Bit 2: BP Close - leaving container
Bit 3: TY (Type) - mapping vs sequence
```

**Key Insight**: YAML's context sensitivity is handled by more states, not by abandoning the table-driven approach. The state machine pre-resolves ambiguity.

---

### Technique 2: SIMD Structural Character Detection

Detect YAML structural characters in parallel before the state machine processes them.

**YAML Structural Characters**:
```
Newline: \n (0x0A)           - Primary structure delimiter
Colon:   :  (0x3A) + space   - Key-value separator
Hyphen:  -  (0x2D) + space   - Sequence item
Hash:    #  (0x23)           - Comment start
Quote:   "  (0x22), ' (0x27) - String delimiters
Flow:    { } [ ] ,           - Flow style
Block:   | >                 - Block scalars
Anchor:  & *                 - Anchors/aliases
```

**AVX2 Implementation** (32 bytes/iteration):

```rust
unsafe fn yaml_classify_avx2(chunk: &[u8; 32]) -> YamlMasks {
    let data = _mm256_loadu_si256(chunk.as_ptr() as *const __m256i);

    // Single-character detection
    let newlines = _mm256_cmpeq_epi8(data, _mm256_set1_epi8(0x0A));
    let colons = _mm256_cmpeq_epi8(data, _mm256_set1_epi8(0x3A));
    let hyphens = _mm256_cmpeq_epi8(data, _mm256_set1_epi8(0x2D));
    let spaces = _mm256_cmpeq_epi8(data, _mm256_set1_epi8(0x20));
    let quotes = _mm256_or_si256(
        _mm256_cmpeq_epi8(data, _mm256_set1_epi8(0x22)),
        _mm256_cmpeq_epi8(data, _mm256_set1_epi8(0x27))
    );

    YamlMasks {
        newlines: _mm256_movemask_epi8(newlines) as u32,
        colons: _mm256_movemask_epi8(colons) as u32,
        hyphens: _mm256_movemask_epi8(hyphens) as u32,
        spaces: _mm256_movemask_epi8(spaces) as u32,
        quotes: _mm256_movemask_epi8(quotes) as u32,
    }
}
```

**NEON Nibble Lookup** (16 bytes/iteration, but faster per byte):

The nibble lookup approach is ideal for YAML's many special characters:

```rust
// Characters to detect: \n : - # " ' { } [ ] , | > & * ! ?
// All are in ASCII range, can be encoded in nibble tables

const YAML_LO_NIBBLE: [u8; 16] = [...];  // Low 4 bits pattern
const YAML_HI_NIBBLE: [u8; 16] = [...];  // High 4 bits pattern

unsafe fn yaml_classify_neon(chunk: &[u8; 16]) -> u16 {
    let data = vld1q_u8(chunk.as_ptr());
    let lo = vandq_u8(data, vdupq_n_u8(0x0F));
    let hi = vshrq_n_u8(data, 4);

    let lo_result = vqtbl1q_u8(vld1q_u8(YAML_LO_NIBBLE.as_ptr()), lo);
    let hi_result = vqtbl1q_u8(vld1q_u8(YAML_HI_NIBBLE.as_ptr()), hi);

    let matches = vandq_u8(lo_result, hi_result);
    neon_movemask(matches)  // Custom 16-bit mask extraction
}
```

---

### Technique 3: Indentation Tracking with Newline Index

rapidyaml tracks newline offsets for O(log n) line lookups. We can do better with succinctly's lightweight index approach.

**Structure**:
```rust
pub struct IndentationIndex {
    /// Bit vector: 1 at each newline position
    newlines: Vec<u64>,
    /// Cumulative newline count per word
    newline_rank: Vec<u32>,
    /// Indentation level at each line (computed lazily or eagerly)
    line_indent: Vec<u8>,  // Most YAML files have <256 indent levels
}
```

**Operations**:
```rust
// O(log n) - Find line number for byte offset
fn line_at(&self, offset: usize) -> usize {
    self.newline_rank1(offset)
}

// O(log n) - Find byte offset of line start
fn line_start(&self, line: usize) -> usize {
    if line == 0 { 0 } else { self.newline_select1(line - 1) + 1 }
}

// O(1) - Get indentation at line (after index built)
fn indent_at(&self, line: usize) -> u8 {
    self.line_indent[line]
}
```

**Integration Advantage**: Building the newline index during parsing adds minimal overhead but enables fast `locate_offset()` without re-scanning.

---

### Technique 4: Flat Array Tree (rapidyaml-style)

rapidyaml's key insight: index-based linking is faster than pointer chasing.

**Node Structure** (cache-aligned):
```rust
#[repr(C, align(64))]  // One cache line per node
pub struct YamlNode {
    // Indices into node array (u32 = 4B each, supports 4B nodes)
    parent: u32,
    first_child: u32,
    last_child: u32,
    next_sibling: u32,
    prev_sibling: u32,

    // Byte ranges into source (no string copies)
    key_start: u32,
    key_end: u32,
    value_start: u32,
    value_end: u32,

    // Type and flags (1 byte)
    node_type: NodeType,  // Mapping, Sequence, Scalar, Alias
    flags: NodeFlags,     // HasAnchor, HasTag, BlockStyle, etc.

    // Padding to 64 bytes
    _pad: [u8; 22],
}
```

**Benefits**:
- Contiguous memory = predictable prefetch
- Index arithmetic faster than pointer dereference
- Easy serialization (no pointer fixup)
- Compatible with memory-mapped files

---

### Technique 5: Speculative Indentation Parsing

Most YAML files follow predictable patterns. Optimize for the common case.

**Common Pattern Detection** (SIMD-assisted):
```rust
// Detect "  key: value\n" pattern (2-space indent)
fn is_simple_kv_line(line: &[u8]) -> Option<(usize, usize, usize)> {
    // SIMD: Find colon position
    let colon_mask = simd_find_colon(line);
    let colon_pos = colon_mask.trailing_zeros() as usize;

    // Check: colon followed by space, no special chars in key
    if colon_pos < line.len() - 1 && line[colon_pos + 1] == b' ' {
        let key_end = colon_pos;
        let value_start = colon_pos + 2;
        let indent = count_leading_spaces(line);  // SIMD popcount
        return Some((indent, key_end, value_start));
    }
    None
}
```

**Fallback**: When pattern doesn't match, fall back to full YFSM parsing for that line.

---

### Technique 6: Integrated Index Construction

**Key Innovation**: Building semi-index during parse (not as separate pass).

rapidyaml builds a tree. We build IB/BP/TY bits directly:

```rust
fn parse_line(&mut self, line: &[u8], line_start: usize) {
    let indent = self.count_indent(line);

    // Emit virtual brackets for indentation change
    while self.current_indent > indent {
        self.bp_writer.write_bit(false);  // Close
        self.indent_stack.pop();
        self.current_indent = self.indent_stack.last().copied().unwrap_or(0);
    }

    // Process line content with YFSM
    for (i, &byte) in line[indent..].iter().enumerate() {
        let pos = line_start + indent + i;
        let (next_state, phi) = self.yfsm_step(byte);

        if phi & IB_BIT != 0 {
            self.ib_writer.set_bit(pos);
        }
        if phi & BP_OPEN != 0 {
            self.bp_writer.write_bit(true);
            if indent > self.current_indent {
                self.indent_stack.push(indent);
                self.current_indent = indent;
            }
        }
        if phi & BP_CLOSE != 0 {
            self.bp_writer.write_bit(false);
        }
        if phi & TY_BIT != 0 {
            self.ty_writer.write_bit(true);  // Sequence
        }

        self.state = next_state;
    }
}
```

**Benefit**: Single pass produces complete index. No re-traversal needed.

---

### Performance Projection

| Component | rapidyaml | succinctly YAML (projected) |
|-----------|-----------|----------------------------|
| Character classification | Scalar | SIMD (1.8x faster) |
| State transitions | Manual | YFSM tables (1.3-1.7x faster) |
| Tree construction | Flat array | Same + BP bits |
| String handling | Zero-copy | Same |
| Indentation | O(log n) lookup | O(1) with lightweight index |

**Conservative estimate**: 150-200 MB/s for YAML 1.2 parsing with full index construction.

**Aggressive estimate**: 200-300 MB/s if SIMD character detection eliminates most state machine iterations (similar to how simdjson outperforms rapidjson by 10x through structural indexing).

---

### Unlocked Optimizations (Integrated Approach)

Having parsing and indexing in the same codebase enables optimizations impossible with separate libraries:

1. **Fused SIMD + State Machine**: SIMD detects structural positions, state machine only runs at those positions (not every byte).

2. **Predictive Bit Writing**: When SIMD shows next 32 bytes have no structural chars, write 32 zeros to BP in one operation.

3. **Branch Elimination**: YFSM table entries can encode "skip N bytes" for quoted strings, avoiding per-byte state checks.

4. **Aligned Processing**: Ensure line-start positions align with SIMD boundaries where possible.

5. **Streaming Index**: Build cumulative rank arrays incrementally, enabling O(1) queries before parse completes.

---

## References

### YAML Specification
- YAML 1.2.2: https://yaml.org/spec/1.2.2/

### Fast YAML Parsers
- rapidyaml: https://github.com/biojppm/rapidyaml
- libfyaml: https://github.com/pantoniou/libfyaml
- VYaml: https://github.com/hadashiA/VYaml

### Academic Work
- Adams, M.D. "Principled Parsing for Indentation-Sensitive Languages" (POPL 2013)
- Marlow & Jones "Indentation-sensitive parsing for Parsec" (Haskell Symposium 2014)

### JSON Semi-Indexing (Related)
- Langdale & Lemire "Parsing Gigabytes of JSON per Second" (2019)
- simdjson: https://github.com/simdjson/simdjson
