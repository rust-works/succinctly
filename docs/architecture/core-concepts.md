# Core Concepts

[Home](../../) > [Docs](../) > [Architecture](./) > Core Concepts

Introduction to succinct data structures and why they matter.

## What are Succinct Data Structures?

Succinct data structures store data in space close to the information-theoretic minimum while still supporting efficient operations. Unlike compressed data that must be decompressed before use, succinct structures operate directly on the compressed representation.

## Key Properties

### Space Efficiency
- Store n bits in n + o(n) bits
- Overhead is sublinear in data size
- Compact designs in the literature reach ~3% (Poppy); succinctly spends more for speed: its rank
  directory is 25% of the bit vector, and a full `BitVec` with select is 27.5-47.5%
  ([bitvec.md](bitvec.md#space-analysis))
- The semi-indexes built on these structures are measured against their *input*, not a bit vector:
  19-38% of the input for JSON, 37.5% for DSV, 50-70% for YAML
  ([semi-indexing.md](semi-indexing.md#index-size))

### Time Efficiency
- O(1) rank operations
- O(log n) select operations
- No decompression needed

## Core Operations

### Rank

`rank1(i)`: Count the number of 1-bits in positions 0 through i-1.

```
Bits:    1 0 1 1 0 1 0 0
Index:   0 1 2 3 4 5 6 7

rank1(0) = 0  (no bits before position 0)
rank1(3) = 2  (positions 0,2 have 1-bits)
rank1(6) = 4  (positions 0,2,3,5 have 1-bits)
```

### Select

`select1(k)`: Find the position of the k-th 1-bit (1-indexed).

```
Bits:    1 0 1 1 0 1 0 0
Index:   0 1 2 3 4 5 6 7

select1(1) = 0  (1st 1-bit at position 0)
select1(2) = 2  (2nd 1-bit at position 2)
select1(4) = 5  (4th 1-bit at position 5)
```

## Hierarchical Index Structure

Succinctly uses a two-level index for rank operations:

```
Level 0 (superblocks): Every 512 bits
Level 1 (blocks):      Every 64 bits

For each superblock: cumulative popcount from start
For each block:      popcount within the superblock
```

This allows O(1) rank. The space depends on how wide the counts are: compact layouts in the
literature spend ~3% of the bit vector, while succinctly's `RankDirectory` spends one 128-bit entry
(a 32-bit cumulative count and seven 9-bit offsets) per 512 bits, 25% of the bit vector, to keep each
query to one cache line ([bitvec.md](bitvec.md#space-analysis)). The JSON, YAML and DSV indexes use a
plainer form still, a `u32` cumulative count per 64-bit word, which is 50% of the bit vector it counts.

## Balanced Parentheses

Trees can be encoded as balanced parentheses:

```
Tree:           Encoding:
    A           A(B()C(D()))
   / \          ( ( ) ( ( ) ) )
  B   C
      |
      D
```

Operations supported:
- `find_close(i)`: Find matching closing parenthesis
- `find_open(i)`: Find matching opening parenthesis
- `enclose(i)`: Find parent node
- `rank`/`select` on open/close parens

## Semi-Indexing

For JSON/YAML/DSV, we build structural indices without fully parsing:

1. **Scan once**: Identify structural characters
2. **Build BP tree**: Encode structure as balanced parentheses
3. **Navigate lazily**: Only parse values when accessed

This gives O(1) navigation with minimal upfront cost.

## See Also

- [BitVec](bitvec.md) - Bit vector implementation details
- [Balanced Parentheses](balanced-parens.md) - Tree encoding
- [Semi-Indexing](semi-indexing.md) - Parser approach
