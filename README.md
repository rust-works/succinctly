# succinctly

[![CI](https://github.com/rust-works/succinctly/actions/workflows/rust.yml/badge.svg)](https://github.com/rust-works/succinctly/actions/workflows/rust.yml)
[![crates.io](https://img.shields.io/crates/v/succinctly.svg)](https://crates.io/crates/succinctly)
[![docs.rs](https://docs.rs/succinctly/badge.svg)](https://docs.rs/succinctly)
[![License](https://img.shields.io/crates/l/succinctly.svg)](LICENSE)

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

| Platform | Implementation | Throughput | Notes |
|----------|----------------|------------|-------|
| **x86_64** (AMD Ryzen 9 7950X) | PFSM (BMI2) | **679 MiB/s** | Table-driven, fastest on x86 |
| | AVX2 | 732 MiB/s | 32 bytes/iteration |
| | Scalar | 494 MiB/s | Baseline |
| **ARM** (Apple M1 Max) | NEON | **574 MiB/s** | 32 bytes/iteration |
| | Scalar (PFSM) | 557 MiB/s | Table-driven |
| | Scalar | 405 MiB/s | Baseline |

### Rank/Select Operations

| Platform | Rank (O(1)) | Select (O(log n)) |
|----------|-------------|-------------------|
| **x86_64** (AMD Ryzen 9 7950X) | ~3 ns | ~50 ns |
| **ARM** (Apple M1 Max) | ~21 ns | ~320 ns |

### Platform-Specific Optimizations

| Platform | Operation | Throughput | Speedup |
|----------|-----------|------------|---------|
| **x86_64** | Popcount (AVX-512 VPOPCNTDQ) | 96.8 GiB/s | 5.2x vs scalar |
| **ARM** | NEON JSON (string-heavy) | 3.7 GiB/s | 1.69x vs scalar |

See [docs/OPTIMIZATION-SUMMARY.md](docs/OPTIMIZATION-SUMMARY.md) for detailed benchmarks.

## Feature Flags

### Popcount Strategies (mutually exclusive)

| Feature | Description |
|---------|-------------|
| *(default)* | Uses Rust's `count_ones()` which auto-vectorizes |
| `simd` | Explicit SIMD intrinsics (NEON on ARM, POPCNT on x86) |
| `portable-popcount` | Portable bitwise algorithm (no intrinsics) |

### Other Features

| Feature | Description |
|---------|-------------|
| `std` | Enable std library (default, required for runtime CPU detection) |
| `serde` | Enable serialization/deserialization support |
| `cli` | Build the CLI tool |
| `regex` | Enable regex support in jq queries |

### Test Features

| Feature | Description |
|---------|-------------|
| `large-tests` | Test with 1GB bitvectors (~125MB RAM) |
| `huge-tests` | Test with 5GB bitvectors (~625MB RAM) |
| `mmap-tests` | Memory-mapped file tests |

## CLI Tool

The library includes a CLI tool for JSON operations:

```bash
# Build the CLI
cargo build --release --features cli

# Generate synthetic JSON for benchmarking
./target/release/succinctly json generate 10mb -o benchmark.json

# Query JSON files (jq-compatible)
./target/release/succinctly jq '.users[].name' input.json
./target/release/succinctly jq -r '.users[0]' input.json
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

| Platform | Features Used |
|----------|---------------|
| x86_64 | AVX2, AVX-512 VPOPCNTDQ, SSE4.2, SSE2 |
| aarch64 | NEON (mandatory) |

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

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE](LICENSE) or http://opensource.org/licenses/MIT)

at your option.

## Acknowledgments

This library implements algorithms from:

- Vigna, "Broadword Implementation of Rank/Select Queries" (2008)
- Zhou et al., "Space-Efficient, High-Performance Rank & Select" (Poppy, 2013)
- Sadakane & Navarro, "Fully-Functional Succinct Trees" (2010)
