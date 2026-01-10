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
./scripts/compare-hashes.sh
```

## Results: 100MB Files

| Pattern           | jq       | succinctly | Speedup      | jq Mem  | succ Mem | Mem Ratio |
|-------------------|----------|------------|--------------|---------|----------|-----------|
| **nested**        | 3.44s    |  **0.50s** | **6.9x**     |  205 MB |   321 MB | 1.57x     |
| **strings**       | 3.30s    |  **0.82s** | **4.0x**     |  160 MB |   252 MB | 1.57x     |
| **unicode**       | 3.40s    |  **1.84s** | **1.8x**     |  424 MB |   543 MB | 1.28x     |
| **mixed**         | 0.92s    |  **0.54s** | **1.7x**     |  248 MB |   215 MB | 0.87x     |
| **numbers**       | 3.70s    |  **2.26s** | **1.6x**     |  984 MB |   597 MB | 0.61x     |
| **users**         | 4.09s    |  **2.53s** | **1.6x**     |  682 MB |   808 MB | 1.18x     |
| **comprehensive** | 6.83s    |  **4.40s** | **1.6x**     | 1313 MB |  1463 MB | 1.11x     |
| **pathological**  | 14.21s   | **10.15s** | **1.4x**     | 5256 MB |  4545 MB | 0.86x     |
| **arrays**        | 10.84s   |  **8.75s** | **1.2x**     | 3612 MB |  3539 MB | 0.98x     |
| **literals**      | 5.17s    |  **4.74s** | **1.1x**     | 1122 MB |  1480 MB | 1.32x     |

## Results: 10MB Files

| Pattern           | jq       | succinctly | Speedup      | jq Mem  | succ Mem | Mem Ratio |
|-------------------|----------|------------|--------------|---------|----------|-----------|
| **nested**        | 0.33s    |  **0.05s** | **6.6x**     |   25 MB |    35 MB | 1.40x     |
| **strings**       | 0.33s    |  **0.08s** | **4.1x**     |   16 MB |    29 MB | 1.81x     |
| **comprehensive** | 0.84s    |  **0.40s** | **2.1x**     |  135 MB |   149 MB | 1.10x     |
| **unicode**       | 0.35s    |  **0.17s** | **2.1x**     |   42 MB |    58 MB | 1.38x     |
| **users**         | 0.43s    |  **0.22s** | **2.0x**     |   70 MB |    85 MB | 1.21x     |
| **numbers**       | 0.38s    |  **0.21s** | **1.8x**     |   96 MB |    63 MB | 0.66x     |
| **mixed**         | 0.09s    |  **0.05s** | **1.8x**     |   27 MB |    25 MB | 0.93x     |
| **pathological**  | 1.40s    |  **0.92s** | **1.5x**     |  526 MB |   458 MB | 0.87x     |
| **arrays**        | 1.07s    |  **0.80s** | **1.3x**     |  369 MB |   357 MB | 0.97x     |
| **literals**      | 0.53s    |  **0.42s** | **1.3x**     |  105 MB |   151 MB | 1.44x     |

## Results: 1MB Files

| Pattern           | jq       | succinctly | Speedup      | jq Mem  | succ Mem | Mem Ratio |
|-------------------|----------|------------|--------------|---------|----------|-----------|
| **nested**        | 0.03s    |  **0.00s** | **>6x**      |  4.6 MB |  5.8 MB  | 1.26x     |
| **strings**       | 0.03s    |  **0.01s** | **3.0x**     |  3.6 MB |  5.1 MB  | 1.42x     |
| **comprehensive** | 0.07s    |  **0.03s** | **2.3x**     |   17 MB |   18 MB  | 1.09x     |
| **unicode**       | 0.04s    |  **0.01s** | **4.0x**     |  6.1 MB |  9.4 MB  | 1.54x     |
| **users**         | 0.04s    |  **0.02s** | **2.0x**     |  9.4 MB |   11 MB  | 1.14x     |
| **numbers**       | 0.04s    |  **0.02s** | **2.0x**     |   11 MB |  9.8 MB  | 0.85x     |
| **pathological**  | 0.14s    |  **0.08s** | **1.8x**     |   55 MB |   49 MB  | 0.89x     |
| **literals**      | 0.05s    |  **0.04s** | **1.3x**     |   10 MB |   19 MB  | 1.81x     |
| **arrays**        | 0.11s    |  **0.10s** | **1.1x**     |   38 MB |   39 MB  | 1.04x     |

## Pattern Descriptions

| Pattern           | Description                                    |
|-------------------|------------------------------------------------|
| **arrays**        | Arrays of arrays (tests iteration performance) |
| **comprehensive** | Mixed content with all JSON features           |
| **literals**      | Mix of null, true, false literals              |
| **mixed**         | Heterogeneous nested structures                |
| **nested**        | Deeply nested objects (tests tree navigation)  |
| **numbers**       | Number-heavy documents with various formats    |
| **pathological**  | Worst-case patterns (deep nesting, escapes).   |
| **strings**       | String-heavy with escape sequences             |
| **unicode**       | UTF-8 multibyte sequences                      |
| **users**         | Realistic user record objects                  |

## Key Findings

### Speed

- **1.1-6.9x faster** across all patterns and sizes
- **Best performance on nested data**: 6.9x speedup on deeply nested structures
- **String-heavy data**: 4.0x speedup due to efficient escape handling
- **Consistent wins**: succinctly is faster on every pattern tested

### Memory

- **Comparable memory usage** overall
- **Lower memory on some patterns**: numbers (0.61x), pathological (0.86x), mixed (0.87x)
- **Higher memory on others**: strings (1.57x), nested (1.57x)
- **Trade-off**: succinctly builds succinct indexes which require additional memory, but this overhead is often offset by more efficient processing

### Why succinctly is faster

1. **Succinct indexing**: JSON structure is pre-indexed using balanced parentheses, enabling O(1) navigation
2. **SIMD acceleration**: Uses NEON on ARM for character classification
3. **Table-driven parser**: PFSM (Parallel Finite State Machine) with lookup tables
4. **Lazy evaluation**: Only materializes values that are actually accessed

## Reproducing Benchmarks

```bash
# Build release binary
cargo build --release --features cli

# Generate benchmark data
./target/release/succinctly json generate-suite

# Run comparison
./scripts/compare-hashes.sh > docs/jq-comparison-results.jsonl
```
