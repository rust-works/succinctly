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

**CPU**: ARM Neoverse-V1 (8 cores)
**OS**: Linux 6.14.0-1018-aws
**yq version**: v4.48.1 (https://github.com/mikefarah/yq/)
**succinctly**: Built with `--release --features cli`
**SIMD**: NEON (128-bit), SVE (256-bit vectors)
**Date**: 2026-01-29

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
# Build with benchmark runner feature
cargo build --release --features bench-runner

# CLI benchmark tool (recommended - includes memory measurement)
./target/release/succinctly bench run yq_bench

# Criterion benchmark (wall time only)
cargo bench --bench yq_comparison
```

---

## Summary Results

### ARM (Neoverse-V2 / Graviton 4) - yq Identity Comparison (comprehensive pattern)

| Size       | succinctly             | yq                     | Speedup     | succ Mem | yq Mem  | Mem Ratio |
|------------|------------------------|------------------------|-------------|----------|---------|-----------|
| **1KB**    |   1.6 ms  (0.8 MiB/s)  |   3.6 ms  (0.3 MiB/s)  | **2.2x**    |    4 MB  |    9 MB | **0.42x** |
| **10KB**   |   1.8 ms  (5.4 MiB/s)  |   5.1 ms  (1.9 MiB/s)  | **2.9x**    |    4 MB  |   10 MB | **0.38x** |
| **100KB**  |   3.1 ms (29.7 MiB/s)  |  20.0 ms  (4.6 MiB/s)  | **6.5x**    |    4 MB  |   18 MB | **0.24x** |
| **1MB**    |  14.7 ms (62.6 MiB/s)  | 152.2 ms  (6.1 MiB/s)  | **10.3x**   |    6 MB  |   74 MB | **0.08x** |
| **10MB**   | 127.4 ms (73.4 MiB/s)  |   1.41 s  (6.6 MiB/s)  | **11.1x**   |   26 MB  |  703 MB | **0.04x** |
| **100MB**  |   1.21 s (76.5 MiB/s)  |  13.33 s  (7.0 MiB/s)  | **11.0x**   |  225 MB  |    7 GB | **0.03x** |

**Key metrics:**
- ✅ NEON SIMD optimizations active (block scalar + anchor parsing)
- ✅ SVE2 with SVEBITPERM support (BDEP/BEXT for DSV)
- ✅ **10.3x faster** than yq on 1MB files
- ✅ 77 MiB/s throughput vs yq's 7 MiB/s at 100MB
- ✅ Up to **15.4x faster** on nested structures (100MB)
- ✅ **31x less memory** on 100MB files (225 MB vs 7 GB)

### ARM (Neoverse-V1 / Graviton 3) - yq Identity Comparison (comprehensive pattern)

| Size       | succinctly             | yq                     | Speedup     | succ Mem | yq Mem  | Mem Ratio |
|------------|------------------------|------------------------|-------------|----------|---------|-----------|
| **1KB**    |   1.8 ms               |   4.0 ms               | **2.2x**    |    4 MB  |    9 MB | **0.42x** |
| **10KB**   |   1.9 ms               |   5.6 ms               | **2.9x**    |    4 MB  |   10 MB | **0.39x** |
| **100KB**  |   3.2 ms               |  21.0 ms               | **6.5x**    |    4 MB  |   17 MB | **0.24x** |
| **1MB**    |  15.2 ms               | 152.9 ms               | **10.1x**   |    6 MB  |   73 MB | **0.08x** |
| **10MB**   | 128.6 ms               |   1.42 s               | **11.1x**   |   26 MB  |  743 MB | **0.04x** |
| **100MB**  |   1.22 s               |  13.30 s               | **10.9x**   |  225 MB  |    6 GB | **0.04x** |

**Key metrics:**
- ✅ NEON SIMD optimizations active (block scalar + anchor parsing)
- ✅ **10.1x faster** than yq on 1MB files
- ✅ **11.1x faster** than yq on 10MB files
- ✅ 90+ MiB/s throughput at 100MB
- ✅ **25x less memory** on 100MB files (225 MB vs 6 GB)
- ✅ SVE-capable CPU (256-bit vectors)

### ARM (Apple M1 Max) - yq Identity Comparison (comprehensive pattern)

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.1 ms    |    8.4 ms    | **1.6x**      |    7 MB  |   14 MB | **0.54x** |
| **10KB**   |    5.5 ms    |    9.1 ms    | **1.7x**      |    7 MB  |   14 MB | **0.51x** |
| **100KB**  |    6.1 ms    |   19.5 ms    | **3.2x**      |    8 MB  |   23 MB | **0.33x** |
| **1MB**    |   18.6 ms    |  113.7 ms    | **6.1x**      |   10 MB  |   82 MB | **0.12x** |
| **10MB**   |  119.8 ms    |    1.02 s    | **8.5x**      |   31 MB  |  689 MB | **0.05x** |
| **100MB**  |    1.09 s    |    9.74 s    | **8.9x**      |  239 MB  |    6 GB | **0.04x** |

**Key metrics:**
- ✅ **8.9x faster** than yq on 100MB files
- ✅ **25x less memory** on large files (100MB: 239 MB vs 6 GB)
- ✅ Speedup increases with file size (amortizes index construction)
- ✅ Memory efficiency improves dramatically at scale
- ✅ **Streaming JSON output** eliminates intermediate String allocations

### ARM (Apple M4 Pro) - yq Identity Comparison (comprehensive pattern)

**yq version**: v4.52.2

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    3.5 ms    |    5.8 ms    | **1.6x**      |    4 MB  |   12 MB | **0.37x** |
| **10KB**   |    3.5 ms    |    6.5 ms    | **1.8x**      |    4 MB  |   13 MB | **0.33x** |
| **100KB**  |    4.6 ms    |   14.9 ms    | **3.2x**      |    5 MB  |   21 MB | **0.21x** |
| **1MB**    |   13.1 ms    |   84.6 ms    | **6.5x**      |    7 MB  |   88 MB | **0.08x** |
| **10MB**   |   94.5 ms    |  741.7 ms    | **7.9x**      |   31 MB  |  719 MB | **0.04x** |
| **100MB**  |  873.9 ms    |    7.03 s    | **8.0x**      |  243 MB  |    6 GB | **0.04x** |

**Key metrics:**
- ✅ **8.0x faster than yq** on 100MB files (comprehensive pattern)
- ✅ **12.2x faster than yq** on 100MB nested patterns (best performance)
- ✅ **25x less memory** on large files (100MB: 243 MB vs 6 GB)
- ✅ **Fastest ARM platform tested** for YAML processing

### x86_64 (AMD Ryzen 9 7950X) - yq Identity Comparison (comprehensive pattern)

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.5 ms    |   60.8 ms    | **24x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.7 ms    |   62.9 ms    | **23x**       |    5 MB  |   22 MB | **0.22x** |
| **100KB**  |    3.7 ms    |   78.2 ms    | **21x**       |    5 MB  |   22 MB | **0.23x** |
| **1MB**    |   13.8 ms    |  203.3 ms    | **15x**       |    7 MB  |   85 MB | **0.08x** |
| **10MB**   |  114.6 ms    |    1.31 s    | **11x**       |   27 MB  |  626 MB | **0.04x** |
| **100MB**  |    1.06 s    |   12.97 s    | **12x**       |  226 MB  |    7 GB | **0.03x** |

### x86_64 (AMD Ryzen 9 7950X) - Key Achievements

**Key Achievements:**
- ✅ **Full yq CLI compatibility** - quoted strings preserved as strings
- ✅ **24x faster than yq** on small files (end-to-end CLI comparison)
- ✅ **15x faster than yq** on 1MB files (comprehensive pattern)
- ✅ **27x less memory** on 100MB files (226 MB vs 6 GB)
- ✅ **BMI2 PDEP** for O(1) select_in_word (7.6-16x faster select1)
- ✅ **Comprehensive test suite** - 32 tests including 8 direct byte-for-byte comparisons

**See also:** [docs/parsing/yaml.md](../parsing/yaml.md) for optimization details.

---

## Detailed Results by Pattern (ARM - Neoverse-V2 / Graviton 4)

Date: 2026-01-29

### Pattern: comprehensive

Mixed YAML content with various features.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.6 ms    |    3.6 ms    | **2.2x**      |    4 MB  |    9 MB | **0.42x** |
| **10KB**   |    1.8 ms    |    5.1 ms    | **2.9x**      |    4 MB  |   10 MB | **0.38x** |
| **100KB**  |    3.1 ms    |   20.0 ms    | **6.5x**      |    4 MB  |   18 MB | **0.24x** |
| **1MB**    |   14.7 ms    |  152.2 ms    | **10.3x**     |    6 MB  |   74 MB | **0.08x** |
| **10MB**   |  127.4 ms    |    1.41 s    | **11.1x**     |   26 MB  |  703 MB | **0.04x** |
| **100MB**  |    1.21 s    |   13.33 s    | **11.0x**     |  225 MB  |    7 GB | **0.03x** |

### Pattern: nested

Deeply nested mapping structures. **Best speedup pattern** due to efficient BP tree navigation.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    3.8 ms    | **2.2x**      |    4 MB  |    9 MB | **0.42x** |
| **10KB**   |    1.7 ms    |    4.8 ms    | **2.7x**      |    4 MB  |   10 MB | **0.39x** |
| **100KB**  |    2.3 ms    |   13.6 ms    | **6.0x**      |    4 MB  |   14 MB | **0.27x** |
| **1MB**    |    8.8 ms    |  113.4 ms    | **12.8x**     |    5 MB  |   57 MB | **0.09x** |
| **10MB**   |   67.2 ms    |    1.04 s    | **15.5x**     |   17 MB  |  449 MB | **0.04x** |
| **100MB**  |  541.0 ms    |    8.32 s    | **15.4x**     |  117 MB  |    4 GB | **0.03x** |

### Pattern: pathological

Worst-case structural density. Tests parser robustness.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.6 ms    |    3.8 ms    | **2.4x**      |    4 MB  |    9 MB | **0.42x** |
| **10KB**   |    1.8 ms    |    5.3 ms    | **3.0x**      |    4 MB  |   10 MB | **0.38x** |
| **100KB**  |    3.2 ms    |   23.9 ms    | **7.5x**      |    4 MB  |   19 MB | **0.23x** |
| **1MB**    |   15.0 ms    |  191.8 ms    | **12.8x**     |    6 MB  |   95 MB | **0.06x** |
| **10MB**   |  121.0 ms    |    1.74 s    | **14.4x**     |   27 MB  |  772 MB | **0.03x** |
| **100MB**  |    1.11 s    |   16.35 s    | **14.7x**     |  229 MB  |    7 GB | **0.03x** |

### Pattern: users

Realistic user record arrays (common in config files).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    3.9 ms    | **2.3x**      |    4 MB  |    9 MB | **0.42x** |
| **10KB**   |    1.8 ms    |    5.4 ms    | **3.0x**      |    4 MB  |   10 MB | **0.39x** |
| **100KB**  |    3.2 ms    |   22.7 ms    | **7.1x**      |    4 MB  |   18 MB | **0.23x** |
| **1MB**    |   16.3 ms    |  189.3 ms    | **11.6x**     |    6 MB  |  105 MB | **0.06x** |
| **10MB**   |  143.5 ms    |    1.79 s    | **12.5x**     |   29 MB  |  933 MB | **0.03x** |
| **100MB**  |    1.40 s    |   17.43 s    | **12.4x**     |  261 MB  |    9 GB | **0.03x** |

### Pattern: sequences

Sequence-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.6 ms    |    3.6 ms    | **2.2x**      |    4 MB  |    9 MB | **0.43x** |
| **10KB**   |    1.8 ms    |    4.9 ms    | **2.8x**      |    4 MB  |   10 MB | **0.39x** |
| **100KB**  |    3.4 ms    |   22.1 ms    | **6.5x**      |    4 MB  |   18 MB | **0.23x** |
| **1MB**    |   17.8 ms    |  178.4 ms    | **10.0x**     |    6 MB  |  105 MB | **0.06x** |
| **10MB**   |  154.7 ms    |    1.67 s    | **10.8x**     |   32 MB  |  909 MB | **0.04x** |
| **100MB**  |    1.48 s    |   15.59 s    | **10.5x**     |  282 MB  |    9 GB | **0.03x** |

### Pattern: strings

String-heavy YAML with quoted variants.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    3.6 ms    | **2.1x**      |    4 MB  |    9 MB | **0.41x** |
| **10KB**   |    1.7 ms    |    4.4 ms    | **2.6x**      |    4 MB  |   10 MB | **0.39x** |
| **100KB**  |    2.7 ms    |   14.1 ms    | **5.2x**      |    4 MB  |   15 MB | **0.27x** |
| **1MB**    |   11.7 ms    |  101.8 ms    | **8.7x**      |    6 MB  |   55 MB | **0.10x** |
| **10MB**   |   95.4 ms    |  937.4 ms    | **9.8x**      |   23 MB  |  472 MB | **0.05x** |
| **100MB**  |  929.8 ms    |    8.91 s    | **9.6x**      |  198 MB  |    4 GB | **0.04x** |

### Pattern: numbers

Numeric-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    3.6 ms    | **2.2x**      |    4 MB  |    9 MB | **0.42x** |
| **10KB**   |    1.7 ms    |    5.0 ms    | **2.9x**      |    4 MB  |   10 MB | **0.40x** |
| **100KB**  |    3.2 ms    |   20.2 ms    | **6.3x**      |    4 MB  |   17 MB | **0.25x** |
| **1MB**    |   16.0 ms    |  157.0 ms    | **9.8x**      |    6 MB  |   76 MB | **0.08x** |
| **10MB**   |  136.7 ms    |    1.40 s    | **10.2x**     |   25 MB  |  640 MB | **0.04x** |
| **100MB**  |    1.29 s    |   12.89 s    | **10.0x**     |  216 MB  |    6 GB | **0.04x** |

### Pattern: unicode

Unicode-heavy strings in various scripts.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.6 ms    |    3.5 ms    | **2.2x**      |    4 MB  |    9 MB | **0.41x** |
| **10KB**   |    1.7 ms    |    4.2 ms    | **2.5x**      |    4 MB  |   10 MB | **0.39x** |
| **100KB**  |    2.6 ms    |   13.0 ms    | **5.0x**      |    4 MB  |   15 MB | **0.27x** |
| **1MB**    |   11.1 ms    |   93.9 ms    | **8.5x**      |    6 MB  |   54 MB | **0.10x** |
| **10MB**   |   91.1 ms    |  869.6 ms    | **9.6x**      |   23 MB  |  458 MB | **0.05x** |
| **100MB**  |  892.4 ms    |    8.32 s    | **9.3x**      |  198 MB  |    5 GB | **0.04x** |

### Pattern: mixed

Mixed mappings and sequences.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.6 ms    |    3.5 ms    | **2.2x**      |    4 MB  |    9 MB | **0.42x** |
| **10KB**   |    1.8 ms    |    5.0 ms    | **2.8x**      |    4 MB  |   10 MB | **0.38x** |
| **100KB**  |    3.0 ms    |   19.7 ms    | **6.5x**      |    4 MB  |   18 MB | **0.23x** |
| **1MB**    |   14.4 ms    |  151.0 ms    | **10.5x**     |    6 MB  |   77 MB | **0.07x** |
| **10MB**   |  121.1 ms    |    1.37 s    | **11.3x**     |   25 MB  |  739 MB | **0.03x** |
| **100MB**  |    1.14 s    |   12.75 s    | **11.2x**     |  214 MB  |    7 GB | **0.03x** |

### Pattern: navigation

Navigation pattern for testing M2 streaming.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.6 ms    |    3.7 ms    | **2.3x**      |    4 MB  |   10 MB | **0.42x** |
| **10KB**   |    1.8 ms    |    5.0 ms    | **2.8x**      |    4 MB  |   10 MB | **0.39x** |
| **100KB**  |    3.1 ms    |   20.5 ms    | **6.6x**      |    4 MB  |   19 MB | **0.23x** |
| **1MB**    |   15.6 ms    |  165.2 ms    | **10.6x**     |    6 MB  |   85 MB | **0.07x** |
| **10MB**   |  138.2 ms    |    1.59 s    | **11.5x**     |   29 MB  |  704 MB | **0.04x** |
| **100MB**  |    1.37 s    |   15.55 s    | **11.4x**     |  253 MB  |    7 GB | **0.03x** |

---

## Detailed Results by Pattern (ARM - Neoverse-V1 / Graviton 3)

### Pattern: comprehensive

Mixed YAML content with various features.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.8 ms    |    4.0 ms    | **2.2x**      |    4 MB  |    9 MB | **0.42x** |
| **10KB**   |    1.9 ms    |    5.6 ms    | **2.9x**      |    4 MB  |   10 MB | **0.39x** |
| **100KB**  |    3.2 ms    |   21.0 ms    | **6.5x**      |    4 MB  |   17 MB | **0.24x** |
| **1MB**    |   15.2 ms    |  152.9 ms    | **10.1x**     |    6 MB  |   73 MB | **0.08x** |
| **10MB**   |  128.6 ms    |    1.42 s    | **11.1x**     |   26 MB  |  743 MB | **0.04x** |
| **100MB**  |    1.22 s    |   13.30 s    | **10.9x**     |  225 MB  |    6 GB | **0.04x** |

### Pattern: config

Small configuration files (constant size across all benchmarks).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.8 ms    |    3.8 ms    | **2.3x**      |    4 MB  |    9 MB | **0.43x** |
| **10KB**   |    1.7 ms    |    4.0 ms    | **2.4x**      |    4 MB  |    9 MB | **0.42x** |
| **100KB**  |    1.7 ms    |    3.9 ms    | **2.3x**      |    4 MB  |    9 MB | **0.43x** |
| **1MB**    |    1.8 ms    |    4.0 ms    | **2.3x**      |    4 MB  |    9 MB | **0.42x** |
| **10MB**   |    1.7 ms    |    4.0 ms    | **2.3x**      |    4 MB  |    9 MB | **0.42x** |
| **100MB**  |    1.7 ms    |    3.8 ms    | **2.3x**      |    4 MB  |    9 MB | **0.42x** |

### Pattern: mixed

Mixed mappings and sequences.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    3.9 ms    | **2.2x**      |    4 MB  |    9 MB | **0.42x** |
| **10KB**   |    1.9 ms    |    5.6 ms    | **3.0x**      |    4 MB  |   10 MB | **0.38x** |
| **100KB**  |    3.3 ms    |   20.9 ms    | **6.4x**      |    4 MB  |   18 MB | **0.23x** |
| **1MB**    |   14.8 ms    |  151.7 ms    | **10.2x**     |    6 MB  |   80 MB | **0.07x** |
| **10MB**   |  120.9 ms    |    1.39 s    | **11.5x**     |   25 MB  |  734 MB | **0.03x** |
| **100MB**  |    1.15 s    |   12.81 s    | **11.1x**     |  214 MB  |    6 GB | **0.03x** |

### Pattern: navigation

Navigation-heavy YAML content (for M2 streaming benchmarks).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    3.9 ms    | **2.4x**      |    4 MB  |    9 MB | **0.43x** |
| **10KB**   |    1.8 ms    |    5.3 ms    | **2.9x**      |    4 MB  |   10 MB | **0.39x** |
| **100KB**  |    3.2 ms    |   21.1 ms    | **6.6x**      |    4 MB  |   19 MB | **0.23x** |
| **1MB**    |   15.8 ms    |  168.5 ms    | **10.7x**     |    6 MB  |   81 MB | **0.07x** |
| **10MB**   |  139.8 ms    |    1.61 s    | **11.5x**     |   29 MB  |  693 MB | **0.04x** |
| **100MB**  |    1.38 s    |   15.70 s    | **11.4x**     |  253 MB  |    8 GB | **0.03x** |

### Pattern: nested

Deeply nested mapping structures. **Best speedup pattern** due to efficient BP tree navigation.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    4.0 ms    | **2.4x**      |    4 MB  |   10 MB | **0.42x** |
| **10KB**   |    1.7 ms    |    5.0 ms    | **2.9x**      |    4 MB  |   10 MB | **0.40x** |
| **100KB**  |    2.4 ms    |   13.9 ms    | **5.8x**      |    4 MB  |   14 MB | **0.27x** |
| **1MB**    |    8.9 ms    |  113.4 ms    | **12.8x**     |    5 MB  |   56 MB | **0.09x** |
| **10MB**   |   68.2 ms    |    1.06 s    | **15.5x**     |   17 MB  |  462 MB | **0.04x** |
| **100MB**  |  542.4 ms    |    8.39 s    | **15.5x**     |  117 MB  |    4 GB | **0.03x** |

### Pattern: numbers

Numeric-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    3.8 ms    | **2.3x**      |    4 MB  |    9 MB | **0.43x** |
| **10KB**   |    1.8 ms    |    5.1 ms    | **2.8x**      |    4 MB  |   10 MB | **0.39x** |
| **100KB**  |    3.3 ms    |   21.0 ms    | **6.4x**      |    4 MB  |   17 MB | **0.25x** |
| **1MB**    |   16.3 ms    |  158.6 ms    | **9.7x**      |    6 MB  |   80 MB | **0.07x** |
| **10MB**   |  138.4 ms    |    1.41 s    | **10.2x**     |   25 MB  |  625 MB | **0.04x** |
| **100MB**  |    1.30 s    |   12.89 s    | **9.9x**      |  216 MB  |    6 GB | **0.04x** |

### Pattern: pathological

Worst-case structural density. Tests parser robustness.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    3.9 ms    | **2.3x**      |    4 MB  |    9 MB | **0.42x** |
| **10KB**   |    1.8 ms    |    5.7 ms    | **3.1x**      |    4 MB  |   11 MB | **0.38x** |
| **100KB**  |    3.3 ms    |   24.9 ms    | **7.6x**      |    4 MB  |   19 MB | **0.22x** |
| **1MB**    |   15.5 ms    |  191.5 ms    | **12.4x**     |    6 MB  |   91 MB | **0.07x** |
| **10MB**   |  122.6 ms    |    1.75 s    | **14.2x**     |   27 MB  |  756 MB | **0.04x** |
| **100MB**  |    1.11 s    |   16.49 s    | **14.8x**     |  229 MB  |    7 GB | **0.03x** |

### Pattern: sequences

Sequence-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    3.9 ms    | **2.3x**      |    4 MB  |    9 MB | **0.43x** |
| **10KB**   |    1.9 ms    |    5.4 ms    | **2.9x**      |    4 MB  |   10 MB | **0.38x** |
| **100KB**  |    3.5 ms    |   22.9 ms    | **6.5x**      |    4 MB  |   19 MB | **0.22x** |
| **1MB**    |   18.2 ms    |  181.4 ms    | **10.0x**     |    6 MB  |  103 MB | **0.06x** |
| **10MB**   |  156.0 ms    |    1.66 s    | **10.6x**     |   32 MB  |  826 MB | **0.04x** |
| **100MB**  |    1.50 s    |   15.72 s    | **10.5x**     |  282 MB  |    8 GB | **0.03x** |

### Pattern: strings

String-heavy YAML with quoted variants.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.8 ms    |    3.9 ms    | **2.1x**      |    4 MB  |    9 MB | **0.41x** |
| **10KB**   |    1.8 ms    |    4.8 ms    | **2.7x**      |    4 MB  |   10 MB | **0.39x** |
| **100KB**  |    2.8 ms    |   14.8 ms    | **5.2x**      |    4 MB  |   15 MB | **0.26x** |
| **1MB**    |   11.7 ms    |  104.1 ms    | **8.9x**      |    6 MB  |   54 MB | **0.10x** |
| **10MB**   |   95.8 ms    |  926.1 ms    | **9.7x**      |   23 MB  |  451 MB | **0.05x** |
| **100MB**  |  930.2 ms    |    8.90 s    | **9.6x**      |  198 MB  |    4 GB | **0.04x** |

### Pattern: unicode

Unicode-heavy strings in various scripts.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    3.8 ms    | **2.3x**      |    4 MB  |    9 MB | **0.41x** |
| **10KB**   |    1.8 ms    |    4.4 ms    | **2.5x**      |    4 MB  |   10 MB | **0.39x** |
| **100KB**  |    2.7 ms    |   13.9 ms    | **5.1x**      |    4 MB  |   15 MB | **0.27x** |
| **1MB**    |   11.3 ms    |   94.7 ms    | **8.4x**      |    6 MB  |   53 MB | **0.10x** |
| **10MB**   |   93.1 ms    |  857.8 ms    | **9.2x**      |   23 MB  |  464 MB | **0.05x** |
| **100MB**  |  896.2 ms    |    8.27 s    | **9.2x**      |  198 MB  |    4 GB | **0.05x** |

### Pattern: users

Realistic user record arrays (common in config files).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    1.7 ms    |    3.9 ms    | **2.3x**      |    4 MB  |    9 MB | **0.43x** |
| **10KB**   |    1.8 ms    |    5.4 ms    | **2.9x**      |    4 MB  |   10 MB | **0.39x** |
| **100KB**  |    3.2 ms    |   23.1 ms    | **7.3x**      |    4 MB  |   18 MB | **0.23x** |
| **1MB**    |   16.5 ms    |  190.4 ms    | **11.6x**     |    6 MB  |   97 MB | **0.06x** |
| **10MB**   |  144.9 ms    |    1.81 s    | **12.5x**     |   29 MB  |  938 MB | **0.03x** |
| **100MB**  |    1.42 s    |   17.50 s    | **12.3x**     |  261 MB  |    9 GB | **0.03x** |

---

## Detailed Results by Pattern (ARM - Apple M1 Max)

### Pattern: comprehensive

Mixed YAML content with various features.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.1 ms    |    8.4 ms    | **1.6x**      |    7 MB  |   14 MB | **0.54x** |
| **10KB**   |    5.5 ms    |    9.1 ms    | **1.7x**      |    7 MB  |   14 MB | **0.51x** |
| **100KB**  |    6.1 ms    |   19.5 ms    | **3.2x**      |    8 MB  |   23 MB | **0.33x** |
| **1MB**    |   18.6 ms    |  113.7 ms    | **6.1x**      |   10 MB  |   82 MB | **0.12x** |
| **10MB**   |  119.8 ms    |    1.02 s    | **8.5x**      |   31 MB  |  689 MB | **0.05x** |
| **100MB**  |    1.09 s    |    9.74 s    | **8.9x**      |  239 MB  |    6 GB | **0.04x** |

### Pattern: config

Small configuration files (constant size across all benchmarks).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    4.8 ms    |    7.4 ms    | **1.5x**      |    7 MB  |   13 MB | **0.55x** |
| **10KB**   |    4.6 ms    |    7.3 ms    | **1.6x**      |    7 MB  |   14 MB | **0.54x** |
| **100KB**  |    4.7 ms    |    7.2 ms    | **1.5x**      |    7 MB  |   13 MB | **0.55x** |
| **1MB**    |    5.0 ms    |    7.1 ms    | **1.4x**      |    7 MB  |   13 MB | **0.55x** |
| **10MB**   |    4.6 ms    |    7.2 ms    | **1.6x**      |    7 MB  |   14 MB | **0.54x** |
| **100MB**  |    5.5 ms    |    6.9 ms    | **1.3x**      |    7 MB  |   13 MB | **0.55x** |

### Pattern: mixed

Mixed mappings and sequences.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.7 ms    |    8.2 ms    | **1.4x**      |    7 MB  |   14 MB | **0.54x** |
| **10KB**   |    5.3 ms    |    9.1 ms    | **1.7x**      |    7 MB  |   15 MB | **0.51x** |
| **100KB**  |    6.3 ms    |   20.7 ms    | **3.3x**      |    8 MB  |   24 MB | **0.33x** |
| **1MB**    |   16.5 ms    |  112.1 ms    | **6.8x**      |   10 MB  |   88 MB | **0.11x** |
| **10MB**   |  109.5 ms    |  984.3 ms    | **9.0x**      |   30 MB  |  642 MB | **0.05x** |
| **100MB**  |    1.01 s    |    9.28 s    | **9.2x**      |  228 MB  |    7 GB | **0.03x** |

### Pattern: navigation

Navigation-heavy YAML content (for M2 streaming benchmarks).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    6.1 ms    |    8.8 ms    | **1.4x**      |    7 MB  |   14 MB | **0.54x** |
| **10KB**   |    4.8 ms    |    8.5 ms    | **1.8x**      |    7 MB  |   14 MB | **0.51x** |
| **100KB**  |    6.0 ms    |   19.8 ms    | **3.3x**      |    8 MB  |   23 MB | **0.34x** |
| **1MB**    |   18.3 ms    |  124.2 ms    | **6.8x**      |   10 MB  |   99 MB | **0.10x** |
| **10MB**   |  128.6 ms    |    1.14 s    | **8.8x**      |   34 MB  |  729 MB | **0.05x** |
| **100MB**  |    1.21 s    |   11.31 s    | **9.4x**      |  275 MB  |    7 GB | **0.04x** |

### Pattern: nested

Deeply nested mapping structures. **Best speedup pattern** due to efficient BP tree navigation.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    4.6 ms    |    7.3 ms    | **1.6x**      |    7 MB  |   14 MB | **0.54x** |
| **10KB**   |    6.1 ms    |   10.0 ms    | **1.6x**      |    7 MB  |   14 MB | **0.52x** |
| **100KB**  |    6.2 ms    |   15.9 ms    | **2.6x**      |    8 MB  |   20 MB | **0.38x** |
| **1MB**    |   10.9 ms    |   89.6 ms    | **8.2x**      |    9 MB  |   60 MB | **0.15x** |
| **10MB**   |   67.2 ms    |  782.6 ms    | **11.6x**     |   22 MB  |  431 MB | **0.05x** |
| **100MB**  |  488.4 ms    |    6.25 s    | **12.8x**     |  128 MB  |    4 GB | **0.03x** |

### Pattern: numbers

Numeric-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.4 ms    |    7.3 ms    | **1.4x**      |    7 MB  |   13 MB | **0.55x** |
| **10KB**   |    5.7 ms    |    9.0 ms    | **1.6x**      |    7 MB  |   14 MB | **0.52x** |
| **100KB**  |    7.2 ms    |   19.8 ms    | **2.7x**      |    8 MB  |   22 MB | **0.35x** |
| **1MB**    |   18.7 ms    |  115.4 ms    | **6.2x**      |   10 MB  |   81 MB | **0.12x** |
| **10MB**   |  128.7 ms    |  991.9 ms    | **7.7x**      |   30 MB  |  601 MB | **0.05x** |
| **100MB**  |    1.19 s    |    9.28 s    | **7.8x**      |  247 MB  |    5 GB | **0.04x** |

### Pattern: pathological

Worst-case structural density. Tests parser robustness.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    5.1 ms    |    7.8 ms    | **1.5x**      |    7 MB  |   14 MB | **0.54x** |
| **10KB**   |    5.5 ms    |    9.8 ms    | **1.8x**      |    7 MB  |   15 MB | **0.51x** |
| **100KB**  |    6.1 ms    |   22.7 ms    | **3.7x**      |    8 MB  |   24 MB | **0.32x** |
| **1MB**    |   16.6 ms    |  147.9 ms    | **8.9x**      |   10 MB  |   93 MB | **0.11x** |
| **10MB**   |  116.0 ms    |    1.31 s    | **11.3x**     |   32 MB  |  741 MB | **0.04x** |
| **100MB**  |    1.03 s    |   12.31 s    | **12.0x**     |  236 MB  |    7 GB | **0.03x** |

### Pattern: sequences

Sequence-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    4.7 ms    |    7.3 ms    | **1.6x**      |    7 MB  |   13 MB | **0.55x** |
| **10KB**   |    4.9 ms    |    8.4 ms    | **1.7x**      |    7 MB  |   14 MB | **0.51x** |
| **100KB**  |    6.7 ms    |   22.3 ms    | **3.3x**      |    8 MB  |   24 MB | **0.33x** |
| **1MB**    |   18.4 ms    |  130.7 ms    | **7.1x**      |   11 MB  |  102 MB | **0.11x** |
| **10MB**   |  137.8 ms    |    1.20 s    | **8.7x**      |   48 MB  |  854 MB | **0.06x** |
| **100MB**  |    1.29 s    |   11.49 s    | **8.9x**      |  406 MB  |    8 GB | **0.05x** |

### Pattern: strings

String-heavy YAML with quoted variants.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    4.6 ms    |    7.2 ms    | **1.6x**      |    7 MB  |   13 MB | **0.55x** |
| **10KB**   |    4.7 ms    |    7.8 ms    | **1.6x**      |    7 MB  |   14 MB | **0.53x** |
| **100KB**  |    6.5 ms    |   15.0 ms    | **2.3x**      |    8 MB  |   20 MB | **0.39x** |
| **1MB**    |   15.0 ms    |   77.7 ms    | **5.2x**      |   10 MB  |   61 MB | **0.16x** |
| **10MB**   |   88.4 ms    |  665.0 ms    | **7.5x**      |   28 MB  |  450 MB | **0.06x** |
| **100MB**  |  836.3 ms    |    6.44 s    | **7.7x**      |  231 MB  |    5 GB | **0.05x** |

### Pattern: unicode

Unicode-heavy strings in various scripts.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    4.8 ms    |    7.8 ms    | **1.6x**      |    7 MB  |   13 MB | **0.55x** |
| **10KB**   |    4.7 ms    |    7.9 ms    | **1.7x**      |    7 MB  |   14 MB | **0.53x** |
| **100KB**  |    5.7 ms    |   14.6 ms    | **2.6x**      |    8 MB  |   20 MB | **0.38x** |
| **1MB**    |   13.1 ms    |   73.8 ms    | **5.6x**      |   10 MB  |   60 MB | **0.16x** |
| **10MB**   |   85.0 ms    |  633.3 ms    | **7.4x**      |   28 MB  |  412 MB | **0.07x** |
| **100MB**  |  792.4 ms    |    6.11 s    | **7.7x**      |  209 MB  |    4 GB | **0.05x** |

### Pattern: users

Realistic user record arrays (common in config files).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    4.8 ms    |    7.5 ms    | **1.5x**      |    7 MB  |   13 MB | **0.55x** |
| **10KB**   |    5.3 ms    |    9.1 ms    | **1.7x**      |    7 MB  |   14 MB | **0.52x** |
| **100KB**  |    6.1 ms    |   21.6 ms    | **3.5x**      |    8 MB  |   24 MB | **0.32x** |
| **1MB**    |   17.7 ms    |  137.8 ms    | **7.8x**      |   10 MB  |  100 MB | **0.10x** |
| **10MB**   |  134.3 ms    |    1.28 s    | **9.5x**      |   34 MB  |  858 MB | **0.04x** |
| **100MB**  |    1.27 s    |   12.77 s    | **10.0x**     |  282 MB  |    9 GB | **0.03x** |

---

## Detailed Results by Pattern (ARM - Apple M4 Pro)

**CPU**: Apple M4 Pro (12 cores)
**OS**: macOS 15.6.1
**yq version**: v4.52.2
**Date**: 2026-02-02

### Pattern: comprehensive

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    3.5 ms    |    5.8 ms    | **1.6x**      |    4 MB  |   12 MB | **0.37x** |
| **10KB**   |    3.5 ms    |    6.5 ms    | **1.8x**      |    4 MB  |   13 MB | **0.33x** |
| **100KB**  |    4.6 ms    |   14.9 ms    | **3.2x**      |    5 MB  |   21 MB | **0.21x** |
| **1MB**    |   13.1 ms    |   84.6 ms    | **6.5x**      |    7 MB  |   88 MB | **0.08x** |
| **10MB**   |   94.5 ms    |  741.7 ms    | **7.9x**      |   31 MB  |  719 MB | **0.04x** |
| **100MB**  |  873.9 ms    |    7.03 s    | **8.0x**      |  243 MB  |    6 GB | **0.04x** |

### Pattern: mixed

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    3.3 ms    |    5.4 ms    | **1.6x**      |    4 MB  |   12 MB | **0.36x** |
| **10KB**   |    3.4 ms    |    6.3 ms    | **1.8x**      |    4 MB  |   13 MB | **0.33x** |
| **100KB**  |    4.4 ms    |   15.0 ms    | **3.4x**      |    5 MB  |   22 MB | **0.21x** |
| **1MB**    |   13.1 ms    |   86.3 ms    | **6.6x**      |    7 MB  |   90 MB | **0.08x** |
| **10MB**   |   93.1 ms    |  754.4 ms    | **8.1x**      |   32 MB  |  710 MB | **0.04x** |
| **100MB**  |  857.8 ms    |    7.38 s    | **8.6x**      |  249 MB  |    6 GB | **0.04x** |

### Pattern: navigation

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    3.3 ms    |    5.4 ms    | **1.6x**      |    4 MB  |   12 MB | **0.37x** |
| **10KB**   |    3.3 ms    |    6.1 ms    | **1.8x**      |    4 MB  |   13 MB | **0.34x** |
| **100KB**  |    4.3 ms    |   14.2 ms    | **3.3x**      |    5 MB  |   22 MB | **0.21x** |
| **1MB**    |   12.8 ms    |   86.9 ms    | **6.8x**      |    7 MB  |   81 MB | **0.09x** |
| **10MB**   |   94.5 ms    |  776.8 ms    | **8.2x**      |   32 MB  |  697 MB | **0.05x** |
| **100MB**  |  894.4 ms    |    7.98 s    | **8.9x**      |  254 MB  |    6 GB | **0.04x** |

### Pattern: nested

Deeply nested mapping structures. **Best speedup pattern** due to efficient BP tree navigation.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    3.5 ms    |    5.5 ms    | **1.6x**      |    4 MB  |   12 MB | **0.36x** |
| **10KB**   |    3.3 ms    |    6.3 ms    | **1.9x**      |    4 MB  |   13 MB | **0.33x** |
| **100KB**  |    4.0 ms    |   15.2 ms    | **3.8x**      |    4 MB  |   22 MB | **0.21x** |
| **1MB**    |    9.9 ms    |   89.4 ms    | **9.0x**      |    7 MB  |   77 MB | **0.09x** |
| **10MB**   |   67.1 ms    |  781.9 ms    | **11.7x**     |   29 MB  |  662 MB | **0.04x** |
| **100MB**  |  625.5 ms    |    7.61 s    | **12.2x**     |  219 MB  |    6 GB | **0.03x** |

### Pattern: numbers

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    3.3 ms    |    5.3 ms    | **1.6x**      |    4 MB  |   12 MB | **0.37x** |
| **10KB**   |    3.4 ms    |    6.2 ms    | **1.8x**      |    4 MB  |   12 MB | **0.35x** |
| **100KB**  |    4.4 ms    |   13.5 ms    | **3.1x**      |    5 MB  |   21 MB | **0.22x** |
| **1MB**    |   13.5 ms    |   78.9 ms    | **5.8x**      |    7 MB  |   80 MB | **0.09x** |
| **10MB**   |   99.8 ms    |  673.0 ms    | **6.7x**      |   29 MB  |  600 MB | **0.05x** |
| **100MB**  |  900.2 ms    |    6.27 s    | **7.0x**      |  220 MB  |    6 GB | **0.04x** |

### Pattern: pathological

Worst-case structural density. Tests parser robustness.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    3.4 ms    |    5.4 ms    | **1.6x**      |    4 MB  |   12 MB | **0.36x** |
| **10KB**   |    3.4 ms    |    6.3 ms    | **1.9x**      |    4 MB  |   13 MB | **0.33x** |
| **100KB**  |    4.3 ms    |   16.6 ms    | **3.8x**      |    5 MB  |   22 MB | **0.20x** |
| **1MB**    |   12.2 ms    |  102.2 ms    | **8.3x**      |    7 MB  |   91 MB | **0.08x** |
| **10MB**   |   83.0 ms    |  894.1 ms    | **10.8x**     |   30 MB  |  731 MB | **0.04x** |
| **100MB**  |  726.8 ms    |    8.56 s    | **11.8x**     |  234 MB  |    6 GB | **0.04x** |

### Pattern: sequences

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    3.3 ms    |    5.4 ms    | **1.6x**      |    4 MB  |   12 MB | **0.37x** |
| **10KB**   |    3.3 ms    |    6.0 ms    | **1.8x**      |    4 MB  |   13 MB | **0.34x** |
| **100KB**  |    4.4 ms    |   15.5 ms    | **3.5x**      |    5 MB  |   22 MB | **0.21x** |
| **1MB**    |   14.9 ms    |   91.2 ms    | **6.1x**      |    9 MB  |  105 MB | **0.08x** |
| **10MB**   |  110.2 ms    |  794.8 ms    | **7.2x**      |   44 MB  |  833 MB | **0.05x** |
| **100MB**  |    1.02 s    |    7.75 s    | **7.6x**      |  286 MB  |    7 GB | **0.04x** |

### Pattern: strings

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    3.3 ms    |    5.3 ms    | **1.6x**      |    4 MB  |   12 MB | **0.37x** |
| **10KB**   |    3.3 ms    |    5.5 ms    | **1.7x**      |    4 MB  |   12 MB | **0.35x** |
| **100KB**  |    4.2 ms    |   10.9 ms    | **2.6x**      |    4 MB  |   18 MB | **0.24x** |
| **1MB**    |   10.1 ms    |   53.6 ms    | **5.3x**      |    7 MB  |   63 MB | **0.11x** |
| **10MB**   |   69.4 ms    |  455.0 ms    | **6.6x**      |   28 MB  |  455 MB | **0.06x** |
| **100MB**  |  650.1 ms    |    4.34 s    | **6.7x**      |  202 MB  |    4 GB | **0.05x** |

### Pattern: unicode

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    3.3 ms    |    5.4 ms    | **1.6x**      |    4 MB  |   12 MB | **0.36x** |
| **10KB**   |    3.3 ms    |    5.6 ms    | **1.7x**      |    4 MB  |   12 MB | **0.35x** |
| **100KB**  |    3.9 ms    |   10.9 ms    | **2.8x**      |    4 MB  |   18 MB | **0.25x** |
| **1MB**    |    9.7 ms    |   52.6 ms    | **5.4x**      |    7 MB  |   57 MB | **0.12x** |
| **10MB**   |   66.5 ms    |  426.0 ms    | **6.4x**      |   28 MB  |  453 MB | **0.06x** |
| **100MB**  |  615.0 ms    |    4.09 s    | **6.7x**      |  203 MB  |    4 GB | **0.05x** |

### Pattern: users

Realistic user record arrays (common in config files).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    3.4 ms    |    6.0 ms    | **1.8x**      |    4 MB  |   12 MB | **0.36x** |
| **10KB**   |    3.3 ms    |    6.2 ms    | **1.9x**      |    4 MB  |   13 MB | **0.34x** |
| **100KB**  |    4.5 ms    |   15.1 ms    | **3.4x**      |    5 MB  |   22 MB | **0.21x** |
| **1MB**    |   13.6 ms    |   95.6 ms    | **7.0x**      |    7 MB  |   96 MB | **0.08x** |
| **10MB**   |   98.2 ms    |  886.1 ms    | **9.0x**      |   32 MB  |  861 MB | **0.04x** |
| **100MB**  |  921.2 ms    |    9.05 s    | **9.8x**      |  262 MB  |    7 GB | **0.03x**

---

## Detailed Results by Pattern (x86_64 - AMD Ryzen 9 7950X)

### Pattern: comprehensive

Mixed YAML content with various features.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.5 ms    |   60.8 ms    | **24x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.7 ms    |   62.9 ms    | **23x**       |    5 MB  |   22 MB | **0.22x** |
| **100KB**  |    3.7 ms    |   78.2 ms    | **21x**       |    5 MB  |   22 MB | **0.23x** |
| **1MB**    |   13.8 ms    |  203.3 ms    | **15x**       |    7 MB  |   85 MB | **0.08x** |
| **10MB**   |  114.6 ms    |    1.31 s    | **11x**       |   27 MB  |  626 MB | **0.04x** |
| **100MB**  |    1.06 s    |   12.97 s    | **12x**       |  226 MB  |    7 GB | **0.03x** |

### Pattern: config

Small configuration files (constant size across all benchmarks).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.5 ms    |   60.7 ms    | **24x**       |    5 MB  |   22 MB | **0.22x** |
| **10KB**   |    2.4 ms    |   60.4 ms    | **25x**       |    5 MB  |   23 MB | **0.20x** |
| **100KB**  |    2.4 ms    |   60.2 ms    | **25x**       |    5 MB  |   23 MB | **0.21x** |
| **1MB**    |    2.5 ms    |   60.9 ms    | **25x**       |    5 MB  |   22 MB | **0.21x** |
| **10MB**   |    2.4 ms    |   60.4 ms    | **25x**       |    5 MB  |   22 MB | **0.22x** |
| **100MB**  |    2.6 ms    |   61.4 ms    | **23x**       |    5 MB  |   22 MB | **0.22x** |

### Pattern: mixed

Mixed mappings and sequences.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.7 ms    |   61.2 ms    | **23x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.8 ms    |   64.9 ms    | **23x**       |    5 MB  |   22 MB | **0.21x** |
| **100KB**  |    3.7 ms    |   83.9 ms    | **23x**       |    5 MB  |   22 MB | **0.22x** |
| **1MB**    |   13.6 ms    |  204.4 ms    | **15x**       |    7 MB  |   86 MB | **0.08x** |
| **10MB**   |  106.6 ms    |    1.27 s    | **12x**       |   26 MB  |  629 MB | **0.04x** |
| **100MB**  |  985.1 ms    |   12.06 s    | **12x**       |  215 MB  |    6 GB | **0.04x** |

### Pattern: navigation

Navigation-heavy YAML content (for M2 streaming benchmarks).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.5 ms    |   69.0 ms    | **28x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.5 ms    |   61.8 ms    | **24x**       |    5 MB  |   22 MB | **0.21x** |
| **100KB**  |    3.9 ms    |   77.1 ms    | **20x**       |    5 MB  |   23 MB | **0.22x** |
| **1MB**    |   14.8 ms    |  213.9 ms    | **14x**       |    7 MB  |   92 MB | **0.08x** |
| **10MB**   |  121.3 ms    |    1.60 s    | **13x**       |   30 MB  |  740 MB | **0.04x** |
| **100MB**  |    1.18 s    |   15.33 s    | **13x**       |  254 MB  |    8 GB | **0.03x** |

### Pattern: nested

Deeply nested mapping structures. **Best speedup pattern** due to efficient BP tree navigation.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.4 ms    |   61.2 ms    | **25x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.5 ms    |   62.7 ms    | **25x**       |    5 MB  |   22 MB | **0.22x** |
| **100KB**  |    3.0 ms    |   72.8 ms    | **24x**       |    5 MB  |   22 MB | **0.24x** |
| **1MB**    |    9.0 ms    |  162.5 ms    | **18x**       |    6 MB  |   55 MB | **0.11x** |
| **10MB**   |   61.5 ms    |  968.6 ms    | **16x**       |   19 MB  |  428 MB | **0.04x** |
| **100MB**  |  487.0 ms    |    7.88 s    | **16x**       |  118 MB  |    4 GB | **0.03x** |

### Pattern: numbers

Numeric-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.6 ms    |   61.3 ms    | **23x**       |    5 MB  |   22 MB | **0.22x** |
| **10KB**   |    2.6 ms    |   62.2 ms    | **24x**       |    5 MB  |   22 MB | **0.21x** |
| **100KB**  |    3.9 ms    |   77.3 ms    | **20x**       |    5 MB  |   22 MB | **0.22x** |
| **1MB**    |   15.3 ms    |  199.0 ms    | **13x**       |    7 MB  |   70 MB | **0.10x** |
| **10MB**   |  119.6 ms    |    1.28 s    | **11x**       |   26 MB  |  570 MB | **0.05x** |
| **100MB**  |    1.12 s    |   11.98 s    | **11x**       |  216 MB  |    5 GB | **0.04x** |

### Pattern: pathological

Worst-case structural density. Tests parser robustness.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.4 ms    |   61.0 ms    | **25x**       |    5 MB  |   22 MB | **0.22x** |
| **10KB**   |    2.6 ms    |   62.3 ms    | **24x**       |    5 MB  |   22 MB | **0.23x** |
| **100KB**  |    3.9 ms    |   78.9 ms    | **20x**       |    5 MB  |   23 MB | **0.22x** |
| **1MB**    |   15.1 ms    |  236.7 ms    | **16x**       |    7 MB  |   92 MB | **0.08x** |
| **10MB**   |  109.9 ms    |    1.61 s    | **15x**       |   28 MB  |  749 MB | **0.04x** |
| **100MB**  |  989.0 ms    |   15.74 s    | **16x**       |  230 MB  |    7 GB | **0.03x** |

### Pattern: sequences

Sequence-heavy YAML content.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.6 ms    |   62.0 ms    | **24x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.6 ms    |   63.0 ms    | **24x**       |    5 MB  |   23 MB | **0.22x** |
| **100KB**  |    4.1 ms    |   77.8 ms    | **19x**       |    6 MB  |   22 MB | **0.25x** |
| **1MB**    |   16.2 ms    |  220.2 ms    | **14x**       |    8 MB  |   90 MB | **0.08x** |
| **10MB**   |  131.2 ms    |    1.61 s    | **12x**       |   33 MB  |  941 MB | **0.04x** |
| **100MB**  |    1.23 s    |   15.67 s    | **13x**       |  283 MB  |    9 GB | **0.03x** |

### Pattern: strings

String-heavy YAML with quoted variants.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.6 ms    |   60.5 ms    | **24x**       |    5 MB  |   22 MB | **0.22x** |
| **10KB**   |    2.5 ms    |   61.8 ms    | **25x**       |    5 MB  |   23 MB | **0.21x** |
| **100KB**  |    3.4 ms    |   71.0 ms    | **21x**       |    5 MB  |   22 MB | **0.22x** |
| **1MB**    |   11.4 ms    |  151.4 ms    | **13x**       |    7 MB  |   58 MB | **0.12x** |
| **10MB**   |   88.6 ms    |  862.7 ms    | **10x**       |   24 MB  |  411 MB | **0.06x** |
| **100MB**  |  814.3 ms    |    8.30 s    | **10x**       |  199 MB  |    4 GB | **0.05x** |

### Pattern: unicode

Unicode-heavy strings in various scripts.

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.7 ms    |   61.1 ms    | **23x**       |    5 MB  |   22 MB | **0.21x** |
| **10KB**   |    2.7 ms    |   61.7 ms    | **23x**       |    5 MB  |   22 MB | **0.22x** |
| **100KB**  |    3.6 ms    |   74.2 ms    | **21x**       |    5 MB  |   23 MB | **0.22x** |
| **1MB**    |   11.1 ms    |  150.0 ms    | **14x**       |    7 MB  |   55 MB | **0.13x** |
| **10MB**   |   80.3 ms    |  863.0 ms    | **11x**       |   24 MB  |  464 MB | **0.05x** |
| **100MB**  |  813.1 ms    |    8.25 s    | **10x**       |  199 MB  |    5 GB | **0.04x** |

### Pattern: users

Realistic user record arrays (common in config files).

| Size       | succinctly   | yq           | Speedup       | succ Mem | yq Mem  | Mem Ratio |
|------------|--------------|--------------|---------------|----------|---------|-----------|
| **1KB**    |    2.6 ms    |   61.0 ms    | **23x**       |    5 MB  |   23 MB | **0.22x** |
| **10KB**   |    2.9 ms    |   62.9 ms    | **22x**       |    5 MB  |   22 MB | **0.23x** |
| **100KB**  |    3.8 ms    |   84.3 ms    | **22x**       |    5 MB  |   22 MB | **0.23x** |
| **1MB**    |   15.5 ms    |  234.4 ms    | **15x**       |    7 MB  |  100 MB | **0.07x** |
| **10MB**   |  126.3 ms    |    1.65 s    | **13x**       |   30 MB  |  836 MB | **0.04x** |
| **100MB**  |    1.21 s    |   16.82 s    | **14x**       |  262 MB  |    9 GB | **0.03x** |

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
- **1.4-12.5x faster** across all patterns and sizes
- **Best performance on nested structures**: 12.5x speedup on 100MB deeply nested files
- **Pathological patterns**: 11.9x speedup on worst-case structural density
- **Larger files benefit more**: Speedup increases with file size (amortizes index construction)
- **100MB files**: 7.6-12.5x faster depending on pattern

**AMD Ryzen 9 7950X (x86_64):**
- **10-25x faster** across all patterns and sizes
- **Best performance on nested structures**: 18x speedup on 1MB, 17x on 100MB
- **Small files dominate by startup**: 21-25x faster on 1-10KB (yq startup ~59ms)
- **100MB files**: 10-17x faster depending on pattern
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
- **17-33x less memory** on large files (100MB) with streaming output
- **Consistent ~7 MB baseline** for small files regardless of pattern
- **Linear scaling**: Memory grows proportionally with file size
- **yq memory explosion**: yq uses 4-9 GB for 100MB files vs 128-406 MB for succinctly

| Size       | succinctly    | yq          | Ratio        |
|------------|---------------|-------------|--------------|
| **1KB**    | 7 MB          | 13-14 MB    | ~0.5x        |
| **10KB**   | 7 MB          | 14-15 MB    | ~0.5x        |
| **100KB**  | 8 MB          | 20-24 MB    | ~0.35x       |
| **1MB**    | 10-11 MB      | 58-106 MB   | ~0.12x       |
| **10MB**   | 22-48 MB      | 446-834 MB  | ~0.05x       |
| **100MB**  | 128-406 MB    | 4-9 GB      | ~0.04x       |

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
- **12.5x faster** on 100MB nested files
- **8.7x faster** on 100MB comprehensive files
- **6.3x faster** on 1MB files

**Speed (AMD Ryzen 9 7950X):**
- **19x faster** on 100KB files
- **11x faster** on 1MB files
- **8x faster** on 100MB files

**Memory (Apple M1 Max, 100MB files):**
- **17-33x less memory** (128-406 MB vs 4-9 GB)
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
| `.`         | P9 streaming  | 1.21s      | 11.31s   | **9.4x**     | 275 MB   | 7 GB     | **0.04x** |
| `.[0]`      | M2 streaming  | 444ms      | 5.84s    | **13.2x**    | 254 MB   | 5 GB     | **0.05x** |
| `.[]`       | M2 streaming  | 2.53s      | 13.30s   | **5.3x**     | 269 MB   | 8 GB     | **0.03x** |
| `length`    | OwnedValue    | 445ms      | 5.84s    | **13.1x**    | 254 MB   | 5 GB     | **0.05x** |

**Key insights**:
- **M2 streaming (`.[0]`) is 2.7x faster than identity (`.`)** on the same file - navigates to first element without streaming entire document
- **succinctly uses 3-7% of yq's memory** across all query types
- **All paths use similar base memory** because the YAML index dominates (~3x input size)
- Navigation queries benefit from M2's lazy evaluation - only accessed elements are materialized
- `length` and `.[0]` show similar performance because both only need to count/access the first level

### Running M2-Focused Benchmarks

```bash
# Build with benchmark runner feature
cargo build --release --features bench-runner

# Benchmark all query types on the navigation pattern
./target/release/succinctly bench run yq_bench --patterns navigation --queries all

# Compare identity vs navigation queries
./target/release/succinctly bench run yq_bench --queries identity,first_element --sizes 10mb,100mb

# Memory is collected by default; use --no-memory to skip
./target/release/succinctly bench run yq_bench --queries all --sizes 100mb

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
# Build release binary with benchmark runner
cargo build --release --features bench-runner

# Generate benchmark files (includes navigation pattern for M2 testing)
./target/release/succinctly yaml generate-suite

# Run CLI benchmark tool (recommended - includes memory measurement)
./target/release/succinctly bench run yq_bench

# Run with specific patterns/sizes
./target/release/succinctly bench run yq_bench --patterns comprehensive,nested --sizes 1mb,10mb

# Run navigation-focused benchmarks (M2 streaming)
./target/release/succinctly bench run yq_bench --patterns navigation --queries all --sizes 10mb,100mb

# Skip memory collection for faster runs
./target/release/succinctly bench run yq_bench --no-memory --queries identity,first_element,length

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
