# YAML Succinct Parsing: Buffer Layout

[Home](../../) > [Docs](../) > [Parsing](./) > YAML Succinct

This document traces through a small YAML example to show what each buffer
in `YamlIndex` and `AdvancePositions` contains and why it exists.

## Example YAML

```yaml
a: &x b
c:
- d
- *x
e:
  f: g
```

This is 30 bytes. Here are the byte positions:

```
pos:                                1  1  1  1  1  1  1  1  1  1  2  2  2  2  2  2  2  2  2  2
      0  1  2  3  4  5  6  7  8  9  0  1  2  3  4  5  6  7  8  9  0  1  2  3  4  5  6  7  8  9
char: a  :  _  &  x  _  b  $  c  :  $  -  _  d  $  -  _  *  x  $  e  :  $  _  _  f  :  _  g  $
```

## Which construct exercises which buffer

| Construct                         | Exercises                                                      |
|-----------------------------------|----------------------------------------------------------------|
| `a`, `b`, `c`, `d`, `e`, `f`, `g` | `ib`, `ib_rank`, `bp`, `open_positions`, `bp_to_text_end`      |
| Root mapping, nested `e:` mapping | `ty` (bit=0), `containers`, `containers_rank`                  |
| Sequence under `c:`               | `ty` (bit=1), `containers`, `containers_rank`                  |
| `- d`, `- *x` (sequence items)    | text-derived (no `seq_items` buffer)                           |
| `&x` on `b`                       | `anchors`, `bp_to_anchor`                                      |
| `*x`                              | `aliases`                                                      |
| Multi-line                        | `newlines`                                                     |

## Text-position-indexed buffers (1 bit per byte)

```
pos:                                1  1  1  1  1  1  1  1  1  1  2  2  2  2  2  2  2  2  2  2
      0  1  2  3  4  5  6  7  8  9  0  1  2  3  4  5  6  7  8  9  0  1  2  3  4  5  6  7  8  9
char: a  :  _  &  x  _  b  $  c  :  $  -  _  d  $  -  _  *  x  $  e  :  $  _  _  f  :  _  g  $

ib:   1  0  0  0  0  0  1  0  1  0  0  0  0  1  0  0  0  1  0  0  1  0  0  0  0  1  0  0  1  0
iw:   1  0  0  0  0  0  1  0  1  0  0  1  0  1  0  1  0  1  0  0  1  0  0  0  0  1  0  0  1  0
                                       ^           ^
                                       seq cont    seq item 2
                                       + item 1    (no scalar)
                                       (no scalar)

nl:   0  0  0  0  0  0  0  0  1  0  0  1  0  0  0  1  0  0  0  0  1  0  0  1  0  0  0  0  0  0

ib = interest bits, iw = ib_words, nl = newlines
```

`ib` is set only at scalar text positions (where `set_ib()` is called during parsing).

`ib_words` (inside `AdvancePositions`) is set at all unique node start positions,
including containers and sequence items. It is a strict superset of `ib`. The two
extra bits at positions 11 and 15 are where containers/seq-items open at the `-`
character, but no `set_ib()` is called there -- the scalar follows after `- `.

`newlines` marks the first byte of each line after a line terminator.
Built lazily via `OnceCell` on first call to `to_line_column()` or `to_offset()`
(used by `yq-locate` CLI). Not constructed during normal parsing or query evaluation.

## bp_to_text (text positions, duplicates stacked)

```
pos:                                1  1  1  1  1  1  1  1  1  1  2  2  2  2  2  2  2  2  2  2
      0  1  2  3  4  5  6  7  8  9  0  1  2  3  4  5  6  7  8  9  0  1  2  3  4  5  6  7  8  9
char: a  :  _  &  x  _  b  $  c  :  $  -  _  d  $  -  _  *  x  $  e  :  $  _  _  f  :  _  g  $

r0:   1  0  0  0  0  0  1  0  1  0  0  1  0  1  0  1  0  1  0  0  1  0  0  0  0  1  0  0  1  0
r1:   1  0  0  0  0  0  0  0  0  0  0  1  0  0  0  0  0  0  0  0  0  0  0  0  0  1  0  0  0  0
      |                                |                                         |
      root map                         seq container                             nested map
      + key "a"                        + seq item 1                              + key "f"
```

Row 0 = `ib_words` (the unique positions). Row 1 shows the three positions where
a container shares its text position with its first child. These duplicates are
what `advance_words` encodes.

## BP-open-indexed buffers (1 bit per BP open, 13 opens)

```
bp:                                 1  1  1
      0  1  2  3  4  5  6  7  8  9  0  1  2
node: M  a  b  c  S  .  d  .  *  e  M  f  g
                     -------------- --------

adv:  1  0  1  1  1  0  1  1  1  1  1  0  1
con:  1  0  0  0  1  0  0  0  0  0  1  0  0
seq:  0  0  0  0  0  1  0  1  0  0  0  0  0
ty:                  S                 M

M = mapping, S = sequence, . = seq item wrapper, * = alias *x
```

The `0` bits in `advance` at indices 1, 5, 11 correspond to the duplicate
positions from row 1 above -- the container and its first child share a text
position, so no advance is needed.

## Design constraint: bitmaps, not position vectors

Position vectors (`Vec<u32>`) store one 32-bit integer per element. For a text
of L bytes with N nodes (N ~ L/7.5 for typical YAML), a position vector costs
4N bytes -- roughly 53% of the text size. Three position vectors (one each for
`ib`, `ib_words`, `newlines`) would cost ~160% of the text, far exceeding the
3-6% semi-index overhead budget.

Bitmaps cost 1 bit per text byte (L/8 bytes = 12.5% of text). Three bitmaps
cost 3L/8 = 37.5% of text -- still large, but bitmaps compress well because
they are sparse, and their rank-select indices should stay under **5% overhead**
relative to the bitmap they index.

| Representation               | Cost per text byte | 3 structures |
|------------------------------|--------------------|--------------|
| Position vector              | 4N/L ~ 0.53        | ~160%        |
| Bitmap                       | 1 bit = 0.125      | ~37.5%       |
| Cumulative rank (`Vec<u32>`) | 4 B/word = 0.0625  | ~18.75%      |

Rank support is a cumulative popcount array holding one `u32` per 64-bit word
([`build_cumulative_rank`](../../src/yaml/advance_positions.rs)), so each rank
index costs **50% of the bitmap it indexes** -- 6.25% of the text for the
text-length bitmaps. That misses the 5% target above by an order of magnitude,
and it is a deliberate trade rather than an oversight: rank is one array load
plus one masked popcount, with no hierarchical navigation.

A two-level `CompactRank` directory (L1: one `u32` per 128 words + L2: one `u16`
per 8 words, ~3.5% of the bitmap) did meet the 5% target and was used by these
structures briefly (`bc169609`). It was swapped out for the cumulative arrays in
`0156707a`, which cited cache locality and the single-lookup query for
sequential streaming access; that commit recorded no measurements, and its
claimed "~3.5% to ~3.1%" memory figure is wrong in the direction of the trade it
made. Left with no callers, `CompactRank` was deleted from the crate in #321.
Whether to close the resulting 10x gap against the 5% target is still open --
see [ADR-0011](../adrs/adr-0011.md) and issue #321.

The same budget-driven reasoning cut the other way for `seq_items`: since the
sequence-item property is derivable from the text (`-` + whitespace), storing it
was pure overhead and the bitvector was eliminated outright (issues #75/#104).
Measured saving: 19–41 KB retained per MB of input (3–5% of the index) plus a
2-bit-per-byte transient build allocation (−12.5% build peak). See
[yaml.md O4](yaml.md#o4-seq_items-bitvector-elimination--accepted-).

## Current derived index structures

Every raw bitmap that needs O(1) rank queries gets a cumulative popcount array,
one `u32` per 64-bit word, at ~50% of the bitmap it indexes:

| Bitmap                     | Rank index                        | Type       | Overhead |
|----------------------------|-----------------------------------|------------|----------|
| `ib`                       | `ib_rank` (in `YamlIndex`)        | `Vec<u32>` | ~50%     |
| `ib_words` (in AdvancePos) | `ib_rank` (in `AdvancePositions`) | `Vec<u32>` | ~50%     |
| `advance_words`            | `advance_rank`                    | `Vec<u32>` | ~50%     |
| `containers`               | `containers_rank`                 | `Vec<u32>` | ~50%     |

Entry `i` holds the popcount of words `[0, i)`, so the array is one longer than
the word count.

Query: `rank1(pos) = rank[pos / 64] + popcount(words[pos / 64] & ((1 << pos % 64) - 1))`
-- one load and one masked popcount, regardless of bitmap size
([`ib_rank1`](../../src/yaml/advance_positions.rs), [`advance_rank1`](../../src/yaml/advance_positions.rs)).

`AdvancePositions.ib_words` additionally has `ib_select_samples` to support O(1)
select queries, needed by the `get()` random access path: one `u32` per 256
1-bits, so its cost scales with density rather than with the bitmap -- ~1.7% of
the bitmap at the typical N ~ L/7.5.

## Relationship between YamlIndex and AdvancePositions

```
                     YamlIndex                                AdvancePositions
                     ---------                                ----------------

      Primary             Derived                    Primary          Derived
      -------             -------                    -------          -------

      ib ----------------> ib_rank                   ib_words ------> ib_rank
      (scalars only)       Vec<u32> cumulative       (all nodes)      Vec<u32> cumulative
         |                                              |
         `---------------> ib_len ..................... ib_len
                           = text len    duplicate      = text len
                                                        |
                                                        |-----------> ib_select_samples
                                                        |             sampled select
                                                        |
                                                        `-----------> ib_ones
                                                                      total popcount

      containers --------> containers_rank
                           Vec<u32> cumulative       advance_words --> advance_rank
                                                     (dup markers)    Vec<u32> cumulative
      ty ----------------> ty_len                       |
                                                        `-----------> num_opens
      bp
      (BalancedParens with internal
       rank, select, min-excess)

      bp_to_text_end       (no index)
      newlines             (lazy OnceCell, has internal rank)
      anchors <-- inverse --> bp_to_anchor
      aliases
```

`ib_len` is duplicated between `YamlIndex` and `AdvancePositions` (both store
the text length). `num_opens` is derivable from `bp` (half of `bp.len()`).

## Architecture: separation of index and data

`YamlIndex` does not store the raw YAML text. The raw bytes are passed into
`build()`, used to construct all index structures, and then not retained.
At query time, the caller reunites the text with the index through a cursor:

```
  build time                          query time
  ----------                          ----------

  yaml: &[u8] --parse-->  YamlIndex       YamlCursor
                          (no text)       +---------------+
                              |           | index: &Index |<-- structural navigation
                              `---------->| text:  &[u8]  |<-- value extraction
                                          | bp_pos        |
                                          +---------------+
```

The index provides structural navigation (first_child, next_sibling, parent --
all pure BP tree operations). The text provides value extraction
(`text[start..end]` using positions from `open_positions` and `bp_to_text_end`).
