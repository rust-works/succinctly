# jq vs succinctly Comparison Benchmarks

Comprehensive benchmarks comparing `succinctly jq .` vs `jq .` for JSON formatting/printing.

## Test Environment

- **Platform**: Apple M1 Max
- **OS**: macOS
- **jq version**: System jq
- **succinctly**: Built with `--release --features cli`

## Methodology

Benchmarks measure:
- **Wall time**: Total elapsed time
- **Peak memory**: Maximum resident set size (RSS)
- **Output correctness**: MD5 hash comparison ensures identical output

Run with:
```bash
./target/release/succinctly dev bench jq
```

## Pattern: arrays

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |   10.72s | **   7.98s** | **     1.3x** |    4 GB |   137 MB |      0.04x |
| **10mb**  |    1.07s | ** 700.8ms** | **     1.5x** |  368 MB |    20 MB |      0.05x |
| **1mb**   |  113.2ms | **  69.0ms** | **     1.6x** |   39 MB |     8 MB |      0.21x |
| **100kb** |   16.2ms | **  11.3ms** | **     1.4x** |    6 MB |     7 MB |      1.12x |
| **10kb**  |    7.5ms | **   6.2ms** | **     1.2x** |    3 MB |     7 MB |      2.36x |
| **1kb**   |    6.7ms | **   5.7ms** | **     1.2x** |    2 MB |     7 MB |      2.69x |

## Pattern: comprehensive

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    6.90s | **   3.80s** | **     1.8x** |    1 GB |   107 MB |      0.08x |
| **10mb**  |  692.4ms | ** 364.7ms** | **     1.9x** |  135 MB |    17 MB |      0.13x |
| **1mb**   |   68.9ms | **  38.2ms** | **     1.8x** |   16 MB |     8 MB |      0.48x |
| **100kb** |   14.0ms | **   9.0ms** | **     1.6x** |    4 MB |     7 MB |      1.69x |
| **10kb**  |    7.2ms | **   6.7ms** | **     1.1x** |    3 MB |     7 MB |      2.52x |
| **1kb**   |    6.7ms | **   6.0ms** | **     1.1x** |    2 MB |     7 MB |      2.67x |

## Pattern: literals

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    5.14s | **   4.70s** | **     1.1x** |    1 GB |   133 MB |      0.12x |
| **10mb**  |  510.9ms | ** 397.1ms** | **     1.3x** |  103 MB |    19 MB |      0.19x |
| **1mb**   |   54.3ms | **  51.9ms** | **     1.0x** |   10 MB |     8 MB |      0.78x |
| **100kb** |   11.7ms | **   9.6ms** | **     1.2x** |    3 MB |     7 MB |      2.02x |
| **10kb**  |    7.4ms | **   6.5ms** | **     1.1x** |    3 MB |     7 MB |      2.56x |
| **1kb**   |    7.3ms | **   5.9ms** | **     1.2x** |    2 MB |     7 MB |      2.69x |

## Pattern: mixed

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |  910.5ms | ** 477.7ms** | **     1.9x** |  248 MB |    23 MB |      0.09x |
| **10mb**  |   96.3ms | **  50.8ms** | **     1.9x** |   28 MB |     8 MB |      0.29x |
| **1mb**   |   14.7ms | **  10.1ms** | **     1.5x** |    5 MB |     7 MB |      1.40x |
| **100kb** |    7.5ms | **   6.8ms** | **     1.1x** |    3 MB |     7 MB |      2.42x |
| **10kb**  |    6.6ms | **   5.7ms** | **     1.2x** |    2 MB |     7 MB |      2.68x |
| **1kb**   |    6.6ms | **   5.5ms** | **     1.2x** |    2 MB |     7 MB |      2.66x |

## Pattern: nested

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    3.39s | ** 532.3ms** | **     6.4x** |  205 MB |   226 MB |      1.10x |
| **10mb**  |  347.2ms | **  54.8ms** | **     6.3x** |   25 MB |    29 MB |      1.17x |
| **1mb**   |   40.7ms | **  11.3ms** | **     3.6x** |    5 MB |     9 MB |      1.94x |
| **100kb** |    9.1ms | **   6.5ms** | **     1.4x** |    3 MB |     7 MB |      2.48x |
| **10kb**  |    6.6ms | **   6.1ms** | **     1.1x** |    3 MB |     7 MB |      2.65x |
| **1kb**   |    6.4ms | **   5.8ms** | **     1.1x** |    2 MB |     7 MB |      2.66x |

## Pattern: numbers

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    3.66s | **   2.29s** | **     1.6x** |  983 MB |   119 MB |      0.12x |
| **10mb**  |  370.7ms | ** 213.8ms** | **     1.7x** |   97 MB |    18 MB |      0.19x |
| **1mb**   |   41.5ms | **  25.0ms** | **     1.7x** |   13 MB |     8 MB |      0.63x |
| **100kb** |    9.4ms | **   7.2ms** | **     1.3x** |    4 MB |     7 MB |      1.91x |
| **10kb**  |    6.8ms | **   5.5ms** | **     1.3x** |    3 MB |     7 MB |      2.58x |
| **1kb**   |    6.6ms | **   5.5ms** | **     1.2x** |    2 MB |     7 MB |      2.69x |

## Pattern: pathological

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |   13.92s | **   7.87s** | **     1.8x** |    5 GB |   138 MB |      0.03x |
| **10mb**  |    1.35s | ** 677.0ms** | **     2.0x** |  526 MB |    20 MB |      0.04x |
| **1mb**   |  140.9ms | **  65.6ms** | **     2.1x** |   55 MB |     8 MB |      0.15x |
| **100kb** |   19.4ms | **  11.2ms** | **     1.7x** |    8 MB |     7 MB |      0.89x |
| **10kb**  |    7.5ms | **   6.2ms** | **     1.2x** |    3 MB |     7 MB |      2.23x |
| **1kb**   |    6.7ms | **   6.1ms** | **     1.1x** |    3 MB |     7 MB |      2.65x |

## Pattern: strings

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    3.18s | ** 690.5ms** | **     4.6x** |  161 MB |   111 MB |      0.69x |
| **10mb**  |  331.0ms | **  75.2ms** | **     4.4x** |   16 MB |    17 MB |      1.07x |
| **1mb**   |   37.1ms | **  11.7ms** | **     3.2x** |    4 MB |     8 MB |      2.16x |
| **100kb** |    9.3ms | **   7.3ms** | **     1.3x** |    3 MB |     7 MB |      2.66x |
| **10kb**  |    6.5ms | **   6.3ms** | **     1.0x** |    2 MB |     7 MB |      2.69x |
| **1kb**   |    6.7ms | **   6.0ms** | **     1.1x** |    2 MB |     7 MB |      2.67x |

## Pattern: unicode

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    3.30s | **   1.67s** | **     2.0x** |  424 MB |   127 MB |      0.30x |
| **10mb**  |  334.8ms | ** 157.1ms** | **     2.1x** |   41 MB |    19 MB |      0.46x |
| **1mb**   |   38.8ms | **  20.7ms** | **     1.9x** |    6 MB |     8 MB |      1.25x |
| **100kb** |    9.8ms | **   7.6ms** | **     1.3x** |    3 MB |     7 MB |      2.27x |
| **10kb**  |    7.3ms | **   6.2ms** | **     1.2x** |    3 MB |     7 MB |      2.65x |
| **1kb**   |   13.0ms | **  13.0ms** | **     1.0x** |    3 MB |     7 MB |      2.61x |

## Pattern: users

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    4.13s | **   2.00s** | **     2.1x** |  681 MB |    91 MB |      0.13x |
| **10mb**  |  413.7ms | ** 196.8ms** | **     2.1x** |   70 MB |    15 MB |      0.21x |
| **1mb**   |   45.2ms | **  24.1ms** | **     1.9x** |    9 MB |     8 MB |      0.80x |
| **100kb** |   10.4ms | **   7.7ms** | **     1.4x** |    3 MB |     7 MB |      2.14x |
| **10kb**  |    7.0ms | **   5.9ms** | **     1.2x** |    3 MB |     7 MB |      2.64x |
| **1kb**   |    6.5ms | **   5.8ms** | **     1.1x** |    2 MB |     7 MB |      2.66x |

## Pattern Descriptions

| Pattern           | Description                                    |
|-------------------|------------------------------------------------|
| **arrays**        | Arrays of arrays (tests iteration performance) |
| **comprehensive** | Mixed content with all JSON features           |
| **literals**      | Mix of null, true, false literals              |
| **mixed**         | Heterogeneous nested structures                |
| **nested**        | Deeply nested objects (tests tree navigation)  |
| **numbers**       | Number-heavy documents with various formats    |
| **pathological**  | Worst-case patterns (deep nesting, escapes)    |
| **strings**       | String-heavy with escape sequences             |
| **unicode**       | UTF-8 multibyte sequences                      |
| **users**         | Realistic user record objects                  |

## Key Findings

### Speed

- **1.0-6.4x faster** across all patterns and sizes
- **Best performance on nested data**: 6.4x speedup on deeply nested structures
- **String-heavy data**: 4.6x speedup due to efficient escape handling
- **Consistent wins**: succinctly is faster on every pattern tested

### Memory

- **Dramatically lower memory usage** on most patterns
- **3-37x less memory** on larger files: pathological (0.03x), arrays (0.04x), comprehensive (0.08x)
- **Slightly higher on small files**: 1-2x overhead due to minimum index size
- **Streaming output**: Uses lazy cursor evaluation - only materializes values when needed

### Why succinctly is faster

1. **Succinct indexing**: JSON structure is pre-indexed using balanced parentheses, enabling O(1) navigation
2. **SIMD acceleration**: Uses NEON on ARM for character classification
3. **Table-driven parser**: PFSM (Parallel Finite State Machine) with lookup tables
4. **Lazy evaluation**: Only materializes values that are actually accessed
5. **Streaming output**: For identity queries, outputs directly from source without building intermediate structures

## Reproducing Benchmarks

```bash
# Build release binary
cargo build --release --features cli

# Generate benchmark data
./target/release/succinctly json generate-suite

# Run comparison
./target/release/succinctly dev bench jq
```
