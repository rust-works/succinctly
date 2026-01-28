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

### ARM (Neoverse-V2 / Graviton 4) - yq Identity Comparison (comprehensive pattern)

| Size       | succinctly              | yq                      | Speedup    | succ Mem | yq Mem  | Mem Ratio |
|------------|-------------------------|-------------------------|------------|----------|---------|-----------|
| **10KB**   |   2.0 ms  (5.0 MiB/s)  |   5.7 ms  (1.7 MiB/s)  | **2.9x**   |    4 MB  |   10 MB | **0.36x** |
| **100KB**  |   3.7 ms (25.1 MiB/s)  |  21.4 ms  (4.3 MiB/s)  | **5.8x**   |    4 MB  |   18 MB | **0.22x** |
| **1MB**    |  19.7 ms (46.7 MiB/s)  | 156.4 ms  (5.9 MiB/s)  | **7.9x**   |    6 MB  |   76 MB | **0.08x** |
| **10MB**   | 173.5 ms (53.8 MiB/s)  |   1.46 s  (6.4 MiB/s)  | **8.4x**   |   31 MB  |  668 MB | **0.05x** |
| **100MB**  |   1.66 s (55.8 MiB/s)  |  13.54 s  (6.9 MiB/s)  | **8.1x**   |  275 MB  |    6 GB | **0.04x** |

**Key metrics:**
- ✅ NEON SIMD optimizations active (block scalar + anchor parsing)
- ✅ SVE2 with SVEBITPERM support (BDEP/BEXT for DSV)
- ✅ **7.9x faster** than yq on 1MB files
- ✅ 56 MiB/s throughput vs yq's 7 MiB/s at 100MB
- ✅ Up to **9.9x faster** on nested structures (1MB)
- ✅ **23x less memory** on 100MB files (275 MB vs 6 GB)

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
| **10KB**   |    5.8 ms    |    9.0 ms    | **1.6x**      |    7 MB  |   14 MB | **0.50x** |
| **100KB**  |    7.3 ms    |   19.5 ms    | **2.7x**      |    8 MB  |   23 MB | **0.35x** |
| **1MB**    |   20.5 ms    |  111.2 ms    | **5.4x**      |   10 MB  |   82 MB | **0.12x** |
| **10MB**   |  161.6 ms    |    1.01 s    | **6.3x**      |   36 MB  |  740 MB | **0.05x** |
| **100MB**  |    1.50 s    |    9.68 s    | **6.4x**      |  297 MB  |    6 GB | **0.05x** |

**Key metrics:**
- ✅ **6.4x faster** than yq on 100MB files
- ✅ **20x less memory** on large files (100MB: 297 MB vs 6 GB)
- ✅ Speedup increases with file size (amortizes index construction)
- ✅ Memory efficiency improves dramatically at scale
- ✅ **Streaming JSON output** eliminates intermediate String allocations

### x86_64 (AMD Ryzen 9 7950X) - yq Identity Comparison (comprehensive pattern)

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.5 ms    |   58.8 ms    | **24x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.6 ms    |   59.9 ms    | **23x**       |    5 MB  |   23 MB | **0.21x** |
| **100KB**  |    4.0 ms    |   74.6 ms    | **19x**       |    5 MB  |   22 MB | **0.21x** |
| **1MB**    |   17.3 ms    |  195.3 ms    | **11x**       |    7 MB  |   85 MB | **0.09x** |
| **10MB**   |  143.7 ms    |    1.25 s    | **9x**        |   32 MB  |  647 MB | **0.05x** |
| **100MB**  |    1.44 s    |   12.09 s    | **8x**        |  276 MB  |    6 GB | **0.04x** |

### x86_64 (AMD Ryzen 9 7950X) - Key Achievements

**Key Achievements:**
- ✅ **Full yq CLI compatibility** - quoted strings preserved as strings
- ✅ **24x faster than yq** on small files (end-to-end CLI comparison)
- ✅ **11x faster than yq** on 1MB files
- ✅ **22x less memory** on 100MB files (276 MB vs 6 GB)
- ✅ **BMI2 PDEP** for O(1) select_in_word (7.6-16x faster select1)
- ✅ **Comprehensive test suite** - 32 tests including 8 direct byte-for-byte comparisons

**See also:** [docs/parsing/yaml.md](../parsing/yaml.md) for optimization details.

---

## Detailed Results by Pattern (ARM - Neoverse-V2 / Graviton 4)

### Pattern: comprehensive

Mixed YAML content with various features.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    4.1 ms    | **2.4x**      |    4 MB  |    9 MB | **0.39x** |
| **10KB**   |    2.0 ms    |    5.7 ms    | **2.9x**      |    4 MB  |   10 MB | **0.36x** |
| **100KB**  |    3.7 ms    |   21.4 ms    | **5.8x**      |    4 MB  |   18 MB | **0.22x** |
| **1MB**    |   19.7 ms    |  156.4 ms    | **7.9x**      |    6 MB  |   76 MB | **0.08x** |
| **10MB**   |  173.5 ms    |    1.46 s    | **8.4x**      |   31 MB  |  668 MB | **0.05x** |
| **100MB**  |    1.66 s    |   13.54 s    | **8.1x**      |  275 MB  |    6 GB | **0.04x** |

### Pattern: nested

Deeply nested mapping structures. **Best speedup pattern** due to efficient BP tree navigation.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.8 ms    |    4.1 ms    | **2.3x**      |    4 MB  |    9 MB | **0.40x** |
| **10KB**   |    1.9 ms    |    5.3 ms    | **2.8x**      |    4 MB  |   10 MB | **0.37x** |
| **100KB**  |    2.6 ms    |   14.7 ms    | **5.7x**      |    4 MB  |   15 MB | **0.25x** |
| **1MB**    |   11.8 ms    |  116.1 ms    | **9.9x**      |    5 MB  |   55 MB | **0.09x** |
| **10MB**   |  127.3 ms    |    1.06 s    | **8.3x**      |   19 MB  |  419 MB | **0.05x** |
| **100MB**  |    1.02 s    |    8.44 s    | **8.3x**      |  135 MB  |    4 GB | **0.04x** |

### Pattern: pathological

Worst-case structural density. Tests parser robustness.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    3.8 ms    | **2.3x**      |    4 MB  |    9 MB | **0.40x** |
| **10KB**   |    2.0 ms    |    5.9 ms    | **3.0x**      |    4 MB  |   10 MB | **0.36x** |
| **100KB**  |    3.8 ms    |   25.4 ms    | **6.7x**      |    4 MB  |   19 MB | **0.21x** |
| **1MB**    |   20.6 ms    |  200.1 ms    | **9.7x**      |    6 MB  |   89 MB | **0.07x** |
| **10MB**   |  225.7 ms    |    1.77 s    | **7.9x**      |   30 MB  |  723 MB | **0.04x** |
| **100MB**  |    2.09 s    |   16.55 s    | **7.9x**      |  264 MB  |    7 GB | **0.04x** |

### Pattern: users

Realistic user record arrays (common in config files).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.8 ms    |    4.1 ms    | **2.3x**      |    4 MB  |    9 MB | **0.40x** |
| **10KB**   |    1.9 ms    |    5.6 ms    | **2.9x**      |    4 MB  |   10 MB | **0.36x** |
| **100KB**  |    3.8 ms    |   24.0 ms    | **6.3x**      |    4 MB  |   19 MB | **0.21x** |
| **1MB**    |   22.1 ms    |  195.8 ms    | **8.9x**      |    7 MB  |  106 MB | **0.06x** |
| **10MB**   |  264.1 ms    |    1.82 s    | **6.9x**      |   33 MB  |  935 MB | **0.04x** |
| **100MB**  |    2.62 s    |   17.71 s    | **6.8x**      |  304 MB  |    9 GB | **0.03x** |

### Pattern: sequences

Sequence-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.8 ms    |    4.1 ms    | **2.3x**      |    4 MB  |    9 MB | **0.40x** |
| **10KB**   |    2.0 ms    |    5.7 ms    | **2.9x**      |    4 MB  |   10 MB | **0.36x** |
| **100KB**  |    4.0 ms    |   23.3 ms    | **5.8x**      |    4 MB  |   18 MB | **0.22x** |
| **1MB**    |   23.6 ms    |  187.3 ms    | **7.9x**      |    7 MB  |  106 MB | **0.07x** |
| **10MB**   |  241.3 ms    |    1.68 s    | **7.0x**      |   35 MB  |  815 MB | **0.04x** |
| **100MB**  |    2.30 s    |   15.78 s    | **6.9x**      |  311 MB  |    8 GB | **0.04x** |

### Pattern: strings

String-heavy YAML with quoted variants.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    3.9 ms    | **2.3x**      |    4 MB  |    9 MB | **0.40x** |
| **10KB**   |    1.8 ms    |    4.8 ms    | **2.6x**      |    4 MB  |   10 MB | **0.38x** |
| **100KB**  |    3.2 ms    |   14.9 ms    | **4.7x**      |    4 MB  |   15 MB | **0.26x** |
| **1MB**    |   15.6 ms    |  104.2 ms    | **6.7x**      |    6 MB  |   57 MB | **0.10x** |
| **10MB**   |  165.5 ms    |  953.7 ms    | **5.8x**      |   25 MB  |  483 MB | **0.05x** |
| **100MB**  |    1.61 s    |    9.01 s    | **5.6x**      |  222 MB  |    4 GB | **0.05x** |

### Pattern: numbers

Numeric-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    3.9 ms    | **2.3x**      |    4 MB  |    9 MB | **0.40x** |
| **10KB**   |    2.0 ms    |    5.5 ms    | **2.8x**      |    4 MB  |   10 MB | **0.37x** |
| **100KB**  |    3.8 ms    |   21.5 ms    | **5.7x**      |    4 MB  |   17 MB | **0.23x** |
| **1MB**    |   20.8 ms    |  162.1 ms    | **7.8x**      |    6 MB  |   79 MB | **0.08x** |
| **10MB**   |  222.2 ms    |    1.42 s    | **6.4x**      |   28 MB  |  630 MB | **0.04x** |
| **100MB**  |    2.10 s    |   12.96 s    | **6.2x**      |  247 MB  |    6 GB | **0.04x** |

### Pattern: unicode

Unicode-heavy strings in various scripts.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    4.0 ms    | **2.3x**      |    4 MB  |    9 MB | **0.40x** |
| **10KB**   |    1.8 ms    |    4.7 ms    | **2.6x**      |    4 MB  |   10 MB | **0.38x** |
| **100KB**  |    3.1 ms    |   14.4 ms    | **4.6x**      |    4 MB  |   15 MB | **0.26x** |
| **1MB**    |   15.1 ms    |   98.5 ms    | **6.5x**      |    6 MB  |   55 MB | **0.11x** |
| **10MB**   |  149.9 ms    |  879.2 ms    | **5.9x**      |   25 MB  |  461 MB | **0.05x** |
| **100MB**  |    1.45 s    |    8.23 s    | **5.7x**      |  222 MB  |    4 GB | **0.05x** |

### Pattern: mixed

Mixed mappings and sequences.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.8 ms    |    4.1 ms    | **2.3x**      |    4 MB  |    9 MB | **0.39x** |
| **10KB**   |    1.9 ms    |    5.5 ms    | **2.9x**      |    4 MB  |   10 MB | **0.37x** |
| **100KB**  |    3.6 ms    |   21.3 ms    | **5.9x**      |    4 MB  |   17 MB | **0.23x** |
| **1MB**    |   19.2 ms    |  156.0 ms    | **8.1x**      |    6 MB  |   77 MB | **0.08x** |
| **10MB**   |  204.2 ms    |    1.40 s    | **6.8x**      |   28 MB  |  696 MB | **0.04x** |
| **100MB**  |    1.92 s    |   12.75 s    | **6.6x**      |  244 MB  |    6 GB | **0.04x** |

---

## Detailed Results by Pattern (ARM - Apple M1 Max)

### Pattern: comprehensive

Mixed YAML content with various features.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.6 ms    |    7.3 ms    | **1.3x**      |    7 MB  |   14 MB | **0.54x** |
| **10KB**   |    5.8 ms    |    9.0 ms    | **1.6x**      |    7 MB  |   14 MB | **0.50x** |
| **100KB**  |    7.3 ms    |   19.5 ms    | **2.7x**      |    8 MB  |   23 MB | **0.35x** |
| **1MB**    |   20.5 ms    |  111.2 ms    | **5.4x**      |   10 MB  |   82 MB | **0.12x** |
| **10MB**   |  161.6 ms    |    1.01 s    | **6.3x**      |   36 MB  |  740 MB | **0.05x** |
| **100MB**  |    1.50 s    |    9.68 s    | **6.4x**      |  297 MB  |    6 GB | **0.05x** |

### Pattern: nested

Deeply nested mapping structures. **Best speedup pattern** due to efficient BP tree navigation.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    4.6 ms    |    7.2 ms    | **1.5x**      |    7 MB  |   14 MB | **0.53x** |
| **10KB**   |    4.6 ms    |    8.1 ms    | **1.8x**      |    7 MB  |   14 MB | **0.51x** |
| **100KB**  |    5.4 ms    |   14.6 ms    | **2.7x**      |    8 MB  |   20 MB | **0.38x** |
| **1MB**    |   14.6 ms    |   87.8 ms    | **6.0x**      |    9 MB  |   61 MB | **0.15x** |
| **10MB**   |   87.4 ms    |  773.9 ms    | **8.9x**      |   24 MB  |  428 MB | **0.06x** |
| **100MB**  |  681.2 ms    |    6.29 s    | **9.2x**      |  156 MB  |    4 GB | **0.04x** |

### Pattern: pathological

Worst-case structural density. Tests parser robustness.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    4.8 ms    |    7.1 ms    | **1.5x**      |    7 MB  |   14 MB | **0.53x** |
| **10KB**   |    5.1 ms    |    8.4 ms    | **1.6x**      |    7 MB  |   14 MB | **0.50x** |
| **100KB**  |    7.0 ms    |   23.5 ms    | **3.4x**      |    8 MB  |   24 MB | **0.32x** |
| **1MB**    |   23.0 ms    |  144.2 ms    | **6.3x**      |   11 MB  |   91 MB | **0.12x** |
| **10MB**   |  156.0 ms    |    1.29 s    | **8.3x**      |   37 MB  |  755 MB | **0.05x** |
| **100MB**  |    1.41 s    |   12.30 s    | **8.7x**      |  286 MB  |    7 GB | **0.04x** |

### Pattern: users

Realistic user record arrays (common in config files).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    4.8 ms    |    7.0 ms    | **1.5x**      |    7 MB  |   13 MB | **0.54x** |
| **10KB**   |    4.7 ms    |    8.2 ms    | **1.7x**      |    7 MB  |   15 MB | **0.50x** |
| **100KB**  |    6.7 ms    |   20.5 ms    | **3.1x**      |    8 MB  |   24 MB | **0.33x** |
| **1MB**    |   23.5 ms    |  138.2 ms    | **5.9x**      |   11 MB  |   95 MB | **0.11x** |
| **10MB**   |  196.0 ms    |    1.27 s    | **6.5x**      |   41 MB  |  878 MB | **0.05x** |
| **100MB**  |    1.88 s    |   12.80 s    | **6.8x**      |  352 MB  |    9 GB | **0.04x** |

### Pattern: sequences

Sequence-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    4.5 ms    |    7.1 ms    | **1.6x**      |    7 MB  |   14 MB | **0.53x** |
| **10KB**   |    5.3 ms    |    8.0 ms    | **1.5x**      |    7 MB  |   14 MB | **0.51x** |
| **100KB**  |    7.2 ms    |   21.7 ms    | **3.0x**      |    8 MB  |   24 MB | **0.33x** |
| **1MB**    |   26.2 ms    |  130.4 ms    | **5.0x**      |   11 MB  |   95 MB | **0.12x** |
| **10MB**   |  194.5 ms    |    1.18 s    | **6.0x**      |   55 MB  |  972 MB | **0.06x** |
| **100MB**  |    1.82 s    |   11.44 s    | **6.3x**      |  486 MB  |    8 GB | **0.06x** |

### Pattern: strings

String-heavy YAML with quoted variants.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    4.5 ms    |    7.0 ms    | **1.5x**      |    7 MB  |   13 MB | **0.54x** |
| **10KB**   |    4.9 ms    |    7.5 ms    | **1.5x**      |    7 MB  |   14 MB | **0.52x** |
| **100KB**  |    6.6 ms    |   15.4 ms    | **2.3x**      |    8 MB  |   20 MB | **0.39x** |
| **1MB**    |   18.6 ms    |   78.8 ms    | **4.2x**      |   10 MB  |   63 MB | **0.16x** |
| **10MB**   |  118.0 ms    |  667.8 ms    | **5.7x**      |   31 MB  |  459 MB | **0.07x** |
| **100MB**  |    1.13 s    |    6.38 s    | **5.6x**      |  266 MB  |    5 GB | **0.06x** |

### Pattern: numbers

Numeric-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    4.5 ms    |    6.9 ms    | **1.5x**      |    7 MB  |   13 MB | **0.54x** |
| **10KB**   |    4.7 ms    |    8.0 ms    | **1.7x**      |    7 MB  |   14 MB | **0.50x** |
| **100KB**  |    6.5 ms    |   18.8 ms    | **2.9x**      |    8 MB  |   22 MB | **0.35x** |
| **1MB**    |   22.6 ms    |  117.1 ms    | **5.2x**      |   10 MB  |   81 MB | **0.13x** |
| **10MB**   |  164.9 ms    |  975.5 ms    | **5.9x**      |   35 MB  |  602 MB | **0.06x** |
| **100MB**  |    1.52 s    |    9.36 s    | **6.2x**      |  301 MB  |    6 GB | **0.05x** |

### Pattern: unicode

Unicode-heavy strings in various scripts.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    4.7 ms    |    7.2 ms    | **1.5x**      |    7 MB  |   13 MB | **0.55x** |
| **10KB**   |    4.9 ms    |    7.4 ms    | **1.5x**      |    7 MB  |   14 MB | **0.53x** |
| **100KB**  |    5.7 ms    |   13.9 ms    | **2.4x**      |    8 MB  |   20 MB | **0.39x** |
| **1MB**    |   16.6 ms    |   73.0 ms    | **4.4x**      |   10 MB  |   60 MB | **0.16x** |
| **10MB**   |  115.8 ms    |  637.6 ms    | **5.5x**      |   31 MB  |  457 MB | **0.07x** |
| **100MB**  |    1.08 s    |    6.26 s    | **5.8x**      |  243 MB  |    4 GB | **0.06x** |

### Pattern: mixed

Mixed mappings and sequences.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.4 ms    |    9.2 ms    | **1.7x**      |    7 MB  |   14 MB | **0.53x** |
| **10KB**   |    5.6 ms    |   10.1 ms    | **1.8x**      |    7 MB  |   14 MB | **0.51x** |
| **100KB**  |    7.3 ms    |   20.7 ms    | **2.8x**      |    8 MB  |   24 MB | **0.32x** |
| **1MB**    |   20.9 ms    |  113.0 ms    | **5.4x**      |   10 MB  |   86 MB | **0.12x** |
| **10MB**   |  151.1 ms    |  981.3 ms    | **6.5x**      |   36 MB  |  646 MB | **0.06x** |
| **100MB**  |    1.40 s    |    9.19 s    | **6.6x**      |  285 MB  |    6 GB | **0.04x** |

---

## Detailed Results by Pattern (x86_64 - AMD Ryzen 9 7950X)

### Pattern: comprehensive

Mixed YAML content with various features.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.5 ms    |   58.8 ms    | **24x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.6 ms    |   59.9 ms    | **23x**       |    5 MB  |   23 MB | **0.21x** |
| **100KB**  |    4.0 ms    |   74.6 ms    | **19x**       |    5 MB  |   22 MB | **0.21x** |
| **1MB**    |   17.3 ms    |  195.3 ms    | **11x**       |    7 MB  |   85 MB | **0.09x** |
| **10MB**   |  143.7 ms    |    1.25 s    | **9x**        |   32 MB  |  647 MB | **0.05x** |
| **100MB**  |    1.44 s    |   12.09 s    | **8x**        |  276 MB  |    6 GB | **0.04x** |

### Pattern: config

Small configuration files (constant size across all benchmarks).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.5 ms    |   59.1 ms    | **24x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.5 ms    |   60.2 ms    | **24x**       |    5 MB  |   21 MB | **0.22x** |
| **100KB**  |    2.4 ms    |   59.7 ms    | **25x**       |    5 MB  |   22 MB | **0.21x** |
| **1MB**    |    2.5 ms    |   69.3 ms    | **27x**       |    5 MB  |   23 MB | **0.21x** |
| **10MB**   |    2.5 ms    |   60.0 ms    | **24x**       |    4 MB  |   22 MB | **0.20x** |
| **100MB**  |    2.3 ms    |   61.7 ms    | **27x**       |    5 MB  |   23 MB | **0.21x** |

### Pattern: mixed

Mixed mappings and sequences.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.6 ms    |   60.2 ms    | **23x**       |    4 MB  |   22 MB | **0.20x** |
| **10KB**   |    2.7 ms    |   62.1 ms    | **23x**       |    5 MB  |   22 MB | **0.22x** |
| **100KB**  |    4.1 ms    |   75.4 ms    | **18x**       |    5 MB  |   22 MB | **0.22x** |
| **1MB**    |   16.5 ms    |  200.6 ms    | **12x**       |    7 MB  |   89 MB | **0.08x** |
| **10MB**   |  137.2 ms    |    1.24 s    | **9x**        |   32 MB  |  677 MB | **0.05x** |
| **100MB**  |    1.28 s    |   11.73 s    | **9x**        |  266 MB  |    6 GB | **0.04x** |

### Pattern: navigation

Navigation-heavy YAML content (for M2 streaming benchmarks).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.5 ms    |   58.7 ms    | **24x**       |    5 MB  |   23 MB | **0.21x** |
| **10KB**   |    2.6 ms    |   61.2 ms    | **23x**       |    5 MB  |   22 MB | **0.22x** |
| **100KB**  |    4.0 ms    |   74.5 ms    | **19x**       |    5 MB  |   22 MB | **0.23x** |
| **1MB**    |   18.8 ms    |  209.1 ms    | **11x**       |    8 MB  |   92 MB | **0.08x** |
| **10MB**   |  165.3 ms    |    1.51 s    | **9x**        |   35 MB  |  836 MB | **0.04x** |
| **100MB**  |    1.64 s    |   14.76 s    | **9x**        |  312 MB  |    8 GB | **0.04x** |

### Pattern: nested

Deeply nested mapping structures. **Best speedup pattern** due to efficient BP tree navigation.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.4 ms    |   60.6 ms    | **25x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.4 ms    |   60.3 ms    | **25x**       |    5 MB  |   22 MB | **0.21x** |
| **100KB**  |    3.0 ms    |   69.2 ms    | **23x**       |    5 MB  |   22 MB | **0.23x** |
| **1MB**    |   10.7 ms    |  155.5 ms    | **14x**       |    6 MB  |   55 MB | **0.11x** |
| **10MB**   |   82.7 ms    |  918.2 ms    | **11x**       |   21 MB  |  423 MB | **0.05x** |
| **100MB**  |  653.9 ms    |    7.69 s    | **12x**       |  142 MB  |    4 GB | **0.04x** |

### Pattern: numbers

Numeric-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.5 ms    |   61.3 ms    | **25x**       |    5 MB  |   22 MB | **0.22x** |
| **10KB**   |    2.7 ms    |   61.3 ms    | **23x**       |    4 MB  |   22 MB | **0.20x** |
| **100KB**  |    4.2 ms    |   75.0 ms    | **18x**       |    5 MB  |   22 MB | **0.22x** |
| **1MB**    |   18.1 ms    |  189.4 ms    | **10x**       |    7 MB  |   70 MB | **0.10x** |
| **10MB**   |  150.1 ms    |    1.23 s    | **8x**        |   31 MB  |  572 MB | **0.05x** |
| **100MB**  |    1.39 s    |   11.28 s    | **8x**        |  260 MB  |    5 GB | **0.05x** |

### Pattern: pathological

Worst-case structural density. Tests parser robustness.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.5 ms    |   59.7 ms    | **24x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.5 ms    |   62.0 ms    | **25x**       |    5 MB  |   22 MB | **0.22x** |
| **100KB**  |    4.2 ms    |   78.6 ms    | **19x**       |    5 MB  |   22 MB | **0.22x** |
| **1MB**    |   18.0 ms    |  228.4 ms    | **13x**       |    8 MB  |   94 MB | **0.08x** |
| **10MB**   |  143.8 ms    |    1.54 s    | **11x**       |   33 MB  |  741 MB | **0.04x** |
| **100MB**  |    1.30 s    |   15.08 s    | **12x**       |  278 MB  |    7 GB | **0.04x** |

### Pattern: sequences

Sequence-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.4 ms    |   58.6 ms    | **25x**       |    5 MB  |   22 MB | **0.22x** |
| **10KB**   |    2.5 ms    |   60.8 ms    | **24x**       |    5 MB  |   22 MB | **0.21x** |
| **100KB**  |    4.2 ms    |   76.4 ms    | **18x**       |    5 MB  |   23 MB | **0.23x** |
| **1MB**    |   20.4 ms    |  218.1 ms    | **11x**       |    8 MB  |   97 MB | **0.09x** |
| **10MB**   |  171.2 ms    |    1.52 s    | **9x**        |   40 MB  |  893 MB | **0.05x** |
| **100MB**  |    1.61 s    |   15.06 s    | **9x**        |  354 MB  |    9 GB | **0.04x** |

### Pattern: strings

String-heavy YAML with quoted variants.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.4 ms    |   64.0 ms    | **26x**       |    5 MB  |   23 MB | **0.21x** |
| **10KB**   |    2.7 ms    |   60.2 ms    | **23x**       |    5 MB  |   23 MB | **0.21x** |
| **100KB**  |    3.8 ms    |   69.7 ms    | **19x**       |    5 MB  |   22 MB | **0.22x** |
| **1MB**    |   14.4 ms    |  145.9 ms    | **10x**       |    7 MB  |   55 MB | **0.13x** |
| **10MB**   |  117.3 ms    |  823.0 ms    | **7x**        |   28 MB  |  423 MB | **0.07x** |
| **100MB**  |    1.12 s    |    7.94 s    | **7x**        |  235 MB  |    4 GB | **0.05x** |

### Pattern: unicode

Unicode-heavy strings in various scripts.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.3 ms    |   59.5 ms    | **25x**       |    5 MB  |   22 MB | **0.22x** |
| **10KB**   |    2.5 ms    |   60.0 ms    | **24x**       |    4 MB  |   22 MB | **0.20x** |
| **100KB**  |    3.7 ms    |   68.6 ms    | **19x**       |    5 MB  |   22 MB | **0.22x** |
| **1MB**    |   13.5 ms    |  142.8 ms    | **11x**       |    7 MB  |   54 MB | **0.13x** |
| **10MB**   |  111.9 ms    |  840.3 ms    | **8x**        |   28 MB  |  464 MB | **0.06x** |
| **100MB**  |    1.01 s    |    7.95 s    | **8x**        |  235 MB  |    5 GB | **0.05x** |

### Pattern: users

Realistic user record arrays (common in config files).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.3 ms    |   60.9 ms    | **26x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.4 ms    |   60.3 ms    | **25x**       |    5 MB  |   23 MB | **0.21x** |
| **100KB**  |    4.1 ms    |   75.5 ms    | **18x**       |    5 MB  |   23 MB | **0.22x** |
| **1MB**    |   19.4 ms    |  223.4 ms    | **12x**       |    8 MB  |   95 MB | **0.08x** |
| **10MB**   |  168.5 ms    |    1.59 s    | **9x**        |   36 MB  |  838 MB | **0.04x** |
| **100MB**  |    1.63 s    |   16.16 s    | **10x**       |  323 MB  |    9 GB | **0.04x** |

---

## Pattern Descriptions

| Pattern           | Description                                     | Best For               |
|-------------------|-------------------------------------------------|------------------------|
| **comprehensive** | Mixed content with various YAML features        | General benchmarking   |
| **config**        | Small configuration files (constant size)       | Startup overhead       |
| **mixed**         | Mixed mappings and sequences                    | Balanced workloads     |
| **navigation**    | Navigation-heavy content (M2 streaming)         | Cursor navigation      |
| **nested**        | Deeply nested mappings (tests tree navigation)  | BP tree performance    |
| **numbers**       | Numeric-heavy content                           | Number parsing         |
| **pathological**  | Maximum structural density                      | Parser stress testing  |
| **sequences**     | Sequence-heavy content (arrays)                 | Array-heavy workloads  |
| **strings**       | String-heavy with quoted variants               | String parsing         |
| **unicode**       | Unicode strings in various scripts              | UTF-8 handling         |
| **users**         | Realistic user record objects (config-like)     | Real-world configs     |

---

## Key Findings

### Speed

**ARM Neoverse-V2 (Graviton 4):**
- **2.3-9.9x faster** across all patterns and sizes
- **Best performance on nested structures**: 9.9x speedup on 1MB deeply nested files
- **Pathological patterns**: 9.7x speedup on worst-case structural density
- **Larger files benefit more**: Speedup increases with file size (amortizes index construction)
- **100MB files**: 5.6-8.3x faster depending on pattern

**Apple M1 Max (ARM):**
- **1.3-9.2x faster** across all patterns and sizes
- **Best performance on nested structures**: 9.2x speedup on 100MB deeply nested files
- **Pathological patterns**: 8.7x speedup on worst-case structural density
- **Larger files benefit more**: Speedup increases with file size (amortizes index construction)
- **100MB files**: 5.6-9.2x faster depending on pattern

**AMD Ryzen 9 7950X (x86_64):**
- **7-27x faster** across all patterns and sizes
- **Best performance on nested structures**: 14x speedup on 1MB, 12x on 100MB
- **Small files dominate by startup**: 23-27x faster on 1-10KB (yq startup ~59ms)
- **100MB files**: 7-12x faster depending on pattern
- **BMI2 PDEP**: O(1) select_in_word gives 7.6-16x faster BP select1 queries

### Memory

**ARM Neoverse-V2 (Graviton 4):**
- **25x less memory** on 100MB files (135-311 MB vs 4-9 GB)
- **Consistent ~4 MB baseline** for small files
- **Linear scaling**: Memory grows proportionally with file size

| Size       | succinctly    | yq          | Ratio        |
|------------|---------------|-------------|--------------|
| **1KB**    | 4 MB          | 9 MB        | ~0.39x       |
| **10KB**   | 4 MB          | 10 MB       | ~0.36x       |
| **100KB**  | 4 MB          | 15-19 MB    | ~0.22x       |
| **1MB**    | 5-7 MB        | 56-108 MB   | ~0.08x       |
| **10MB**   | 19-35 MB      | 419-935 MB  | ~0.04x       |
| **100MB**  | 135-311 MB    | 4-9 GB      | ~0.04x       |

**Apple M1 Max:**
- **17-26x less memory** on large files (100MB) with streaming output
- **Consistent ~7 MB baseline** for small files regardless of pattern
- **Linear scaling**: Memory grows proportionally with file size
- **yq memory explosion**: yq uses 4-9 GB for 100MB files vs 156-486 MB for succinctly

| Size       | succinctly    | yq          | Ratio        |
|------------|---------------|-------------|--------------|
| **1KB**    | 7 MB          | 13-14 MB    | ~0.5x        |
| **10KB**   | 7 MB          | 14-15 MB    | ~0.5x        |
| **100KB**  | 8 MB          | 20-24 MB    | ~0.35x       |
| **1MB**    | 9-11 MB       | 60-95 MB    | ~0.12x       |
| **10MB**   | 24-55 MB      | 428-972 MB  | ~0.05x       |
| **100MB**  | 156-486 MB    | 4-9 GB      | ~0.05x       |

**AMD Ryzen 9 7950X:**
- **20-25x less memory** on 100MB files (142-354 MB vs 4-9 GB)
- **Consistent ~4-5 MB baseline** for small files
- Lower baseline than ARM due to smaller yq Go runtime on x86_64

| Size       | succinctly    | yq          | Ratio        |
|------------|---------------|-------------|--------------|
| **1KB**    | 4-5 MB        | 22-23 MB    | ~0.21x       |
| **10KB**   | 4-5 MB        | 22-23 MB    | ~0.21x       |
| **100KB**  | 5 MB          | 22-23 MB    | ~0.22x       |
| **1MB**    | 6-8 MB        | 54-97 MB    | ~0.10x       |
| **10MB**   | 21-40 MB      | 423-893 MB  | ~0.05x       |
| **100MB**  | 142-354 MB    | 4-9 GB      | ~0.04x       |

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
- 10KB files: **23x faster** than yq (2.6ms vs 59.9ms)
- 100KB files: **19x faster** than yq (4.0ms vs 74.6ms)
- 1MB files: **11x faster** than yq (17.3ms vs 195.3ms)

**Optimizations (cumulative):**
- **P0**: Multi-character classification infrastructure (8 types in parallel)
- **P0+**: Hybrid scalar/SIMD space skipping (fast path for 0-8 spaces, SIMD for longer runs)
- **P2**: Conditional SIMD for unquoted value/key scanning (32-byte chunks via `classify_yaml_chars`)
- **P2.7**: Block scalar SIMD - AVX2 newline scanning + indentation checking (19-25% improvement)
- **P4**: Anchor/Alias SIMD - AVX2 scans for anchor name terminators (6-17% improvement on anchor-heavy workloads)
- **P9**: Direct YAML-to-JSON streaming - eliminated intermediate DOM for identity queries (8-22% improvement, 2.3x on yq benchmarks)
- **P11**: BP Select1 for yq-locate - zero-cost generic select support for O(1) offset-to-BP lookups (2.5-5.9x faster select1, fixes issue #26)
- **P12**: Advance Index for bp_to_text - memory-efficient bitmap encoding (~1.5× measured, 20-25% faster yq identity queries on 1MB files)
- **P12-A**: Build regression mitigation (A1: inline zero-fill, A2: combined monotonicity check, A4: lazy newline index) - 11-85% faster `yaml_bench` build times ([issue #72](https://github.com/rust-works/succinctly/issues/72))
- **O1**: Sequential cursor for AdvancePositions - `Cell<SequentialCursor>` with three-path dispatch for amortized O(1) sequential access (3-13% faster yq identity queries at 1-100KB, [issue #74](https://github.com/rust-works/succinctly/issues/74))
- **O2**: Gap-skipping via advance_rank1 - replaced O(G) linear loop in `advance_cursor_to()` with O(1) rank lookup (2-6% faster on nested/users at small-medium sizes, [issue #74](https://github.com/rust-works/succinctly/issues/74))
- **BMI2 PDEP select_in_word**: O(1) select_in_word on x86_64 using BMI2 PDEP instruction (7.6-16x faster BP select1 queries, with AMD Zen 1/2 slow-path detection)
- **M2**: Navigation streaming - cursor-based streaming for navigation queries, supports both JSON and YAML output

**Rejected Optimizations:**
- **P1 (YFSM)**: Table-driven state machine for string parsing tested but showed only 0-2% improvement vs expected 15-25%. YAML strings are too simple compared to JSON (where PFSM succeeded with 33-77% gains). P0+ SIMD already optimal. See [docs/parsing/yaml.md](../parsing/yaml.md) for full analysis.
- **P2.6, P2.8, P3, P5, P6, P7, P8**: Various optimizations rejected due to micro-benchmark/real-world mismatches, grammar incompatibilities, or memory bottlenecks. See [docs/parsing/yaml.md](../parsing/yaml.md) for detailed analyses.

### Trade-offs

- **Small files (<1KB)**: Process startup dominates; speedup is modest (~1.5x)
- **Large files (1MB+)**: Index construction amortizes; best speedups (6-25x)
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

### Results (ARM Neoverse-V2 / Graviton 4)

Criterion benchmarks measuring library-level succinctly execution vs system yq subprocess.

**5% Slice Selection** (`users` pattern, `.users[:N]`):

| Size      | succinctly   | yq           | Speedup       |
|-----------|--------------|--------------|---------------|
| **10KB**  |    1.36 ms   |    3.93 ms   | **2.9x**      |
| **100KB** |    5.44 ms   |   14.04 ms   | **2.6x**      |
| **1MB**   |   45.87 ms   |  104.4 ms    | **2.3x**      |

**Single Field Extraction** (`users` pattern, `.users[N].name`):

| Position          | Size      | succinctly   | yq           | Speedup       |
|-------------------|-----------|--------------|--------------|---------------|
| **first (0%)**    | **10KB**  |    0.93 ms   |    3.56 ms   | **3.8x**      |
| **first (0%)**    | **100KB** |    1.52 ms   |   13.17 ms   | **8.7x**      |
| **first (0%)**    | **1MB**   |    7.44 ms   |   97.98 ms   | **13.2x**     |
| **mid (50%)**     | **10KB**  |    0.93 ms   |    3.67 ms   | **3.9x**      |
| **mid (50%)**     | **100KB** |    1.54 ms   |   13.24 ms   | **8.6x**      |
| **mid (50%)**     | **1MB**   |    7.66 ms   |   98.38 ms   | **12.8x**     |
| **end (93%)**     | **10KB**  |    0.94 ms   |    3.69 ms   | **3.9x**      |
| **end (93%)**     | **100KB** |    1.56 ms   |   13.26 ms   | **8.5x**      |
| **end (93%)**     | **1MB**   |    7.73 ms   |   98.16 ms   | **12.7x**     |

**Key findings:**
- **Field access position-independent**: Accessing first, middle, or end element takes nearly identical time for succinctly (O(1) BP navigation)
- **yq must parse entire document**: yq time is constant regardless of which element is accessed
- **Speedup increases with file size**: 3.8x at 10KB to 13.2x at 1MB for field access
- **5% slice vs identity**: Selection is ~2.3x slower than identity at 1MB (expected: materializing 5% of content still requires full index construction)

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
- **9.2x faster** on 100MB nested files
- **6.4x faster** on 100MB comprehensive files
- **5.4x faster** on 1MB files

**Speed (AMD Ryzen 9 7950X):**
- **19x faster** on 100KB files
- **11x faster** on 1MB files
- **8x faster** on 100MB files

**Memory (Apple M1 Max, 100MB files):**
- **17-26x less memory** (156-486 MB vs 4-9 GB)
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
