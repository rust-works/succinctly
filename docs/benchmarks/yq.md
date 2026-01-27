# yq vs succinctly YAML Comparison Benchmarks

Benchmarks comparing `succinctly yq .` (identity filter) vs `yq .` (Mike Farah's yq v4.48.1) for YAML formatting/printing.

**See also**: [YAML 1.2 Compliance](../compliance/yaml/1.2.md) for type handling details (Norway problem, booleans, etc.)

## Architectural Differences

Before reviewing the benchmarks, it's important to understand the fundamental differences between these tools:

| Aspect               | succinctly                        | yq (mikefarah)                    |
|----------------------|-----------------------------------|-----------------------------------|
| **Architecture**     | Semi-indexer                      | Full validating parser            |
| **Approach**         | Build structural index, lazy eval | Parse entire document into DOM    |
| **Validation**       | Minimal (syntax only)             | Full YAML validation              |
| **Tags**             | Not supported                     | Full support (`!custom`, `!!str`) |
| **Anchors/Aliases**  | Supported                         | Supported                         |
| **Memory model**     | ~3-6% index overhead              | Full DOM in memory                |

**What this means for benchmarks**: The performance differences shown below come from two sources:

1. **Architectural advantage**: Semi-indexing with lazy evaluation is inherently faster for query workloads because it only materializes values that are accessed.

2. **Less validation work**: succinctly performs minimal validation compared to yq's full YAML validation. This is a deliberate trade-off for speed.

succinctly supports most yq query patterns and can serve as a drop-in replacement for common use cases. The main limitation is tags. See [Compatibility with yq](#compatibility-with-yq) below for details.

For detailed architectural documentation, see [Semi-Indexing Architecture](../architecture/semi-indexing.md).

## Platforms

### Platform 1: ARM (AWS Graviton 4 - Neoverse-V2)

**CPU**: ARM Neoverse-V2 (AWS Graviton 4)
**OS**: Linux 6.14.0-1018-aws
**yq version**: v4.48.1 (https://github.com/mikefarah/yq/)
**succinctly**: Built with `--release --features cli`
**SIMD**: NEON (128-bit), SVE2 (128-bit vectors), SVEBITPERM (BDEP/BEXT)

### Platform 2: ARM (AWS Graviton 3 - Neoverse-V1)

**CPU**: ARM Neoverse-V1 (4 cores)
**OS**: Linux 6.14.0-1018-aws
**yq version**: Not installed (succinctly-only benchmarks)
**succinctly**: Built with `--release --features cli`
**SIMD**: NEON (128-bit), SVE (256-bit vectors)

### Platform 4: ARM (Apple Silicon)

**CPU**: Apple M1 Max
**OS**: macOS Darwin 25.1.0
**yq version**: v4.48.1 (https://github.com/mikefarah/yq/)
**succinctly**: Built with `--release --features cli`
**SIMD**: ARM NEON (16 bytes/iteration for string scanning)

### Platform 5: x86_64 (AMD Zen 4)

**CPU**: AMD Ryzen 9 7950X (Zen 4)
**OS**: Linux 6.6.87.2-microsoft-standard-WSL2 (WSL2)
**yq version**: v4.49.2 (https://github.com/mikefarah/yq/)
**succinctly**: Built with `--release --features cli` and `-C target-cpu=native`
**SIMD**: SSE2/AVX2 (16/32 bytes/iteration for string scanning), BMI2 (PDEP for select_in_word)
**Optimizations**: P0+ SIMD optimizations (multi-character classification + hybrid scalar/SIMD integration)

## Methodology

Benchmarks measure:
- **Wall time**: Total elapsed time
- **Peak memory**: Maximum resident set size (RSS)
- **Output correctness**: MD5 hash comparison ensures identical output

Run with:
```bash
# CLI benchmark tool (recommended - includes memory measurement)
succinctly dev bench yq

# Criterion benchmark (wall time only)
cargo bench --bench yq_comparison
```

---

## Summary Results

### ARM (Neoverse-V2 / Graviton 4) - yq Identity Comparison

| Size      | succinctly             | yq                     | Speedup    |
|-----------|------------------------|------------------------|------------|
| **10KB**  | 2.0 ms (4.9 MiB/s)     | 5.5 ms (1.8 MiB/s)     | **2.7x**   |
| **100KB** | 3.7 ms (24.9 MiB/s)    | 21.6 ms (4.3 MiB/s)    | **5.8x**   |
| **1MB**   | 19.5 ms (47.3 MiB/s)   | 154.6 ms (6.0 MiB/s)   | **7.9x**   |

**Key metrics:**
- ✅ NEON SIMD optimizations active (block scalar + anchor parsing)
- ✅ SVE2 with SVEBITPERM support (BDEP/BEXT for DSV)
- ✅ **7.9x faster** than yq on 1MB files
- ✅ 47 MiB/s throughput vs yq's 6 MiB/s
- ✅ Up to **11.2x faster** on nested structures (100MB)

### ARM (Neoverse-V1 / Graviton 3) - succinctly yq Performance

Note: System `yq` was not installed for comparison. Results show succinctly performance only.

| Size      | succinctly Time | Throughput     |
|-----------|-----------------|----------------|
| **1KB**   | 1.11 ms         | 1.09 MiB/s     |
| **10KB**  | 1.27 ms         | 7.71 MiB/s     |
| **100KB** | 2.87 ms         | 32.0 MiB/s     |
| **1MB**   | 17.71 ms        | 52.1 MiB/s     |

**Key metrics:**
- ✅ NEON SIMD optimizations active
- ✅ 52 MiB/s throughput on 1MB files
- ✅ Scales well: 1MB throughput 47x higher than 1KB
- ✅ SVE-capable CPU (256-bit vectors)

### ARM (Apple M1 Max) - yq Identity Comparison (comprehensive pattern)

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **10KB**   |    6.4 ms    |    9.2 ms    | **1.4x**      |    7 MB  |   14 MB | **0.51x** |
| **100KB**  |    7.1 ms    |   20.9 ms    | **2.9x**      |    8 MB  |   23 MB | **0.33x** |
| **1MB**    |   22.8 ms    |  115.3 ms    | **5.1x**      |   10 MB  |   78 MB | **0.12x** |
| **10MB**   |  165.3 ms    |    1.03 s    | **6.2x**      |   30 MB  |  678 MB | **0.04x** |
| **100MB**  |    1.52 s    |    9.84 s    | **6.5x**      |  228 MB  |    7 GB | **0.03x** |

**Key metrics:**
- ✅ **6.5x faster** than yq on 100MB files
- ✅ **31x less memory** on large files (100MB: 228 MB vs 7 GB)
- ✅ Speedup increases with file size (amortizes index construction)
- ✅ Memory efficiency improves dramatically at scale
- ✅ **Streaming JSON output** eliminates intermediate String allocations

### x86_64 (AMD Ryzen 9 7950X) - yq Identity Comparison (comprehensive pattern)

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.6 ms    |   61.4 ms    | **23x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.6 ms    |   63.0 ms    | **24x**       |    5 MB  |   22 MB | **0.21x** |
| **100KB**  |    4.1 ms    |  100.9 ms    | **25x**       |    5 MB  |   22 MB | **0.21x** |
| **1MB**    |   17.9 ms    |  207.8 ms    | **12x**       |    7 MB  |   82 MB | **0.09x** |
| **10MB**   |  151.4 ms    |    1.34 s    | **9x**        |   30 MB  |  627 MB | **0.05x** |
| **100MB**  |    1.42 s    |   13.30 s    | **9x**        |  258 MB  |    6 GB | **0.04x** |

### x86_64 (AMD Ryzen 9 7950X) - Key Achievements

**Key Achievements:**
- ✅ **Full yq CLI compatibility** - quoted strings preserved as strings
- ✅ **25x faster than yq** on 100KB files (end-to-end CLI comparison)
- ✅ **12x faster than yq** on 1MB files
- ✅ **25x less memory** on 100MB files (258 MB vs 6 GB)
- ✅ **BMI2 PDEP** for O(1) select_in_word (7.6-16x faster select1)
- ✅ **Comprehensive test suite** - 32 tests including 8 direct byte-for-byte comparisons

**See also:** [docs/parsing/yaml.md](../parsing/yaml.md) for optimization details.

---

## Detailed Results by Pattern (ARM - Apple M1 Max)

### Pattern: comprehensive

Mixed YAML content with various features.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.7 ms    |    8.7 ms    | **1.5x**      |    7 MB  |   14 MB | **0.54x** |
| **10KB**   |    6.4 ms    |    9.2 ms    | **1.4x**      |    7 MB  |   14 MB | **0.51x** |
| **100KB**  |    7.1 ms    |   20.9 ms    | **2.9x**      |    8 MB  |   23 MB | **0.33x** |
| **1MB**    |   22.8 ms    |  115.3 ms    | **5.1x**      |   10 MB  |   78 MB | **0.12x** |
| **10MB**   |  165.3 ms    |    1.03 s    | **6.2x**      |   30 MB  |  678 MB | **0.04x** |
| **100MB**  |    1.52 s    |    9.84 s    | **6.5x**      |  228 MB  |    7 GB | **0.03x** |

### Pattern: nested

Deeply nested mapping structures. **Best speedup pattern** due to efficient BP tree navigation.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.5 ms    |    7.8 ms    | **1.4x**      |    7 MB  |   13 MB | **0.54x** |
| **10KB**   |    5.2 ms    |    8.5 ms    | **1.6x**      |    7 MB  |   14 MB | **0.52x** |
| **100KB**  |    6.6 ms    |   15.6 ms    | **2.4x**      |    8 MB  |   20 MB | **0.38x** |
| **1MB**    |   14.3 ms    |   91.2 ms    | **6.4x**      |    9 MB  |   61 MB | **0.15x** |
| **10MB**   |   89.0 ms    |  787.3 ms    | **8.8x**      |   21 MB  |  418 MB | **0.05x** |
| **100MB**  |  678.2 ms    |    6.35 s    | **9.4x**      |  122 MB  |    3 GB | **0.04x** |

### Pattern: pathological

Worst-case structural density. Tests parser robustness.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.3 ms    |    8.5 ms    | **1.6x**      |    7 MB  |   13 MB | **0.54x** |
| **10KB**   |    6.1 ms    |    9.7 ms    | **1.6x**      |    7 MB  |   15 MB | **0.50x** |
| **100KB**  |    7.7 ms    |   23.2 ms    | **3.0x**      |    8 MB  |   25 MB | **0.31x** |
| **1MB**    |   22.4 ms    |  148.9 ms    | **6.6x**      |   10 MB  |   94 MB | **0.11x** |
| **10MB**   |  156.5 ms    |    1.31 s    | **8.3x**      |   30 MB  |  714 MB | **0.04x** |
| **100MB**  |    1.41 s    |   12.60 s    | **8.9x**      |  225 MB  |    7 GB | **0.03x** |

### Pattern: users

Realistic user record arrays (common in config files).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.2 ms    |    7.5 ms    | **1.4x**      |    7 MB  |   13 MB | **0.55x** |
| **10KB**   |    5.4 ms    |    8.6 ms    | **1.6x**      |    7 MB  |   14 MB | **0.52x** |
| **100KB**  |    8.0 ms    |   21.7 ms    | **2.7x**      |    8 MB  |   24 MB | **0.32x** |
| **1MB**    |   25.0 ms    |  139.7 ms    | **5.6x**      |   10 MB  |   95 MB | **0.11x** |
| **10MB**   |  193.3 ms    |    1.29 s    | **6.7x**      |   33 MB  |  888 MB | **0.04x** |
| **100MB**  |    1.88 s    |   13.01 s    | **6.9x**      |  261 MB  |    9 GB | **0.03x** |

### Pattern: sequences

Sequence-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    4.9 ms    |    7.5 ms    | **1.6x**      |    7 MB  |   13 MB | **0.55x** |
| **10KB**   |    5.5 ms    |    9.5 ms    | **1.7x**      |    7 MB  |   15 MB | **0.50x** |
| **100KB**  |    7.3 ms    |   21.7 ms    | **3.0x**      |    8 MB  |   24 MB | **0.33x** |
| **1MB**    |   25.5 ms    |  132.9 ms    | **5.2x**      |   11 MB  |  106 MB | **0.10x** |
| **10MB**   |  194.9 ms    |    1.20 s    | **6.2x**      |   46 MB  |  846 MB | **0.05x** |
| **100MB**  |    1.83 s    |   11.64 s    | **6.3x**      |  384 MB  |    8 GB | **0.05x** |

### Pattern: strings

String-heavy YAML with quoted variants.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.8 ms    |    8.1 ms    | **1.4x**      |    7 MB  |   13 MB | **0.55x** |
| **10KB**   |    5.7 ms    |    9.0 ms    | **1.6x**      |    7 MB  |   14 MB | **0.53x** |
| **100KB**  |    6.9 ms    |   15.6 ms    | **2.3x**      |    8 MB  |   20 MB | **0.39x** |
| **1MB**    |   17.9 ms    |   79.3 ms    | **4.4x**      |    9 MB  |   64 MB | **0.15x** |
| **10MB**   |  121.2 ms    |  671.1 ms    | **5.5x**      |   27 MB  |  479 MB | **0.06x** |
| **100MB**  |    1.15 s    |    6.53 s    | **5.7x**      |  210 MB  |    5 GB | **0.04x** |

### Pattern: numbers

Numeric-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.5 ms    |    7.9 ms    | **1.4x**      |    7 MB  |   13 MB | **0.54x** |
| **10KB**   |    5.9 ms    |    9.2 ms    | **1.6x**      |    7 MB  |   14 MB | **0.52x** |
| **100KB**  |    7.0 ms    |   20.2 ms    | **2.9x**      |    8 MB  |   22 MB | **0.35x** |
| **1MB**    |   22.2 ms    |  116.9 ms    | **5.3x**      |   10 MB  |   82 MB | **0.12x** |
| **10MB**   |  164.4 ms    |    1.00 s    | **6.1x**      |   29 MB  |  578 MB | **0.05x** |
| **100MB**  |    1.52 s    |    9.38 s    | **6.2x**      |  227 MB  |    6 GB | **0.04x** |

### Pattern: unicode

Unicode-heavy strings in various scripts.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.6 ms    |    8.1 ms    | **1.5x**      |    7 MB  |   13 MB | **0.55x** |
| **10KB**   |    5.8 ms    |    8.6 ms    | **1.5x**      |    7 MB  |   14 MB | **0.53x** |
| **100KB**  |    6.4 ms    |   16.2 ms    | **2.5x**      |    8 MB  |   20 MB | **0.39x** |
| **1MB**    |   17.4 ms    |   74.8 ms    | **4.3x**      |    9 MB  |   59 MB | **0.16x** |
| **10MB**   |  116.9 ms    |  640.5 ms    | **5.5x**      |   27 MB  |  444 MB | **0.06x** |
| **100MB**  |    1.11 s    |    6.21 s    | **5.6x**      |  198 MB  |    4 GB | **0.04x** |

### Pattern: mixed

Mixed mappings and sequences.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.2 ms    |    8.0 ms    | **1.5x**      |    7 MB  |   14 MB | **0.54x** |
| **10KB**   |    5.7 ms    |    9.4 ms    | **1.6x**      |    7 MB  |   15 MB | **0.51x** |
| **100KB**  |    7.2 ms    |   20.4 ms    | **2.8x**      |    8 MB  |   24 MB | **0.33x** |
| **1MB**    |   21.1 ms    |  114.2 ms    | **5.4x**      |   10 MB  |   88 MB | **0.11x** |
| **10MB**   |  151.8 ms    |  998.8 ms    | **6.6x**      |   29 MB  |  703 MB | **0.04x** |
| **100MB**  |    1.42 s    |    9.43 s    | **6.6x**      |  217 MB  |    6 GB | **0.03x** |

---

## Detailed Results by Pattern (x86_64 - AMD Ryzen 9 7950X)

### Pattern: comprehensive

Mixed YAML content with various features.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.6 ms    |   61.4 ms    | **23x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.6 ms    |   63.0 ms    | **24x**       |    5 MB  |   22 MB | **0.21x** |
| **100KB**  |    4.1 ms    |  100.9 ms    | **25x**       |    5 MB  |   22 MB | **0.21x** |
| **1MB**    |   17.9 ms    |  207.8 ms    | **12x**       |    7 MB  |   82 MB | **0.09x** |
| **10MB**   |  151.4 ms    |    1.34 s    | **9x**        |   30 MB  |  627 MB | **0.05x** |
| **100MB**  |    1.42 s    |   13.30 s    | **9x**        |  258 MB  |    6 GB | **0.04x** |

### Pattern: nested

Deeply nested mapping structures. **Best speedup pattern** due to efficient BP tree navigation.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.5 ms    |   64.2 ms    | **25x**       |    4 MB  |   22 MB | **0.20x** |
| **10KB**   |    2.6 ms    |   64.2 ms    | **24x**       |    4 MB  |   22 MB | **0.20x** |
| **100KB**  |    3.2 ms    |   73.6 ms    | **23x**       |    5 MB  |   23 MB | **0.21x** |
| **1MB**    |   11.2 ms    |  168.8 ms    | **15x**       |    6 MB  |   55 MB | **0.11x** |
| **10MB**   |   87.1 ms    |  992.9 ms    | **11x**       |   20 MB  |  430 MB | **0.05x** |
| **100MB**  |  689.1 ms    |    7.87 s    | **11x**       |  136 MB  |    4 GB | **0.04x** |

### Pattern: pathological

Worst-case structural density. Tests parser robustness.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.7 ms    |   63.2 ms    | **24x**       |    4 MB  |   22 MB | **0.20x** |
| **10KB**   |    2.8 ms    |   64.6 ms    | **23x**       |    4 MB  |   22 MB | **0.20x** |
| **100KB**  |    4.4 ms    |   83.5 ms    | **19x**       |    5 MB  |   22 MB | **0.22x** |
| **1MB**    |   19.0 ms    |  246.6 ms    | **13x**       |    7 MB  |   94 MB | **0.08x** |
| **10MB**   |  153.8 ms    |    1.65 s    | **11x**       |   31 MB  |  739 MB | **0.04x** |
| **100MB**  |    1.39 s    |   16.16 s    | **12x**       |  265 MB  |    7 GB | **0.03x** |

### Pattern: users

Realistic user record arrays (common in config files).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.6 ms    |   63.4 ms    | **25x**       |    4 MB  |   22 MB | **0.20x** |
| **10KB**   |    2.6 ms    |   65.1 ms    | **25x**       |    5 MB  |   22 MB | **0.21x** |
| **100KB**  |    4.3 ms    |   83.2 ms    | **19x**       |    5 MB  |   23 MB | **0.21x** |
| **1MB**    |   21.1 ms    |  241.3 ms    | **11x**       |    8 MB  |   92 MB | **0.08x** |
| **10MB**   |  181.5 ms    |    1.70 s    | **9x**        |   34 MB  |  835 MB | **0.04x** |
| **100MB**  |    1.76 s    |   17.38 s    | **10x**       |  305 MB  |    8 GB | **0.04x** |

### Pattern: sequences

Sequence-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.4 ms    |   63.1 ms    | **26x**       |    4 MB  |   22 MB | **0.20x** |
| **10KB**   |    2.6 ms    |   65.1 ms    | **25x**       |    5 MB  |   23 MB | **0.21x** |
| **100KB**  |    4.5 ms    |   82.3 ms    | **18x**       |    5 MB  |   22 MB | **0.23x** |
| **1MB**    |   20.0 ms    |  228.2 ms    | **11x**       |    8 MB  |   89 MB | **0.09x** |
| **10MB**   |  168.0 ms    |    1.66 s    | **10x**       |   36 MB  |  942 MB | **0.04x** |
| **100MB**  |    1.59 s    |   15.98 s    | **10x**       |  312 MB  |    8 GB | **0.04x** |

### Pattern: strings

String-heavy YAML with quoted variants.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.5 ms    |   63.1 ms    | **25x**       |    4 MB  |   23 MB | **0.20x** |
| **10KB**   |    2.6 ms    |   66.3 ms    | **25x**       |    4 MB  |   22 MB | **0.20x** |
| **100KB**  |    3.8 ms    |   74.2 ms    | **20x**       |    5 MB  |   22 MB | **0.21x** |
| **1MB**    |   14.6 ms    |  156.2 ms    | **11x**       |    7 MB  |   54 MB | **0.12x** |
| **10MB**   |  119.1 ms    |  905.0 ms    | **8x**        |   26 MB  |  452 MB | **0.06x** |
| **100MB**  |    1.13 s    |    8.48 s    | **8x**        |  223 MB  |    4 GB | **0.05x** |

### Pattern: numbers

Numeric-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.5 ms    |   62.6 ms    | **25x**       |    4 MB  |   22 MB | **0.20x** |
| **10KB**   |    2.7 ms    |   64.7 ms    | **24x**       |    4 MB  |   22 MB | **0.20x** |
| **100KB**  |    4.4 ms    |   77.9 ms    | **18x**       |    5 MB  |   23 MB | **0.21x** |
| **1MB**    |   18.9 ms    |  207.1 ms    | **11x**       |    7 MB  |   67 MB | **0.10x** |
| **10MB**   |  157.2 ms    |    1.31 s    | **8x**        |   30 MB  |  562 MB | **0.05x** |
| **100MB**  |    1.46 s    |   11.95 s    | **8x**        |  248 MB  |    5 GB | **0.05x** |

### Pattern: unicode

Unicode-heavy strings in various scripts.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.4 ms    |   62.1 ms    | **26x**       |    4 MB  |   22 MB | **0.20x** |
| **10KB**   |    2.5 ms    |   63.8 ms    | **26x**       |    4 MB  |   22 MB | **0.21x** |
| **100KB**  |    3.7 ms    |   73.4 ms    | **20x**       |    5 MB  |   22 MB | **0.21x** |
| **1MB**    |   13.5 ms    |  151.7 ms    | **11x**       |    7 MB  |   52 MB | **0.13x** |
| **10MB**   |  105.6 ms    |  885.9 ms    | **8x**        |   26 MB  |  465 MB | **0.06x** |
| **100MB**  |    1.03 s    |    8.12 s    | **8x**        |  223 MB  |    4 GB | **0.05x** |

### Pattern: mixed

Mixed mappings and sequences.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.5 ms    |   63.4 ms    | **25x**       |    4 MB  |   22 MB | **0.20x** |
| **10KB**   |    2.6 ms    |   65.0 ms    | **25x**       |    4 MB  |   22 MB | **0.20x** |
| **100KB**  |    4.1 ms    |   79.9 ms    | **19x**       |    5 MB  |   22 MB | **0.22x** |
| **1MB**    |   16.9 ms    |  212.7 ms    | **13x**       |    7 MB  |   88 MB | **0.08x** |
| **10MB**   |  140.6 ms    |    1.31 s    | **9x**        |   30 MB  |  624 MB | **0.05x** |
| **100MB**  |    1.31 s    |   12.29 s    | **9x**        |  245 MB  |    6 GB | **0.04x** |

---

## Pattern Descriptions

| Pattern           | Description                                     | Best For               |
|-------------------|-------------------------------------------------|------------------------|
| **comprehensive** | Mixed content with various YAML features        | General benchmarking   |
| **nested**        | Deeply nested mappings (tests tree navigation)  | BP tree performance    |
| **pathological**  | Maximum structural density                      | Parser stress testing  |
| **users**         | Realistic user record objects (config-like)     | Real-world configs     |
| **sequences**     | Sequence-heavy content (arrays)                 | Array-heavy workloads  |
| **strings**       | String-heavy with quoted variants               | String parsing         |
| **numbers**       | Numeric-heavy content                           | Number parsing         |
| **unicode**       | Unicode strings in various scripts              | UTF-8 handling         |
| **mixed**         | Mixed mappings and sequences                    | Balanced workloads     |

---

## Key Findings

### Speed

**Apple M1 Max (ARM):**
- **1.4-9.4x faster** across all patterns and sizes
- **Best performance on nested structures**: 9.4x speedup on 100MB deeply nested files
- **Pathological patterns**: 8.9x speedup on worst-case structural density
- **Larger files benefit more**: Speedup increases with file size (amortizes index construction)
- **100MB files**: 5.6-9.4x faster depending on pattern

**AMD Ryzen 9 7950X (x86_64):**
- **8-25x faster** across all patterns and sizes (1MB+)
- **Best performance on nested structures**: 15x speedup on 1MB, 11x on 100MB
- **Small files dominate by startup**: 23-26x faster on 1-10KB (yq startup ~62ms)
- **100MB files**: 8-12x faster depending on pattern
- **BMI2 PDEP**: O(1) select_in_word gives 7.6-16x faster BP select1 queries

### Memory

**Apple M1 Max:**
- **20-35x less memory** on large files (100MB) with streaming output
- **Consistent ~7 MB baseline** for small files regardless of pattern
- **Linear scaling**: Memory grows proportionally with file size
- **yq memory explosion**: yq uses 3-9 GB for 100MB files vs 122-384 MB for succinctly

| Size       | succinctly    | yq          | Ratio        |
|------------|---------------|-------------|--------------|
| **1KB**    | 7 MB          | 13-14 MB    | ~0.5x        |
| **10KB**   | 7 MB          | 14-15 MB    | ~0.5x        |
| **100KB**  | 8 MB          | 20-25 MB    | ~0.35x       |
| **1MB**    | 9-11 MB       | 59-106 MB   | ~0.12x       |
| **10MB**   | 21-46 MB      | 418-888 MB  | ~0.05x       |
| **100MB**  | 122-384 MB    | 3-9 GB      | ~0.04x       |

**AMD Ryzen 9 7950X:**
- **20-25x less memory** on 100MB files (136-312 MB vs 4-8 GB)
- **Consistent ~4-5 MB baseline** for small files
- Lower baseline than ARM due to smaller yq Go runtime on x86_64

| Size       | succinctly    | yq          | Ratio        |
|------------|---------------|-------------|--------------|
| **1KB**    | 4-5 MB        | 22-23 MB    | ~0.20x       |
| **10KB**   | 4-5 MB        | 22-23 MB    | ~0.20x       |
| **100KB**  | 5 MB          | 22-23 MB    | ~0.21x       |
| **1MB**    | 6-8 MB        | 52-94 MB    | ~0.10x       |
| **10MB**   | 20-36 MB      | 430-942 MB  | ~0.05x       |
| **100MB**  | 136-312 MB    | 4-8 GB      | ~0.04x       |

### Why succinctly is faster

1. **Semi-index architecture**: YAML structure is pre-indexed using balanced parentheses, enabling O(1) navigation
2. **Direct YAML→JSON transcoding**: Escape sequences are translated directly without intermediate string allocation
3. **SIMD string scanning**: ARM NEON accelerates quoted string parsing (6-9% faster than scalar)
4. **SIMD indentation counting**: Vectorized space counting for block-style parsing (10-24% faster on small files)
5. **Streaming JSON output**: Writes JSON directly to output without building intermediate `String` objects, reducing memory by 12-14% on large files
6. **Lazy evaluation**: Only materializes values that are actually accessed

### SIMD Optimizations

The YAML parser uses platform-specific SIMD for hot paths:

| Platform | Instruction Set | Width            | Operations                                               | Status |
|----------|-----------------|------------------|----------------------------------------------------------|--------|
| ARM64    | NEON            | 16 bytes/iter    | String scanning, indentation count                       | ✓      |
| ARM64    | SVE2 BDEP       | O(1)/word        | select_in_word (bit deposit for k-th set bit)            | ✓      |
| x86_64   | SSE2/AVX2       | 16-32 bytes/iter | String scanning, indentation, multi-char classification  | ✓ P0   |
| x86_64   | BMI2 PDEP       | O(1)/word        | select_in_word (parallel bit deposit for k-th set bit)   | ✓ new  |

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
- 10KB files: **24x faster** than yq (2.9ms vs 68.9ms)
- 100KB files: **19x faster** than yq (4.4ms vs 85.2ms)
- 1MB files: **11x faster** than yq (20.6ms vs 216.5ms)

**Optimizations (cumulative):**
- **P0**: Multi-character classification infrastructure (8 types in parallel)
- **P0+**: Hybrid scalar/SIMD space skipping (fast path for 0-8 spaces, SIMD for longer runs)
- **P2**: Conditional SIMD for unquoted value/key scanning (32-byte chunks via `classify_yaml_chars`)
- **P2.7**: Block scalar SIMD - AVX2 newline scanning + indentation checking (19-25% improvement)
- **P4**: Anchor/Alias SIMD - AVX2 scans for anchor name terminators (6-17% improvement on anchor-heavy workloads)
- **P9**: Direct YAML-to-JSON streaming - eliminated intermediate DOM for identity queries (8-22% improvement, 2.3x on yq benchmarks)
- **P11**: BP Select1 for yq-locate - zero-cost generic select support for O(1) offset-to-BP lookups (2.5-5.9x faster select1, fixes issue #26)
- **P12**: Advance Index for bp_to_text - memory-efficient bitmap encoding (~1.5× measured, 20-25% faster yq identity queries on 1MB files)
- **BMI2 PDEP select_in_word**: O(1) select_in_word on x86_64 using BMI2 PDEP instruction (7.6-16x faster BP select1 queries, with AMD Zen 1/2 slow-path detection)
- **M2**: Navigation streaming - cursor-based streaming for navigation queries, supports both JSON and YAML output

**Rejected Optimizations:**
- **P1 (YFSM)**: Table-driven state machine for string parsing tested but showed only 0-2% improvement vs expected 15-25%. YAML strings are too simple compared to JSON (where PFSM succeeded with 33-77% gains). P0+ SIMD already optimal. See [docs/parsing/yaml.md](../parsing/yaml.md) for full analysis.
- **P2.6, P2.8, P3, P5, P6, P7, P8**: Various optimizations rejected due to micro-benchmark/real-world mismatches, grammar incompatibilities, or memory bottlenecks. See [docs/parsing/yaml.md](../parsing/yaml.md) for detailed analyses.

### Trade-offs

- **Small files (<1KB)**: Process startup dominates; speedup is modest (~1.5x)
- **Large files (1MB+)**: Index construction amortizes; best speedups (5-13x)
- **Memory**: succinctly uses ~50% memory for small files, ~4% for large files
- **Startup overhead**: ~6ms baseline for small files due to process startup

---

## Selection Benchmarks (Lazy Evaluation)

The `yq_select` benchmark demonstrates lazy evaluation benefits when selecting partial data (~5% of input).

### Query Types

**5% Slice Selection** - Extract first ~5% of array elements:
- `.users[:4]` (4 of 75 users from 10KB)
- `.users[:37]` (37 of 742 users from 100KB)
- `.users[:377]` (377 of 7540 users from 1MB)

**Single Field Extraction** - Navigate to specific elements:
- `.users[0].name` (first user)
- `.users[3770]` (middle of 1MB file)
- `.users[7000]` (near end of 1MB file)

### Why Selection is Faster

With lazy evaluation, succinctly only materializes the requested subset:
1. **Semi-index navigation**: O(1) skip to array element N using BP tree
2. **Partial parsing**: Only parses values that are output
3. **No DOM construction**: Streams directly from source bytes

Traditional parsers (like yq) must parse the entire document before selection.

Run with:
```bash
cargo bench --bench yq_select
```

---

## Compatibility with yq

### Drop-in Replacement Status

**`succinctly yq` is now yq-compatible** for YAML scalar type preservation. Quoted strings are preserved as strings, and unquoted scalars undergo YAML type detection.

### Type Preservation Behavior

`succinctly yq` correctly implements YAML 1.2 type preservation:

**Example**:
```yaml
# Source YAML
version: "1.0"   # Quoted string
id: "001"        # Quoted string
number: 123      # Unquoted number
code: "007"      # Quoted string
```

**Output (both `yq` and `succinctly yq`):**
```json
{
  "version": "1.0",
  "id": "001",
  "number": 123,
  "code": "007"
}
```

**Behavior**:
- **Quoted strings** (`"1.0"`, `"001"`) are always output as JSON strings
- **Unquoted scalars** (`123`, `true`, `null`) undergo type detection per YAML 1.2 spec
- **Preserves semantic meaning** of the original YAML document

### Compatible Query Patterns

✓ **All standard yq query patterns produce identical output**:

| Pattern           | Example                        | Notes                       |
|-------------------|--------------------------------|-----------------------------|
| Identity          | `.`                            | Full document output        |
| Field selection   | `.users[].name`                | String and number fields    |
| Filtering         | `.users[] \| select(.age > 30)`| Comparison operations       |
| Boolean selection | `.users[] \| select(.active)`  | Boolean values              |
| Array indexing    | `.users[0]`                    | Array element access        |
| Nested paths      | `.config.version`              | Preserves quoted strings    |
| Number fields     | `.scores.total`                | Unquoted numbers            |
| Quoted numbers    | `.id`                          | Preserves `"001"` as string |

### Migration from yq

`succinctly yq` can be used as a drop-in replacement for `yq` in most scenarios:

```bash
# These produce identical output:
yq -o json '.' file.yaml
succinctly yq -o json '.' file.yaml

# Version strings preserved correctly
$ echo 'version: "1.0"' | succinctly yq -o json '.'
{"version":"1.0"}

# Leading-zero IDs preserved
$ echo 'id: "001"' | succinctly yq -o json '.'
{"id":"001"}
```

### Performance Benefits

`succinctly yq` offers significant performance advantages over `yq`:

**Speed (Apple M1 Max):**
- **9.4x faster** on 100MB nested files
- **6.5x faster** on 100MB comprehensive files
- **5.1x faster** on 1MB files

**Speed (AMD Ryzen 9 7950X):**
- **25x faster** on 100KB files
- **12x faster** on 1MB files
- **9x faster** on 100MB files

**Memory (Apple M1 Max, 100MB files):**
- **20-35x less memory** (122-384 MB vs 3-9 GB)
- Linear scaling vs exponential growth

**Architecture:**
- **Streaming output** for better scalability
- **Lazy evaluation** - only materializes accessed values
- **Full yq compatibility** for type preservation

---

## BP Select1 Micro-benchmark (P11)

The `bp_select_micro` benchmark measures the performance of select1 queries on balanced parentheses, comparing the new sampled select index (WithSelect) against the old binary search on rank1 approach.

### Results (Apple M1 Max, 10K queries each)

| BP Size    | select1 (new) | binary_search_rank1 (old) | Speedup  |
|------------|---------------|---------------------------|----------|
| 1K opens   | 326 µs        | 820 µs                    | **2.5x** |
| 10K opens  | 318 µs        | 1.31 ms                   | **4.1x** |
| 100K opens | 308 µs        | 1.68 ms                   | **5.4x** |
| 1M opens   | 356 µs        | 2.10 ms                   | **5.9x** |

### Results (ARM Neoverse-V2, 10K queries each)

| BP Size    | select1 (new) | binary_search_rank1 (old) | Speedup   |
|------------|---------------|---------------------------|-----------|
| 1K opens   | 182 µs        | 759 µs                    | **4.2x**  |
| 10K opens  | 162 µs        | 1.12 ms                   | **6.9x**  |
| 100K opens | 156 µs        | 1.42 ms                   | **9.1x**  |
| 1M opens   | 173 µs        | 1.73 ms                   | **10.0x** |

### Results (AMD Ryzen 9 7950X with BMI2 PDEP, 10K queries each)

| BP Size    | select1 (new) | binary_search_rank1 (old) | Speedup   |
|------------|---------------|---------------------------|-----------|
| 1K opens   | 49 µs         | 371 µs                    | **7.6x**  |
| 10K opens  | 47 µs         | 538 µs                    | **11.4x** |
| 100K opens | 47 µs         | 687 µs                    | **14.5x** |
| 1M opens   | 56 µs         | 890 µs                    | **16.0x** |

### Key Observations

- **select1 stays nearly constant**: ~47-56 µs regardless of BP size (O(1) amortized via PDEP)
- **binary_search scales with log(n)**: 371-890 µs as BP grows from 1K to 1M
- **Larger documents benefit more**: 16x speedup at 1M opens vs 7.6x at 1K
- **BMI2 PDEP dominates on x86_64**: 7.6-16x speedup vs 2.5-5.9x on M1 Max and 4.2-10x on Neoverse-V2
- **AMD Zen 4 has fast BMI2**: PDEP runs in 3 cycles (vs 18 cycles on AMD Zen 1/2, which is auto-detected and avoided)

### Why It Matters

This optimization enables fast `yq-locate` queries on large YAML documents. The `find_bp_at_text_pos()` function uses select1 to convert from an index in `bp_to_text` back to a BP position.

Run with:
```bash
cargo bench --bench bp_select_micro
```

---

## M2 Streaming Navigation Benchmarks

The M2 streaming optimization enables fast navigation queries without building an intermediate DOM. M2 supports both JSON (`-o json -I0`) and YAML (`-I0`) output formats.

### Query Types and Execution Paths

| Query Type        | Example  | Execution Path | Output Format | Description                                     |
|-------------------|----------|----------------|---------------|-------------------------------------------------|
| **Identity**      | `.`      | P9 streaming   | JSON or YAML  | Full document streaming output                  |
| **First Element** | `.[0]`   | M2 streaming   | JSON or YAML  | Navigate to first array element                 |
| **Iteration**     | `.[]`    | M2 streaming   | JSON or YAML  | Iterate over array elements                     |
| **Length**        | `length` | OwnedValue     | JSON or YAML  | Produces computed value (not cursor-streamable) |

M2 avoids OwnedValue DOM construction for navigation queries, streaming directly from the YAML cursor to output.

**Note on `length`**: The `length` builtin uses efficient cursor iteration to count elements (O(n) traversal without value materialization). However, it produces a computed `OwnedValue::Int` result rather than a cursor to a document subtree, so it cannot use M2 streaming for output.

### Benchmark Results (Apple M1 Max, navigation pattern)

**10MB file:**

| Query       | Path          | succinctly | yq       | Speedup      | succ Mem | yq Mem   | Mem Ratio |
|-------------|---------------|------------|----------|--------------|----------|----------|-----------|
| `.`         | P9 streaming  | 186ms      | 1.16s    | **6.2x**     | 32 MB    | 698 MB   | **0.05x** |
| `.[0]`      | M2 streaming  | 50ms       | 597ms    | **12.1x**    | 32 MB    | 489 MB   | **0.07x** |
| `.[]`       | M2 streaming  | 260ms      | 1.33s    | **5.1x**     | 34 MB    | 738 MB   | **0.05x** |
| `length`    | OwnedValue    | 50ms       | 595ms    | **11.9x**    | 32 MB    | 488 MB   | **0.07x** |

**100MB file:**

| Query       | Path          | succinctly | yq       | Speedup      | succ Mem | yq Mem   | Mem Ratio |
|-------------|---------------|------------|----------|--------------|----------|----------|-----------|
| `.`         | P9 streaming  | 1.78s      | 11.54s   | **6.5x**     | 254 MB   | 7 GB     | **0.03x** |
| `.[0]`      | M2 streaming  | 444ms      | 5.84s    | **13.2x**    | 254 MB   | 5 GB     | **0.05x** |
| `.[]`       | M2 streaming  | 2.53s      | 13.30s   | **5.3x**     | 269 MB   | 8 GB     | **0.03x** |
| `length`    | OwnedValue    | 445ms      | 5.84s    | **13.1x**    | 254 MB   | 5 GB     | **0.05x** |

**Key insights**:
- **M2 streaming (`.[0]`) is 3.7-4.0x faster than identity (`.`)** on the same file - navigates to first element without streaming entire document
- **succinctly uses 3-7% of yq's memory** across all query types
- **All paths use similar base memory** because the YAML index dominates (~3x input size)
- Navigation queries benefit from M2's lazy evaluation - only accessed elements are materialized
- `length` and `.[0]` show similar performance because both only need to count/access the first level

### Running M2-Focused Benchmarks

```bash
# Benchmark all query types on the navigation pattern
./target/release/succinctly dev bench yq --patterns navigation --queries all

# Compare identity vs navigation queries
./target/release/succinctly dev bench yq --queries identity,first_element --sizes 10mb,100mb

# Memory-focused comparison
./target/release/succinctly dev bench yq --memory --queries all --sizes 100mb

# Available query types: identity, first_element, iteration, length
```

Note: The benchmark tool uses JSON output (`-o json`) for hash comparison. M2 YAML streaming uses the same code path with YAML serialization instead of JSON.

### When M2 Streaming Helps

M2 streaming provides the greatest benefit when:
1. **Extracting single elements**: `.[0]`, `.users[5]`
2. **Field access**: `.config`, `.users`
3. **Iteration**: `.[]`, `.users[]`
4. **Chained navigation**: `.[0].name`, `.users[].email`

### When OwnedValue is Required

Some operations require the full DOM (OwnedValue path):
- `length`, `keys`, `values` builtins
- Complex filters: `select(.age > 30)`
- Arithmetic on document values
- Any operation needing multiple passes

---

## Reproducing Benchmarks

```bash
# Build release binary
cargo build --release --features cli

# Generate benchmark files (includes navigation pattern for M2 testing)
cargo run --release --features cli -- yaml generate-suite

# Run CLI benchmark tool (recommended - includes memory measurement)
./target/release/succinctly dev bench yq

# Run with specific patterns/sizes
./target/release/succinctly dev bench yq --patterns comprehensive,nested --sizes 1mb,10mb

# Run navigation-focused benchmarks (M2 streaming)
./target/release/succinctly dev bench yq --patterns navigation --queries all --sizes 10mb,100mb

# Run memory-focused comparison
./target/release/succinctly dev bench yq --memory --queries identity,first_element,length

# Run Criterion benchmarks (wall time only)
cargo bench --bench yq_comparison

# Run selection/lazy evaluation benchmarks
cargo bench --bench yq_select

# Run BP select1 micro-benchmark
cargo bench --bench bp_select_micro

# Manual testing
./target/release/succinctly yq -o json -I 0 . input.yaml
time yq -o json -I 0 . input.yaml
```

### Benchmark Output Files

The CLI benchmark tool generates:
- `data/bench/results/yq-bench.jsonl` - Raw results in JSON Lines format
- `data/bench/results/yq-bench.md` - Formatted markdown tables
