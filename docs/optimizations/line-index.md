# Line Index: Sparse Monotone Sets Beat Dense Bitmaps

[Home](../../) > [Docs](../) > [Optimizations](./) > Line index

How succinctly stores line-start offsets, why a dense bitmap was the wrong shape for them, and the
arithmetic that decides between the two. Implemented in
[src/text/lines.rs](../../src/text/lines.rs) for [issue #228](https://github.com/rust-works/succinctly/issues/228);
the decision record is [ADR-0012](../adrs/adr-0012.md).

## The problem

Three modules each kept a line-start index as a `BitVec`: one bit per text byte, set at the byte
after every LF/CRLF/CR. Rank gave the line number, select gave the line start.

That is a correct use of rank/select and the wrong representation, because **line starts are sparse
and monotone**. A dense bitmap's cost is fixed by the size of the *universe* (the text), not by the
size of the *set* (the lines):

```
bitmap        1/8 byte per input byte                = 0.1250 B/byte
rank directory  128 bits metadata per 512 bits data  = 0.0313 B/byte
select samples  16 bytes per 256 ones                ≈ 0.0031 B/byte at 20 B/line
                                                       ---------
                                                       ~0.159 B/byte = 15.9% of input
```

Every file paid ~16% of its size, whether it had 2 lines or 20,000. The worst case in the corpus is
a 100 KB GeoJSON file with 60 long lines: 15,608 bytes of index for 60 numbers.

## The fix

Store the line starts themselves, Elias-Fano encoded
([src/bits/elias_fano.rs](../../src/bits/elias_fano.rs)). For `n` values over universe `u`, Elias-Fano
splits each value into `low_width = floor(log2(u/n))` low bits stored densely and a unary-coded high
part, costing about `2 + log2(u/n)` bits per element. With `A` = average bytes per line, that is
`2 + log2(A)` bits **per line** rather than `1.27 * A` bits per line.

| Average line length | Dense bitmap | `LineIndex` | Factor |
|---------------------|--------------|-------------|--------|
| 20 B (YAML/CSV)     | 15.9%        | ~3.9%       | ~4x    |
| 78 B (pretty JSON)  | 15.7%        | ~1.3%       | ~12x   |
| minified (one line) | 15.6%        | ~0          | huge   |

Measured over the real-workload corpus (both columns are the structures' own `heap_size()`, recorded
per file in [benchmarks/corpus-shape.md](../benchmarks/corpus-shape.md)):

| file                     | bytes  | lines | dense bitmap | `LineIndex` | factor   |
|--------------------------|--------|-------|--------------|-------------|----------|
| us-election.geojson      | 99,715 | 60    | 15,608       | 108         | **145x** |
| gapminder-five-year.csv  | 82,079 | 1,705 | 12,952       | 1,636       | **7.9x** |
| prometheus-ci.yml        | 18,935 | 453   | 2,992        | 432         | **6.9x** |
| world-gdp-with-codes.csv | 4,515  | 223   | 728          | 180         | **4.0x** |
| nginx-deployment.yml     | 836    | 43    | 160          | 44          | **3.6x** |

## Where the crossover is

Elias-Fano is not unconditionally smaller. Setting the two costs equal:

```
2 + log2(A) bits/line  vs  1.27 * A bits/line     →  equal at A ≈ 2.6
```

Below ~2.6 bytes per line — a file that is almost entirely newlines — the dense bitmap wins. The
corpus runs 19-78 bytes per line, so this does not arise in practice, but the honest statement is
"sparser than ~1 line per 3 bytes", not "always smaller".

The same arithmetic rules out two representations that look plausible:

| Representation                             | Cost at A=20    | Verdict                                                                       |
|--------------------------------------------|-----------------|-------------------------------------------------------------------------------|
| Dense bitmap + rank + select               | 15.9% of input  | The status quo                                                                |
| Cumulative-rank `Vec<u32>` (the DSV index) | 18.75% of input | **Worse** — 4 B per 8 B of bitmap is 50% overhead vs the rank directory's 25% |
| `Vec<u32>` of line starts                  | 20% of input    | **Worse below 26 B/line**; better above                                       |
| Elias-Fano line starts                     | ~3.9% of input  | Adopted                                                                       |

The `Vec<u32>` row is the trap: it looks like the obvious sparse encoding, and it loses on the
corpus's dominant shape (short YAML and CSV lines) while winning on pretty-printed JSON. Checking the
corpus before choosing is what separates it from Elias-Fano, which wins on both.

## Don't build what nobody reads

The bigger win was not the encoding. `JsonIndex::build` built its newline index **eagerly**, so every
jq query paid an O(n) scan and a full bitmap allocation for a structure only `at_position(line; col)`
and `jq-locate --line --column` ever read. YAML had already made its equivalent lazy (`OnceCell`) in
P12-A/A4; JSON now matches.

Cost of the lazy path is paid only by callers who need it:

| Structure                                    | Before                   | After                 |
|----------------------------------------------|--------------------------|-----------------------|
| `JsonIndex::build` allocation, 99 KB GeoJSON | 60,514 B                 | 44,906 B (**−25.8%**) |
| `JsonIndex::build` allocation, 548 B JSON    | 409 B                    | 289 B (**−29.3%**)    |
| `YamlIndex::build`                           | unchanged (already lazy) | unchanged             |

The deltas equal the dense index's `heap_size()` exactly — the allocation is simply gone.

## Query cost, and why O(log n) is fine here

`to_line_column` needs "how many line starts precede this offset", which is a predecessor query.
Elias-Fano has no rank, so `EliasFano::predecessor` is a binary search over `get` — O(log n) with a
sampled select per step, against the bitmap's O(1) rank.

That trade is deliberate. Every consumer is cold: error reporting, `$__loc__`'s document sibling
`at_position`, and the two locate CLIs. Nothing in the parse or query hot path touches it.
`to_offset` does not even need the predecessor — a line number is a direct index, so it stays O(1).

If a hot consumer ever appears, the accelerator is already designed: `select_samples[j] -
j * SAMPLE_RATE` is the high part of element `j*256` and is monotone in `j`, so binary-searching that
plain `Vec<u32>` narrows to a 256-element block with zero select calls.

## Transient build memory

Building Elias-Fano needs the values up front (`universe` sizes `low_width`), so `LineIndex::build`
collects a `Vec<u32>` of line starts first. A counting pass sizes it exactly, which matters more than
it looks: without it, `Vec` doubling makes the transient 1.5-2x the vector — the failure mode O4
documented, where transient scratch dwarfed the retained structure.

Measured momentary peak against the old *retained* cost:

| file                                   | old retained | new peak | new retained |
|----------------------------------------|--------------|----------|--------------|
| codeql-analysis.yml (925 B, 39 lines)  | 168          | 204      | 44           |
| nginx-deployment.yml (836 B, 43 lines) | 160          | 220      | 44           |
| prometheus-ci.yml (18.9 KB, 453 lines) | 2,992        | 2,248    | 432          |

Peak is higher on small short-lined files (crossover ~26 B/line) and lower on larger ones — and it is
momentary, where the old cost was permanent. Removing the transient entirely would need a streaming
`EliasFanoBuilder`; at this scale that is gold-plating.

## Lessons

- **A rank/select structure can be correct and still be the wrong shape.** The question is not "is
  the bitmap's rank fast" but "should cost scale with the universe or with the set".
- **Cost that scales with the wrong quantity hides in small files.** 15.9% of a 900-byte manifest is
  143 bytes and invisible; the same rule costs 15,608 bytes on a 100 KB file with 60 lines.
- **The obvious sparse encoding was the wrong one.** `Vec<u32>` of offsets loses on the corpus's
  dominant shape. Checking [corpus-shape.md](../benchmarks/corpus-shape.md) before choosing is the
  same discipline that rejected P5.
- **Not building it beats making it smaller.** A 4-12x smaller structure is worth less than not
  constructing the structure at all on the hot path.
- **Exact-size the scratch vector.** `Vec::with_capacity` from a counting pass costs one cheap
  auto-vectorised scan and removes a 1.5-2x transient spike.

## See Also

- [ADR-0012](../adrs/adr-0012.md) — the decision and the rejected options
- [end-positions.md](end-positions.md) — the other compact-encoding work (P12, O1, O2)
- [cache-memory.md](cache-memory.md) — the DSV lightweight index, whose 5-9x iteration win does not
  transfer to this use
- [hierarchical-structures.md](hierarchical-structures.md) — the rank directory and select index this
  avoids
