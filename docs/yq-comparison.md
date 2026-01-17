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
**Optimizations**: P0+ SIMD optimizations (multi-character classification + hybrid scalar/SIMD integration)

## Methodology

Benchmarks measure wall time for the identity filter (`.`) which parses and re-emits the YAML document. This exercises the full parsing pipeline.

Run with:
```bash
cargo bench --bench yq_comparison
```

---

## Summary Results

### ARM (Apple M1 Max) - yq Identity Comparison (users pattern)

| Size      | succinctly   | yq           | Speedup       |
|-----------|--------------|--------------|---------------|
| **10KB**  |   4.5 ms     |   7.5 ms     | **1.7x**      |
| **100KB** |  10.6 ms     |  19.8 ms     | **1.9x**      |
| **1MB**   |  72.9 ms     | 118.6 ms     | **1.6x**      |

#### Throughput Comparison (ARM)

| Size      | succinctly      | yq             | Ratio         |
|-----------|-----------------|----------------|---------------|
| **10KB**  |   2.2 MiB/s     |   1.3 MiB/s    | **1.7x**      |
| **100KB** |   8.6 MiB/s     |   4.6 MiB/s    | **1.9x**      |
| **1MB**   |  12.7 MiB/s     |   7.8 MiB/s    | **1.6x**      |

### x86_64 (AMD Ryzen 9 7950X) - yq Identity Comparison (P2 Optimized)

| Size      | succinctly        | yq              | Speedup        |
|-----------|-------------------|-----------------|----------------|
| **10KB**  |   2.0 ms (4.9 MiB/s)  |  60.2 ms (166 KiB/s) | **30x**   |
| **100KB** |   7.1 ms (13.0 MiB/s) |  74.2 ms (1.2 MiB/s) | **10.5x** |
| **1MB**   |  59.7 ms (15.4 MiB/s) | 194.2 ms (4.7 MiB/s) | **3.3x**  |

### x86_64 (AMD Ryzen 9 7950X) - Internal Micro-Benchmarks (P2 Optimized)

#### Performance Summary (Selected Benchmarks)

| Benchmark | P0+ Baseline | P2 Optimized | Improvement |
|-----------|--------------|--------------|-------------|
| simple_kv/1000 | 41.9 µs | 36.8 µs | **-12%** |
| simple_kv/10000 | 418 µs | 371 µs | **-11%** |
| nested/d5_w2 | 5.8 µs | 5.1 µs | **-12%** |
| large/10kb | 24.4 µs | 22.4 µs | **-8%** |
| large/100kb | 222 µs | 198 µs | **-11%** |
| large/1mb | 2.18 ms | 1.82 ms | **-17%** |
| long_strings/4096b/double | 135 µs | 135 µs | (unchanged) |
| long_strings/4096b/single | 139 µs | 133 µs | **-4%** |

#### Overall Throughput (P2 Optimized - 2026-01-17)

| Workload Category | P0+ Baseline | P2 Optimized | Improvement |
|-------------------|--------------|--------------|-------------|
| Simple KV         | 343-482 MiB/s | 435-484 MiB/s | **+9-11%** |
| Nested structures | 277-454 MiB/s | 277-454 MiB/s | (unchanged) |
| Sequences         | 268-384 MiB/s | 268-384 MiB/s | (unchanged) |
| Large files       | 392-484 MiB/s | 422-515 MiB/s | **+8-17%** |

**Key Achievements (P2):**
- ✅ **Large files: +8-17% faster** (515 MiB/s on 1MB files)
- ✅ **Unquoted values: +9-12% faster** (SIMD classify_yaml_chars integration)
- ✅ **30x faster than yq** on 10KB files (end-to-end CLI comparison)
- ✅ **No regressions** - conditional SIMD avoids overhead for short values

**See also:** [docs/parsing/yaml.md](parsing/yaml.md) for full P2 optimization details and implementation plan.

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

#### x86_64 (AVX2) P2 Optimized Results (2026-01-17)

**P2 Integration of `classify_yaml_chars`:**
- Uses SIMD to scan 32 bytes at once for unquoted value/key terminators
- Conditional activation: only uses SIMD for values ≥32 bytes remaining
- Avoids overhead for typical short YAML values (<32 bytes)

**Unquoted value/key parsing improvements:**
- simple_kv/1000: **-12% faster** (41.9µs → 36.8µs)
- simple_kv/10000: **-11% faster** (418µs → 371µs)
- large/100kb: **-11% faster** (222µs → 198µs)
- large/1mb: **-17% faster** (2.18ms → 1.82ms)

**End-to-end yq comparison (x86_64):**
- 10KB files: **30x faster** than yq (2.0ms vs 60.2ms)
- 100KB files: **10.5x faster** than yq (7.1ms vs 74.2ms)
- 1MB files: **3.3x faster** than yq (59.7ms vs 194.2ms)

**Optimizations (cumulative):**
- **P0**: Multi-character classification infrastructure (8 types in parallel)
- **P0+**: Hybrid scalar/SIMD space skipping (fast path for 0-8 spaces, SIMD for longer runs)
- **P2**: Conditional SIMD for unquoted value/key scanning (32-byte chunks via `classify_yaml_chars`)

**Rejected Optimizations:**
- **P1 (YFSM)**: Table-driven state machine for string parsing tested but showed only 0-2% improvement vs expected 15-25%. YAML strings are too simple compared to JSON (where PFSM succeeded with 33-77% gains). P0+ SIMD already optimal. See [docs/parsing/yaml.md](parsing/yaml.md#p1-yfsm-yaml-finite-state-machine---rejected-) for full analysis.

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
