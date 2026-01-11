# succinctly

[![CI](https://github.com/rust-works/succinctly/actions/workflows/ci.yml/badge.svg)](https://github.com/rust-works/succinctly/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/succinctly.svg)](https://crates.io/crates/succinctly)
[![docs.rs](https://docs.rs/succinctly/badge.svg)](https://docs.rs/succinctly)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

High-performance succinct data structures for Rust.

Succinctly provides space-efficient data structures with fast rank and select operations, optimized for both x86_64 (with AVX2/AVX-512) and ARM (NEON) architectures. The library is `no_std` compatible and designed for high-throughput applications.

## Features

- **Bitvector with O(1) rank and O(log n) select** - Poppy-style 3-level directory with ~3% space overhead
- **Balanced parentheses for tree navigation** - RangeMin structure with O(1) operations and ~6% overhead
- **JSON semi-indexing with SIMD acceleration** - Up to 950 MiB/s throughput with table-driven PFSM parser
- **jq-style query expressions** - Navigate JSON without full parsing
- **`no_std` compatible** - Works in embedded and WASM environments
- **Cross-platform SIMD** - Runtime detection for AVX2, AVX-512, SSE4.2, and ARM NEON

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
succinctly = "0.1"
```

Or with cargo:

```bash
cargo add succinctly
```

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
use succinctly::bp::BalancedParens;

// Encode a tree as balanced parentheses: ((()())())
// In bits: 1=open, 0=close -> 1110100100
let bp = BalancedParens::new(&[0b0010010111], 10);

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

### JSON Semi-Indexing

| Platform                       | Implementation | Throughput    | Notes                        |
|--------------------------------|----------------|---------------|------------------------------|
| **x86_64** (AMD Ryzen 9 7950X) | PFSM (BMI2)    | **679 MiB/s** | Table-driven, fastest on x86 |
|                                | AVX2           | 732 MiB/s     | 32 bytes/iteration           |
|                                | Scalar         | 494 MiB/s     | Baseline                     |
| **ARM** (Apple M1 Max)         | NEON           | **574 MiB/s** | 32 bytes/iteration           |
|                                | Scalar (PFSM)  | 557 MiB/s     | Table-driven                 |
|                                | Scalar         | 405 MiB/s     | Baseline                     |

### Rank/Select Operations

| Platform                       | Rank (O(1)) | Select (O(log n)) |
|--------------------------------|-------------|-------------------|
| **x86_64** (AMD Ryzen 9 7950X) | ~3 ns       | ~50 ns            |
| **ARM** (Apple M1 Max)         | ~21 ns      | ~320 ns           |

### jq Comparison: Identity Operation (`.`)

Comparison of `succinctly jq .` vs `jq .` for formatting/printing JSON files.

#### x86_64 (AMD Ryzen 9 7950X, 10MB files)

| Pattern           | jq       | succinctly    | Speedup  | jq Mem  | succ Mem | Mem Ratio  |
|-------------------|----------|---------------|----------|---------|----------|------------|
| **nested**        |  147.8ms |  **54.5ms**   | **2.7x** |   22 MB |    26 MB |      1.16x |
| **strings**       |  141.3ms |  **59.0ms**   | **2.4x** |   14 MB |    14 MB |      1.02x |
| **numbers**       |  268.7ms | **127.3ms**   | **2.1x** |   87 MB |    15 MB |      0.18x |
| **pathological**  |  796.7ms | **410.2ms**   | **1.9x** |  472 MB |    17 MB |      0.04x |
| **comprehensive** |  381.9ms | **220.9ms**   | **1.7x** |  149 MB |    14 MB |      0.09x |
| **users**         |  205.6ms | **125.4ms**   | **1.6x** |   68 MB |    12 MB |      0.18x |
| **unicode**       |  169.4ms | **113.0ms**   | **1.5x** |   31 MB |    16 MB |      0.51x |
| **arrays**        |  738.2ms | **388.9ms**   | **1.9x** |  367 MB |    17 MB |      0.05x |
| **literals**      |  279.5ms | **228.7ms**   | **1.2x** |   50 MB |    16 MB |      0.33x |

#### ARM (Apple M1 Max, 10MB files)

| Pattern           | jq       | succinctly    | Speedup  | jq Mem  | succ Mem | Mem Ratio  |
|-------------------|----------|---------------|----------|---------|----------|------------|
| **nested**        |  347.2ms |  **54.8ms**   | **6.3x** |   25 MB |    29 MB |      1.17x |
| **strings**       |  331.0ms |  **75.2ms**   | **4.4x** |   16 MB |    17 MB |      1.07x |
| **pathological**  |    1.35s | **677.0ms**   | **2.0x** |  526 MB |    20 MB |      0.04x |
| **unicode**       |  334.8ms | **157.1ms**   | **2.1x** |   41 MB |    19 MB |      0.46x |
| **users**         |  413.7ms | **196.8ms**   | **2.1x** |   70 MB |    15 MB |      0.21x |
| **comprehensive** |  692.4ms | **364.7ms**   | **1.9x** |  135 MB |    17 MB |      0.13x |
| **numbers**       |  370.7ms | **213.8ms**   | **1.7x** |   97 MB |    18 MB |      0.19x |
| **arrays**        |    1.07s | **700.8ms**   | **1.5x** |  368 MB |    20 MB |      0.05x |
| **literals**      |  510.9ms | **397.1ms**   | **1.3x** |  103 MB |    19 MB |      0.19x |

**Key findings**:
- **1.2-6.3x faster** across all patterns
- **5-25x less memory** on most patterns due to streaming lazy evaluation
- Best speedup on nested/string-heavy data

### Platform-Specific Optimizations

| Platform   | Operation                    | Throughput | Speedup         |
|------------|------------------------------|------------|-----------------|
| **x86_64** | Popcount (AVX-512 VPOPCNTDQ) | 96.8 GiB/s |  5.2x vs scalar |
| **ARM**    | NEON JSON (string-heavy)     |  3.7 GiB/s | 1.69x vs scalar |

See [docs/OPTIMIZATION-SUMMARY.md](docs/OPTIMIZATION-SUMMARY.md) and [docs/jq-comparison.md](docs/jq-comparison.md) for detailed benchmarks.

## Feature Flags

### Popcount Strategies (mutually exclusive)

| Feature             | Description                                           |
|---------------------|-------------------------------------------------------|
| *(default)*         | Uses Rust's `count_ones()` which auto-vectorizes      |
| `simd`.             | Explicit SIMD intrinsics (NEON on ARM, POPCNT on x86) |
| `portable-popcount` | Portable bitwise algorithm (no intrinsics)            |

### Other Features

| Feature | Description                                                      |
|---------|------------------------------------------------------------------|
| `std`   | Enable std library (default, required for runtime CPU detection) |
| `serde` | Enable serialization/deserialization support                     |
| `cli`   | Build the CLI tool                                               |
| `regex` | Enable regex support in jq queries                               |

### Test Features

| Feature       | Description                           |
|---------------|---------------------------------------|
| `large-tests` | Test with 1GB bitvectors (~125MB RAM) |
| `huge-tests`  | Test with 5GB bitvectors (~625MB RAM) |
| `mmap-tests`  | Memory-mapped file tests              |

## CLI Tool

The library includes a CLI tool for JSON operations:

```bash
# Build the CLI
cargo build --release --features cli

# Generate synthetic JSON for benchmarking
./target/release/succinctly json generate 10mb -o benchmark.json

# Query JSON files (jq-compatible output by default)
./target/release/succinctly jq '.users[].name' input.json
./target/release/succinctly jq -r '.users[0]' input.json

# Preserve original input formatting (numbers, escapes)
./target/release/succinctly jq --preserve-input . input.json
```

## Architecture

### Module Structure

```
succinctly
├── bits         # Bitvector with rank/select
├── trees        # Tree encodings (balanced parentheses)
├── json         # JSON semi-indexing
└── jq           # jq query language
```

### Core Data Structures

- **`bits::BitVec`** - Bitvector with 3-level Poppy-style rank directory (~3% overhead) and sampled select index (~1-3% overhead)
- **`trees::BalancedParens`** - Hierarchical min-excess structure for O(1) tree navigation (~6% overhead)
- **`json::JsonIndex`** - Semi-index combining Interest Bits (IB) and Balanced Parentheses (BP) for fast JSON navigation

### SIMD Support

The library uses runtime CPU feature detection to select the best implementation:

| Platform | Features Used                         |
|----------|---------------------------------------|
| x86_64   | AVX2, AVX-512 VPOPCNTDQ, SSE4.2, SSE2 |
| aarch64  | NEON (mandatory)                      |

## Documentation

- [API Documentation](https://docs.rs/succinctly) - Full API reference on docs.rs
- [CLAUDE.md](CLAUDE.md) - Detailed architecture guide
- [docs/OPTIMIZATION-SUMMARY.md](docs/OPTIMIZATION-SUMMARY.md) - Performance optimization history

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
