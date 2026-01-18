# Semi-Indexing

[Home](/) > [Docs](../) > [Architecture](./) > Semi-Indexing

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
| vs yq | similar | **45-687x faster** |

The index overhead (3-6%) is dwarfed by the memory savings from not materializing the entire document.

See [../benchmarks/](../benchmarks/) for detailed numbers across file sizes and patterns.

## Trade-offs

1. **Memory**: Index overhead (3-6%)
2. **Latency**: Index build is upfront cost
3. **Complexity**: More complex than DOM parsing

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
