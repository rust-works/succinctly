# YamlIndex Post-CompactRank Optimization Plan

Analysis of optimization opportunities after replacing `Vec<u32>` rank indices
with `CompactRank` (~3.5% overhead).

## Context

The CompactRank migration replaced four `Vec<u32>` cumulative rank indices (50%
overhead each) with two-level `CompactRank` structures (~3.5% overhead). This
reduced memory usage by 20-52% in CLI benchmarks. This document identifies the
remaining optimization opportunities.

## Current YamlIndex Memory Layout

| Structure             | Type                     | Size                | Notes                                |
|-----------------------|--------------------------|---------------------|--------------------------------------|
| `ib`                  | bitmap                   | L/8 bytes           | 1 bit/text byte                      |
| `ib_rank`             | CompactRank              | ~3.5% of ib         | Two-level directory                  |
| `bp`                  | BalancedParens+Select    | ~2B/8 + indices     | B = BP length                        |
| `ty`                  | bitmap                   | T/8 bytes           | T = container count                  |
| `seq_items`           | bitmap                   | B/8 bytes           | 1 bit/BP position                    |
| `containers`          | bitmap                   | B/8 bytes           | 1 bit/BP position                    |
| `containers_rank`     | CompactRank              | ~3.5% of containers |                                      |
| `open_positions`      | AdvancePositions         | ~1.1N bytes         | N = BP opens (compact)               |
| **`bp_to_text_end`**  | **`Vec<u32>`**           | **4N bytes**        | **50% of BP — last dense structure** |
| `newlines`            | BitVec                   | L/8 + rank/select   | 1 bit/text byte                      |
| `anchors`             | BTreeMap<String, usize>  | variable            | Anchor definitions                   |
| `bp_to_anchor`        | BTreeMap<usize, String>  | variable            | Reverse anchor map                   |
| `aliases`             | BTreeMap<usize, usize>   | variable            | Alias references                     |

Where L = text length, B = BP length (~2N), N = BP opens, T = container count.

For typical YAML with N ≈ L/7.5:

| Component                | Cost             | % of text |
|--------------------------|------------------|-----------|
| Bitmaps (ib, nl, bp+idx) | ~0.5L            | ~50%      |
| Bitmaps (ty, seq, cont)  | ~0.4N ≈ 0.05L    | ~5%       |
| CompactRank indices      | ~3.5% of bitmaps | ~2%       |
| `open_positions`         | ~1.1N ≈ 0.15L    | ~15%      |
| **`bp_to_text_end`**     | **4N ≈ 0.53L**   | **~53%**  |
| Total                    |                  | ~125%     |

**`bp_to_text_end` is now the single largest structure** — roughly 40% of the
entire index.

---

## Opportunity 1: `bp_to_text_end` Compression

**Impact: High (memory)**
**Effort: Medium**

### Problem

`bp_to_text_end` stores 4 bytes per BP open (`Vec<u32>`). Containers store `0`
as a sentinel (they have no text end position). Only scalar nodes have
meaningful end positions.

In typical YAML, containers + sequence items make up 30-50% of BP opens,
meaning 30-50% of entries are wasted zeros.

### Current usage

```
index.rs:294    bp_to_text_end_pos(bp_pos)  →  self.bp_to_text_end.get(open_idx)
light.rs:86     text_end_position()         →  bp_to_text_end_pos(bp_pos)
light.rs:281    value()                     →  text_end_position()  [for unquoted scalars]
```

The access pattern is random by `open_idx` (from `bp.rank1(bp_pos)`). It is
called once per scalar node during traversal — on the hot streaming path.

### Approach A: Bitmap + sparse `Vec<u32>`

Store a 1-bit "has_end" bitmap marking which BP opens have non-zero end
positions. Use CompactRank to translate open_idx to a sparse index.

```rust
struct ScalarEndPositions {
    /// 1 if this BP open has a non-zero end position.
    has_end: Vec<u64>,
    has_end_rank: CompactRank,
    /// Only non-zero end positions, densely packed.
    ends: Vec<u32>,
}
```

Query: `has_end[open_idx]? → ends[has_end_rank.rank1(open_idx)]`

| Metric         | Current       | Approach A                      |
|----------------|---------------|---------------------------------|
| Storage        | 4N bytes      | ~2.5N bytes                     |
| Lookup cost    | 1 array read  | bitmap test + rank + array read |
| Savings        | —             | ~38%                            |

The savings come from eliminating the zero entries. With 30-50% zeros removed
from the dense vec, plus the small bitmap + CompactRank overhead.

### Approach B: AdvancePositions-style dual bitmap

If non-zero end positions are monotonically non-decreasing (they should be:
scalars appear in text order, ends follow starts), use the same IB + advance
bitmap pattern proven in `AdvancePositions`.

- IB bitmap: 1 bit per text byte, set at each unique end position
- Advance bitmap: 1 bit per scalar BP open, set when end position advances

Query: `advance_rank1(scalar_idx) → ib_select1(advance_count - 1)`

| Metric         | Current       | Approach B    |
|----------------|---------------|---------------|
| Storage        | 4N bytes      | ~1.1S bytes   |
| Where S        | = scalar count| (~60-70% of N)|
| Total          | 4N            | ~0.8N         |
| Savings        | —             | ~80%          |

This is the same architecture that achieved ~3.5× compression for
`open_positions`. The savings are larger here because end positions are
more spread across the text than start positions.

### Approach C: Bitmap + delta encoding

Store scalar lengths (end - start) instead of absolute end positions. Most
scalar values are short (< 256 bytes), so a `Vec<u16>` or even `Vec<u8>` with
overflow handling could work. Combined with Approach A's bitmap to skip
containers.

```rust
struct ScalarEndPositions {
    has_end: Vec<u64>,
    has_end_rank: CompactRank,
    /// Delta from start position. u16 handles scalars up to 65535 bytes.
    deltas: Vec<u16>,
    /// Overflow entries for scalars > 65535 bytes.
    overflow: Vec<(u32, u32)>,  // (sparse_idx, absolute_end)
}
```

Query: start_pos + deltas[sparse_idx], with overflow fallback.

| Metric         | Current       | Approach C    |
|----------------|---------------|---------------|
| Storage        | 4N bytes      | ~1.4N bytes   |
| Savings        | —             | ~65%          |

### Recommendation

**Approach B** (AdvancePositions-style) provides the best compression (~80%
savings) using a proven pattern already in the codebase. The implementation can
reuse or generalize `AdvancePositions` infrastructure.

**Approach A** is simpler to implement as a first step and still delivers ~38%
savings. Could serve as a stepping stone.

### Monotonicity verification needed

Before committing to Approach B, verify that `bp_to_text_end` non-zero values
are monotonically non-decreasing. Containers store 0, but the non-zero scalar
end positions should follow text order. Edge cases to check:

- Anchors/aliases: aliases have their own `bp_to_text_end` entries at the alias
  text position, not the anchor target position
- Multi-document streams: end positions reset across documents
- Explicit keys (`?`): may cause non-monotonic positions (same edge case that
  `OpenPositions` handles with its `Dense` fallback)

If non-monotonic, fall back to Approach A or use the same `Dense` fallback
pattern as `OpenPositions`.

---

## Opportunity 2: Remove Dead Code — `count_seq_items_before()`

**Impact: Low (code cleanliness)**
**Effort: Trivial**

### Problem

[`count_seq_items_before()`](../../src/yaml/index.rs) (line 442) is defined in
`YamlIndex` but never called anywhere in the codebase. It performs an O(n)
linear popcount scan without using CompactRank.

### Action

Remove the function entirely. If it's needed in the future, it should use
CompactRank like `count_containers_before()` does.

---

## Opportunity 3: Add Select Samples to YamlIndex IB

**Impact: Medium (performance on locate/navigation queries)**
**Effort: Low**

### Problem

`ib_select1()` and `ib_select1_from()` in YamlIndex use binary search over
`rank_at_word()` for select queries. With CompactRank, each binary search step
now costs 2 lookups + up to 7 popcounts (vs the old single array lookup with
`Vec<u32>`).

For a 1MB file with ~16K IB words:

| Method              | Binary search steps | Cost per step                | Total cost              |
|---------------------|---------------------|------------------------------|-------------------------|
| Old (`Vec<u32>`)    | ~14                 | 1 array read                 | 14 reads                |
| Current (CompactRank) | ~14               | 2 reads + up to 7 popcounts  | 28 reads + 98 popcounts |

`AdvancePositions` already solves this with `ib_select_samples` (one sample
per 256 ones), reducing select to a linear scan over ~256 ones instead of
binary search.

### Callsites

| Location                 | Function                  | Used for                    |
|--------------------------|---------------------------|-----------------------------|
| `light.rs:988`           | `ib_select1_from`         | `raw_bytes()` container end |
| `light.rs:1753-1767`     | `ib_rank1` + `ib_select1` | `at_offset()` navigation    |
| `locate.rs:88-102`       | `ib_rank1` + `ib_select1` | `yq-locate` offset lookup   |

These are on the `yq-locate` and `at_offset` paths, not the main streaming
identity path. But they are called for every navigation query result.

### Proposed change

Add a `ib_select_samples: Vec<u32>` field to `YamlIndex`, built during
construction using the same `build_select_samples()` function already in
`advance_positions.rs`. Modify `ib_select1()` to use samples for initial
narrowing instead of full binary search.

The `build_select_samples()` function could be moved to a shared location
(e.g., `bits/select_samples.rs` or reused from `advance_positions`).

### Memory cost

One `u32` per 256 ones in IB. For a 1MB file with ~128K IB ones (typical
density ~12.5%), this is ~500 samples × 4 bytes = ~2KB. Negligible.

---

## Opportunity 4: Shared IB Bitmap Between YamlIndex and AdvancePositions

**Impact: Medium (memory)**
**Effort: High**

### Problem

Two text-length bitmaps exist independently:

| Bitmap                     | Location               | Marks                    | Size    |
|----------------------------|------------------------|--------------------------|---------|
| `YamlIndex.ib`             | `index.rs`             | Scalar positions only    | L/8     |
| `AdvancePositions.ib_words`| `advance_positions.rs` | All node start positions | L/8     |

`ib_words` is a strict superset of `ib` (it includes container and seq item
positions in addition to scalars). Both are L/8 bytes, so together they cost
L/4 bytes = 25% of text size.

### Potential approach

Store only `ib_words` (the superset) and derive scalar-only queries using the
`containers` and `seq_items` bitmaps:

```
ib_scalar_rank(pos) = ib_words_rank(pos) - containers_rank(pos) - seq_items_rank(pos)
```

This eliminates one L/8-byte bitmap and its CompactRank index.

### Why this is hard

1. `ib` is built during parsing (by `set_ib()` calls), while `ib_words` is
   built during `AdvancePositions::build_unchecked()` from `bp_to_text` entries.
   They are constructed at different times from different data.

2. The relationship `ib_words = ib ∪ container_positions ∪ seq_item_positions`
   needs verification — are there edge cases where container/seq_item positions
   don't appear in `bp_to_text`?

3. `AdvancePositions` currently owns its `ib_words`. Sharing would require
   either a reference (lifetime complexity) or moving ownership to `YamlIndex`
   and passing a reference to `AdvancePositions`.

4. The derived rank formula adds 2 extra rank lookups per query, which may
   hurt the hot path.

### Recommendation

Defer this optimization. The coupling and complexity outweigh the L/8 byte
savings (~12.5% of text). Focus on Opportunity 1 (`bp_to_text_end`) for a
larger and simpler memory win.

---

## Opportunity 5: Remove Trivial Wrapper Functions

**Impact: Negligible**
**Effort: Trivial**

### Problem

Two one-line wrapper functions in `index.rs` add indirection without value:

```rust
fn build_ib_rank(words: &[u64]) -> CompactRank {
    CompactRank::build(words)
}

fn build_containers_rank(words: &[u64]) -> CompactRank {
    CompactRank::build(words)
}
```

### Action

Replace calls with `CompactRank::build()` directly at the 6 call sites in
`index.rs` (lines 138, 139, 193, 194, 243, 244). Remove the wrapper functions.

---

## Opportunity 6: CompactRank Intra-Block Popcount Cost

**Impact: Low-Medium (performance)**
**Effort: Medium**

### Problem

`CompactRank::rank_at_word()` does up to 7 `count_ones()` calls for the
intra-block remainder (words between the L2 block boundary and the query
position):

```rust
// compact_rank.rs:128-131
let block_start = block_idx * L2_WORDS;
for &word in &words[block_start..word_idx] {
    count += word.count_ones() as usize;
}
```

For single rank calls (like `count_containers_before` in the streaming path),
this is negligible. But binary search paths compound the cost:

| Path                        | rank_at_word calls | Max intra-block popcounts |
|-----------------------------|--------------------|---------------------------|
| `count_containers_before`   | 1                  | 7                         |
| `ib_rank1`                  | 1                  | 7                         |
| `ib_select1` (binary search)| ~14 (1MB file)     | 98                        |
| `ib_select1_from` (galloping)| ~8 (with hint)    | 56                        |

### Possible mitigations

**Option A: Reduce L2_WORDS to 4** — max 3 popcounts per query, but overhead
doubles from 3.1% to 6.25%. Exceeds the 5% target.

**Option B: Add L3 (one u8 per word)** — zero popcounts, but adds 12.5%
overhead. Total CompactRank overhead becomes ~16%. Defeats the purpose.

**Option C: Rely on Opportunity 3** — Adding select samples eliminates most
binary search iterations, which is where the compounding happens. Single rank
calls (the hot path) are unaffected.

### Recommendation

**Option C** — Address this indirectly through Opportunity 3 (select samples).
The single-rank hot path (`count_containers_before`, `ib_rank1`) is fast
enough. The binary search paths are the problem, and select samples solve them
more elegantly than changing CompactRank's block size.

---

## Opportunity 7: Parser Clone Avoidance

**Impact: Medium (build performance)**
**Effort: Low**

### Status

The current branch is named `avoid-clone` and `src/yaml/parser.rs` has
uncommitted changes, indicating this is already in progress.

The parser's `finalize()` method uses `core::mem::take()` to move buffers
out of the parser without cloning. Verify the uncommitted changes address
any remaining unnecessary clones during `SemiIndex` construction.

---

## Priority Summary

| #  | Opportunity                        | Type        | Impact  | Effort  | Depends on  |
|----|------------------------------------|-------------|---------|---------|-------------|
| 1  | `bp_to_text_end` compression       | Memory      | High    | Medium  | —           |
| 2  | Remove dead `count_seq_items_before`| Cleanup    | Low     | Trivial | —           |
| 3  | IB select samples                  | Performance | Medium  | Low     | —           |
| 5  | Remove trivial wrappers            | Cleanup     | Negl.   | Trivial | —           |
| 7  | Parser clone avoidance             | Build perf  | Medium  | Low     | In progress |
| 6  | CompactRank intra-block cost       | Performance | Low-Med | Medium  | #3 first    |
| 4  | Shared IB bitmap                   | Memory      | Medium  | High    | Defer       |

### Recommended execution order

1. **Opportunities 2 + 5** (trivial cleanup — do immediately)
2. **Opportunity 3** (IB select samples — low effort, addresses #6 indirectly)
3. **Opportunity 1** (bp_to_text_end compression — the big win)
4. **Opportunity 7** (finish parser clone avoidance — already in progress)

Opportunity 4 (shared bitmap) and 6 (intra-block cost) are deferred — the
ROI doesn't justify the complexity.

---

## Relationship to Other Plans

| Plan                             | Relationship                                                |
|----------------------------------|-------------------------------------------------------------|
| [yq-memory-optimization.md]      | M2 streaming eliminates DOM; this plan reduces index memory |
| [compact-index-investigation.md] | CompactRank was the outcome; this plan builds on it         |

The two optimizations are complementary:
- **yq-memory-optimization** reduces runtime memory (DOM → streaming)
- **This plan** reduces index memory (bp_to_text_end compression)

Together, the goal is total memory ≈ 1.1-1.5× input size (currently ~1.25×
for index alone, plus runtime overhead).

## Changelog

| Date       | Change                                                    |
|------------|-----------------------------------------------------------|
| 2026-01-27 | Initial analysis post-CompactRank migration               |
