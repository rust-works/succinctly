# UTF-8 Validation Benchmarks

Benchmarks for `succinctly text validate utf8` - scalar UTF-8 validation throughput.

This serves as the baseline for future SIMD implementations.

## Summary

| Platform                      | ASCII (GiB/s) | CJK (GiB/s) | Emoji (GiB/s) | Mixed (GiB/s) |
|-------------------------------|---------------|-------------|---------------|---------------|
| Apple M4 Pro (ARM)            | 2.0           | 2.5         | 2.9           | 2.1           |
| AMD Ryzen 9 7950X (x86_64)    | 2.2           | 1.3         | 1.0           | 1.8           |

**Key Finding**: Apple M4 Pro is **2-3x faster** than AMD Ryzen 9 7950X on multi-byte sequences (CJK, emoji) while maintaining comparable ASCII throughput.

## Apple M4 Pro (ARM)

**Date**: 2026-02-06
**Commit**: b58037f (utf8-validation-scalar branch)
**CPU**: Apple M4 Pro (12 cores)
**OS**: macOS 15.6.1
**Rust**: 1.93.0

### Performance by Pattern Type (1MB)

| Pattern       | Time (µs) | Throughput (GiB/s) |
|---------------|-----------|-------------------|
| ASCII         | 487.6     | 2.00              |
| Mixed         | 461.0     | 2.12              |
| CJK (3-byte)  | 396.5     | 2.46              |
| Emoji (4-byte)| 340.6     | 2.87              |
| 2-byte Latin  | 431.4     | 2.26              |

### Detailed Results by Pattern

#### ASCII (Pure 7-bit)

| Size  | Time       | Throughput (GiB/s) |
|-------|------------|-------------------|
| 1KB   | 449.5 ns   | 2.12              |
| 10KB  | 4.47 µs    | 2.13              |
| 100KB | 45.3 µs    | 2.10              |
| 1MB   | 487.6 µs   | 2.00              |
| 10MB  | 4.73 ms    | 2.07              |

#### Mixed (Realistic content)

| Size  | Time       | Throughput (GiB/s) |
|-------|------------|-------------------|
| 1KB   | 463.3 ns   | 2.06              |
| 10KB  | 4.47 µs    | 2.14              |
| 100KB | 45.1 µs    | 2.12              |
| 1MB   | 461.0 µs   | 2.12              |
| 10MB  | 4.64 ms    | 2.11              |

#### CJK (3-byte sequences)

| Size  | Time       | Throughput (GiB/s) |
|-------|------------|-------------------|
| 1KB   | 382.1 ns   | 2.50              |
| 10KB  | 3.82 µs    | 2.49              |
| 100KB | 38.7 µs    | 2.46              |
| 1MB   | 396.5 µs   | 2.46              |
| 10MB  | 3.94 ms    | 2.48              |

#### Emoji (4-byte sequences)

| Size  | Time       | Throughput (GiB/s) |
|-------|------------|-------------------|
| 1KB   | 355.3 ns   | 2.68              |
| 10KB  | 3.31 µs    | 2.88              |
| 100KB | 33.4 µs    | 2.85              |
| 1MB   | 340.6 µs   | 2.87              |
| 10MB  | 3.44 ms    | 2.84              |

#### Latin Extended (2-byte sequences)

| Size  | Time       | Throughput (GiB/s) |
|-------|------------|-------------------|
| 1KB   | 458.0 ns   | 2.08              |
| 10KB  | 3.99 µs    | 2.39              |
| 100KB | 40.6 µs    | 2.35              |
| 1MB   | 431.4 µs   | 2.26              |
| 10MB  | 4.31 ms    | 2.27              |

### Sequence Type Comparison (1MB)

Direct comparison of byte sequence lengths at 1MB:

| Sequence Type      | Time (µs) | Throughput (GiB/s) |
|--------------------|-----------|-------------------|
| ASCII (1-byte)     | 505.0     | 1.93              |
| Extended (2-byte)  | 403.9     | 2.42              |
| CJK (3-byte)       | 394.4     | 2.48              |
| Emoji (4-byte)     | 350.7     | 2.79              |
| Mixed              | 469.8     | 2.08              |

**Observation**: Longer UTF-8 sequences validate *faster* on M4 Pro due to fewer continuation byte validations per MB of data.

## AMD Ryzen 9 7950X (x86_64)

**Date**: 2026-02-06
**Commit**: 71ce3fa (utf8-validation-scalar branch)
**CPU**: AMD Ryzen 9 7950X 16-Core Processor
**OS**: Ubuntu 22.04.5 LTS
**Rust**: 1.92.0

### Performance by Pattern Type

| Pattern         | 1KB (MiB/s) | 10KB (MiB/s) | 100KB (MiB/s) | 1MB (MiB/s) | 10MB (MiB/s) | 100MB (MiB/s) |
|-----------------|-------------|--------------|---------------|-------------|--------------|---------------|
| ascii           | 2,271       | 2,299        | 2,308         | 2,308       | 2,245        | 2,262         |
| log_file        | 1,434       | 1,461        | 2,226         | 2,176       | 2,147        | 2,137         |
| pathological    | 1,358       | 1,416        | 2,165         | 2,149       | 2,147        | 2,153         |
| source_code     | 1,953       | 2,095        | 2,055         | 1,987       | 1,984        | 1,979         |
| json_like       | 1,915       | 1,307        | 1,278         | 1,911       | 1,873        | 1,879         |
| mixed           | 967         | 1,278        | 1,936         | 1,856       | 1,823        | 1,819         |
| cjk             | 1,684       | 1,852        | 1,386         | 1,358       | 1,368        | 1,359         |
| greek_cyrillic  | 904         | 1,680        | 1,192         | 1,147       | 1,134        | 1,119         |
| emoji           | 651         | 1,225        | 1,946         | 1,040       | 1,061        | 1,036         |
| latin           | 1,528       | 1,531        | 894           | 848         | 874          | 860           |
| all_lengths     | 1,221       | 1,094        | 536           | 500         | 502          | 492           |

### Detailed Results by Pattern

#### ASCII (Pure 7-bit)

| Size  | Time (ms) | Throughput (MiB/s) |
|-------|-----------|-------------------|
| 1KB   | 0.00      | 2,271             |
| 10KB  | 0.00      | 2,299             |
| 100KB | 0.04      | 2,308             |
| 1MB   | 0.43      | 2,308             |
| 10MB  | 4.45      | 2,245             |
| 100MB | 44.22     | 2,262             |

#### CJK (3-byte sequences)

| Size  | Time (ms) | Throughput (MiB/s) |
|-------|-----------|-------------------|
| 1KB   | 0.00      | 1,684             |
| 10KB  | 0.01      | 1,852             |
| 100KB | 0.07      | 1,386             |
| 1MB   | 0.74      | 1,358             |
| 10MB  | 7.31      | 1,368             |
| 100MB | 73.58     | 1,359             |

#### Emoji (4-byte sequences)

| Size  | Time (ms) | Throughput (MiB/s) |
|-------|-----------|-------------------|
| 1KB   | 0.00      | 651               |
| 10KB  | 0.01      | 1,225             |
| 100KB | 0.05      | 1,946             |
| 1MB   | 0.96      | 1,040             |
| 10MB  | 9.43      | 1,061             |
| 100MB | 96.56     | 1,036             |

#### Mixed (Realistic content)

| Size  | Time (ms) | Throughput (MiB/s) |
|-------|-----------|-------------------|
| 1KB   | 0.00      | 967               |
| 10KB  | 0.01      | 1,278             |
| 100KB | 0.05      | 1,936             |
| 1MB   | 0.54      | 1,856             |
| 10MB  | 5.49      | 1,823             |
| 100MB | 54.98     | 1,819             |

#### Latin Extended (2-byte sequences)

| Size  | Time (ms) | Throughput (MiB/s) |
|-------|-----------|-------------------|
| 1KB   | 0.00      | 1,528             |
| 10KB  | 0.01      | 1,531             |
| 100KB | 0.11      | 894               |
| 1MB   | 1.18      | 848               |
| 10MB  | 11.45     | 874               |
| 100MB | 116.34    | 860               |

#### All Lengths (Uniform 1-4 byte mix)

| Size  | Time (ms) | Throughput (MiB/s) |
|-------|-----------|-------------------|
| 1KB   | 0.00      | 1,221             |
| 10KB  | 0.01      | 1,094             |
| 100KB | 0.18      | 536               |
| 1MB   | 2.00      | 500               |
| 10MB  | 19.94     | 502               |
| 100MB | 203.44    | 492               |

## Key Findings

### Cross-Platform Comparison (1MB)

| Pattern       | M4 Pro (GiB/s) | Ryzen 9 (GiB/s) | M4 Pro Advantage |
|---------------|----------------|-----------------|------------------|
| ASCII         | 2.0            | 2.2             | 0.9x (Ryzen wins)|
| Mixed         | 2.1            | 1.8             | **1.2x**         |
| CJK (3-byte)  | 2.5            | 1.3             | **1.9x**         |
| Emoji (4-byte)| 2.9            | 1.0             | **2.9x**         |
| 2-byte Latin  | 2.3            | 0.8             | **2.9x**         |

### Throughput by Character Type (AMD Ryzen 9 7950X)

1. **ASCII-dominant content** (~2.2-2.3 GiB/s): Pure ASCII, log files, source code, pathological
2. **Mixed content** (~1.8-1.9 GiB/s): JSON-like, mixed prose
3. **Multi-byte content** (~1.0-1.4 GiB/s): CJK (3-byte), emoji (4-byte), Greek/Cyrillic (2-byte)
4. **Uniform multi-byte** (~0.5 GiB/s): All-lengths pattern with maximum byte diversity

### Throughput by Character Type (Apple M4 Pro)

1. **Emoji (4-byte)** (~2.8-2.9 GiB/s): Fastest due to fewest characters per MB
2. **CJK (3-byte)** (~2.4-2.5 GiB/s): Excellent performance on ideographic content
3. **2-byte Latin** (~2.3-2.4 GiB/s): Extended Latin, Greek, Cyrillic
4. **ASCII/Mixed** (~2.0-2.1 GiB/s): Comparable performance across content types

### Performance Characteristics

- **M4 Pro inverts the pattern**: Multi-byte sequences are *faster* than ASCII (fewer characters to validate per MB)
- **Ryzen 9 follows expected pattern**: ASCII is fastest, multi-byte is slower
- **Branch prediction**: M4 Pro's branch predictor handles UTF-8 state machine better
- **Memory bandwidth**: Both platforms are memory-bound at large sizes

### Scaling

- Throughput is consistent from 1MB to 100MB (memory-bound)
- Small files (1KB-10KB) show higher variance due to measurement overhead
- No cache effects visible - sequential access pattern is optimal for hardware prefetching

## Running the Benchmarks

```bash
# Generate test files
./target/release/succinctly text generate-suite

# Run CLI benchmark
./target/release/succinctly dev bench utf8

# Run Criterion benchmark
cargo bench --bench utf8_validate_bench

# Via unified runner
./target/release/succinctly bench run utf8_bench
```

## Test Data Patterns

| Pattern         | Description                                      |
|-----------------|--------------------------------------------------|
| ascii           | Pure 7-bit ASCII (single-byte sequences)         |
| latin           | Latin Extended characters (2-byte sequences)     |
| greek_cyrillic  | Greek and Cyrillic (2-byte sequences)            |
| cjk             | Chinese/Japanese/Korean (3-byte sequences)       |
| emoji           | Emoji and symbols (4-byte sequences)             |
| mixed           | Realistic prose with occasional non-ASCII        |
| all_lengths     | Uniform mix of all sequence lengths (1-4 bytes)  |
| log_file        | Log file style (mostly ASCII with timestamps)    |
| source_code     | Source code style (ASCII with unicode in strings)|
| json_like       | JSON-like structure with unicode strings         |
| pathological    | Maximum multi-byte density                       |
