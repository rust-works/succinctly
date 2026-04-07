# BalancedParens

[Knowledge Map](index.md) > BalancedParens

A succinct tree representation that encodes any tree as a sequence of open/close parentheses, enabling O(1) navigation with ~6% overhead.

## What It Does

Any tree can be encoded depth-first as balanced parentheses (1 = open, 0 = close):

```
       A            Encoding: ( ( ) ( ( ) ) ( ) )
      /|\            As bits: 1 1 0 1 1 0 0 1 0 0
     B C D                    A B   C E     D
       |
       E
```

From this encoding, all tree operations reduce to bit operations:

| Operation    | How                                          | Complexity     |
|--------------|----------------------------------------------|----------------|
| Parent       | `enclose(i)` — first unmatched open before i | O(1) amortized |
| First child  | `i + 1` if position i is an open paren       | O(1)           |
| Next sibling | `find_close(i) + 1`                          | O(1) amortized |
| Subtree size | `(find_close(i) - i + 1) / 2`                | O(1) amortized |

## How It Works

The key operation is `find_close(i)` — finding the matching close for an open paren. This uses a **RangeMin index** over the "excess" (running count of opens minus closes):

```
Bits:    1  1  0  1  1  0  0  1  0  0
Excess:  1  2  1  2  3  2  1  2  1  0
```

The RangeMin index precomputes minimum excess values over blocks, enabling O(1) lookups via a sparse table.

### SIMD Acceleration

RangeMin construction uses SIMD for the horizontal minimum step:

| Platform | Instruction          | Speedup                                    |
|----------|----------------------|--------------------------------------------|
| ARM64    | NEON `vminvq_s16`    | **2.8x** — direct signed horizontal min    |
| x86_64   | SSE4.1 `PHMINPOSUW`  | **1-3%** — unsigned only, needs bias trick |

### Generic Select Support

`BalancedParens<W, S>` is generic over select support:
- `NoSelect` (ZST) — used by JSON, zero overhead
- `WithSelect` — used by YAML for `at_offset`/`yq-locate`, enables O(1) sampled select1

## Used By

- [JsonIndex](json-index.md) — encodes JSON nesting structure (objects, arrays, values)
- [YamlIndex](yaml-index.md) — encodes virtual brackets from indentation
- [DsvIndex](dsv-index.md) — encodes row/field structure

## Depends On

- [BitVec](bitvec.md) — the parenthesis sequence and RangeMin index are stored as bitvectors

## Academic Papers

- Sadakane & Navarro 2010 — fully-functional succinct trees, RangeMin for O(1) navigation
- Navarro & Sadakane 2014 — space-optimal BP representation

## Source & Docs

- Implementation: [src/trees/bp.rs](../src/trees/bp.rs)
- Architecture doc: [architecture/balanced-parens.md](architecture/balanced-parens.md)
- Optimization: [optimizations/hierarchical-structures.md](optimizations/hierarchical-structures.md)
