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

| Benchmark                                 | Description                          | Key Finding                                      |
|-------------------------------------------|--------------------------------------|--------------------------------------------------|
| [jq](jq.md)                               | succinctly jq vs system jq           | 1.1-2.3x faster across platforms                 |
| [yq](yq.md)                               | succinctly yq vs system yq           | 4.5-11x faster (Graviton 4), 7-25x (x86_64)      |
| [Rust JSON Parsers](rust-parsers.md)      | vs serde_json, sonic-rs, simd-json   | Competitive with specialized parsers             |
| [Rust YAML Parsers](rust-yaml-parsers.md) | vs serde_yaml                        | 8-14x faster parsing, 10-39x less memory         |
| [Cross-Language](cross-language.md)       | Multi-language parser comparison     | Best-in-class for semi-indexing                  |
| [DSV](dsv.md)                             | CSV/TSV parsing performance          | 85-1676 MiB/s (API)                              |

## Methodology

All benchmarks use:
- Criterion.rs for statistical accuracy
- Multiple file sizes (1KB to 1MB+)
- Multiple patterns (comprehensive, users, nested, etc.)
- Warm cache conditions

For detailed benchmarking information, see:
- **[Benchmarking Guide](../guides/benchmarking.md)** - Complete guide to running and interpreting benchmarks
- **[Benchmark Inventory](inventory.md)** - Index of all benchmark reports in this project

## See Also

- [Optimization Techniques](../optimizations/) - How we achieved these results
- [Parsing Implementation](../parsing/) - Parser architecture
