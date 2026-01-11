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

## Benchmark Results

### ARM (Apple M1 Max)

#### Parse Only (Index/Parse Time)

| Size   | serde_json | simd-json  | sonic-rs       | succinctly |
|--------|------------|------------|----------------|------------|
| 1kb    | 177 MiB/s  | 206 MiB/s  | **773 MiB/s**  | 678 MiB/s  |
| 10kb   | 167 MiB/s  | 202 MiB/s  | **692 MiB/s**  | 609 MiB/s  |
| 100kb  | 156 MiB/s  | 205 MiB/s  | **660 MiB/s**  | 550 MiB/s  |
| 1mb    | 160 MiB/s  | 212 MiB/s  | **667 MiB/s**  | 529 MiB/s  |
| 10mb   | 155 MiB/s  | 202 MiB/s  | **671 MiB/s**  | 542 MiB/s  |
| 100mb  | 153 MiB/s  | 182 MiB/s  | **687 MiB/s**  | 534 MiB/s  |

#### Parse + Traverse (Full Pipeline)

| Size   | serde_json | simd-json  | sonic-rs       | succinctly_fast |
|--------|------------|------------|----------------|-----------------|
| 1kb    | 165 MiB/s  | 287 MiB/s  | **446 MiB/s**  | 356 MiB/s       |
| 10kb   | 155 MiB/s  | 254 MiB/s  | **421 MiB/s**  | 302 MiB/s       |
| 100kb  | 145 MiB/s  | 249 MiB/s  | **403 MiB/s**  | 271 MiB/s       |
| 1mb    | 148 MiB/s  | 255 MiB/s  | **417 MiB/s**  | 272 MiB/s       |
| 10mb   | 141 MiB/s  | 239 MiB/s  | **423 MiB/s**  | 281 MiB/s       |
| 100mb  | 139 MiB/s  | 227 MiB/s  | **425 MiB/s**  | 283 MiB/s       |

#### Peak Memory Usage (100MB file)

| Parser         | Peak Memory | vs JSON Size |
|----------------|-------------|--------------|
| serde_json     | 655 MB      | 8.2x         |
| simd-json      | 1654 MB     | 20.7x        |
| sonic-rs       | 955 MB      | 11.9x        |
| **succinctly** | **37 MB**   | **0.46x**    |

### Key Findings

- **sonic-rs is fastest** for both parse-only and parse+traverse when full document access is needed
- **succinctly has the lowest memory** - uses 17-45x less memory than DOM parsers
- succinctly's semi-index overhead is only 46% of input size vs 8-21x for DOM parsers
- For selective field access (jq-style queries), succinctly's lazy evaluation avoids parsing unused data
