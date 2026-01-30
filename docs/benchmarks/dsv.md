# DSV (CSV/TSV) Benchmarks

[Home](/) > [Docs](../) > [Benchmarks](./) > DSV

Performance benchmarks for DSV parsing via `succinctly jq --input-dsv`.

## Summary

| Metric             | ARM (M1 Max) | x86_64 (Ryzen 9)  | ARM (Graviton 4) | ARM (Graviton 3) |
|--------------------|--------------|-------------------|------------------|------------------|
| **CLI Throughput** | 6-29 MiB/s   | 10-103 MiB/s      | 7-89 MiB/s       | 7-90 MiB/s       |
| **API Iteration**  | -            | 85-1676 MiB/s     | ~1000 MiB/s      | ~1000 MiB/s      |
| **Parse Speed**    | ~0.8 GB/s    | 1.3-3.7 GB/s      | ~3.8 GB/s        | ~3.8 GB/s        |

## Platforms

### Platform 1: Apple M1 Max (ARM)
- **CPU**: Apple M1 Max
- **SIMD**: NEON (16 bytes/iter)
- **Build**: `cargo build --release --features cli`

### Platform 2: AMD Ryzen 9 7950X (x86_64)
- **CPU**: AMD Ryzen 9 7950X (Zen 4)
- **SIMD**: AVX2 + BMI2 (64 bytes/iter)
- **Build**: `RUSTFLAGS="-C target-cpu=native" cargo build --release --features cli`

### Platform 3: ARM Neoverse-V2 (AWS Graviton 4)
- **CPU**: ARM Neoverse-V2 (AWS Graviton 4)
- **SIMD**: NEON (16 bytes/iter), SVE2 with SVEBITPERM
- **Build**: `cargo build --release --features cli`

### Platform 4: ARM Neoverse-V1 (AWS Graviton 3)
- **CPU**: ARM Neoverse-V1 (8 cores)
- **SIMD**: NEON (16 bytes/iter), SVE (256-bit vectors)
- **Build**: `cargo build --release --features cli`
- **Date**: 2026-01-29

## CLI Throughput (End-to-End)

Full jq pipeline including DSV parsing, iteration, and JSON output.

### ARM (M1 Max) - 10MB Files

| Pattern      | Time  | Throughput |
|--------------|-------|------------|
| strings      | 352ms | 28.4 MiB/s |
| multiline    | 503ms | 19.9 MiB/s |
| quoted       | 627ms | 16.0 MiB/s |
| pathological | 716ms | 14.0 MiB/s |
| tabular      | 908ms | 11.0 MiB/s |
| users        | 954ms | 10.5 MiB/s |
| numeric      | 957ms | 10.5 MiB/s |
| mixed        | 949ms | 10.5 MiB/s |
| wide         | 1.40s | 7.2 MiB/s  |
| long         | 1.57s | 6.4 MiB/s  |

### x86_64 (Ryzen 9) - 10MB Files

| Pattern      | Time   | Throughput  |
|--------------|--------|-------------|
| strings      |  93ms  | 107.2 MiB/s |
| quoted       | 179ms  |  55.8 MiB/s |
| pathological | 213ms  |  47.0 MiB/s |
| wide         | 238ms  |  42.0 MiB/s |
| multiline    | 302ms  |  33.1 MiB/s |
| users        | 521ms  |  19.2 MiB/s |
| numeric      | 542ms  |  18.5 MiB/s |
| tabular      | 572ms  |  17.5 MiB/s |
| mixed        | 573ms  |  17.5 MiB/s |
| long         | 1.02s  |   9.8 MiB/s |

### ARM Neoverse-V2 (Graviton 4) - 10MB Files

| Pattern      | Time  | Throughput |
|--------------|-------|------------|
| strings      | 113ms | 88.7 MiB/s |
| quoted       | 221ms | 45.3 MiB/s |
| pathological | 265ms | 37.7 MiB/s |
| wide         | 321ms | 31.1 MiB/s |
| multiline    | 412ms | 24.3 MiB/s |
| users        | 681ms | 14.7 MiB/s |
| numeric      | 731ms | 13.7 MiB/s |
| tabular      | 756ms | 13.2 MiB/s |
| mixed        | 751ms | 13.3 MiB/s |
| long         | 1.35s | 7.4 MiB/s  |

### ARM Neoverse-V1 (Graviton 3) - 10MB Files

| Pattern      | Time  | Throughput |
|--------------|-------|------------|
| strings      | 112ms | 89.3 MiB/s |
| quoted       | 222ms | 45.0 MiB/s |
| pathological | 264ms | 37.9 MiB/s |
| wide         | 320ms | 31.3 MiB/s |
| multiline    | 407ms | 24.6 MiB/s |
| users        | 691ms | 14.5 MiB/s |
| numeric      | 722ms | 13.8 MiB/s |
| tabular      | 751ms | 13.3 MiB/s |
| mixed        | 746ms | 13.4 MiB/s |
| long         | 1.32s | 7.6 MiB/s  |

### ARM Neoverse-V1 (Graviton 3) - Full Results by Pattern

**Pattern: tabular**

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    7.32s |   13.7 MiB/s |  142 MB |
| **10mb**  |  751.3ms |   13.3 MiB/s |   18 MB |
| **1mb**   |   77.5ms |   12.9 MiB/s |    5 MB |
| **100kb** |    9.3ms |   10.5 MiB/s |    4 MB |
| **10kb**  |    2.5ms |    3.9 MiB/s |    4 MB |
| **1kb**   |    1.8ms |    0.5 MiB/s |    4 MB |

**Pattern: users**

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    6.90s |   14.5 MiB/s |  142 MB |
| **10mb**  |  691.2ms |   14.5 MiB/s |   18 MB |
| **1mb**   |   70.8ms |   14.1 MiB/s |    5 MB |
| **100kb** |    8.5ms |   11.5 MiB/s |    4 MB |
| **10kb**  |    2.4ms |    4.1 MiB/s |    4 MB |
| **1kb**   |    1.8ms |    0.5 MiB/s |    4 MB |

**Pattern: numeric**

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    7.24s |   13.8 MiB/s |  142 MB |
| **10mb**  |  722.1ms |   13.8 MiB/s |   18 MB |
| **1mb**   |   73.1ms |   13.7 MiB/s |    5 MB |
| **100kb** |    8.6ms |   11.4 MiB/s |    4 MB |
| **10kb**  |    2.3ms |    4.2 MiB/s |    4 MB |
| **1kb**   |    1.8ms |    0.5 MiB/s |    4 MB |

**Pattern: strings (fastest)**

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    1.11s |   90.5 MiB/s |  142 MB |
| **10mb**  |  112.0ms |   89.3 MiB/s |   18 MB |
| **1mb**   |   12.5ms |   79.7 MiB/s |    5 MB |
| **100kb** |    2.7ms |   35.9 MiB/s |    4 MB |
| **10kb**  |    1.7ms |    5.7 MiB/s |    4 MB |
| **1kb**   |    1.7ms |    0.6 MiB/s |    4 MB |

**Pattern: quoted**

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    2.08s |   48.1 MiB/s |  142 MB |
| **10mb**  |  222.2ms |   45.0 MiB/s |   18 MB |
| **1mb**   |   23.5ms |   42.6 MiB/s |    5 MB |
| **100kb** |    3.9ms |   25.2 MiB/s |    4 MB |
| **10kb**  |    1.9ms |    5.2 MiB/s |    4 MB |
| **1kb**   |    1.8ms |    0.6 MiB/s |    4 MB |

**Pattern: multiline**

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    3.42s |   29.3 MiB/s |  142 MB |
| **10mb**  |  406.5ms |   24.6 MiB/s |   18 MB |
| **1mb**   |   40.9ms |   24.5 MiB/s |    5 MB |
| **100kb** |    5.8ms |   17.0 MiB/s |    4 MB |
| **10kb**  |    2.0ms |    4.8 MiB/s |    4 MB |
| **1kb**   |    1.7ms |    0.6 MiB/s |    4 MB |

**Pattern: wide**

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    3.28s |   30.5 MiB/s |  142 MB |
| **10mb**  |  319.9ms |   31.3 MiB/s |   18 MB |
| **1mb**   |   33.1ms |   30.3 MiB/s |    5 MB |
| **100kb** |    4.6ms |   21.3 MiB/s |    4 MB |
| **10kb**  |    2.0ms |    5.0 MiB/s |    4 MB |
| **1kb**   |    1.8ms |    0.6 MiB/s |    4 MB |

**Pattern: long (slowest)**

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |   13.19s |    7.6 MiB/s |  142 MB |
| **10mb**  |    1.32s |    7.6 MiB/s |   18 MB |
| **1mb**   |  140.2ms |    7.1 MiB/s |    5 MB |
| **100kb** |   15.9ms |    6.2 MiB/s |    4 MB |
| **10kb**  |    3.1ms |    3.1 MiB/s |    4 MB |
| **1kb**   |    1.9ms |    0.5 MiB/s |    4 MB |

**Pattern: mixed**

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    7.51s |   13.3 MiB/s |  142 MB |
| **10mb**  |  746.1ms |   13.4 MiB/s |   18 MB |
| **1mb**   |   76.0ms |   13.2 MiB/s |    5 MB |
| **100kb** |    9.0ms |   10.8 MiB/s |    4 MB |
| **10kb**  |    2.5ms |    3.9 MiB/s |    4 MB |
| **1kb**   |    2.0ms |    0.5 MiB/s |    4 MB |

**Pattern: pathological**

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    2.47s |   40.5 MiB/s |  142 MB |
| **10mb**  |  263.9ms |   37.9 MiB/s |   18 MB |
| **1mb**   |   28.1ms |   35.6 MiB/s |    5 MB |
| **100kb** |    4.5ms |   21.8 MiB/s |    4 MB |
| **10kb**  |    2.0ms |    4.8 MiB/s |    4 MB |
| **1kb**   |    1.8ms |    0.5 MiB/s |    4 MB |

## Query Comparison

Single column selection (`.[0]`) vs full output (`.`) - x86_64:

| Pattern | Full Output | Single Column | Speedup   |
|---------|-------------|---------------|-----------|
| wide    | 44.7 MiB/s  | 83.8 MiB/s    | **1.88x** |
| users   | 22.1 MiB/s  | 41.0 MiB/s    | **1.85x** |
| tabular | 19.5 MiB/s  | 34.9 MiB/s    | **1.79x** |
| strings | 126.4 MiB/s | 169.0 MiB/s   | **1.34x** |

**Average speedup**: 1.67x for single column selection.

## Library API Performance (x86_64)

Direct API usage without JSON serialization overhead.

### Sequential Iteration

| Pattern | Throughput |
|---------|------------|
| strings | 1676 MiB/s |
| quoted  | 1331 MiB/s |
| mixed   | 792 MiB/s  |

### Random Access

| Pattern | Ops/sec | ns/field |
|---------|---------|----------|
| strings | 10.7M   | 93 ns    |
| mixed   | 5.8M    | 173 ns   |

### Parse Speed (Index Build Only)

| Pattern | Throughput |
|---------|------------|
| strings | 3.7 GB/s   |
| mixed   | 1.3 GB/s   |

## Memory Usage

| File Size | Peak Memory |
|-----------|-------------|
| 1 MB      | 5-9 MB      |
| 10 MB     | 17-21 MB    |
| 100 MB    | 138-143 MB  |

**Index overhead**: ~3-4% of input size.

## Optimization Impact

### NEON PMULL Prefix XOR (2026-01-22)

Replaced scalar prefix XOR (6 shifts + 6 XORs) with carryless multiplication via NEON PMULL instruction:

| Metric             | Before     | After      | Improvement |
|--------------------|------------|------------|-------------|
| 10MB index build   | 3.18 GiB/s | 3.84 GiB/s | **+25%**    |
| 10MB index+iterate | 0.93 GiB/s | 1.00 GiB/s | **+8%**     |

**Key insight**: `prefix_xor(x) = clmul(x, 0xFFFF...FFFF)` in GF(2). Carryless multiplication by all-1s propagates each bit to all higher positions via XOR, computing prefix XOR in O(1) instead of O(log n).

**Platform support**: Works on all ARM64 with NEON (Apple M1+, Graviton 2+, all aarch64).

### Lightweight Index (2026-01-12)

Replaced full BitVec with simple cumulative rank arrays:

| Metric          | Before    | After          | Improvement  |
|-----------------|-----------|----------------|--------------|
| Field iteration | 145 MiB/s | 792-1331 MiB/s | **5-9x**     |
| Memory overhead | ~6%       | ~3-4%          | **50% less** |

## Running Benchmarks

```bash
# Build with benchmark runner
cargo build --release --features bench-runner

# CLI benchmarks
./target/release/succinctly bench run dsv_bench

# Single column
./target/release/succinctly bench run dsv_bench --query '.[0]'

# Library benchmarks
RUSTFLAGS="-C target-cpu=native" cargo build --release --examples
./target/release/examples/dsv_end_to_end_bench
./target/release/examples/dsv_single_field_bench
```

## See Also

- [DSV Parsing](../parsing/dsv.md) - Implementation details
- [Hierarchical Structures](../optimizations/hierarchical-structures.md) - Why lightweight index is faster
