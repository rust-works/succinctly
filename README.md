# succinctly

[![CI](https://github.com/rust-works/succinctly/actions/workflows/ci.yml/badge.svg)](https://github.com/rust-works/succinctly/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/succinctly.svg)](https://crates.io/crates/succinctly)
[![docs.rs](https://docs.rs/succinctly/badge.svg)](https://docs.rs/succinctly)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

High-performance succinct data structures for Rust.

Succinctly provides space-efficient data structures with fast rank and select operations, optimized for both x86_64 (with AVX2/AVX-512) and ARM (NEON) architectures. The library is `no_std` compatible and designed for high-throughput applications.

## Features

- **Bitvector with O(1) rank and sampled select** - Poppy-style 3-level directory, tuned for query speed over compactness (~28-48% overhead)
- **Balanced parentheses for tree navigation** - hierarchical min-excess index for fast `find_close`/`enclose`, plus an O(1) rank directory (~58% overhead, tuned for speed)
- **JSON semi-indexing with SIMD acceleration** - Up to 880 MiB/s throughput on x86_64 (AMD Zen 4) with table-driven PFSM parser
- **YAML semi-indexing** - Complete YAML 1.2 parser with anchor/alias resolution (~250-400 MiB/s)
- **DSV/CSV semi-indexing** - High-performance CSV/TSV parsing (85-1676 MiB/s) with BMI2 acceleration
- **jq/yq-style query expressions** - Navigate JSON and YAML without full parsing
- **`no_std` compatible** - Works in embedded and WASM environments
- **Cross-platform SIMD** - Runtime detection for AVX2, AVX-512, SSE4.2, and ARM NEON

### What is Semi-Indexing?

Unlike traditional parsers that build a complete in-memory representation (DOM) of documents, succinctly uses **semi-indexing**: it builds a lightweight structural index (~3-6% overhead) and extracts values lazily on demand. This approach offers:

- **18-46x less memory** than DOM parsers (index is ~24% of input vs 600-800% for DOM)
- **Faster queries** because only accessed values are materialized
- **Streaming output** without intermediate allocations

**Trade-off**: Semi-indexing performs minimal validation compared to full parsers. For strict validation or features like YAML tags, use traditional tools. See [Architecture](docs/architecture/semi-indexing.md) for details.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
succinctly = "0.7"
```

Or with cargo:

```bash
cargo add succinctly
```

### CLI release binaries

The `succinctly-linux-x64` and `succinctly-windows-x64` archives are built with the x86-64 `POPCNT` instruction
(Intel Nehalem / AMD K10, 2008, or newer): the rank/select hot paths are 30-70% faster in micro-benchmarks, and
queries are typically 2-3% faster end to end, up to 20% on structure-heavy YAML, on a Ryzen 9 7950X (see
[docs/optimizations/simd.md](docs/optimizations/simd.md)). On an older CPU
the binary stops at startup with a message naming the requirement; `cargo install succinctly --features cli` builds
for the baseline target and runs everywhere. The Linux and macOS ARM64 archives are unaffected.

## Quick Start

### Bitvector with Rank/Select

```rust
use succinctly::{BitVec, RankSelect};

// Create a bitvector from u64 words
let words = vec![0b1010_1010_1010_1010u64; 8];
let bv = BitVec::from_words(words, 512);

// Query rank (count of 1-bits in [0, i))
assert_eq!(bv.rank1(8), 4);

// Query select (position of k-th 1-bit, 0-indexed)
assert_eq!(bv.select1(0), Some(1));  // First 1-bit is at position 1
assert_eq!(bv.select1(3), Some(7));  // Fourth 1-bit is at position 7
```

### Balanced Parentheses for Tree Navigation

```rust
use succinctly::trees::BalancedParens;

// Encode a tree as balanced parentheses: ((()())())
// In bits: 1=open, 0=close -> 1110100100
let bp = BalancedParens::new(vec![0b0010010111], 10);

// Find matching close parenthesis
assert_eq!(bp.find_close(0), Some(9));  // Outermost pair
assert_eq!(bp.find_close(1), Some(6));  // Second level

// Navigate the tree
assert_eq!(bp.first_child(0), Some(1));   // First child of root
assert_eq!(bp.next_sibling(1), Some(7));  // Sibling of first child
```

### JSON Semi-Indexing

```rust
use succinctly::json::{JsonIndex, StandardJson};

let json = br#"{"users": [{"name": "Alice"}, {"name": "Bob"}]}"#;
let index = JsonIndex::build(json);
let root = index.root(json);

// Navigate without parsing the entire document
if let StandardJson::Object(obj) = root {
    if let Some(StandardJson::Array(users)) = obj.get("users") {
        // Iterate array elements
        for user in users {
            // Access nested fields efficiently
        }
    }
}
```

### jq-Style Queries

```rust
use succinctly::jq::{parse, eval, QueryResult};
use succinctly::json::{JsonIndex, StandardJson};

let json = br#"{"users": [{"name": "Alice", "age": 30}, {"name": "Bob", "age": 25}]}"#;
let index = JsonIndex::build(json);
let cursor = index.root(json);

// Get all user names
let expr = parse(".users[].name").unwrap();
if let QueryResult::Many(names) = eval(&expr, cursor) {
    // names contains ["Alice", "Bob"]
}

// Get first user's age
let expr = parse(".users[0].age").unwrap();
if let QueryResult::One(StandardJson::Number(age)) = eval(&expr, cursor) {
    assert_eq!(age, 30.0);
}
```

## Performance

### Comparison with Rust JSON Parsers

Benchmark comparing succinctly against popular Rust JSON libraries (x86_64, 1MB file):

| Library        | Parse Time | Throughput | Peak Memory | vs succinctly                    |
|----------------|------------|------------|-------------|----------------------------------|
| **sonic-rs**   | 1.00 ms    | 810 MiB/s  | 9.97 MB     | 1.6x faster, **26x more memory** |
| **succinctly** | 1.58 ms    | 510 MiB/s  | 382 KB      | baseline                         |
| serde_json     | 4.83 ms    | 167 MiB/s  | 7.00 MB     | 3.1x slower, **18x more memory** |
| simd-json      | 5.10 ms    | 158 MiB/s  | 17.1 MB     | 3.2x slower, **46x more memory** |

**Key findings**:
- **18-46x less memory** than DOM parsers (consistent 46% overhead vs JSON size)
- **Competitive parsing speed**: Only 1.6x slower than fastest DOM parser
- **3-5x faster** than serde_json and simd-json

See [docs/benchmarks/rust-parsers.md](docs/benchmarks/rust-parsers.md) for detailed benchmarks across all file sizes.

### JSON Semi-Indexing

| Platform                       | Implementation | Throughput    | Notes                        |
|--------------------------------|----------------|---------------|------------------------------|
| **x86_64** (AMD Ryzen 9 7950X) | PFSM (BMI2)    | **679 MiB/s** | Table-driven, fastest on x86 |
|                                | AVX2           | 732 MiB/s     | 32 bytes/iteration           |
|                                | Scalar         | 494 MiB/s     | Baseline                     |
| **ARM** (Apple M1 Max)         | NEON           | **574 MiB/s** | 32 bytes/iteration           |
|                                | Scalar (PFSM)  | 557 MiB/s     | Table-driven                 |
|                                | Scalar         | 405 MiB/s     | Baseline                     |
| **ARM** (Neoverse-V1)          | NEON/PFSM      | **~500 MiB/s**| Estimated based on jq perf   |

### Rank/Select Operations

| Platform                       | Rank (O(1)) | Select (O(log n)) |
|--------------------------------|-------------|-------------------|
| **x86_64** (AMD Ryzen 9 7950X) | ~3 ns       | ~50 ns            |
| **ARM** (Apple M1 Max)         | ~21 ns      | ~320 ns           |

### jq Comparison: Identity Operation (`.`)

Comparison of `succinctly jq .` vs `jq .` for formatting/printing JSON files.

#### x86_64 (AMD Ryzen 9 7950X, 10MB files, jq 1.7.1)

| Pattern           | jq       | succinctly    | Speedup  | jq Mem  | succ Mem | Mem Ratio  |
|-------------------|----------|---------------|----------|---------|----------|------------|
| **numbers**       |  236.2ms |    **60.8ms** | **3.9x** |   91 MB |    37 MB |      0.41x |
| **nested**        |  144.5ms |    **39.0ms** | **3.7x** |   23 MB |    35 MB |      1.49x |
| **arrays**        |  608.6ms |   **164.7ms** | **3.7x** |  385 MB |    51 MB |      0.13x |
| **strings**       |  128.9ms |    **36.0ms** | **3.6x** |   15 MB |    33 MB |      2.21x |
| **comprehensive** |  335.2ms |   **105.0ms** | **3.2x** |  156 MB |    42 MB |      0.27x |
| **unicode**       |  150.2ms |    **53.5ms** | **2.8x** |   32 MB |    38 MB |      1.17x |
| **pathological**  |  670.4ms |   **244.7ms** | **2.7x** |  495 MB |    64 MB |      0.13x |
| **users**         |  181.8ms |    **71.2ms** | **2.6x** |   71 MB |    31 MB |      0.44x |
| **literals**      |  241.7ms |   **114.1ms** | **2.1x** |   53 MB |    46 MB |      0.86x |

#### ARM (Neoverse-V1, 1MB files)

*The Neoverse-V1 and Apple M1 Max tables below predate the jq 1.7.1 re-run of #5336 (jq 1.6 and a system jq); the 7950X table above and the [Apple M4 Pro section](docs/benchmarks/jq.md) are jq 1.7.1.*

| Pattern           | jq       | succinctly    | Speedup  |
|-------------------|----------|---------------|----------|
| **nested**        |   23.8ms |  **8.8ms**    | **2.7x** |
| **strings**       |   21.2ms |  **9.4ms**    | **2.3x** |
| **arrays**        |   80.1ms |  **42.5ms**   | **1.9x** |
| **users**         |   28.8ms |  **17.4ms**   | **1.7x** |
| **comprehensive** |   45.6ms |  **27.9ms**   | **1.6x** |

#### ARM (Apple M1 Max, 10MB files)

| Pattern           | jq       | succinctly    | Speedup  | jq Mem  | succ Mem | Mem Ratio  |
|-------------------|----------|---------------|----------|---------|----------|------------|
| **nested**        |  330.7ms |  **63.7ms**   | **5.2x** |   25 MB |    31 MB |      1.26x |
| **strings**       |  307.7ms |  **78.7ms**   | **3.9x** |   16 MB |    19 MB |      1.23x |
| **pathological**  |    1.33s | **631.3ms**   | **2.1x** |  525 MB |    38 MB |      0.07x |
| **unicode**       |  319.6ms | **156.5ms**   | **2.0x** |   41 MB |    21 MB |      0.52x |
| **users**         |  386.6ms | **189.6ms**   | **2.0x** |   71 MB |    17 MB |      0.24x |
| **comprehensive** |  641.6ms | **342.9ms**   | **1.9x** |  134 MB |    19 MB |      0.14x |
| **numbers**       |  349.6ms | **209.3ms**   | **1.7x** |   96 MB |    20 MB |      0.21x |
| **arrays**        |    1.02s | **677.8ms**   | **1.5x** |  367 MB |    57 MB |      0.16x |
| **literals**      |  484.3ms | **397.9ms**   | **1.2x** |  103 MB |    22 MB |      0.21x |

**Key findings**:
- **1.2-5.2x faster** across all patterns
- **4-14x less memory** on most patterns due to streaming lazy evaluation
- Best speedup on nested/string-heavy data

### Platform-Specific Optimizations

| Platform             | Operation                    | Throughput | Speedup         |
|----------------------|------------------------------|------------|-----------------|
| **x86_64**           | Popcount (AVX-512 VPOPCNTDQ) | 96.8 GiB/s | 5.2x vs scalar† |
| **ARM (M1 Max)**     | NEON JSON (string-heavy)     |  3.7 GiB/s | 1.69x vs scalar |
| **ARM (Neoverse-V1)**| Popcount (NEON)              | 46.5 GiB/s |  N/A            |
|                      | NEON movemask (parallel)     |  1.37 ns   | 2.5x vs serial  |

† Popcount speedup is measured against a *baseline* build, where `count_ones()` lowers to scalar broadword. Compiled with `-C target-cpu=native`, `count_ones()` auto-vectorizes to VPOPCNTDQ and the explicit path drops to ≈1× (parity); the ~5–9× win holds only for baseline/portable builds. Full measured data: [Popcount Strategies](docs/optimizations/simd.md#popcount-strategies-explicit-simd-vs-auto-vectorized-count_ones) (#45).

See [docs/benchmarks/rust-parsers.md](docs/benchmarks/rust-parsers.md), [docs/benchmarks/jq.md](docs/benchmarks/jq.md), and [docs/optimizations/history.md](docs/optimizations/history.md) for detailed benchmarks.

## Feature Flags

### Popcount Strategies (mutually exclusive)

| Feature             | Description                                                         |
|---------------------|---------------------------------------------------------------------|
| *(default)*         | Uses Rust's `count_ones()` which auto-vectorizes                    |
| `simd`              | Explicit SIMD intrinsics (NEON on ARM, AVX-512 VPOPCNTDQ on x86_64) |
| `portable-popcount` | Portable bitwise algorithm (no intrinsics)                          |

### Other Features

| Feature | Description                                                      |
|---------|------------------------------------------------------------------|
| `std`   | Enable std library (default, required for runtime CPU detection) |
| `serde` | Enable serialization/deserialization support                     |
| `cli`   | Build the CLI tool                                               |
| `regex` | Enable regex support in jq queries (included in `cli`)           |

### Test Features

| Feature       | Description                           |
|---------------|---------------------------------------|
| `large-tests` | Test with 1GB bitvectors (~125MB RAM) |
| `huge-tests`  | Test with 5GB bitvectors (~625MB RAM) |
| `mmap-tests`  | Memory-mapped file tests              |

## CLI Tool

The library includes CLI tools for JSON, YAML, and DSV operations:

```bash
# Build the CLI
cargo build --release --features cli

# Install short aliases (sjq, syq, sjq-locate, syq-locate)
succinctly install-aliases

# Query JSON files (jq-compatible)
sjq '.users[].name' input.json
sjq -r '.users[0]' input.json

# Query YAML files (yq-compatible)
syq '.users[].name' config.yaml
syq -o json '.' config.yaml          # Output as JSON
syq '.spec.containers[]' k8s.yaml
syq --doc 0 '.' multi-doc.yaml       # First document only

# Query CSV/TSV files (converted to JSON on-the-fly)
sjq --input-dsv ',' '.[] | .[0]' users.csv
sjq --input-dsv '\t' '.[0]' data.tsv

# Output as CSV/TSV/DSV
sjq -r '.users[] | [.name, .age] | @csv' data.json
sjq -r '.users[] | [.name, .age] | @dsv("|")' data.json
syq -r '.users[] | [.name, .age] | @csv' data.yaml

# Find jq/yq expression for a position (useful for editor integration)
sjq-locate input.json --offset 42
sjq-locate input.json --line 5 --column 10
syq-locate config.yaml --offset 42
syq-locate config.yaml --line 5 --column 10

# Generate synthetic JSON/YAML for benchmarking
succinctly json generate 10mb -o benchmark.json
succinctly yaml generate 10kb -o benchmark.yaml
```

The `sjq`/`syq` aliases are symlinks created by `succinctly install-aliases`. All commands also work via the full `succinctly jq`/`succinctly yq` syntax.

## Architecture

### Module Structure

```
succinctly
├── bits         # Bitvector with rank/select
├── trees        # Tree encodings (balanced parentheses)
├── json         # JSON semi-indexing
├── yaml         # YAML semi-indexing
├── dsv          # DSV/CSV semi-indexing
└── jq           # jq/yq query language evaluator
```

### Core Data Structures

- **`bits::BitVec`** - Bitvector with 3-level Poppy-style rank directory (~25% overhead) and sampled select index; ~28-48% resident overhead in total, rising with bit density ([benchmarks](docs/benchmarks/rust-succinct-libs.md))
- **`trees::BalancedParens`** - Hierarchical min-excess structure and rank directory for tree navigation (~58% overhead; ~64% with select support, see [balanced-parens.md](docs/architecture/balanced-parens.md#space-overhead))
- **`json::JsonIndex`** - Semi-index combining Interest Bits (IB) and Balanced Parentheses (BP) for fast JSON navigation
- **`yaml::YamlIndex`** - YAML semi-index with anchor/alias resolution and multi-document support
- **`dsv::DsvIndex`** - Lightweight DSV semi-index with BMI2-accelerated quote masking

### SIMD Support

The library uses runtime CPU feature detection to select the best implementation:

| Platform | Features Used                         |
|----------|---------------------------------------|
| x86_64   | AVX2, AVX-512 VPOPCNTDQ, SSE4.2, SSE2 |
| aarch64  | NEON (mandatory)                      |

## Documentation

Choose your path:

- **New to succinctly?** -> [Getting Started](docs/getting-started/)
- **Using the library?** -> [API Guide](docs/guides/api.md)
- **Using the CLI?** -> [CLI Guide](docs/guides/cli.md)
- **Setting an environment variable?** -> [Environment Variables](docs/reference/environment-variables.md)
- **Contributing?** -> [CONTRIBUTING.md](CONTRIBUTING.md)
- **Performance tuning?** -> [Optimization Techniques](docs/optimizations/)
- **Understanding internals?** -> [Architecture](docs/architecture/)
- **Benchmarks?** -> [Performance Comparisons](docs/benchmarks/)
- **Full documentation map** -> [docs/](docs/)

For AI-assisted development, see [CLAUDE.md](CLAUDE.md).

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

Before submitting a PR:

```bash
# Run tests
cargo test

# Run clippy
cargo clippy --all-targets --all-features -- -D warnings

# Format code
cargo fmt
```

## License

Licensed under the [MIT license](LICENSE).

## Acknowledgments

This library implements algorithms from:

- Vigna, ["Broadword Implementation of Rank/Select Queries"](https://vigna.di.unimi.it/ftp/papers/Broadword.pdf) (WEA 2008)
- Zhou, Andersen, Kaminsky, ["Space-Efficient, High-Performance Rank & Select"](https://www.cs.cmu.edu/~dga/papers/zhou-sea2013.pdf) (SEA 2013)
- Sadakane & Navarro, "Fully-Functional Succinct Trees" (SODA 2010)
- Langdale & Lemire, ["Parsing Gigabytes of JSON per Second"](https://arxiv.org/abs/1902.08318) (VLDB 2019)
- Muła, Kurz, Lemire, ["Faster Population Counts Using AVX2 Instructions"](https://arxiv.org/abs/1611.07612) (2016)

## Related Work

- [haskell-works](https://github.com/haskell-works) - Haskell implementations of the same techniques (`hw-json`, `hw-json-simd`, `hw-rankselect`, `hw-balancedparens`)
