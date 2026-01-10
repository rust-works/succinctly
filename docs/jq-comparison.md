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

## Results: 100MB Files

| Pattern       | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|---------------|----------|--------------|---------------|---------|----------|------------|
| **nested**    |    3.40s | ** 502.5ms** | **     6.8x** |  205 MB |   226 MB |      1.10x |
| **strings**   |    3.21s | ** 735.0ms** | **     4.4x** |  159 MB |   111 MB |      0.70x |
| **users**     |    4.14s | **   1.99s** | **     2.1x** |  681 MB |    91 MB |      0.13x |
| **unicode**   |    3.33s | **   1.67s** | **     2.0x** |  422 MB |   127 MB |      0.30x |
| **mixed**     |  920.7ms | ** 458.8ms** | **     2.0x** |  248 MB |    23 MB |      0.09x |
| **patholog.** |   13.91s | **   7.84s** | **     1.8x** |    5 GB |   138 MB |      0.03x |
| **compreh.**  |    6.75s | **   3.90s** | **     1.7x** |    1 GB |   107 MB |      0.08x |
| **numbers**   |    3.72s | **   2.27s** | **     1.6x** |  984 MB |   119 MB |      0.12x |
| **arrays**    |   10.68s | **   8.02s** | **     1.3x** |    4 GB |   137 MB |      0.04x |
| **literals**  |    5.03s | **   4.73s** | **     1.1x** |    1 GB |   133 MB |      0.12x |

## Results: 10MB Files

| Pattern       | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|---------------|----------|--------------|---------------|---------|----------|------------|
| **nested**    |  351.0ms | **  59.6ms** | **     5.9x** |   25 MB |    29 MB |      1.17x |
| **strings**   |  317.8ms | **  71.5ms** | **     4.4x** |   18 MB |    17 MB |      0.98x |
| **patholog.** |    1.42s | ** 646.2ms** | **     2.2x** |  526 MB |    20 MB |      0.04x |
| **unicode**   |  317.4ms | ** 145.6ms** | **     2.2x** |   41 MB |    19 MB |      0.46x |
| **users**     |  395.3ms | ** 181.7ms** | **     2.2x** |   70 MB |    15 MB |      0.21x |
| **compreh.**  |  689.1ms | ** 361.6ms** | **     1.9x** |  135 MB |    17 MB |      0.13x |
| **mixed**     |   92.7ms | **  51.6ms** | **     1.8x** |   27 MB |     8 MB |      0.31x |
| **numbers**   |  379.0ms | ** 217.2ms** | **     1.7x** |   96 MB |    18 MB |      0.19x |
| **arrays**    |    1.08s | ** 717.9ms** | **     1.5x** |  367 MB |    20 MB |      0.05x |
| **literals**  |  511.1ms | ** 418.8ms** | **     1.2x** |  103 MB |    19 MB |      0.19x |

## Results: 1MB Files

| Pattern       | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|---------------|----------|--------------|---------------|---------|----------|------------|
| **nested**    |   40.5ms | **  11.2ms** | **     3.6x** |    5 MB |     9 MB |      1.94x |
| **strings**   |   39.3ms | **  12.5ms** | **     3.2x** |    4 MB |     8 MB |      2.16x |
| **patholog.** |  146.5ms | **  64.8ms** | **     2.3x** |   55 MB |     8 MB |      0.15x |
| **users**     |   46.4ms | **  23.7ms** | **     2.0x** |    9 MB |     8 MB |      0.80x |
| **unicode**   |   40.3ms | **  20.7ms** | **     1.9x** |    7 MB |     8 MB |      1.17x |
| **compreh.**  |   70.3ms | **  37.0ms** | **     1.9x** |   17 MB |     8 MB |      0.47x |
| **numbers**   |   45.0ms | **  26.2ms** | **     1.7x** |   11 MB |     8 MB |      0.69x |
| **arrays**    |  112.9ms | **  70.6ms** | **     1.6x** |   38 MB |     8 MB |      0.21x |
| **mixed**     |   13.2ms | **   9.7ms** | **     1.4x** |    5 MB |     7 MB |      1.39x |
| **literals**  |   57.2ms | **  44.1ms** | **     1.3x** |   10 MB |     8 MB |      0.78x |

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

- **1.1-6.8x faster** across all patterns and sizes
- **Best performance on nested data**: 6.8x speedup on deeply nested structures
- **String-heavy data**: 4.4x speedup due to efficient escape handling
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
