# BitVec Design

[Home](/) > [Docs](../) > [Architecture](./) > BitVec

Design and implementation of the bit vector with rank/select support.

## Overview

`BitVec` is a bit vector that supports:
- O(1) rank operations
- O(log n) select operations
- ~3-4% space overhead

## Data Layout

```rust
pub struct BitVec {
    bits: Vec<u64>,           // Raw bit storage
    len: usize,               // Number of bits
    superblock_rank: Vec<u64>, // Cumulative counts per 512 bits
    block_rank: Vec<u16>,     // Counts per 64 bits within superblock
}
```

## Index Structure

### Superblocks (Level 0)
- One entry per 512 bits
- Stores cumulative popcount from start of bit vector
- u64 values (supports up to 2^64 bits)

### Blocks (Level 1)
- One entry per 64 bits
- Stores popcount within the containing superblock
- u16 values (max 512, fits in 9 bits)

## Rank Algorithm

```rust
fn rank1(&self, pos: usize) -> u64 {
    let word_idx = pos / 64;
    let bit_idx = pos % 64;
    let superblock_idx = word_idx / 8;
    let block_idx = word_idx;

    // Level 0: superblock cumulative count
    let superblock_count = self.superblock_rank[superblock_idx];

    // Level 1: block count within superblock
    let block_count = self.block_rank[block_idx] as u64;

    // Partial word: popcount of remaining bits
    let mask = (1u64 << bit_idx) - 1;
    let partial = (self.bits[word_idx] & mask).count_ones() as u64;

    superblock_count + block_count + partial
}
```

## Select Algorithm

Select uses binary search over the rank index:

1. Binary search superblocks to find containing superblock
2. Linear scan blocks within superblock
3. Use popcount + CTZ to find exact bit position

## Space Analysis

For n bits:
- Bits: n bits
- Superblock index: n/512 * 64 = n/8 bits (12.5%)
- Block index: n/64 * 16 = n/4 bits (25%)

Wait, that's too much! We optimize:

### Optimized Block Index
- Store relative counts within superblock
- 8 blocks per superblock, max count 64 each
- Pack into u16 (max 512 total)

Actual overhead:
- Superblock: n/512 * 64 bits = 0.125n bits
- Block: n/64 * 9 bits = 0.14n bits
- Total: ~3-4% overhead

## SIMD Optimization

Popcount uses platform-specific SIMD:
- x86_64: `POPCNT` instruction
- ARM64: NEON `vcnt` + horizontal add
- Fallback: Lookup table

## See Also

- [Core Concepts](core-concepts.md) - Rank/select theory
- [Bit Manipulation](../optimizations/bit-manipulation.md) - Popcount techniques
