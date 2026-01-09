# User Guide

This guide covers how to use succinctly in your Rust projects.

## Installation

Add succinctly to your `Cargo.toml`:

```toml
[dependencies]
succinctly = "0.1"
```

Or with cargo:

```bash
cargo add succinctly
```

## Bitvector with Rank/Select

The `BitVec` type provides a bitvector with O(1) rank and O(log n) select operations.

### Creating a BitVec

```rust
use succinctly::{BitVec, RankSelect};

// From u64 words (most common)
let words = vec![0b1010_1010u64, 0b1111_0000u64];
let bv = BitVec::from_words(words, 128);

// From bytes
let bytes = vec![0b1010_1010u8, 0b1111_0000u8];
let bv = BitVec::from_bytes(&bytes, 16);
```

### Rank Queries

Rank counts the number of 1-bits (or 0-bits) before a given position:

```rust
use succinctly::{BitVec, RankSelect};

let bv = BitVec::from_words(vec![0b1010_1010u64], 8);

// rank1(i) = count of 1-bits in [0, i)
assert_eq!(bv.rank1(0), 0);  // No bits before position 0
assert_eq!(bv.rank1(2), 1);  // One 1-bit in positions 0-1
assert_eq!(bv.rank1(8), 4);  // Four 1-bits in positions 0-7

// rank0(i) = count of 0-bits in [0, i)
assert_eq!(bv.rank0(8), 4);  // Four 0-bits in positions 0-7
```

### Select Queries

Select finds the position of the k-th 1-bit (0-indexed):

```rust
use succinctly::{BitVec, RankSelect};

let bv = BitVec::from_words(vec![0b1010_1010u64], 8);

// select1(k) = position of k-th 1-bit
assert_eq!(bv.select1(0), Some(1));  // First 1-bit at position 1
assert_eq!(bv.select1(1), Some(3));  // Second 1-bit at position 3
assert_eq!(bv.select1(4), None);     // No fifth 1-bit
```

### Configuration

You can customize the select sample rate:

```rust
use succinctly::{BitVec, Config};

let config = Config {
    select_sample_rate: 512,  // Sample every 512th 1-bit (default: 256)
};

let words = vec![0xFFFFFFFFFFFFFFFFu64; 1000];
let bv = BitVec::from_words_with_config(words, 64000, config);
```

## Balanced Parentheses

The `BalancedParens` type encodes trees as balanced parentheses for succinct navigation.

### Tree Encoding

Trees are encoded with 1=open and 0=close:

```
Tree:       A
           /|\
          B C D
          |
          E

Encoding: A(B(E())C()D())
Bits:     1 1 1 0 0 1 0 1 0 0
          A B E     C   D
```

### Creating BalancedParens

```rust
use succinctly::bp::BalancedParens;

// From bits: ((()())())  ->  1110100100
let bp = BalancedParens::new(&[0b0010010111], 10);
```

### Navigation Operations

```rust
use succinctly::bp::BalancedParens;

let bp = BalancedParens::new(&[0b0010010111], 10);

// Find matching parenthesis
assert_eq!(bp.find_close(0), Some(9));  // Match for outermost (
assert_eq!(bp.find_open(9), Some(0));   // Match for outermost )

// Tree navigation
assert_eq!(bp.first_child(0), Some(1));   // First child of root
assert_eq!(bp.next_sibling(1), Some(7));  // Next sibling
assert_eq!(bp.parent(1), Some(0));        // Parent node

// Subtree size (number of nodes)
assert_eq!(bp.subtree_size(0), 5);  // Entire tree
assert_eq!(bp.subtree_size(1), 3);  // Left subtree
```

## JSON Semi-Indexing

Semi-indexing allows fast navigation of JSON without fully parsing it.

### Building an Index

```rust
use succinctly::json::{JsonIndex, StandardJson};

let json = br#"{"name": "Alice", "age": 30}"#;
let index = JsonIndex::build(json);
```

### Navigating with Cursors

```rust
use succinctly::json::{JsonIndex, StandardJson};

let json = br#"{"users": [{"name": "Alice"}, {"name": "Bob"}]}"#;
let index = JsonIndex::build(json);
let root = index.root(json);

match root {
    StandardJson::Object(obj) => {
        if let Some(StandardJson::Array(users)) = obj.get("users") {
            for user in users {
                if let StandardJson::Object(u) = user {
                    if let Some(StandardJson::String(name)) = u.get("name") {
                        println!("User: {}", name);
                    }
                }
            }
        }
    }
    _ => {}
}
```

### Memory-Mapped Files

For large files, use memory mapping:

```rust
use succinctly::json::JsonIndex;
use std::fs::File;
use memmap2::Mmap;

let file = File::open("large.json")?;
let mmap = unsafe { Mmap::map(&file)? };
let index = JsonIndex::build(&mmap);
let root = index.root(&mmap);
```

## jq Query Language

Succinctly supports a subset of jq for querying JSON.

### Basic Queries

```rust
use succinctly::jq::{parse, eval, QueryResult};
use succinctly::json::{JsonIndex, StandardJson};

let json = br#"{"users": [{"name": "Alice", "age": 30}, {"name": "Bob", "age": 25}]}"#;
let index = JsonIndex::build(json);
let cursor = index.root(json);

// Identity - returns entire document
let expr = parse(".").unwrap();

// Field access
let expr = parse(".users").unwrap();

// Array index
let expr = parse(".users[0]").unwrap();

// Negative index (from end)
let expr = parse(".users[-1]").unwrap();

// Chained access
let expr = parse(".users[0].name").unwrap();
```

### Iteration

```rust
// Iterate all array elements
let expr = parse(".users[]").unwrap();
if let QueryResult::Many(users) = eval(&expr, cursor) {
    for user in users {
        // Process each user
    }
}

// Iterate and extract field
let expr = parse(".users[].name").unwrap();
// Returns: ["Alice", "Bob"]
```

### Slicing

```rust
// Slice from index 1 to 3
let expr = parse(".items[1:3]").unwrap();

// Slice from start to index 2
let expr = parse(".items[:2]").unwrap();

// Slice from index 2 to end
let expr = parse(".items[2:]").unwrap();
```

### Optional Access

```rust
// Returns null instead of error if field missing
let expr = parse(".missing_field?").unwrap();

// Safe navigation chain
let expr = parse(".users[0].address?.city").unwrap();
```

### Arithmetic and Comparison

```rust
// Arithmetic
let expr = parse(".a + .b").unwrap();
let expr = parse(".price * .quantity").unwrap();

// Comparison
let expr = parse(".age >= 18").unwrap();
let expr = parse(".name == \"Alice\"").unwrap();

// Boolean operators
let expr = parse(".active and .verified").unwrap();
let expr = parse(".admin or .moderator").unwrap();
```

### Conditionals

```rust
// if-then-else
let expr = parse("if .age >= 18 then \"adult\" else \"minor\" end").unwrap();

// Alternative (default value)
let expr = parse(".nickname // .name").unwrap();  // Use name if nickname is null
```

### Construction

```rust
// Array construction
let expr = parse("[.name, .age]").unwrap();

// Object construction
let expr = parse("{user: .name, years: .age}").unwrap();

// Comma operator (multiple outputs)
let expr = parse(".name, .age").unwrap();
```

### Recursive Descent

```rust
// Find all values at any depth
let expr = parse(".. | .name?").unwrap();
```

## CLI Tool

The CLI tool provides command-line access to JSON operations.

### Installation

```bash
cargo install succinctly --features cli
```

Or build from source:

```bash
cargo build --release --features cli
```

### Generating Test Data

```bash
# Generate 10MB of comprehensive JSON
succinctly json generate 10mb -o test.json

# Generate with specific pattern
succinctly json generate 1mb --pattern nested -o nested.json
succinctly json generate 1mb --pattern arrays -o arrays.json
succinctly json generate 1mb --pattern pathological -o worst.json

# Generate benchmark suite
succinctly json generate-suite
succinctly json generate-suite --max-size 100mb
```

### Querying JSON

```bash
# Basic query
succinctly json query '.users[0].name' -i data.json

# Multiple results
succinctly json query '.users[].name' -i data.json

# Raw output (no JSON formatting)
succinctly json query '.users[0].name' -i data.json --raw

# Memory-mapped for large files
succinctly json query '.data[]' -i huge.json --mmap
```

## Feature Flags

### Popcount Strategies

```toml
# Default: uses Rust's count_ones()
succinctly = "0.1"

# Explicit SIMD intrinsics
succinctly = { version = "0.1", features = ["simd"] }

# Portable bitwise algorithm
succinctly = { version = "0.1", features = ["portable-popcount"] }
```

### Serialization

```toml
# Enable serde support
succinctly = { version = "0.1", features = ["serde"] }
```

```rust
use succinctly::BitVec;
use serde_json;

let bv = BitVec::from_words(vec![0xFF], 8);
let json = serde_json::to_string(&bv)?;
let bv2: BitVec = serde_json::from_str(&json)?;
```

### no_std Usage

Succinctly is `no_std` compatible (requires `alloc`):

```toml
[dependencies]
succinctly = { version = "0.1", default-features = false }
```

```rust
#![no_std]
extern crate alloc;

use succinctly::{BitVec, RankSelect};
use alloc::vec;

let bv = BitVec::from_words(vec![0xFF], 8);
assert_eq!(bv.rank1(4), 4);
```

## Performance Tips

1. **Reuse indices** - Building `JsonIndex` is O(n), reuse it for multiple queries
2. **Use memory mapping** - For files larger than available RAM
3. **Batch queries** - Parse jq expressions once, evaluate multiple times
4. **Consider bit density** - Select is faster when 1-bits are evenly distributed
5. **Profile first** - Use `cargo bench` to identify actual bottlenecks

## Error Handling

```rust
use succinctly::jq::{parse, ParseError};

match parse(".invalid[") {
    Ok(expr) => { /* use expression */ }
    Err(ParseError::UnexpectedEnd) => { /* handle error */ }
    Err(e) => { /* handle other errors */ }
}
```

## Getting Help

- **Documentation**: [docs.rs/succinctly](https://docs.rs/succinctly)
- **Issues**: [GitHub Issues](https://github.com/rust-works/succinctly/issues)
- **Discussions**: [GitHub Discussions](https://github.com/rust-works/succinctly/discussions)
