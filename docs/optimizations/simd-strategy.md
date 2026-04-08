# SIMD Strategy

[Home](../../) > [Docs](../) > [Optimizations](./) > SIMD Strategy

Succinctly uses SIMD acceleration across all parsing modules, with platform-specific implementations selected at compile time and runtime.

## Platform Support

| Extension     | Width   | Platform | Used In                                              |
|---------------|---------|----------|------------------------------------------------------|
| AVX2          | 256-bit | x86_64   | JSON, DSV, YAML block scalars, YAML anchors          |
| BMI2          | 64-bit  | x86_64   | DSV quote masking (PDEP/PEXT), JSON post-processing  |
| SSE4.2        | 128-bit | x86_64   | JSON character classification                        |
| SSE2          | 128-bit | x86_64   | DSV baseline                                         |
| NEON          | 128-bit | ARM64    | JSON, DSV, YAML, BP RangeMin, escape scanning        |
| PMULL         | 64-bit  | ARM64    | DSV prefix XOR                                       |
| SVE2-BITPERM  | varies  | ARM64    | JSON, DSV (BDEP/BEXT)                                |

## Per-Module SIMD Usage

### JSON ([src/json/simd/](../../src/json/simd/))
- **Character classification**: AVX2 processes 32 bytes/iteration identifying structural characters (`{}[]:,"`)
- **SSE4.2**: `PCMPISTRI` for string matching (38% faster than SSE2)
- **Result**: ~880 MiB/s indexing throughput

### DSV ([src/dsv/simd/](../../src/dsv/simd/))
- **Quote state tracking**: BMI2 `PDEP` + carry propagation computes running XOR in one chain (10x vs scalar)
- **Prefix XOR**: AVX2/NEON parallel prefix for quote parity
- **PMULL**: ARM carryless multiply for prefix XOR
- **Result**: 85-1676 MiB/s throughput

### YAML ([src/yaml/simd/](../../src/yaml/simd/))
- **Block scalar scanning** (P2.7): AVX2 scans 32-byte chunks for newlines, checks indentation
- **Anchor name scanning** (P4): AVX2 scans for anchor terminators in 32-byte chunks
- **Escape scanning** (O3): NEON scans for JSON escape characters (`"`, `\`, `< 0x20`)
- **Result**: ~250-400 MiB/s indexing, up to 110 MiB/s yq queries

### BalancedParens ([src/trees/bp.rs](../../src/trees/bp.rs))
- **RangeMin**: ARM NEON `vminvq_s16` for 2.8x faster horizontal minimum
- **x86**: SSE4.1 `PHMINPOSUW` (modest 1-3% gain due to unsigned-only limitation)

## Key Lessons Learned

### Wider SIMD != Faster

AVX-512 for JSON parsing was **7-17% slower** than AVX2 and was removed from the codebase. Reasons:
- Memory-bound workloads don't benefit from wider vectors
- Zen 4 splits 512-bit ops into two 256-bit paths
- AVX2 already saturates memory bandwidth for sequential parsing

See [parsing/yaml.md](../parsing/yaml.md#p8-avx-512-variants---rejected-) for the YAML AVX-512 analysis.

### SIMD Thresholds Matter

A minimum input size threshold is needed before SIMD pays off. For YAML:
- Block scalars: SIMD wins at 32+ bytes
- Anchor names: SIMD wins at 32+ bytes
- Escape scanning: 16-byte threshold required (with `#[inline(always)]`)
- Flow collections: Rejected (P5) because real-world inputs are typically < 30 bytes

### Micro-Benchmarks Mislead

Three consecutive YAML SIMD optimizations showed micro-benchmark wins but caused end-to-end regressions:
- P2.8 (threshold tuning): 2-4% micro gain → 8-15% end-to-end regression
- P3 (branchless classification): 3-29% micro gain → 25-44% end-to-end regression
- P8 (AVX-512): Benchmark design flaw (measured iterations, not throughput)

### Grammar Constrains SIMD Applicability

BMI2 quote indexing works for DSV (context-free `""` escaping) but not YAML (backslash escaping requires preprocessing). The grammar determines which SIMD techniques are applicable, not just the platform.

## Runtime Detection

SIMD paths are selected via `#[cfg(target_arch)]` and Rust's `is_x86_feature_detected!()` / `is_aarch64_feature_detected!()` macros. The `simd` feature flag enables explicit intrinsics; without it, Rust's auto-vectorization handles basic cases.

## See Also

- [SIMD](simd.md) — detailed technique reference for each SIMD extension
- [Targets](targets.md) — CPU-specific build configurations

## Source & Docs

- Optimization guide: [simd.md](simd.md)
- Optimization history: [history.md](history.md)
- Per-module SIMD directories: `src/json/simd/`, `src/yaml/simd/`, `src/dsv/simd/`, `src/util/simd/`
