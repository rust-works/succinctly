# Rust JSON Parser Comparison

Benchmark comparison of succinctly against other popular Rust JSON parsers.

**Platform**: x86_64 (AMD Ryzen 9 7950X, Zen 4)
**Date**: 2026-01-11

## Libraries Compared

| Library       | Version | Type                  | Key Features                                    |
|---------------|---------|------------------------|------------------------------------------------|
| **succinctly**| 0.1.0   | Semi-index (streaming) | Balanced parentheses + interest bits, minimal memory |
| **sonic-rs**  | 0.3.x   | DOM (arena-based)      | SIMD + arena allocation, fastest DOM parser    |
| **serde_json**| 1.0.x   | DOM (standard)         | Most popular, standard library integration     |
| **simd-json** | 0.13.x  | DOM (SIMD-accelerated) | Port of simdjson, requires mutable input      |

## Parse-Only Performance

Time to build the index/parse the document (no traversal).

### Summary Table (1MB file)

| Library        | Time     | Throughput | vs sonic-rs | vs succinctly |
|----------------|----------|------------|-------------|---------------|
| **sonic-rs**   | 0.998 ms | 810 MiB/s  | baseline    | **1.59x faster** |
| **succinctly** | 1.583 ms | 510 MiB/s  | 1.59x slower| baseline      |
| serde_json     | 4.832 ms | 167 MiB/s  | 4.84x slower| 3.05x slower  |
| simd-json      | 5.100 ms | 158 MiB/s  | 5.11x slower| 3.22x slower  |

### Detailed Results by File Size

#### 1KB Files

| Library        | Time      | Throughput | vs succinctly |
|----------------|-----------|------------|---------------|
| **sonic-rs**   |  1.78 µs  | 887 MiB/s  | **1.50x faster** |
| **succinctly** |  2.66 µs  | 593 MiB/s  | baseline      |
| simd-json      |  6.24 µs  | 253 MiB/s  | 2.35x slower  |
| serde_json     |  7.61 µs  | 207 MiB/s  | 2.86x slower  |

#### 10KB Files

| Library        | Time      | Throughput | vs succinctly |
|----------------|-----------|------------|---------------|
| **sonic-rs**   | 11.12 µs  | 845 MiB/s  | **1.52x faster** |
| **succinctly** | 16.91 µs  | 556 MiB/s  | baseline      |
| simd-json      | 40.76 µs  | 231 MiB/s  | 2.41x slower  |
| serde_json     | 52.55 µs  | 179 MiB/s  | 3.11x slower  |

#### 100KB Files

| Library        | Time      | Throughput | vs succinctly |
|----------------|-----------|------------|---------------|
| **sonic-rs**   | 105.6 µs  | 808 MiB/s  | **1.66x faster** |
| **succinctly** | 174.9 µs  | 488 MiB/s  | baseline      |
| simd-json      | 390.1 µs  | 219 MiB/s  | 2.23x slower  |
| serde_json     | 492.0 µs  | 173 MiB/s  | 2.81x slower  |

#### 1MB Files

| Library        | Time     | Throughput | vs succinctly |
|----------------|----------|------------|---------------|
| **sonic-rs**   | 0.998 ms | 810 MiB/s  | **1.59x faster** |
| **succinctly** | 1.583 ms | 510 MiB/s  | baseline      |
| serde_json     | 4.832 ms | 167 MiB/s  | 3.05x slower  |
| simd-json      | 5.100 ms | 158 MiB/s  | 3.22x slower  |

#### 10MB Files

| Library        | Time      | Throughput | vs succinctly |
|----------------|-----------|------------|---------------|
| **sonic-rs**   | 11.40 ms  | 702 MiB/s  | **1.34x faster** |
| **succinctly** | 15.32 ms  | 522 MiB/s  | baseline      |
| serde_json     | 52.21 ms  | 153 MiB/s  | 3.41x slower  |
| simd-json      | 59.59 ms  | 134 MiB/s  | 3.89x slower  |

#### 100MB Files

| Library        | Time      | Throughput | vs succinctly |
|----------------|-----------|------------|---------------|
| **sonic-rs**   | 115.7 ms  | 692 MiB/s  | **1.35x faster** |
| **succinctly** | 155.6 ms  | 514 MiB/s  | baseline      |
| serde_json     | 526.1 ms  | 152 MiB/s  | 3.38x slower  |
| simd-json      | 826.9 ms  |  97 MiB/s  | 5.31x slower  |

## Peak Memory Usage

Memory overhead during parsing/indexing.

### Summary Table

| Size   | serde_json | simd-json | sonic-rs  | **succinctly** | JSON Size |
|--------|-----------|-----------|-----------|----------------|-----------|
| 1KB    | 15.45 KB  | 28.78 KB  |  9.22 KB  | **0.75 KB**    | 1.62 KB   |
| 10KB   | 103.0 KB  | 196.6 KB  | 70.21 KB  | **4.46 KB**    | 9.62 KB   |
| 100KB  | 924.2 KB  |  1.82 MB  | 388.0 KB  | **40.42 KB**   | 87.39 KB  |
| 1MB    |  7.00 MB  | 17.12 MB  |  9.97 MB  | **382.3 KB**   | 827.7 KB  |
| 10MB   | 63.89 MB  | 167.4 MB  | 97.02 MB  | **3.69 MB**    | 8.01 MB   |
| 100MB  | 655.2 MB  | 1654 MB   | 954.5 MB  | **36.89 MB**   | 80.01 MB  |

### Memory Efficiency (vs JSON size)

| Size   | serde_json | simd-json | sonic-rs | **succinctly** |
|--------|-----------|-----------|----------|----------------|
| 1KB    |  9.57x    | 17.82x    | 5.71x    | **0.47x**      |
| 10KB   | 10.71x    | 20.44x    | 7.30x    | **0.46x**      |
| 100KB  | 10.57x    | 21.33x    | 4.44x    | **0.46x**      |
| 1MB    |  8.66x    | 21.17x    | 12.34x   | **0.46x**      |
| 10MB   |  7.98x    | 20.91x    | 12.12x   | **0.46x**      |
| 100MB  |  8.19x    | 20.67x    | 11.93x   | **0.46x**      |

### Memory Comparison (1MB file)

| Library        | Peak Memory | vs JSON Size | vs succinctly          |
|----------------|-------------|--------------|------------------------|
| **succinctly** | **382 KB**  | **0.46x**    | baseline               |
| sonic-rs       | 9.97 MB     | 12.34x       | **26.7x more memory**  |
| serde_json     | 7.00 MB     | 8.66x        | **18.7x more memory**  |
| simd-json      | 17.12 MB    | 21.17x       | **45.8x more memory**  |

## Key Findings

### Parse Performance

1. **sonic-rs is fastest**: 1.3-1.7x faster than succinctly
   - Optimized SIMD + arena allocation
   - DOM parser, builds entire tree in memory

2. **succinctly is competitive**: 510-522 MiB/s (middle of the pack)
   - **3.0-5.3x faster** than serde_json/simd-json
   - Only 1.3-1.7x slower than sonic-rs
   - Builds semi-index, not full DOM

3. **serde_json is slowest**: 150-170 MiB/s
   - Standard library, not optimized for raw speed
   - Still most popular due to ecosystem integration

4. **simd-json disappoints**: 97-158 MiB/s
   - Slower than serde_json on larger files
   - Requires mutable input (extra copy overhead)

### Memory Efficiency

**succinctly dominates**:
- **18-46x less memory** than other parsers
- **Consistent 46% overhead** across all file sizes (vs JSON)
- All other parsers: 4-21x the JSON size

**Why succinctly uses less memory**:
- Semi-index structure (balanced parentheses + interest bits)
- Only ~46% overhead for navigation
- Doesn't materialize full DOM tree
- String data stays in original JSON (zero-copy)

**Why others use more**:
- serde_json: Full DOM with owned strings (8-10x)
- simd-json: Similar to serde but with padding (20-21x)
- sonic-rs: Arena allocation reduces fragmentation but still full DOM (4-12x)

### Use Case Recommendations

| Use Case                           | Recommendation | Why                                            |
|---------------------------------------|----------------|------------------------------------------------|
| **Low memory / streaming**            | **succinctly** | 18-46x less memory, streaming-friendly         |
| **Maximum parse speed**               | sonic-rs       | Fastest parser, 1.3-1.7x faster than succinctly|
| **Serde ecosystem integration**       | serde_json     | Standard library, most compatible              |
| **Selective JSON extraction**         | **succinctly** | Navigate without parsing entire document       |
| **In-place modification**             | simd-json      | Mutable API for in-place editing               |
| **Large files (>10MB)**               | **succinctly** | Memory usage stays manageable (46% overhead)   |
| **Many small requests**               | sonic-rs       | Fast parsing, arena allocator reduces overhead |

## Traversal Performance

*Note: Not included in this benchmark. succinctly offers two traversal modes:*
- **Fast mode** (`cursor.children()`): BP-only traversal, no text extraction
- **Value mode** (`cursor.value()`): Extracts text from original JSON

*See [bench-compare/benches/json_parsers.rs](../bench-compare/benches/json_parsers.rs) for traversal benchmarks.*

## Benchmark Methodology

- **Platform**: AMD Ryzen 9 7950X (Zen 4), Linux WSL2
- **Compiler**: rustc 1.85.0
- **Optimization**: `--release` profile
- **Tool**: Criterion.rs with default settings
- **Data**: Generated with `succinctly json generate` (comprehensive pattern)
- **Memory**: Custom global allocator tracking peak usage
- **Samples**: 10-100 per benchmark (fewer for larger files)

## Reproducing Benchmarks

```bash
# Generate test data
cd succinctly
cargo run --release --features cli -- json generate-suite

# Run benchmarks
cd bench-compare
cargo bench --bench json_parsers
```

## Related Documentation

- [jq-comparison.md](jq-comparison.md) - Comparison with system `jq` command
- [optimization-summary.md](optimization-summary.md) - Optimization history and learnings
- [CLAUDE.md](../CLAUDE.md) - Project architecture and development guide
