# Balanced Parentheses

[Home](../../) > [Docs](../) > [Architecture](./) > Balanced Parentheses

Succinct tree representation using balanced parentheses.

## Overview

`BalancedParens` encodes trees as sequences of opening and closing parentheses, supporting O(1) navigation operations.

## Tree Encoding

Any tree can be represented as balanced parentheses:

```
Tree Structure:
       A
      /|\
     B C D
       |
       E

Encoding (depth-first):
A( B() C( E() ) D() )

As bits (1=open, 0=close):
1 1 0 1 1 0 0 1 0 0
A B   C E     D
```

## Data Structure

```rust
pub struct BalancedParens {
    bp: BitVec,              // Parentheses as bits
    excess_min: Vec<i32>,    // RangeMin index for find_close
    excess_min_pos: Vec<u32>,// Position of minimum
}
```

## Core Operations

### find_close(i)

Find the matching closing parenthesis for position i.

Algorithm:
1. Track "excess" (opens - closes)
2. Find first position where excess drops to starting level
3. Use RangeMin index for O(1) amortized lookup

### find_open(i)

Find the matching opening parenthesis for position i.

Algorithm: Reverse of find_close, scan backwards.

### enclose(i)

Find the parent of the node at position i.

Algorithm: Find the first unmatched open paren before i.

## RangeMin Index

For O(1) find_close, we precompute range minimum queries:

```
Bits:     1 1 0 1 1 0 0 1 0 0
Excess:   1 2 1 2 3 2 1 2 1 0
Position: 0 1 2 3 4 5 6 7 8 9

RangeMin[0..4] = 1 (minimum excess in range)
```

The index uses a sparse table (O(n log n) space) or a linear-space structure.

## Applications

### JSON Navigation

```
JSON: {"a": [1, 2]}

BP:   ( ( ( ) ( ) ) )
       { a [ 1   2 ] }
```

### Tree Queries

- Parent: `enclose(i)`
- First child: `i + 1` if open
- Next sibling: `find_close(i) + 1`
- Subtree size: `(find_close(i) - i + 1) / 2`

## Space Overhead

- BP bits: 2n bits for n nodes
- RangeMin: O(n) or O(n log n) depending on variant
- Total: ~6% overhead with optimized RangeMin

## Implementation Optimizations

The RangeMin index construction uses SIMD acceleration on supported platforms:

| Platform | Instruction | Speedup | Notes |
|----------|-------------|---------|-------|
| ARM64 | NEON VMINV | **2.8x** | Direct signed horizontal minimum |
| x86_64 | SSE4.1 PHMINPOSUW | **1-3%** (large data) | Requires bias trick for signed values |

ARM NEON provides `vminvq_s16` which directly computes the minimum across 8 signed 16-bit values. SSE4.1's `PHMINPOSUW` only handles unsigned values, requiring a bias/unbias workaround that adds overhead.

See [SIMD Optimizations](../optimizations/simd.md#x86-sse41-horizontal-minimum-phminposuw) for implementation details.

### Generic Select Support

`BalancedParens<W, S>` is generic over select support:
- `NoSelect` (ZST) — used by JSON, zero overhead
- `WithSelect` — used by YAML for `at_offset`/`yq-locate`, enables O(1) sampled select1

## Used By

- [JsonIndex](../parsing/json-index.md) — encodes JSON nesting structure (objects, arrays, values)
- [YamlIndex](../parsing/yaml-index.md) — encodes virtual brackets from indentation
- [DsvIndex](../parsing/dsv-index.md) — encodes row/field structure

## Depends On

- [BitVec](bitvec.md) — the parenthesis sequence and RangeMin index are stored as bitvectors

## Academic Papers

- Sadakane & Navarro 2010 — fully-functional succinct trees, RangeMin for O(1) navigation
- Navarro & Sadakane 2014 — space-optimal BP representation

## Source & Docs

- Implementation: [src/trees/bp.rs](../../src/trees/bp.rs)
- Optimization: [optimizations/hierarchical-structures.md](../optimizations/hierarchical-structures.md)

## See Also

- [Core Concepts](core-concepts.md) - Theory background
- [Semi-Indexing](semi-indexing.md) - How BP is used in parsing
- [Hierarchical Structures](../optimizations/hierarchical-structures.md) - Index optimization
