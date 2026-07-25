# BitVec Design

[Home](../../) > [Docs](../) > [Architecture](./) > BitVec

Design and implementation of the bit vector with rank/select support.

## Overview

`BitVec` is a bit vector that supports:
- O(1) rank operations
- O(log n) select operations
- **27.5-47.5% space overhead**, rising with bit density (see [Space Analysis](#space-analysis)) —
  *not* the ~3% of a compact Poppy index, a figure this page previously quoted from the paper

## Data Layout

```rust
pub struct BitVec {
    words: Vec<u64>,          // Raw bit storage
    len: usize,               // Number of valid bits
    ones_count: usize,        // Cached popcount
    rank_dir: RankDirectory,  // 3-level Poppy directory (l0 + cache-aligned l1/l2)
    select_idx: SelectIndex,  // One 16-byte sample per `select_sample_rate` ones
}
```

`heap_size()` reports the total of all three parts.

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

The block index is packed to bring that down:

### Optimized Block Index
- Store relative counts within superblock
- 8 blocks per superblock, max count 64 each
- Pack into u16 (max 512 total)

Actual overhead of the rank directory:
- Superblock: n/512 * 64 bits = 0.125n bits
- Block: n/64 * 9 bits = 0.14n bits
- Total: ~0.265n bits, i.e. **~25% overhead** — 128 bits of metadata per 512 bits of data

This is a deliberate trade of space for query speed, and is *not* the ~3% of a compact Poppy index;
see the note in [src/bits/rank.rs](../../src/bits/rank.rs). The sampled `SelectIndex` adds an entry per
256 set bits on top, so a full `BitVec` measures **27.5-47.5%** resident overhead, rising with bit
density ([../benchmarks/rust-succinct-libs.md](../benchmarks/rust-succinct-libs.md)).

## SIMD Optimization

Popcount uses platform-specific SIMD:
- x86_64: `POPCNT` instruction
- ARM64: NEON `vcnt` + horizontal add
- Fallback: Lookup table

## Used By

**Nothing in the crate, as of [#228](https://github.com/rust-works/succinctly/issues/228).** `BitVec`
is public API — exported from the crate root, exercised by doctests and the serde round-trip tests —
but it has no production callers. This section previously listed four, none of which were accurate:

| Structure                             | What it actually stores                                                                                                                        |
|---------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------|
| [BalancedParens](balanced-parens.md)  | `words: W` (a plain `Vec<u64>` or borrowed slice) plus its own `rank_l1`/`rank_l2` arrays — never a `BitVec`                                   |
| [JsonIndex](../parsing/json-index.md) | interest bits as `W` plus a cumulative-rank `Vec<u32>`; the newline index moved to [`text::LineIndex`](../../src/text/lines.rs)                |
| [YamlIndex](../parsing/yaml-index.md) | same pattern, plus type bits; newline index likewise now `LineIndex`                                                                           |
| [DsvIndex](../parsing/dsv-index.md)   | `DsvIndexLightweight` — plain `Vec<u64>` markers and newlines with cumulative-rank `Vec<u32>`, measured 5-9x faster to iterate than a `BitVec` |

The parsers bypass `BitVec` because they need storage genericity (`W: AsRef<[u64]>` for mmap),
hinted select, or cheap sequential iteration, none of which it offers. See
[ADR-0011](../adrs/adr-0011.md) for why the structures are in-crate at all, and
[ADR-0012](../adrs/adr-0012.md) for why the last auxiliary uses moved to Elias-Fano.

## Academic Papers

- [Vigna 2008](https://vigna.di.unimi.it/ftp/papers/Broadword.pdf) — broadword rank/select algorithms
- [Zhou, Andersen, Kaminsky 2013](https://www.cs.cmu.edu/~dga/papers/zhou-sea2013.pdf) — Poppy structure (3-level directory)
- [Mula, Kurz, Lemire 2016](https://arxiv.org/abs/1611.07612) — Harley-Seal popcount with AVX2

## Source & Docs

- Implementation: [src/bits/](../../src/bits/) (bitvec.rs, rank.rs, select.rs, popcount.rs)
- Optimization techniques: [optimizations/bit-manipulation.md](../optimizations/bit-manipulation.md)

## See Also

- [Core Concepts](core-concepts.md) - Rank/select theory
- [Bit Manipulation](../optimizations/bit-manipulation.md) - Popcount techniques
