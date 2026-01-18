# Core Concepts

[Home](/) > [Docs](../) > [Architecture](./) > Core Concepts

Introduction to succinct data structures and why they matter.

## What are Succinct Data Structures?

Succinct data structures store data in space close to the information-theoretic minimum while still supporting efficient operations. Unlike compressed data that must be decompressed before use, succinct structures operate directly on the compressed representation.

## Key Properties

### Space Efficiency
- Store n bits in n + o(n) bits
- Overhead is sublinear in data size
- Often 3-6% overhead for full functionality

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

This allows O(1) rank with only ~3-4% space overhead.

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
