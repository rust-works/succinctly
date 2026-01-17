# yq vs succinctly YAML Comparison Benchmarks

Benchmarks comparing `succinctly yq .` (identity filter) vs `yq .` (Mike Farah's yq v4.48.1) for YAML formatting/printing.

## Platforms

### Platform 1: ARM (Apple Silicon)

**CPU**: Apple M1 Max
**OS**: macOS Darwin 25.1.0
**yq version**: v4.48.1 (https://github.com/mikefarah/yq/)
**succinctly**: Built with `--release --features cli`
**SIMD**: ARM NEON (16 bytes/iteration for string scanning)

### Platform 2: x86_64 (AMD Zen 4)

**CPU**: AMD Ryzen 9 7950X (Zen 4)
**OS**: Linux 6.6.87.2-microsoft-standard-WSL2 (WSL2)
**succinctly**: Built with `--release --features cli` and `-C target-cpu=native`
**SIMD**: SSE2/AVX2 (16/32 bytes/iteration for string scanning)
**Optimizations**: P0 SIMD optimizations (multi-character classification, enhanced AVX2)

## Methodology

Benchmarks measure wall time for the identity filter (`.`) which parses and re-emits the YAML document. This exercises the full parsing pipeline.

Run with:
```bash
cargo bench --bench yq_comparison
```

---

## Summary Results

### ARM (Apple M1 Max) - yq Identity Comparison (comprehensive pattern)

| Size      | succinctly   | yq           | Speedup       |
|-----------|--------------|--------------|---------------|
| **10KB**  |   4.9 ms     |   8.1 ms     | **1.7x**      |
| **100KB** |  11.4 ms     |  20.0 ms     | **1.8x**      |
| **1MB**   |  72.7 ms     | 119.4 ms     | **1.6x**      |

#### Throughput Comparison (ARM)

| Size      | succinctly      | yq             | Ratio         |
|-----------|-----------------|----------------|---------------|
| **10KB**  |   2.0 MiB/s     |   1.2 MiB/s    | **1.7x**      |
| **100KB** |   8.1 MiB/s     |   4.6 MiB/s    | **1.8x**      |
| **1MB**   |  12.7 MiB/s     |   7.7 MiB/s    | **1.6x**      |

### x86_64 (AMD Ryzen 9 7950X) - Internal Micro-Benchmarks (P0 Optimized)

#### Performance Summary (Selected Benchmarks)

| Benchmark | Baseline | P0 Optimized | Improvement |
|-----------|----------|--------------|-------------|
| simple_kv/10000 | 364 µs (491 MiB/s) | 341 µs (526 MiB/s) | **+6.7%** |
| sequences/10000 | 309 µs (290 MiB/s) | 264 µs (393 MiB/s) | **+14.2%** |
| nested/d3_w5 | 12.54 µs (342 MiB/s) | 11.32 µs (380 MiB/s) | **+9.6%** |
| quoted_strings/double/1000 | 42.1 µs | 35.9 µs | **+16.8%** |
| long_strings/4096b/double | 128.8 µs (2.97 GiB/s) | 104.1 µs (3.67 GiB/s) | **+24.8%** |
| large/100kb | 194.3 µs (491 MiB/s) | 182.1 µs (524 MiB/s) | **+7.0%** |
| large/1mb | timeout | 1.73 ms (550 MiB/s) | **(new)** |

#### Overall Throughput (P0 Optimized - 2026-01-17)

| Workload Category | Baseline Range | P0 Optimized Range | Improvement |
|-------------------|----------------|-------------------|-------------|
| Simple KV         | 176-491 MiB/s  | 187-526 MiB/s     | **+4.5-6.7%** |
| Nested structures | 300-427 MiB/s  | 326-456 MiB/s     | **+8.1-9.8%** |
| Sequences         | 112-290 MiB/s  | 121-393 MiB/s     | **+7.5-14.2%** |
| Quoted strings    | 163-368 MiB/s  | 558-1270 MiB/s    | **+10.9-20.0%** |
| Long strings      | 2.8-3.0 GiB/s  | 3.4-3.7 GiB/s     | **+18.5-25.0%** |
| Large files       | 342-491 MiB/s  | 363-550 MiB/s     | **+6.1-7.0%** |

**Key Achievements:**
- ✅ **String scanning: +18-25% faster** (AVX2 quote/escape detection)
- ✅ **Sequence parsing: +12-14% faster** (improved structural character handling)
- ✅ **1MB files now complete** (previously timed out, now 550 MiB/s)

**See also:** [docs/parsing/yaml.md](parsing/yaml.md) for full P0 optimization details and implementation plan.

---

## Detailed Results by Pattern (ARM - Apple M1 Max)

### Pattern: comprehensive

Mixed YAML content with various features.

| Size      | succinctly           | yq                   | Speedup    |
|-----------|----------------------|----------------------|------------|
| **1KB**   |  4.15 ms (297 KiB/s) |  6.91 ms (178 KiB/s) | **1.7x**   |
| **10KB**  |  4.91 ms (2.0 MiB/s) |  8.10 ms (1.2 MiB/s) | **1.6x**   |
| **100KB** | 11.40 ms (8.1 MiB/s) | 19.95 ms (4.6 MiB/s) | **1.8x**   |
| **1MB**   | 72.72 ms (12.7 MiB/s)| 119.4 ms (7.7 MiB/s) | **1.6x**   |

### Pattern: users

Realistic user record arrays (common in config files).

| Size      | succinctly           | yq                   | Speedup    |
|-----------|----------------------|----------------------|------------|
| **1KB**   |  3.91 ms (274 KiB/s) |  6.99 ms (153 KiB/s) | **1.8x**   |
| **10KB**  |  4.76 ms (2.1 MiB/s) |  8.20 ms (1.2 MiB/s) | **1.7x**   |
| **100KB** | 12.29 ms (7.9 MiB/s) | 22.03 ms (4.4 MiB/s) | **1.8x**   |
| **1MB**   | 92.11 ms (10.9 MiB/s)| 147.7 ms (6.8 MiB/s) | **1.6x**   |

### Pattern: nested

Deeply nested mapping structures.

| Size      | succinctly           | yq                   | Speedup    |
|-----------|----------------------|----------------------|------------|
| **1KB**   |  3.89 ms (258 KiB/s) |  6.96 ms (144 KiB/s) | **1.8x**   |
| **10KB**  |  4.55 ms (1.5 MiB/s) |  7.84 ms (915 KiB/s) | **1.7x**   |
| **100KB** |  7.49 ms (6.7 MiB/s) | 15.25 ms (3.3 MiB/s) | **2.0x**   |
| **1MB**   | 42.33 ms (14.9 MiB/s)| 94.40 ms (6.7 MiB/s) | **2.2x**   |

### Pattern: sequences

Sequence-heavy YAML content.

| Size      | succinctly           | yq                   | Speedup    |
|-----------|----------------------|----------------------|------------|
| **1KB**   |  4.05 ms (250 KiB/s) |  6.89 ms (147 KiB/s) | **1.7x**   |
| **10KB**  |  4.62 ms (2.1 MiB/s) |  8.15 ms (1.2 MiB/s) | **1.8x**   |
| **100KB** | 10.26 ms (9.5 MiB/s) | 21.66 ms (4.5 MiB/s) | **2.1x**   |
| **1MB**   | 64.43 ms (15.5 MiB/s)| 139.1 ms (7.2 MiB/s) | **2.2x**   |

### Pattern: strings

String-heavy YAML with quoted variants.

| Size      | succinctly           | yq                   | Speedup    |
|-----------|----------------------|----------------------|------------|
| **1KB**   |  3.91 ms (259 KiB/s) |  6.84 ms (148 KiB/s) | **1.7x**   |
| **10KB**  |  4.55 ms (2.2 MiB/s) |  7.54 ms (1.3 MiB/s) | **1.7x**   |
| **100KB** | 10.14 ms (9.6 MiB/s) | 14.94 ms (6.5 MiB/s) | **1.5x**   |
| **1MB**   | 64.84 ms (15.4 MiB/s)| 81.77 ms (12.2 MiB/s)| **1.3x**   |

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

- **1.3-2.2x faster** across all patterns and sizes
- **Best performance on nested/sequences**: 2.2x speedup on 1MB deeply nested or sequence-heavy files
- **Nested structures**: Efficient BP tree navigation provides consistent speedups
- **Larger files benefit more**: Speedup increases with file size (amortizes index construction)

### Why succinctly is faster

1. **Semi-index architecture**: YAML structure is pre-indexed using balanced parentheses, enabling O(1) navigation
2. **SIMD string scanning**: ARM NEON accelerates quoted string parsing (6-9% faster than scalar)
3. **SIMD indentation counting**: Vectorized space counting for block-style parsing (10-24% faster on small files)
4. **Streaming output**: For identity queries, outputs directly from source without building intermediate structures
5. **Lazy evaluation**: Only materializes values that are actually accessed

### SIMD Optimizations

The YAML parser uses platform-specific SIMD for hot paths:

| Platform | Instruction Set | Width            | Operations                                    | Status |
|----------|-----------------|------------------|-----------------------------------------------|--------|
| ARM64    | NEON            | 16 bytes/iter    | String scanning, indentation count            | ✓ |
| x86_64   | SSE2/AVX2       | 16-32 bytes/iter | String scanning, indentation, multi-char classification | ✓ P0 |

#### ARM64 (NEON) Results

**String scanning** (double-quoted strings, 1000 entries):
- **Scalar**: 67.1µs @ 688 MiB/s
- **SIMD**:   63.0µs @ 738 MiB/s (+7.3% throughput)

**Indentation scanning** (end-to-end improvement on yq identity):
- **1KB files**: 14-24% faster
- **10KB files**: 10-18% faster
- **100KB+ files**: 5-8% faster (other costs dominate)

#### x86_64 (AVX2) P0 Optimized Results (2026-01-17)

**String scanning improvements:**
- Double-quoted strings (1000 entries): **+17% faster** (42.1µs → 35.9µs)
- Single-quoted strings (1000 entries): **+20% faster** (41.3µs → 33.6µs)
- Long strings (4KB): **+24.8% faster** (128.8µs → 104.1µs, 2.97 → 3.67 GiB/s)

**Overall parsing improvements:**
- Sequences (10,000 items): **+14.2% faster** (309µs → 264µs)
- Nested structures: **+8-10% faster**
- Large files (100KB): **+7.0% faster** (194.3µs → 182.1µs)

**New capabilities:**
- Multi-character classification (8 types in parallel)
- Context-sensitive pattern detection (e.g., `: ` and `- `)
- Newline detection infrastructure

### Trade-offs

- **Small files (<1KB)**: Process startup dominates; speedup is modest (~1.7x)
- **Large files (1MB+)**: Index construction amortizes; best speedups (~2x)
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
