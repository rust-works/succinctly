# DsvIndex

[Home](../../) > [Docs](../) > [Parsing](./) > DsvIndex

Semi-index for delimiter-separated value (CSV/TSV) files. Marks field and row boundaries without materializing data, enabling O(1) field lookup with ~3-4% overhead.

## What It Does

`DsvIndex` scans a DSV file once, producing two bitvectors:

| Component | Marks                                             | Size            |
|-----------|---------------------------------------------------|-----------------|
| Markers   | Delimiter and newline positions (outside quotes)  | ~1 bit/byte     |
| Newlines  | Row boundary positions only                       | ~1 bit/64 bytes |

With rank/select on these bitvectors, any cell `(row, column)` can be located in O(1).

## Quote Handling

The central challenge is determining whether a delimiter or newline is inside or outside quotes. DSV uses `""` escaping (context-free), which enables a powerful bit-parallel approach.

### Quote State via Prefix XOR

Compute cumulative XOR over the quote-bit positions to track parity (inside/outside):

```
Text:     "hello, world",name
Quotes:   1            1
XOR:      1111111111111100000  (1 = inside quotes)
```

Three implementations, selected by CPU features:

| Method          | Platform      | Speedup vs scalar | Technique                |
|-----------------|---------------|-------------------|--------------------------|
| `toggle64_bmi2` | x86 BMI2      | **10x**           | PDEP + carry propagation |
| `prefix_xor`    | AVX2/SSE/NEON | baseline          | Parallel prefix XOR      |
| Scalar          | All           | 1x                | Byte-by-byte loop        |

The BMI2 path uses `PDEP` to scatter quote bits, then a carry-propagation trick to compute the running XOR in a single instruction chain. This is the same technique that makes DSV indexing dramatically faster than JSON or YAML indexing per byte.

## Performance

DSV indexing throughput (API-level, not end-to-end CLI):

| Platform             | Throughput    |
|----------------------|---------------|
| x86_64 (BMI2)       | 85-1676 MiB/s |
| ARM64 (NEON + PMULL) | Comparable    |

## Depends On

- [BitVec](../architecture/bitvec.md) — marker and newline bitvectors with rank/select

## Used By

- [jq Evaluator](../reference/jq-evaluator.md) — DSV input via `--input-dsv` flag
- CLI: `succinctly dsv generate` for test data generation

## Source & Docs

- Implementation: [src/dsv/](../../src/dsv/) (parser.rs, index.rs, index_lightweight.rs, cursor.rs)
- SIMD: [src/dsv/simd/](../../src/dsv/simd/) (avx2.rs, bmi2.rs, sse2.rs, neon.rs, sve2.rs)
- Parsing doc: [dsv.md](dsv.md)
- Benchmark: [benchmarks/dsv.md](../benchmarks/dsv.md)
