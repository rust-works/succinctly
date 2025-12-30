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

# Run benchmarks for specific operation
cargo bench rank_select
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
- `rank0(i)` and `select0(k)`: Corresponding operations for 0-bits

### Rank/Select Implementation Details

**Rank Directory** ([src/rank.rs](src/rank.rs))
- Three-level hierarchical structure:
  - L0: Per-word (64 bits) cumulative popcount
  - L1: Per-512-bits (8 words) checkpoint
  - L2: Per-32768-bits (512 words) checkpoint
- Enables O(1) rank by combining directory lookup with partial word popcount

**Select Index** ([src/select.rs](src/select.rs))
- Samples every N 1-bits (configurable, default 256) to store word positions
- Select query: binary search samples + linear scan within word
- Trade-off between space and query time via `Config::select_sample_rate`

### Balanced Parentheses

**BalancedParens** ([src/bp.rs](src/bp.rs))
- Succinct tree navigation using balanced parentheses encoding (1=open, 0=close)
- RangeMin structure with hierarchical min-excess indices (~6% overhead)
  - L0: 2 bytes per word (min_excess + cum_excess)
  - L1: 4 bytes per 32 words
  - L2: 4 bytes per 1024 words
- State machine-based find_close/find_open for O(1) amortized operations
- Tree operations: `find_close`, `find_open`, `enclose`, `first_child`, `next_sibling`, `depth`, `subtree_size`

**BP Operations**
- `find_close_in_word(word, p)`: Word-level matching using linear scan
- `find_close(words, len, p)`: Vector-level linear scan (fallback/testing)
- `BalancedParens::find_close(p)`: Accelerated using RangeMin state machine

### JSON Semi-Indexing

**JSON Module** ([src/json/mod.rs](src/json/mod.rs))
- Converts JSON text to Interest Bits (IB) and Balanced Parentheses (BP) vectors
- Two cursor implementations:
  - `simple`: 3-state machine, marks all structural characters
  - `standard`: 4-state machine, marks structural characters + value starts
- SIMD acceleration available via `simd` submodule (NEON on ARM, SSE on x86)

### Popcount Strategies

**Popcount Module** ([src/popcount.rs](src/popcount.rs))
- Default: Uses Rust's `count_ones()` (auto-vectorizes on most platforms)
- `simd` feature: Explicit SIMD intrinsics (NEON on ARM, POPCNT on x86)
- `portable-popcount` feature: Bitwise algorithm for comparison/portability
- All strategies are mutually exclusive for benchmarking

**SIMD Module** ([src/simd/mod.rs](src/simd/mod.rs))
- Platform-specific SIMD implementations
- `x86.rs`: SSE/AVX POPCNT intrinsics
- `neon.rs`: ARM NEON intrinsics

### Broadword Algorithms

**Broadword** ([src/broadword.rs](src/broadword.rs))
- `select_in_word(word, k)`: Find position of k-th 1-bit within a single u64
- Used by select operations for final bit position within target word

## Configuration

**Config struct** ([src/lib.rs](src/lib.rs))
- `select_sample_rate`: Controls select index density (default: 256)
  - Lower = faster select, more memory
  - Higher = slower select, less memory
- `build_select0`: Whether to build dedicated select0 index (default: false)
  - Currently unused; select0 uses linear scan

## Feature Flags

**Popcount strategies** (mutually exclusive):
- Default: Rust's built-in `count_ones()`
- `simd`: Explicit SIMD intrinsics
- `portable-popcount`: Portable bitwise algorithm

**Other features**:
- `select0`: Enable select0 index (increases memory)
- `large-tests`: Test with 1GB bitvectors (~125MB RAM)
- `huge-tests`: Test with 5GB bitvectors (~625MB RAM)
- `mmap-tests`: Memory-mapped file tests (requires `memmap2` and `tempfile`)

## Testing Strategy

**Unit tests**: In each module's `#[cfg(test)] mod tests`
- Test edge cases: empty, single bit, partial words, word boundaries, block boundaries
- Comprehensive coverage of all operations

**Property tests**: [tests/property_tests.rs](tests/property_tests.rs), [tests/properties.rs](tests/properties.rs)
- Uses `proptest` for randomized testing
- Verifies rank/select consistency, BP operations correctness

**Integration tests**:
- [tests/json_indexing_tests.rs](tests/json_indexing_tests.rs): JSON parsing
- [tests/bp_coverage_tests.rs](tests/bp_coverage_tests.rs): BP edge cases
- [tests/bitread_tests.rs](tests/bitread_tests.rs): Bit-level reading

## `no_std` Support

The library is `no_std` compatible (except in tests):
- Uses `#![cfg_attr(not(test), no_std)]`
- Depends on `alloc` for `Vec<u64>` storage

## Performance Considerations

- Bit positions are 0-indexed from LSB in each u64 word
- Words are stored little-endian (bit 0 is LSB of first word)
- Rank directory lookups are cache-friendly (sequential access)
- Select uses exponential search + binary search for optimal cache behavior
- SIMD implementations process 16 bytes at a time on supported platforms
