# Benchmarks

[Home](/) > [Docs](../) > Benchmarks

Performance comparisons between succinctly and other tools.

## Understanding the Comparisons

Succinctly uses **semi-indexing** rather than traditional DOM parsing. This architectural difference is important context for interpreting benchmarks:

| Approach        | How it works                           | Memory         | Speed                       |
|-----------------|----------------------------------------|----------------|-----------------------------|
| **Semi-index**  | Build structural index, extract lazily | ~3-6% overhead | Fast for queries            |
| **DOM parsing** | Parse entire document into memory tree | 6-21x input    | Required for all operations |

The benchmarks below compare succinctly against DOM-based tools (jq, yq, serde_json, etc.). Performance advantages come from both the semi-index architecture and doing less validation work. See individual benchmark pages for detailed architectural comparisons.

## Benchmark Results

| Benchmark                           | Description                          | Key Finding                                      |
|-------------------------------------|--------------------------------------|--------------------------------------------------|
| [jq](jq.md)                         | succinctly jq vs system jq           | 1.1-2.3x faster across platforms                 |
| [yq](yq.md)                         | succinctly yq vs system yq           | 4.5-11x faster (Graviton 4), 7-25x (x86_64)      |
| [Rust Parsers](rust-parsers.md)     | vs serde_json, sonic-rs, simd-json   | Competitive with specialized parsers             |
| [Cross-Language](cross-language.md) | Multi-language parser comparison     | Best-in-class for semi-indexing                  |
| [DSV](dsv.md)                       | CSV/TSV parsing performance          | 85-1676 MiB/s (API)                              |

## Methodology

All benchmarks use:
- Criterion.rs for statistical accuracy
- Multiple file sizes (1KB to 1MB+)
- Multiple patterns (comprehensive, users, nested, etc.)
- Warm cache conditions

## Available Benchmark Suites

| Benchmark               | Description                                | Data Required    |
|-------------------------|--------------------------------------------|------------------|
| `pfsm_vs_simd`          | PFSM (table-based) vs SIMD (AVX2/NEON)     | JSON test data   |
| `pfsm_vs_scalar`        | PFSM vs true scalar implementation         | JSON test data   |
| `json_simd`             | All SIMD implementations comparison        | JSON test data   |
| `json_pipeline`         | Full pipeline: index + navigate + print    | JSON test data   |
| `jq_comparison`         | succinctly jq vs system jq                 | CLI + JSON data  |
| `yq_comparison`         | succinctly yq vs system yq                 | CLI + YAML data  |
| `yq_select`             | Partial selection queries                  | CLI + YAML data  |
| `yaml_bench`            | YAML parsing throughput                    | Generated inline |
| `yaml_type_stack_micro` | YAML type stack operations                 | Generated inline |
| `yaml_anchor_micro`     | YAML anchor parsing                        | Generated inline |
| `yaml_transcode_micro`  | YAML→JSON transcoding                      | Generated inline |
| `dsv_bench`             | DSV/CSV parsing and access                 | DSV test data    |
| `rank_select`           | BitVec rank/select operations              | Generated inline |
| `balanced_parens`       | Tree navigation operations                 | Generated inline |
| `elias_fano`            | Elias-Fano encoding for monotone sequences | Generated inline |
| `neon_movemask`         | NEON movemask implementations              | N/A (ARM only)   |
| `popcount_strategies`   | Popcount implementations                   | Generated inline |

## Running Benchmarks

```bash
# Build release binary first
cargo build --release --features cli

# Generate benchmark data
cargo run --release --features cli -- yaml generate-suite
cargo run --release --features cli -- json generate-suite

# Run specific benchmarks
cargo bench --bench jq_comparison
cargo bench --bench yq_comparison
cargo bench --bench yq_select
cargo bench --bench yaml_bench
cargo bench --bench pfsm_vs_simd
cargo bench --bench pfsm_vs_scalar
```

## Platform Notes

### ARM Neoverse-V2 (AWS Graviton 4)
- Uses ARM NEON SIMD (128-bit vectors)
- Supports SVE2 (128-bit vectors) with SVEBITPERM (BDEP/BEXT)
- ~20% faster than Neoverse-V1
- Tested: Linux 6.14.0-1018-aws

### ARM Neoverse-V1 (AWS Graviton 3)
- Uses ARM NEON SIMD (128-bit vectors)
- Supports SVE (256-bit vectors, not yet utilized)
- Tested: 4 cores, Linux 6.14.0-1018-aws

### Apple Silicon (M1/M2/M3)
- Uses ARM NEON SIMD (16 bytes/iteration)
- Best performance with `-C target-cpu=native`

### x86_64 (Intel/AMD)
- Uses AVX2 SIMD (32 bytes/iteration)
- BMI2 PDEP for O(1) `select_in_word` (Zen 3+/Haswell+)
- SSE2 fallback available
- Build with `-C target-cpu=native` for best results

## See Also

- [Optimization Techniques](../optimizations/) - How we achieved these results
- [Parsing Implementation](../parsing/) - Parser architecture
