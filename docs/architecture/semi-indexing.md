# Semi-Indexing

[Home](../../) > [Docs](../) > [Architecture](./) > Semi-Indexing

The semi-indexing approach for JSON, YAML, and DSV parsing.

## Overview

Semi-indexing builds a structural index of a document in a single pass, enabling O(1) navigation without full parsing. Values are extracted lazily only when accessed.

## Key Insight

Most queries access only a small fraction of a document. Building a full DOM is wasteful. Instead:

1. **Index**: Identify structural boundaries (objects, arrays, strings)
2. **Navigate**: Use balanced parentheses for O(1) structure traversal
3. **Extract**: Parse values only when accessed

## Architecture

```
Input Document
      │
      ▼
┌─────────────────┐
│   SIMD Scanner  │  Identify structural chars
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  State Machine  │  Track parsing context
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Balanced Parens │  Encode structure
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Cursor API     │  Lazy navigation
└─────────────────┘
```

## JSON Semi-Index

### Structural Characters
- `{` `}`: Object boundaries
- `[` `]`: Array boundaries
- `:`: Key-value separator
- `,`: Element separator
- `"`: String boundaries

### BP Encoding
```json
{"users": [{"name": "Alice"}]}
```

```
BP:  ( ( ( ( ) ) ) )
     { u [ { n   } ]   }
```

### Interest Bits
Additional bit vectors track:
- String positions (for escape handling)
- Array element positions
- Object key positions

## YAML Semi-Index

YAML is more complex due to:
- Indentation-based structure
- Multiple scalar styles (plain, quoted, block)
- Anchors and aliases

### Oracle Parser
YAML uses an "oracle" that consults the index during parsing:
1. Scan for newlines and indentation
2. Build coarse structure from indentation levels
3. Refine with quote and block scalar detection

## DSV Semi-Index

CSV/TSV uses a lightweight approach:
- Quote-aware scanning
- Row boundary detection
- Column offsets per row

### Quote Handling
DSV must handle:
- Quoted fields with embedded delimiters
- Escaped quotes (`""` in CSV)
- Newlines within quoted fields

## Performance Characteristics

| Format | Index Build | Navigation | Value Extract |
|--------|-------------|------------|---------------|
| JSON   | O(n)        | O(1)       | O(k) for k bytes |
| YAML   | O(n)        | O(1)       | O(k) for k bytes |
| DSV    | O(n)        | O(1)       | O(k) for k bytes |

Where n = document size, k = value size.

## Advantages

1. **Lazy Evaluation**: Only parse accessed values
2. **Zero-Copy**: Navigate without copying
3. **Cache-Friendly**: Sequential scan for index, random access for queries
4. **SIMD-Accelerated**: Parallel character classification

## Real-World Performance

Semi-indexing delivers significant memory and speed improvements over traditional DOM parsers:

| Comparison | Memory Savings | Speed |
|------------|----------------|-------|
| vs serde_json | **18x less** | 3x faster |
| vs simd-json | **46x less** | 3x faster |
| vs jq | **5-25x less** | 1.2-6.3x faster |
| vs yq | similar | **16-40x faster** |

The index is not small (19-38% of the input for JSON, see [Index Size](#index-size)), but it is a
fraction of the input where a DOM is a multiple of it. On the 1 GB `identity` rows of the committed
jq comparisons ([compare/jq.md](../compare/jq.md)), `succinctly jq` peaks at 1.24-1.25x the input,
the text plus an index of about a quarter of it, and jq 1.7.1 at 9.48-9.77x.

See [../benchmarks/](../benchmarks/) for detailed numbers across file sizes and patterns.

## Index Size

What the index costs depends on the format and, for JSON and YAML, on how many nodes the text holds
per byte: the interest bits are one bit per input byte whatever the text says, and the balanced
parentheses are two bits per node. The figures below are the heap the 0.8.0 library's index holds
over 10 MB inputs from `succinctly {json,yaml,dsv} generate 10mb -p <pattern>`, as a percentage of the
input; the input itself is not counted. Sizes are deterministic, so they do not depend on the machine.
Earlier revisions of this page and others said "~3-6%" (or "3-4%"); no measurement supports that, and
it is not true of any of the three indexes (#7955).

**How they were measured.** A probe linked against the library built each index once per file and
read its size with a counting global allocator: the live heap before and after a clone of the index
(a clone allocates each vector at its length, so spare capacity is left out). For JSON and DSV, whose
vectors are public, it also summed them (for JSON, `ib`, the IB rank array, the BP words and
`BalancedParens::index_heap_size`), and the two agree to the byte.

### JSON (`JsonIndex`)

| Part                                  | Size                         | Grows with |
|---------------------------------------|------------------------------|------------|
| Interest bits (IB)                    | 12.5% (one bit per byte)     | Bytes      |
| IB rank (`ib_rank`)                   | 6.25% (a `u32` per word)     | Bytes      |
| Balanced parentheses (BP)             | 2 bits per node              | Nodes      |
| BP directories (`index_heap_size`)    | ~58% of the BP bits          | Nodes      |

A node is a key or a value. The index is therefore 18.75% of the input plus about 0.4 bytes per
node:

| Pattern         | Input bytes per node | Index / input |
|-----------------|---------------------:|--------------:|
| `nested`        |              499,318 |         18.8% |
| `strings`       |                123.0 |         19.1% |
| `unicode`       |                 24.3 |         20.4% |
| `numbers`       |                 14.5 |         21.5% |
| `pretty`        |                 12.3 |         22.0% |
| `wide`          |                  8.3 |         23.5% |
| `users`         |                  7.8 |         23.8% |
| `ndjson`        |                  7.8 |         23.8% |
| `comprehensive` |                  6.0 |         25.3% |
| `literals`      |                  5.3 |         26.2% |
| `pathological`  |                  3.2 |         31.1% |
| `arrays`        |                  2.0 |         38.5% |

`JsonIndex::build` reserves the BP buffer at its worst case, two bits per input byte (25% of the
input), and does not shrink it. An allocator that counts capacity therefore sees 44-51% of the input,
which matches the 0.46x that [rust-parsers.md](../benchmarks/rust-parsers.md) reports. The unwritten pages
of that buffer are never touched, so the process's resident size grows by the table's figure plus 1-3
points (measured on macOS), not by the allocator's.

### YAML (`YamlIndex`)

`YamlIndex::build_yq`, the index `succinctly yq` builds, is 50-70% of the input on the generator's
patterns: 50% on `block-scalars`, 56% on `strings`, 61% on `comprehensive`, 63% on `users` and 70%
on `multi-doc`. Beside the interest bits and their rank array (18.75%), it keeps a map from each node
to where its text starts (19-28%); a map to where each scalar ends, the container bits and the
comment and anchor maps are the remaining 13-22%, and the BP and its directories are under 7%. On
`anchors` it is 2.0x the input, because the anchor and alias names and their positions are kept in
ordered maps. Building it peaks at 1.75x the input on most patterns and 2.75x on `flow`, `multi-doc`
and `sequences`; that peak is freed before a query runs.

### DSV (`DsvIndex`)

`DsvIndex` is 37.5% of the input on every pattern: two bit vectors of one bit per byte (markers for
delimiters and row ends outside quotes, and row ends alone) and a `u32` cumulative count per 64-bit
word of each (6.25% each). Nothing in it depends on the content.

## Trade-offs

1. **Memory**: An index of 19-38% of the input for JSON, 37.5% for DSV and 50-70% for YAML (see
   [Index Size](#index-size)), against a multiple of the input for a DOM
2. **Latency**: Index build is upfront cost
3. **Complexity**: More complex than DOM parsing
4. **Validation**: Minimal; a malformed value is found when it is read, not when the document is
   loaded (see [Validation Is Not Done Up Front](#validation-is-not-done-up-front))

### Validation Is Not Done Up Front

jq and yq reject a malformed document before any filter runs. succinctly does not, and that is
deliberate: checking the whole document first would cost most of what the semi-index saves on a
query that reads a small part of it. #1247 measured the two candidate checks on an Apple M5 Max,
on 8.4 MB of generated JSON (min-of-7, the 3.21 ms process-spawn baseline subtracted):

| Check                                               | Net cost   | vs `jq '.users[0].name'` (13.1 ms) | vs `jq '.'` (187 ms) |
|-----------------------------------------------------|------------|------------------------------------|----------------------|
| Whole-input UTF-8 validation (`text validate utf8`) | **1.1 ms** | +8%                                | +0.6%                |
| Full strict JSON validation (`json validate`)       | **8.6 ms** | **+66%**                           | +4.6%                |

These are indicative sizes, not an interleaved A/B per
[benchmarking.md](../guides/benchmarking.md#ab-benchmarking-method), and the machine was not
idle; re-measure before relying on the exact figures.

Always-on strict validation costs two thirds of the runtime of a cheap navigation query, so it is
not done by default; `jq --validate` and `json validate` run it on request. UTF-8 validation is
cheap (about 7.7 GiB/s on the SIMD path), so it runs once when the runner reads a document, and
invalid UTF-8 there gets the reference tool's treatment (yq rejects the document; jq substitutes
U+FFFD). Every other decode failure (an invalid escape, a lone surrogate) is detected
where the value is read and reported through the evaluator's error channel; re-running a
whole-document validator when one fires would add a second full pass for no better diagnostic.
The design and its stages are in
[plan/decode-failure-routing.md](../plan/decode-failure-routing.md).

## When to Use

Semi-indexing excels when:
- Queries access < 50% of document
- Same document queried multiple times
- Memory is constrained
- Low latency is important

Traditional parsing is better when:
- Entire document is consumed
- Document is very small (< 1KB)
- Simplicity is priority

## See Also

- [Balanced Parentheses](balanced-parens.md) - Structure encoding
- [JSON Parser](../parsing/json.md) - JSON-specific details
- [YAML Parser](../parsing/yaml.md) - YAML-specific details
