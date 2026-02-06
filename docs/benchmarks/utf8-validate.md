# UTF-8 Validation Benchmarks

Benchmarks for `succinctly text validate utf8` - scalar UTF-8 validation throughput.

This serves as the baseline for future SIMD implementations.

## Summary

| Platform                      | ASCII (MiB/s) | CJK (MiB/s) | Emoji (MiB/s) | Mixed (MiB/s) |
|-------------------------------|---------------|-------------|---------------|---------------|
| AMD Ryzen 9 7950X (x86_64)    | 2,261         | 1,359       | 1,036         | 1,819         |

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

### Throughput by Character Type

1. **ASCII-dominant content** (~2.2-2.3 GiB/s): Pure ASCII, log files, source code, pathological
2. **Mixed content** (~1.8-1.9 GiB/s): JSON-like, mixed prose
3. **Multi-byte content** (~1.0-1.4 GiB/s): CJK (3-byte), emoji (4-byte), Greek/Cyrillic (2-byte)
4. **Uniform multi-byte** (~0.5 GiB/s): All-lengths pattern with maximum byte diversity

### Performance Characteristics

- **ASCII is fastest**: Single-byte validation requires no continuation byte checks
- **3-byte (CJK) faster than 4-byte (emoji)**: Fewer continuation bytes to validate per character
- **Latin (2-byte) slower than CJK (3-byte)**: Higher character density means more state machine transitions
- **All-lengths is slowest**: Maximum branch prediction misses from alternating sequence lengths

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
