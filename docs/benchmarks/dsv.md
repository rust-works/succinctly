# DSV (CSV/TSV) Benchmarks

[Home](/) > [Docs](../) > [Benchmarks](./) > DSV

Performance benchmarks for DSV parsing via `succinctly jq --input-dsv`.

## Summary

| Metric | ARM (M1 Max) | x86_64 (Ryzen 9) |
|--------|--------------|------------------|
| **CLI Throughput** | 6-29 MiB/s | 9-40 MiB/s |
| **API Iteration** | - | 85-1676 MiB/s |
| **Parse Speed** | ~0.8 GB/s | 1.3-3.7 GB/s |

## Platforms

### Platform 1: Apple M1 Max (ARM)
- **CPU**: Apple M1 Max
- **SIMD**: NEON (16 bytes/iter)
- **Build**: `cargo build --release --features cli`

### Platform 2: AMD Ryzen 9 7950X (x86_64)
- **CPU**: AMD Ryzen 9 7950X (Zen 4)
- **SIMD**: AVX2 + BMI2 (64 bytes/iter)
- **Build**: `RUSTFLAGS="-C target-cpu=native" cargo build --release --features cli`

## CLI Throughput (End-to-End)

Full jq pipeline including DSV parsing, iteration, and JSON output.

### ARM (M1 Max) - 10MB Files

| Pattern | Time | Throughput |
|---------|------|------------|
| strings | 352ms | 28.4 MiB/s |
| multiline | 503ms | 19.9 MiB/s |
| quoted | 627ms | 16.0 MiB/s |
| pathological | 716ms | 14.0 MiB/s |
| tabular | 908ms | 11.0 MiB/s |
| users | 954ms | 10.5 MiB/s |
| numeric | 957ms | 10.5 MiB/s |
| mixed | 949ms | 10.5 MiB/s |
| wide | 1.40s | 7.2 MiB/s |
| long | 1.57s | 6.4 MiB/s |

### x86_64 (Ryzen 9) - 10MB Files

| Pattern | Time | Throughput |
|---------|------|------------|
| strings | 254ms | 39.4 MiB/s |
| multiline | 336ms | 29.7 MiB/s |
| quoted | 369ms | 27.1 MiB/s |
| tabular | 548ms | 18.2 MiB/s |
| users | 587ms | 17.0 MiB/s |
| numeric | 585ms | 17.1 MiB/s |
| mixed | 549ms | 18.2 MiB/s |
| pathological | 544ms | 18.4 MiB/s |
| wide | 1.09s | 9.1 MiB/s |
| long | 918ms | 10.9 MiB/s |

## Query Comparison

Single column selection (`.[0]`) vs full output (`.`) - x86_64:

| Pattern | Full Output | Single Column | Speedup |
|---------|-------------|---------------|---------|
| wide | 44.7 MiB/s | 83.8 MiB/s | **1.88x** |
| users | 22.1 MiB/s | 41.0 MiB/s | **1.85x** |
| tabular | 19.5 MiB/s | 34.9 MiB/s | **1.79x** |
| strings | 126.4 MiB/s | 169.0 MiB/s | **1.34x** |

**Average speedup**: 1.67x for single column selection.

## Library API Performance (x86_64)

Direct API usage without JSON serialization overhead.

### Sequential Iteration

| Pattern | Throughput |
|---------|------------|
| strings | 1676 MiB/s |
| quoted | 1331 MiB/s |
| mixed | 792 MiB/s |

### Random Access

| Pattern | Ops/sec | ns/field |
|---------|---------|----------|
| strings | 10.7M | 93 ns |
| mixed | 5.8M | 173 ns |

### Parse Speed (Index Build Only)

| Pattern | Throughput |
|---------|------------|
| strings | 3.7 GB/s |
| mixed | 1.3 GB/s |

## Memory Usage

| File Size | Peak Memory |
|-----------|-------------|
| 1 MB | 5-9 MB |
| 10 MB | 17-21 MB |
| 100 MB | 138-143 MB |

**Index overhead**: ~3-4% of input size.

## Optimization Impact

### Lightweight Index (2026-01-12)

Replaced full BitVec with simple cumulative rank arrays:

| Metric | Before | After | Improvement |
|--------|--------|-------|-------------|
| Field iteration | 145 MiB/s | 792-1331 MiB/s | **5-9x** |
| Memory overhead | ~6% | ~3-4% | **50% less** |

## Running Benchmarks

```bash
# CLI benchmarks
./target/release/succinctly dev bench dsv

# Single column
./target/release/succinctly dev bench dsv --query '.[0]'

# Library benchmarks
RUSTFLAGS="-C target-cpu=native" cargo build --release --examples
./target/release/examples/dsv_end_to_end_bench
./target/release/examples/dsv_single_field_bench
```

## See Also

- [DSV Parsing](../parsing/dsv.md) - Implementation details
- [Hierarchical Structures](../optimizations/hierarchical-structures.md) - Why lightweight index is faster
