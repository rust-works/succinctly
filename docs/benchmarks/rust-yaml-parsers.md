# Rust YAML Parser Comparison

Benchmark comparison of succinctly against serde_yaml, the standard Rust YAML parser.

**Platform**: x86_64 (AMD Ryzen 9 7950X, Zen 4)
**Date**: 2026-01-29

> **Note**: For end-to-end CLI performance vs system yq, see [yq.md](yq.md) for additional Neoverse-V2 (Graviton 4), Neoverse-V1 (Graviton 3), and Apple M1 Max benchmarks.

## Libraries Compared

| Library        | Version | Type                   | Key Features                                          |
|----------------|---------|------------------------|-------------------------------------------------------|
| **succinctly** | 0.1.0   | Semi-index (streaming) | Balanced parentheses + interest bits, minimal memory  |
| **serde_yaml** | 0.9.x   | DOM (standard)         | Based on yaml-rust2, most popular Rust YAML library   |

## Parse-Only Performance

Time to build the index/parse the document (no traversal).

### Summary Table (1MB file)

| Library        | Time     | Throughput | vs serde_yaml | vs succinctly    |
|----------------|----------|------------|---------------|------------------|
| **succinctly** | 2.19 ms  | 420 MiB/s  | **10.0x faster** | baseline      |
| serde_yaml     | 21.96 ms | 42.0 MiB/s | baseline      | 10.0x slower     |

### Detailed Results by File Size

#### 1KB Files

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 4.20 µs  | 286 MiB/s  | **8.0x faster**  |
| serde_yaml     | 33.38 µs | 36.0 MiB/s | baseline         |

#### 10KB Files

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 27.39 µs | 358 MiB/s  | **9.6x faster**  |
| serde_yaml     | 263.28 µs| 37.2 MiB/s | baseline         |

#### 100KB Files

| Library        | Time     | Throughput | vs serde_yaml     |
|----------------|----------|------------|-------------------|
| **succinctly** | 232.6 µs | 396 MiB/s  | **9.9x faster**   |
| serde_yaml     | 2.295 ms | 40.1 MiB/s | baseline          |

#### 1MB Files

| Library        | Time     | Throughput | vs serde_yaml     |
|----------------|----------|------------|-------------------|
| **succinctly** | 2.194 ms | 420 MiB/s  | **10.0x faster**  |
| serde_yaml     | 21.96 ms | 42.0 MiB/s | baseline          |

#### 10MB Files

| Library        | Time     | Throughput | vs serde_yaml     |
|----------------|----------|------------|-------------------|
| **succinctly** | 21.45 ms | 435 MiB/s  | **13.5x faster**  |
| serde_yaml     | 289.5 ms | 32.3 MiB/s | baseline          |

## Parse + Traverse Performance

Full document parsing and complete traversal.

### Summary Table (1MB file)

| Library        | Time     | Throughput | vs serde_yaml  | vs succinctly    |
|----------------|----------|------------|----------------|------------------|
| **succinctly** | 5.38 ms  | 171 MiB/s  | **4.2x faster**| baseline         |
| serde_yaml     | 22.50 ms | 41.0 MiB/s | baseline       | 4.2x slower      |

### Detailed Results by File Size

#### 1KB Files

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 8.58 µs  | 140 MiB/s  | **4.1x faster**  |
| serde_yaml     | 34.81 µs | 34.5 MiB/s | baseline         |

#### 10KB Files

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 65.26 µs | 150 MiB/s  | **4.2x faster**  |
| serde_yaml     | 272.1 µs | 36.0 MiB/s | baseline         |

#### 100KB Files

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 575.0 µs | 160 MiB/s  | **4.1x faster**  |
| serde_yaml     | 2.333 ms | 39.4 MiB/s | baseline         |

#### 1MB Files

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 5.385 ms | 171 MiB/s  | **4.2x faster**  |
| serde_yaml     | 22.50 ms | 41.0 MiB/s | baseline         |

#### 10MB Files

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 52.21 ms | 179 MiB/s  | **5.6x faster**  |
| serde_yaml     | 291.3 ms | 32.1 MiB/s | baseline         |

## YAML to JSON Conversion

Convert YAML to JSON output.

### Summary Table (1MB file)

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 7.87 ms  | 117 MiB/s  | **3.0x faster**  |
| serde_yaml     | 23.25 ms | 39.7 MiB/s | baseline         |

### Detailed Results by File Size

#### 1KB Files

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 11.68 µs | 103 MiB/s  | **3.1x faster**  |
| serde_yaml     | 36.09 µs | 33.3 MiB/s | baseline         |

#### 10KB Files

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 96.93 µs | 101 MiB/s  | **2.9x faster**  |
| serde_yaml     | 280.5 µs | 34.9 MiB/s | baseline         |

#### 100KB Files

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 851.9 µs | 108 MiB/s  | **2.8x faster**  |
| serde_yaml     | 2.388 ms | 38.5 MiB/s | baseline         |

#### 1MB Files

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 7.871 ms | 117 MiB/s  | **3.0x faster**  |
| serde_yaml     | 23.25 ms | 39.7 MiB/s | baseline         |

#### 10MB Files

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 78.59 ms | 119 MiB/s  | **3.9x faster**  |
| serde_yaml     | 308.1 ms | 30.3 MiB/s | baseline         |

---

# ARM Neoverse-V2 (AWS Graviton 4)

**Platform**: ARM Neoverse-V2 (AWS Graviton 4)
**Date**: 2026-01-30
**SIMD**: NEON (128-bit), SVE2 (128-bit vectors), SVEBITPERM (BDEP/BEXT)

## Parse-Only Performance (ARM)

### Summary Table (1MB file)

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 3.30 ms  | 279 MiB/s  | **12.2x faster** |
| serde_yaml     | 40.2 ms  | 23.0 MiB/s | baseline         |

### Detailed Results by File Size (ARM)

| Size   | serde_yaml | succinctly  | Speedup          |
|--------|-----------|-------------|------------------|
| 1KB    | 57.3 µs   | 6.36 µs     | **9.0x faster**  |
| 10KB   | 478.4 µs  | 38.8 µs     | **12.3x faster** |
| 100KB  | 4.14 ms   | 340.5 µs    | **12.2x faster** |
| 1MB    | 40.2 ms   | 3.30 ms     | **12.2x faster** |
| 10MB   | 430 ms    | 32.6 ms     | **13.2x faster** |

## Parse + Traverse Performance (ARM)

### Summary Table (1MB file)

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 7.46 ms  | 124 MiB/s  | **5.4x faster**  |
| serde_yaml     | 40.6 ms  | 22.7 MiB/s | baseline         |

### Detailed Results by File Size (ARM)

| Size   | serde_yaml | succinctly  | Speedup          |
|--------|-----------|-------------|------------------|
| 1KB    | 63.9 µs   | 12.0 µs     | **5.3x faster**  |
| 10KB   | 483.7 µs  | 84.2 µs     | **5.7x faster**  |
| 100KB  | 4.16 ms   | 780.8 µs    | **5.3x faster**  |
| 1MB    | 40.6 ms   | 7.46 ms     | **5.4x faster**  |
| 10MB   | 431.4 ms  | 72.0 ms     | **6.0x faster**  |

## YAML to JSON Conversion (ARM)

### Summary Table (1MB file)

| Library        | Time     | Throughput | vs serde_yaml    |
|----------------|----------|------------|------------------|
| **succinctly** | 10.98 ms | 84.0 MiB/s | **3.8x faster**  |
| serde_yaml     | 41.3 ms  | 22.3 MiB/s | baseline         |

### Detailed Results by File Size (ARM)

| Size   | serde_yaml | succinctly  | Speedup          |
|--------|-----------|-------------|------------------|
| 1KB    | 65.5 µs   | 16.4 µs     | **4.0x faster**  |
| 10KB   | 495.7 µs  | 119.7 µs    | **4.1x faster**  |
| 100KB  | 4.25 ms   | 1.15 ms     | **3.7x faster**  |
| 1MB    | 41.3 ms   | 10.98 ms    | **3.8x faster**  |
| 10MB   | 447.7 ms  | 105.0 ms    | **4.3x faster**  |

---

## Peak Memory Usage

Memory overhead during parsing/indexing (consistent across platforms).

### Summary Table

| Size   | serde_yaml | **succinctly** | YAML Size | Ratio     |
|--------|-----------|----------------|-----------|-----------|
| 1KB    | 106.6 KB  | **2.77 KB**    | 1.23 KB   | **38.5x** |
| 10KB   | 369.0 KB  | **20.4 KB**    | 10.0 KB   | **18.1x** |
| 100KB  | 2.63 MB   | **188.8 KB**   | 94.2 KB   | **14.3x** |
| 1MB    | 22.21 MB  | **1.84 MB**    | 944 KB    | **12.0x** |
| 10MB   | 194.1 MB  | **18.7 MB**    | 9.34 MB   | **10.4x** |

**Key Findings:**
- succinctly uses **10-39x less memory** than serde_yaml
- Memory efficiency improves at smaller file sizes (39x at 1KB)
- succinctly maintains ~2x overhead vs input size across all sizes
- serde_yaml ranges from 21-87x overhead vs input size

### Memory Efficiency (vs YAML Size)

| Size   | serde_yaml | succinctly  |
|--------|-----------|-------------|
| 1KB    | 87x       | **2.3x**    |
| 10KB   | 37x       | **2.0x**    |
| 100KB  | 29x       | **2.0x**    |
| 1MB    | 24x       | **2.0x**    |
| 10MB   | 21x       | **2.0x**    |

## Key Findings

### Parse Performance

- **succinctly is 8-14x faster** at parsing across all file sizes
- Best advantage on larger files: **13.5x faster** on 10MB files
- Consistent throughput: 286-435 MiB/s (succinctly) vs 32-42 MiB/s (serde_yaml)
- Parse-only throughput improves with file size (435 MiB/s at 10MB)

### Parse + Traverse Performance

- **succinctly is 4-6x faster** for full document iteration
- Best advantage on larger files: **5.6x faster** on 10MB
- Throughput remains consistent: 140-179 MiB/s vs 32-41 MiB/s

### YAML to JSON Conversion

- **succinctly is 2.8-3.9x faster** for format conversion
- Best performance on 10MB files: **3.9x faster** (119 vs 30 MiB/s)
- Streaming output without intermediate OwnedValue allocation

### Memory Efficiency

- **10-39x less memory** than serde_yaml
- Consistent ~2x overhead vs input size (vs serde_yaml's 21-87x)
- Greater efficiency advantage at smaller file sizes
- Predictable memory usage enables handling larger-than-RAM documents

### Use Case Recommendations

| Use Case                          | Recommendation | Why                                           |
|-----------------------------------|----------------|-----------------------------------------------|
| **Parse YAML for queries**        | succinctly     | 8-14x faster parsing, 10-39x less memory      |
| **Full document iteration**       | succinctly     | 4-6x faster, streaming-friendly               |
| **YAML → JSON conversion**        | succinctly     | 2.8-3.9x faster, direct streaming             |
| **Schema validation**             | serde_yaml     | Full YAML spec compliance, strict validation  |
| **Modify & re-serialize YAML**    | serde_yaml     | Mutable DOM, preserves formatting             |
| **Large YAML files (>100MB)**     | succinctly     | Constant 2x memory overhead, won't OOM        |
| **Memory-constrained environments** | succinctly   | 10-39x less memory usage                      |

## Reproducing These Benchmarks

```bash
# From the bench-compare directory
cd bench-compare

# Generate test data
cargo run --release --features cli -- yaml generate-suite

# Run benchmarks
cargo bench --bench yaml_parsers
```

## See Also

- [yq.md](yq.md) - End-to-end CLI benchmarks (succinctly yq vs system yq)
- [rust-parsers.md](rust-parsers.md) - JSON parser comparison (vs serde_json, sonic-rs, simd-json)
- [../parsing/yaml.md](../parsing/yaml.md) - YAML optimization phases and implementation details
- [../optimizations/](../optimizations/) - Optimization techniques used
- [../architecture/semi-indexing.md](../architecture/semi-indexing.md) - Semi-indexing architecture explanation
