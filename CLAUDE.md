# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## AI Scratch Directory

When working on this project, use `.ai/scratch/` for temporary files:

- **Location**: `.ai/scratch/` in the repository root
- **Purpose**: Store intermediate work, drafts, analysis notes, generated data
- **Ignored by git**: The `.ai/` directory is in `.gitignore`

**Usage examples**:
- Draft documentation before finalizing
- Store benchmark results for comparison
- Keep notes about investigation/debugging sessions
- Save generated test data temporarily

Create the directory if it doesn't exist:
```bash
mkdir -p .ai/scratch
```

## Project Overview

Succinctly is a high-performance Rust library implementing succinct data structures with fast rank and select operations, optimized for both x86_64 (POPCNT) and ARM (NEON) architectures.

## Common Commands

### Building and Testing
```bash
# Standard build
cargo build

# Build with specific popcount strategy
cargo build --features simd
cargo build --features portable-popcount

# Run tests
cargo test

# Run tests with large bitvectors
cargo test --features large-tests
cargo test --features huge-tests
cargo test --features mmap-tests

# Run benchmarks
cargo bench

# Run property tests (longer-running)
cargo test --test property_tests
cargo test --test properties
```

### Testing Individual Components
```bash
# Test specific module
cargo test bitvec
cargo test bp
cargo test json

# Test a single test function
cargo test test_rank1_simple

# Test all SIMD levels explicitly
cargo test --test simd_level_tests

# Run benchmarks for specific operation
cargo bench rank_select
cargo bench json_simd
cargo bench balanced_parens
```

### CLI Tool

```bash
# Build CLI tool
cargo build --release --features cli

# Generate synthetic JSON for benchmarking
./target/release/succinctly json generate 10mb -o benchmark.json
./target/release/succinctly json generate 1mb --pattern pathological -o worst-case.json

# Query JSON files
./target/release/succinctly json query '.users[].name' -i input.json
./target/release/succinctly json query '.users[0]' -i input.json --raw
./target/release/succinctly json query '.items[]' -i input.json --mmap
```

## Code Architecture

### Core Data Structures

**BitVec** ([src/bitvec.rs](src/bitvec.rs))
- Main bitvector with rank/select support
- Memory layout: raw words (`Vec<u64>`), rank directory (~3% overhead), select index (~1-3% overhead)
- Uses 3-level Poppy-style rank directory (L0/L1/L2) for O(1) rank queries
- Select uses sampled index with configurable sample rate (default: 256)

**RankSelect trait** ([src/lib.rs](src/lib.rs))
- `rank1(i)`: Count 1-bits in `[0, i)` - O(1)
- `select1(k)`: Find position of k-th 1-bit - O(log n) with acceleration
- `rank0(i)`: Count 0-bits in `[0, i)` - O(1) (computed as `i - rank1(i)`)

### Balanced Parentheses

**BalancedParens** ([src/bp.rs](src/bp.rs))
- Succinct tree navigation using balanced parentheses encoding (1=open, 0=close)
- RangeMin structure with hierarchical min-excess indices (~6% overhead)
- Tree operations: `find_close`, `find_open`, `enclose`, `first_child`, `next_sibling`, `parent`, `excess`

### JSON Semi-Indexing

**JSON Module** ([src/json/mod.rs](src/json/mod.rs))
- Converts JSON text to Interest Bits (IB) and Balanced Parentheses (BP) vectors
- Two cursor implementations: `simple` (3-state) and `standard` (4-state)
- SIMD acceleration: AVX2 > SSE4.2 > SSE2 on x86_64, NEON on aarch64

**SIMD Module Structure** ([src/json/simd/](src/json/simd/))
- `x86.rs`: SSE2 baseline
- `sse42.rs`: SSE4.2 with PCMPISTRI
- `avx2.rs`: AVX2 256-bit processing
- `neon.rs`: ARM NEON
- `mod.rs`: Runtime CPU feature detection

### jq Query Module

**Current Implementation** ([src/jq/](src/jq/))
- `expr.rs`: AST for jq expressions
- `parser.rs`: Recursive descent parser
- `eval.rs`: Expression evaluator using cursor-based navigation

**Supported jq syntax**:
- `.` - Identity
- `.foo` - Field access
- `.[n]` - Array index (positive and negative)
- `.[n:m]`, `.[n:]`, `.[:m]` - Array slicing
- `.[]` - Iterate all elements
- `.foo?` - Optional access
- `.foo.bar`, `.foo[0].bar` - Chained expressions

## Feature Flags

**Popcount strategies** (mutually exclusive):
- Default: Rust's built-in `count_ones()`
- `simd`: Explicit SIMD intrinsics
- `portable-popcount`: Portable bitwise algorithm

**Other features**:
- `large-tests`: Test with 1GB bitvectors
- `huge-tests`: Test with 5GB bitvectors
- `mmap-tests`: Memory-mapped file tests

## Testing Strategy

**Unit tests**: In each module's `#[cfg(test)] mod tests`

**Property tests**:
- [tests/property_tests.rs](tests/property_tests.rs)
- [tests/properties.rs](tests/properties.rs)
- [tests/bp_properties.rs](tests/bp_properties.rs)

**Integration tests**:
- [tests/json_indexing_tests.rs](tests/json_indexing_tests.rs)
- [tests/bp_coverage_tests.rs](tests/bp_coverage_tests.rs)
- [tests/simd_level_tests.rs](tests/simd_level_tests.rs)

## `no_std` Support

The library is `no_std` compatible (except in tests):
- Uses `#![cfg_attr(not(test), no_std)]`
- Depends on `alloc` for `Vec<u64>` storage

## Benchmark Infrastructure

```bash
# Generate benchmark data
./target/release/succinctly json generate-suite
./target/release/succinctly json generate-suite --max-size 10mb

# Run benchmarks
cargo bench --bench json_simd
cargo bench --bench balanced_parens
```

### Benchmark Patterns
| Pattern | Description |
|---------|-------------|
| comprehensive | Mixed content (realistic) |
| users | User records (nested objects) |
| nested | Deep nesting (tests BP) |
| arrays | Large arrays (tests iteration) |
| strings | String-heavy (tests escapes) |
| unicode | Unicode strings |
| pathological | Worst-case |

## CI/CD

```bash
# Mirror CI checks locally
cargo clippy --all-targets --all-features -- -D warnings
cargo test
./scripts/build.sh
```
