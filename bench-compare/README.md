# JSON Parser Benchmark Comparison

This is a separate crate for benchmarking `succinctly` against other Rust JSON parsers.

By keeping these benchmarks in a separate crate, we avoid adding `simd-json`, `sonic-rs`, and other parser dependencies to the main `succinctly` crate.

## Parsers Compared

- **serde_json**: Standard DOM parser (baseline)
- **simd-json**: SIMD-accelerated parser (simdjson port)
- **sonic-rs**: SIMD + arena-based parser
- **succinctly**: Semi-index with balanced parentheses

## Running Benchmarks

First, generate test data from the parent directory:

```bash
cd ..
cargo run --release --features cli -- json generate 10mb -o data/bench/generated/comprehensive/10mb.json
cargo run --release --features cli -- json generate 100mb -o data/bench/generated/comprehensive/100mb.json
```

Then run benchmarks:

```bash
# All benchmarks
cargo bench --bench json_parsers

# Specific benchmark groups
cargo bench --bench json_parsers -- "parse_only"
cargo bench --bench json_parsers -- "parse_traverse"
cargo bench --bench json_parsers -- "traverse_only"
cargo bench --bench json_parsers -- "memory_overhead"
```

## Benchmark Groups

### parse_only
Measures parse/index time only (no traversal). Tests how fast each parser can build its internal representation.

### parse_traverse
Measures full pipeline: parse + traverse all nodes. Tests end-to-end performance for reading entire documents.

### traverse_only
Measures traversal of pre-parsed data. Isolates the cost of navigating the data structure.

### memory_overhead
Prints memory usage estimates for each parser's internal representation.
