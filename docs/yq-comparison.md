# yq vs succinctly YAML Comparison Benchmarks

Benchmarks comparing `succinctly yq .` (identity filter) vs `yq .` (Mike Farah's yq v4.48.1) for YAML formatting/printing.

## Platform

**CPU**: Apple M1 Max
**OS**: macOS
**yq version**: v4.48.1 (https://github.com/mikefarah/yq/)
**succinctly**: Built with `--release --features cli`

## Methodology

Benchmarks measure wall time for the identity filter (`.`) which parses and re-emits the YAML document. This exercises the full parsing pipeline.

Run with:
```bash
cargo bench --bench yq_comparison
```

---

## Summary Results

### yq Identity Comparison (comprehensive pattern)

| Size      | succinctly   | yq           | Speedup       |
|-----------|--------------|--------------|---------------|
| **10KB**  |   4.0 ms     |   8.0 ms     | **2.0x**      |
| **100KB** |   6.7 ms     |  19.7 ms     | **2.9x**      |
| **1MB**   |  32.0 ms     | 118.9 ms     | **3.7x**      |

### Throughput Comparison

| Size      | succinctly      | yq             | Ratio         |
|-----------|-----------------|----------------|---------------|
| **10KB**  |   2.5 MiB/s     |   1.2 MiB/s    | **2.0x**      |
| **100KB** |  13.7 MiB/s     |   4.7 MiB/s    | **2.9x**      |
| **1MB**   |  28.8 MiB/s     |   7.8 MiB/s    | **3.7x**      |

---

## Detailed Results by Pattern

### Pattern: comprehensive

Mixed YAML content with various features.

| Size      | succinctly          | yq                  | Speedup    |
|-----------|---------------------|---------------------|------------|
| **1KB**   |  3.75 ms (328 KiB/s)|  6.68 ms (184 KiB/s)| **1.8x**   |
| **10KB**  |  4.09 ms (2.4 MiB/s)|  7.97 ms (1.2 MiB/s)| **1.9x**   |
| **100KB** |  6.77 ms (13.6 MiB/s)| 19.9 ms (4.6 MiB/s)| **2.9x**   |
| **1MB**   | 31.4 ms (29.4 MiB/s)|118.7 ms (7.8 MiB/s) | **3.8x**   |

### Pattern: users

Realistic user record arrays (common in config files).

| Size      | succinctly          | yq                  | Speedup    |
|-----------|---------------------|---------------------|------------|
| **1KB**   |  3.74 ms (287 KiB/s)|  6.58 ms (163 KiB/s)| **1.8x**   |
| **10KB**  |  4.01 ms (2.5 MiB/s)|  7.97 ms (1.2 MiB/s)| **2.0x**   |
| **100KB** |  7.14 ms (13.7 MiB/s)| 21.7 ms (4.5 MiB/s)| **3.0x**   |
| **1MB**   | 38.1 ms (26.2 MiB/s)|144.5 ms (6.9 MiB/s) | **3.8x**   |

### Pattern: nested

Deeply nested mapping structures.

| Size      | succinctly          | yq                  | Speedup    |
|-----------|---------------------|---------------------|------------|
| **1KB**   |  3.77 ms (266 KiB/s)|  6.68 ms (150 KiB/s)| **1.8x**   |
| **10KB**  |  3.94 ms (1.8 MiB/s)|  7.67 ms (935 KiB/s)| **1.9x**   |
| **100KB** |  5.26 ms (9.5 MiB/s)| 14.98 ms (3.3 MiB/s)| **2.8x**   |
| **1MB**   | 21.8 ms (28.9 MiB/s)| 93.3 ms (6.7 MiB/s) | **4.3x**   |

### Pattern: sequences

Sequence-heavy YAML content.

| Size      | succinctly          | yq                  | Speedup    |
|-----------|---------------------|---------------------|------------|
| **1KB**   |  3.66 ms (277 KiB/s)|  6.75 ms (150 KiB/s)| **1.8x**   |
| **10KB**  |  3.84 ms (2.5 MiB/s)|  7.98 ms (1.2 MiB/s)| **2.1x**   |
| **100KB** |  5.91 ms (16.5 MiB/s)| 21.3 ms (4.6 MiB/s)| **3.6x**   |
| **1MB**   | 24.6 ms (40.7 MiB/s)|138.6 ms (7.2 MiB/s) | **5.6x**   |

### Pattern: strings

String-heavy YAML with quoted variants.

| Size      | succinctly          | yq                  | Speedup    |
|-----------|---------------------|---------------------|------------|
| **1KB**   |  3.69 ms (274 KiB/s)|  6.59 ms (153 KiB/s)| **1.8x**   |
| **10KB**  |  3.89 ms (2.5 MiB/s)|  7.28 ms (1.3 MiB/s)| **1.9x**   |
| **100KB** |  6.14 ms (15.9 MiB/s)| 14.6 ms (6.7 MiB/s)| **2.4x**   |
| **1MB**   | 25.3 ms (39.5 MiB/s)| 79.6 ms (12.6 MiB/s)| **3.1x**   |

---

## Pattern Descriptions

| Pattern           | Description                                    |
|-------------------|------------------------------------------------|
| **comprehensive** | Mixed content with various YAML features       |
| **users**         | Realistic user record objects (config-like)    |
| **nested**        | Deeply nested mappings (tests tree navigation) |
| **sequences**     | Sequence-heavy content (arrays)                |
| **strings**       | String-heavy with quoted variants              |

---

## Key Findings

### Speed

- **1.8-5.6x faster** across all patterns and sizes
- **Best performance on sequences**: 5.6x speedup on 1MB sequence-heavy files
- **Nested structures**: 4.3x speedup due to efficient BP tree navigation
- **Larger files benefit more**: Speedup increases with file size (amortizes index construction)

### Why succinctly is faster

1. **Semi-index architecture**: YAML structure is pre-indexed using balanced parentheses, enabling O(1) navigation
2. **NEON SIMD**: Uses ARM NEON for character classification during parsing
3. **Streaming output**: For identity queries, outputs directly from source without building intermediate structures
4. **Lazy evaluation**: Only materializes values that are actually accessed

### Trade-offs

- **Small files (<1KB)**: Process startup dominates; speedup is modest (~1.8x)
- **Large files (1MB+)**: Index construction amortizes; best speedups (~4-6x)
- **Memory**: succinctly uses more memory for very small files due to index overhead, but less for large files

---

## Reproducing Benchmarks

```bash
# Build release binary
cargo build --release --features cli

# Run criterion benchmarks
cargo bench --bench yq_comparison

# Or use the CLI for manual testing
./target/release/succinctly yq . input.yaml
time yq . input.yaml
```
