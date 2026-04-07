# BitVec

[Knowledge Map](index.md) > BitVec

A space-efficient bitvector supporting O(1) rank and O(log n) select operations with ~3-4% overhead.

## What It Does

`BitVec` answers two fundamental questions about a sequence of bits:

- **rank1(i)**: How many 1-bits are there in positions [0, i)?
- **select1(k)**: What is the position of the k-th 1-bit?

These operations are the building blocks for all higher-level data structures in succinctly.

## How It Works

A 3-level hierarchical index (based on the Poppy structure from [Zhou et al. 2013](https://www.cs.cmu.edu/~dga/papers/zhou-sea2013.pdf)):

| Level        | Granularity          | Stores                            | Type  |
|--------------|----------------------|-----------------------------------|-------|
| Superblock   | Every 512 bits       | Cumulative popcount from start    | `u64` |
| Block        | Every 64 bits        | Popcount within superblock        | `u16` |
| Partial word | Within a 64-bit word | Computed via popcount instruction | —     |

**Rank** combines all three levels in constant time. **Select** binary-searches the superblock index, then linear-scans blocks, then uses popcount + CTZ for the exact position.

## Space Overhead

- Superblock index: n/512 * 64 = 0.125n bits
- Block index: n/64 * 9 = 0.14n bits
- **Total: ~3-4% of the bitvector size**

## SIMD Popcount

The partial-word popcount step uses platform-specific instructions:

| Platform | Instruction                  | Notes                   |
|----------|------------------------------|-------------------------|
| x86_64   | `POPCNT`                     | Hardware popcount       |
| ARM64    | NEON `vcnt` + horizontal add | Vector popcount         |
| Fallback | Lookup table                 | Portable, no intrinsics |

Selectable via feature flags: default (auto-vectorize), `simd` (explicit intrinsics), `portable-popcount` (bitwise algorithm).

## Used By

- [BalancedParens](balanced-parens.md) — stores its parenthesis sequence as a `BitVec`, uses rank1/select1 for tree navigation
- [JsonIndex](json-index.md) — interest bits and BP encoding are both bitvectors
- [YamlIndex](yaml-index.md) — same pattern, plus type bits
- [DsvIndex](dsv-index.md) — marker and newline bitvectors

## Academic Papers

- [Vigna 2008](https://vigna.di.unimi.it/ftp/papers/Broadword.pdf) — broadword rank/select algorithms
- [Zhou, Andersen, Kaminsky 2013](https://www.cs.cmu.edu/~dga/papers/zhou-sea2013.pdf) — Poppy structure (3-level directory)
- [Mula, Kurz, Lemire 2016](https://arxiv.org/abs/1611.07612) — Harley-Seal popcount with AVX2

## Source & Docs

- Implementation: [src/bits/](../src/bits/) (bitvec.rs, rank.rs, select.rs, popcount.rs)
- Architecture doc: [architecture/bitvec.md](architecture/bitvec.md)
- Optimization techniques: [optimizations/bit-manipulation.md](optimizations/bit-manipulation.md)
