# Benchmarks

[Home](/) > [Docs](../) > Benchmarks

Performance comparisons between succinctly and other tools.

## Benchmark Results

| Benchmark | Description | Key Finding |
|-----------|-------------|-------------|
| [jq](jq.md) | succinctly jq vs system jq | 1.0-1.6x faster |
| [yq](yq.md) | succinctly yq vs system yq | 1.8-8.7x faster (ARM), 16-40x faster (x86_64) |
| [Rust Parsers](rust-parsers.md) | vs serde_json, sonic-rs, simd-json | Competitive with specialized parsers |
| [Cross-Language](cross-language.md) | Multi-language parser comparison | Best-in-class for semi-indexing |
| [DSV](dsv.md) | CSV/TSV parsing performance | 85-1676 MiB/s (API) |

## Methodology

All benchmarks use:
- Criterion.rs for statistical accuracy
- Multiple file sizes (1KB to 1MB+)
- Multiple patterns (comprehensive, users, nested, etc.)
- Warm cache conditions

## Running Benchmarks

```bash
# Build release binary first
cargo build --release --features cli

# Generate benchmark data
cargo run --release --features cli -- yaml generate-suite
cargo run --release --features cli -- json generate 10mb -o benchmark.json

# Run specific benchmarks
cargo bench --bench jq_comparison
cargo bench --bench yq_comparison
cargo bench --bench yq_select
cargo bench --bench yaml_bench
```

## Platform Notes

### Apple Silicon (M1/M2/M3)
- Uses ARM NEON SIMD (16 bytes/iteration)
- Best performance with `-C target-cpu=native`

### x86_64 (Intel/AMD)
- Uses AVX2 SIMD (32 bytes/iteration)
- SSE2 fallback available
- Build with `-C target-cpu=native` for best results

## See Also

- [Optimization Techniques](../optimizations/) - How we achieved these results
- [Parsing Implementation](../parsing/) - Parser architecture
