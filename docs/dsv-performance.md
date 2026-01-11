# DSV Input Performance Benchmarks

Benchmarks for `succinctly jq --input-dsv` parsing DSV (CSV/TSV) files and converting them to JSON.

## Methodology

Benchmarks measure:
- **Wall time**: Total elapsed time
- **Peak memory**: Maximum resident set size (RSS)
- **Throughput**: Input bytes processed per second

Run with:
```bash
./target/release/succinctly dev bench dsv
```

---

# Apple M1 Max (ARM, aarch64)

**CPU**: Apple M1 Max
**OS**: macOS
**Delimiter**: `,` (comma-separated values)
**succinctly**: Built with `--release --features cli`

## Optimized (SIMD + Streaming)

Uses SIMD-accelerated DSV indexing (NEON on ARM) with streaming output.
Memory-efficient: only one row materialized at a time.

### Pattern: tabular

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    9.01s |   11.1 MiB/s |  142 MB  |
| **10mb**  |  907.9ms |   11.0 MiB/s |   21 MB  |
| **1mb**   |  104.7ms |    9.6 MiB/s |    9 MB  |
| **100kb** |   16.2ms |    6.0 MiB/s |    7 MB  |
| **10kb**  |    6.9ms |    1.4 MiB/s |    7 MB  |
| **1kb**   |    6.1ms |    0.2 MiB/s |    7 MB  |

### Pattern: users

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    9.29s |   10.8 MiB/s |  142 MB  |
| **10mb**  |  953.7ms |   10.5 MiB/s |   21 MB  |
| **1mb**   |   96.0ms |   10.4 MiB/s |    9 MB  |
| **100kb** |   14.4ms |    6.8 MiB/s |    7 MB  |
| **10kb**  |    6.2ms |    1.6 MiB/s |    7 MB  |
| **1kb**   |    5.5ms |    0.2 MiB/s |    7 MB  |

### Pattern: numeric

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    9.30s |   10.8 MiB/s |  142 MB  |
| **10mb**  |  956.5ms |   10.5 MiB/s |   21 MB  |
| **1mb**   |   96.5ms |   10.4 MiB/s |    9 MB  |
| **100kb** |   16.3ms |    6.0 MiB/s |    7 MB  |
| **10kb**  |    6.7ms |    1.5 MiB/s |    7 MB  |
| **1kb**   |    5.6ms |    0.2 MiB/s |    7 MB  |

### Pattern: strings

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    3.49s |   28.6 MiB/s |  142 MB  |
| **10mb**  |  351.5ms |   28.4 MiB/s |   21 MB  |
| **1mb**   |   41.3ms |   24.2 MiB/s |    9 MB  |
| **100kb** |    9.2ms |   10.6 MiB/s |    7 MB  |
| **10kb**  |    6.2ms |    1.6 MiB/s |    7 MB  |
| **1kb**   |    5.7ms |    0.2 MiB/s |    7 MB  |

### Pattern: quoted

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    6.06s |   16.5 MiB/s |  142 MB  |
| **10mb**  |  626.8ms |   16.0 MiB/s |   21 MB  |
| **1mb**   |   70.2ms |   14.3 MiB/s |    9 MB  |
| **100kb** |   12.3ms |    7.9 MiB/s |    7 MB  |
| **10kb**  |    6.1ms |    1.6 MiB/s |    7 MB  |
| **1kb**   |    5.7ms |    0.2 MiB/s |    7 MB  |

### Pattern: multiline

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    4.85s |   20.6 MiB/s |  142 MB  |
| **10mb**  |  503.2ms |   19.9 MiB/s |   21 MB  |
| **1mb**   |   59.7ms |   16.8 MiB/s |    9 MB  |
| **100kb** |   11.6ms |    8.4 MiB/s |    7 MB  |
| **10kb**  |    6.4ms |    1.5 MiB/s |    7 MB  |
| **1kb**   |    5.7ms |    0.2 MiB/s |    7 MB  |

### Pattern: wide

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |   13.88s |    7.2 MiB/s |  143 MB  |
| **10mb**  |    1.40s |    7.2 MiB/s |   21 MB  |
| **1mb**   |  147.8ms |    6.8 MiB/s |    9 MB  |
| **100kb** |   20.1ms |    4.9 MiB/s |    7 MB  |
| **10kb**  |    7.8ms |    1.3 MiB/s |    7 MB  |
| **1kb**   |    5.7ms |    0.2 MiB/s |    7 MB  |

### Pattern: long

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |   14.84s |    6.7 MiB/s |  142 MB  |
| **10mb**  |    1.57s |    6.4 MiB/s |   21 MB  |
| **1mb**   |  171.6ms |    5.8 MiB/s |    8 MB  |
| **100kb** |   23.2ms |    4.2 MiB/s |    7 MB  |
| **10kb**  |    7.1ms |    1.4 MiB/s |    7 MB  |
| **1kb**   |    5.5ms |    0.2 MiB/s |    7 MB  |

### Pattern: mixed

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    9.10s |   11.0 MiB/s |  142 MB  |
| **10mb**  |  949.3ms |   10.5 MiB/s |   21 MB  |
| **1mb**   |  103.3ms |    9.7 MiB/s |    9 MB  |
| **100kb** |   14.8ms |    6.6 MiB/s |    7 MB  |
| **10kb**  |    6.4ms |    1.5 MiB/s |    7 MB  |
| **1kb**   |    6.1ms |    0.2 MiB/s |    7 MB  |

### Pattern: pathological

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    6.93s |   14.4 MiB/s |  142 MB  |
| **10mb**  |  715.7ms |   14.0 MiB/s |   21 MB  |
| **1mb**   |   78.5ms |   12.7 MiB/s |    9 MB  |
| **100kb** |   13.6ms |    7.2 MiB/s |    7 MB  |
| **10kb**  |    6.8ms |    1.4 MiB/s |    7 MB  |
| **1kb**   |    5.6ms |    0.2 MiB/s |    7 MB  |

---

# AMD Ryzen 9 7950X (x86_64)

**CPU**: AMD Ryzen 9 7950X 16-Core Processor
**OS**: Linux (WSL2)
**Delimiter**: `,` (comma-separated values)
**succinctly**: Built with `RUSTFLAGS="-C target-cpu=native" cargo build --release --features cli`

## Optimized (SIMD AVX2 + Streaming)

Uses AVX2-accelerated DSV indexing with streaming output and native CPU optimizations.
Memory-efficient: only one row materialized at a time.

### Pattern: tabular

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    5.34s |   18.7 MiB/s |  139 MB  |
| **10mb**  |  548.0ms |   18.2 MiB/s |   18 MB  |
| **1mb**   |   61.6ms |   16.2 MiB/s |    5 MB  |

### Pattern: users

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    5.65s |   17.7 MiB/s |  139 MB  |
| **10mb**  |  586.5ms |   17.0 MiB/s |   18 MB  |
| **1mb**   |   62.2ms |   16.1 MiB/s |    5 MB  |

### Pattern: numeric

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    5.54s |   18.0 MiB/s |  139 MB  |
| **10mb**  |  584.8ms |   17.1 MiB/s |   18 MB  |
| **1mb**   |   61.7ms |   16.2 MiB/s |    5 MB  |

### Pattern: strings

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    2.50s |   39.9 MiB/s |  138 MB  |
| **10mb**  |  253.7ms |   39.4 MiB/s |   18 MB  |
| **1mb**   |   29.4ms |   34.0 MiB/s |    5 MB  |

### Pattern: quoted

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    3.58s |   27.9 MiB/s |  138 MB  |
| **10mb**  |  369.2ms |   27.1 MiB/s |   18 MB  |
| **1mb**   |   43.2ms |   23.1 MiB/s |    6 MB  |

### Pattern: multiline

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    3.20s |   31.3 MiB/s |  138 MB  |
| **10mb**  |  336.3ms |   29.7 MiB/s |   18 MB  |
| **1mb**   |   39.6ms |   25.2 MiB/s |    5 MB  |

### Pattern: wide

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |   10.82s |    9.2 MiB/s |  139 MB  |
| **10mb**  |    1.09s |    9.1 MiB/s |   17 MB  |
| **1mb**   |  108.5ms |    9.2 MiB/s |    5 MB  |

### Pattern: long

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    9.00s |   11.1 MiB/s |  139 MB  |
| **10mb**  |  917.8ms |   10.9 MiB/s |   18 MB  |
| **1mb**   |  100.6ms |    9.9 MiB/s |    5 MB  |

### Pattern: mixed

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    5.56s |   18.0 MiB/s |  138 MB  |
| **10mb**  |  548.7ms |   18.2 MiB/s |   18 MB  |
| **1mb**   |   59.3ms |   16.9 MiB/s |    5 MB  |

### Pattern: pathological

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    3.86s |   25.9 MiB/s |  138 MB  |
| **10mb**  |  387.5ms |   25.8 MiB/s |   18 MB  |
| **1mb**   |   43.0ms |   23.2 MiB/s |    6 MB  |

---

## Baseline (Scalar, Materialized)

Uses scalar DSV parsing with full JSON materialization.
All rows converted to JSON arrays before output.

### Pattern: tabular

Standard CSV with uniform column widths.

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    9.77s |   10.2 MiB/s |    2 GB  |
| **10mb**  |    1.02s |    9.8 MiB/s |  178 MB  |
| **1mb**   |  113.0ms |    8.8 MiB/s |   24 MB  |
| **100kb** |   17.3ms |    5.6 MiB/s |    9 MB  |
| **10kb**  |    6.5ms |    1.5 MiB/s |    7 MB  |
| **1kb**   |    5.7ms |    0.2 MiB/s |    7 MB  |

### Pattern: users

Realistic user records with names, emails, addresses.

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |   10.31s |    9.7 MiB/s |    2 GB  |
| **10mb**  |    1.04s |    9.6 MiB/s |  231 MB  |
| **1mb**   |  113.9ms |    8.8 MiB/s |   30 MB  |
| **100kb** |   16.7ms |    5.9 MiB/s |    9 MB  |
| **10kb**  |    6.9ms |    1.4 MiB/s |    7 MB  |
| **1kb**   |    5.6ms |    0.2 MiB/s |    7 MB  |

### Pattern: numeric

Numeric-heavy CSV (IDs, counts, measurements).

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |   10.22s |    9.8 MiB/s |    2 GB  |
| **10mb**  |    1.03s |    9.7 MiB/s |  164 MB  |
| **1mb**   |  110.2ms |    9.1 MiB/s |   23 MB  |
| **100kb** |   16.5ms |    5.9 MiB/s |    9 MB  |
| **10kb**  |    6.1ms |    1.6 MiB/s |    7 MB  |
| **1kb**   |    5.7ms |    0.2 MiB/s |    7 MB  |

### Pattern: strings

String-heavy CSV with few quoted fields.

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    3.94s |   25.4 MiB/s |  610 MB  |
| **10mb**  |  396.9ms |   25.2 MiB/s |   68 MB  |
| **1mb**   |   46.2ms |   21.6 MiB/s |   13 MB  |
| **100kb** |    9.6ms |   10.2 MiB/s |    8 MB  |
| **10kb**  |    6.0ms |    1.7 MiB/s |    7 MB  |
| **1kb**   |    5.5ms |    0.2 MiB/s |    7 MB  |

### Pattern: quoted

CSV with many quoted fields containing special characters.

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    6.70s |   14.9 MiB/s |  861 MB  |
| **10mb**  |  693.9ms |   14.4 MiB/s |   95 MB  |
| **1mb**   |   76.6ms |   13.1 MiB/s |   17 MB  |
| **100kb** |   13.8ms |    7.1 MiB/s |    8 MB  |
| **10kb**  |    7.0ms |    1.4 MiB/s |    7 MB  |
| **1kb**   |    6.1ms |    0.2 MiB/s |    7 MB  |

### Pattern: multiline

CSV with multiline fields (newlines inside quotes).

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    5.40s |   18.5 MiB/s |  743 MB  |
| **10mb**  |  558.1ms |   17.9 MiB/s |   84 MB  |
| **1mb**   |   65.4ms |   15.3 MiB/s |   15 MB  |
| **100kb** |   12.2ms |    8.0 MiB/s |    8 MB  |
| **10kb**  |    7.0ms |    1.4 MiB/s |    7 MB  |
| **1kb**   |    6.7ms |    0.2 MiB/s |    7 MB  |

### Pattern: wide

Wide tables (many columns per row).

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |   15.34s |    6.5 MiB/s |    3 GB  |
| **10mb**  |    1.52s |    6.6 MiB/s |  350 MB  |
| **1mb**   |  157.4ms |    6.4 MiB/s |   42 MB  |
| **100kb** |   20.3ms |    4.8 MiB/s |   11 MB  |
| **10kb**  |    6.7ms |    1.5 MiB/s |    7 MB  |
| **1kb**   |    6.0ms |    0.2 MiB/s |    7 MB  |

### Pattern: long

Tall tables (many rows, few columns).

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |   16.28s |    6.1 MiB/s |    4 GB  |
| **10mb**  |    1.71s |    5.9 MiB/s |  394 MB  |
| **1mb**   |  190.8ms |    5.2 MiB/s |   49 MB  |
| **100kb** |   25.3ms |    3.9 MiB/s |   11 MB  |
| **10kb**  |    8.1ms |    1.2 MiB/s |    8 MB  |
| **1kb**   |    6.0ms |    0.2 MiB/s |    7 MB  |

### Pattern: mixed

Mixed content with various field types and quoting.

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |   10.05s |    9.9 MiB/s |    2 GB  |
| **10mb**  |    1.03s |    9.7 MiB/s |  187 MB  |
| **1mb**   |  105.8ms |    9.4 MiB/s |   24 MB  |
| **100kb** |   16.7ms |    5.8 MiB/s |    9 MB  |
| **10kb**  |    6.9ms |    1.4 MiB/s |    7 MB  |
| **1kb**   |    5.8ms |    0.2 MiB/s |    7 MB  |

### Pattern: pathological

Worst-case CSV patterns (heavy quoting, escapes, edge cases).

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    7.45s |   13.4 MiB/s |  911 MB  |
| **10mb**  |  759.8ms |   13.2 MiB/s |  109 MB  |
| **1mb**   |   80.6ms |   12.4 MiB/s |   18 MB  |
| **100kb** |   13.9ms |    7.0 MiB/s |    8 MB  |
| **10kb**  |    6.4ms |    1.5 MiB/s |    7 MB  |
| **1kb**   |    5.6ms |    0.2 MiB/s |    7 MB  |

---

# Optimization Comparison

## Memory Reduction (100MB files, Apple M1 Max)

| Pattern        | Baseline | Optimized | Reduction |
|----------------|----------|-----------|-----------|
| **long**       |    4 GB  |   142 MB  | **28x**   |
| **wide**       |    3 GB  |   143 MB  | **21x**   |
| **tabular**    |    2 GB  |   142 MB  | **14x**   |
| **users**      |    2 GB  |   142 MB  | **14x**   |
| **numeric**    |    2 GB  |   142 MB  | **14x**   |
| **mixed**      |    2 GB  |   142 MB  | **14x**   |
| **pathological**|  911 MB |   142 MB  | **6.4x**  |
| **quoted**     |  861 MB  |   142 MB  | **6.1x**  |
| **multiline**  |  743 MB  |   142 MB  | **5.2x**  |
| **strings**    |  610 MB  |   142 MB  | **4.3x**  |

## Throughput Improvement (100MB files, Apple M1 Max)

| Pattern        | Baseline     | Optimized    | Speedup |
|----------------|--------------|--------------|---------|
| **strings**    |   25.4 MiB/s |   28.6 MiB/s | **+13%** |
| **multiline**  |   18.5 MiB/s |   20.6 MiB/s | **+11%** |
| **quoted**     |   14.9 MiB/s |   16.5 MiB/s | **+11%** |
| **long**       |    6.1 MiB/s |    6.7 MiB/s | **+10%** |
| **wide**       |    6.5 MiB/s |    7.2 MiB/s | **+11%** |
| **tabular**    |   10.2 MiB/s |   11.1 MiB/s | **+9%**  |
| **mixed**      |    9.9 MiB/s |   11.0 MiB/s | **+11%** |
| **numeric**    |    9.8 MiB/s |   10.8 MiB/s | **+10%** |
| **users**      |    9.7 MiB/s |   10.8 MiB/s | **+11%** |
| **pathological**|  13.4 MiB/s |   14.4 MiB/s | **+7%**  |

---

# Summary

## Throughput by Pattern (100MB files, Optimized)

### Apple M1 Max (ARM)

| Pattern        | Throughput   | Notes                                     |
|----------------|--------------|-------------------------------------------|
| **strings**    |   28.6 MiB/s | Fastest - few quotes, simple fields       |
| **multiline**  |   20.6 MiB/s | Fast despite newlines in fields           |
| **quoted**     |   16.5 MiB/s | Quote handling overhead                   |
| **pathological**|  14.4 MiB/s | Complex edge cases                        |
| **tabular**    |   11.1 MiB/s | Uniform column widths                     |
| **mixed**      |   11.0 MiB/s | Varied content types                      |
| **numeric**    |   10.8 MiB/s | Number-heavy content                      |
| **users**      |   10.8 MiB/s | Realistic user records                    |
| **wide**       |    7.2 MiB/s | Many columns increases overhead           |
| **long**       |    6.7 MiB/s | Many rows increases per-row overhead      |

### AMD Ryzen 9 7950X (x86_64)

| Pattern          | Throughput   | Notes                                     |
|------------------|--------------|-------------------------------------------|
| **strings**      |   39.9 MiB/s | Fastest - few quotes, simple fields       |
| **multiline**    |   31.3 MiB/s | Fast despite newlines in fields           |
| **quoted**       |   27.9 MiB/s | Quote handling overhead                   |
| **pathological** |   25.9 MiB/s | Complex edge cases                        |
| **tabular**      |   18.7 MiB/s | Uniform column widths                     |
| **mixed**        |   18.0 MiB/s | Varied content types                      |
| **numeric**      |   18.0 MiB/s | Number-heavy content                      |
| **users**        |   17.7 MiB/s | Realistic user records                    |
| **long**         |   11.1 MiB/s | Many rows increases per-row overhead      |
| **wide**         |    9.2 MiB/s | Many columns increases overhead           |

## Pattern Descriptions

| Pattern         | Description                                           |
|-----------------|-------------------------------------------------------|
| **tabular**     | Standard CSV with uniform column widths               |
| **users**       | Realistic user records (names, emails, addresses)     |
| **numeric**     | Numeric-heavy (IDs, counts, measurements)             |
| **strings**     | String-heavy with minimal quoting                     |
| **quoted**      | Many quoted fields with special characters            |
| **multiline**   | Fields containing newlines (inside quotes)            |
| **wide**        | Wide tables (100+ columns per row)                    |
| **long**        | Tall tables (many rows, 3-5 columns)                  |
| **mixed**       | Mixed content with various field types                |
| **pathological**| Worst-case patterns (heavy escaping, edge cases)      |

## Key Observations

1. **Streaming reduces memory 5-28x** - Memory now ~1.4x file size instead of 6-40x
2. **SIMD + streaming improves throughput 7-13%** - Faster parsing and reduced allocation overhead
3. **Memory is now constant per-pattern** - ~142 MB for all 100MB files regardless of row/column count
4. **String-heavy data is fastest** (28.6 MiB/s) - Simple fields without quoting have minimal overhead
5. **Quote handling adds overhead** - Quoted patterns run at ~60% the speed of unquoted
6. **Table shape matters less with streaming** - Wide/long tables no longer cause memory blowup

---

# Proposed Optimizations

## BMI2 PDEP for Quote Masking (x86_64)

**Status**: Implemented, awaiting x86_64 benchmark validation
**File**: [src/dsv/simd/bmi2.rs](../src/dsv/simd/bmi2.rs)
**Date**: 2026-01-12

### Overview

The BMI2 implementation uses the PDEP (Parallel Bit Deposit) instruction for quote-aware parsing, ported from the hw-dsv Haskell library's `indexCsvChunk` algorithm. This replaces the `prefix_xor` fallback with a more efficient approach on supported CPUs.

### Algorithm

The `index_chunk_bmi2` function implements the exact hw-dsv algorithm:

```rust
// From hw-dsv's indexCsvChunk:
// let enters = pdep (oddsMask .<. (0x1 .&.      pc)) qq
// let leaves = pdep (oddsMask .<. (0x1 .&. comp pc)) qq
// let compLeaves    = comp leaves
// let preQuoteMask  = enters + compLeaves
// let quoteMask     = preQuoteMask + carry
// let newCarry      = quoteMask `ltWord` (enters .|. compLeaves .|. carry)
```

Key components:
- **PDEP instruction**: Scatters alternating bit pattern (`0x5555...`) to quote positions
- **Quote parity (`pc`)**: Tracks odd/even quote count to determine inside/outside state
- **Carry propagation**: Uses addition with complement for efficient toggle computation
- **`ltWord` equivalent**: Unsigned comparison for carry detection

### Expected Performance

Based on hw-dsv benchmarks on 7GB CSV files:

| Configuration       | Throughput | Speedup vs Baseline |
|---------------------|------------|---------------------|
| Broadword (no BMI2) |  16.3 MiB/s | 1x                 |
| BMI2 enabled        | 165.0 MiB/s | **10.1x**          |
| BMI2 + AVX2         | 181.0 MiB/s | **11.1x**          |

**Expected improvement for succinctly (x86_64 with BMI2)**:
- Current AVX2 + prefix_xor: ~18-40 MiB/s
- With BMI2 PDEP: ~100-180 MiB/s (estimated 5-10x improvement)

### Why PDEP Is Faster

1. **Single-cycle operation**: PDEP executes in 1-3 cycles on Intel Haswell+ and AMD Zen 3+
2. **Eliminates prefix XOR**: The 6-operation `prefix_xor` loop is replaced by one PDEP instruction
3. **Reduces register pressure**: Fewer temporaries needed
4. **Enables elegant quote tracking**: The enters/leaves approach cleanly handles quote state

### Hardware Requirements

| CPU                  | BMI2 Support | PDEP Performance |
|----------------------|--------------|------------------|
| Intel Haswell+ (2013)| Yes          | Fast (3 cycles)  |
| AMD Zen 1/2 (2017-19)| Yes          | Slow (18 cycles) |
| AMD Zen 3+ (2020+)   | Yes          | Fast (3 cycles)  |
| Apple Silicon        | No           | N/A              |

### Runtime Dispatch

The implementation uses runtime CPU detection:

```rust
// src/dsv/simd/mod.rs
pub fn build_index_simd(text: &[u8], config: &DsvConfig) -> DsvIndex {
    if is_x86_feature_detected!("bmi2") && is_x86_feature_detected!("avx2") {
        return bmi2::build_index_simd(text, config);  // Fastest path
    }
    if is_x86_feature_detected!("avx2") {
        return avx2::build_index_simd(text, config);  // prefix_xor fallback
    }
    sse2::build_index_simd(text, config)              // SSE2 fallback
}
```

### ARM Limitation

ARM processors (including Apple Silicon M1-M4) have no equivalent to PDEP/PEXT:
- SVE2 has BDEP/BEXT but operates on vectors only
- Apple Silicon doesn't implement SVE/SVE2
- Must continue using `prefix_xor` approach on ARM

### Validation

Tests verify BMI2 matches scalar and AVX2 results:
- `test_bmi2_matches_scalar`: Compares against reference implementation
- `test_bmi2_matches_avx2`: Ensures consistency with AVX2 path
- `test_quoted_spanning_chunks`: Tests 64-byte boundary crossing

### Next Steps

1. **Benchmark on x86_64**: Run `./target/release/succinctly dev bench dsv` on AMD Zen 3+ or Intel Haswell+
2. **Verify speedup**: Expect 5-10x improvement over current AVX2 results
3. **Update documentation**: Add BMI2 results to this file after benchmarking

### References

- [hw-dsv source](https://github.com/haskell-works/hw-dsv) - Original Haskell implementation
- [docs/dsv-port-plan.md](dsv-port-plan.md) - Full BMI2 optimization details
- Intel PDEP/PEXT documentation

---

## Reproducing Benchmarks

```bash
# Build release binary
cargo build --release --features cli

# Generate DSV benchmark data
./target/release/succinctly dsv generate-suite

# Run DSV input benchmarks
./target/release/succinctly dev bench dsv
```
