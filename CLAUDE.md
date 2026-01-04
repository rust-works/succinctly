# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

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

# See CLI.md for full documentation
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
- State machine-based `find_close` for O(1) operations via hierarchical search
- Tree operations: `find_close`, `find_open`, `enclose`, `first_child`, `next_sibling`, `parent`, `excess`

**BP Operations**
- `find_unmatched_close_in_word(x)`: Find first unmatched `)` in a word (linear scan)
- `find_close_in_word(word, p)`: Word-level matching for `(` at position `p`
- `find_close(words, len, p)`: Vector-level linear scan (O(n), used by `find_open`/`enclose`)
- `find_open(words, len, p)`: Find matching `(` for `)` at position `p` (linear scan backwards)
- `enclose(words, len, p)`: Find enclosing `(` (parent node)
- `BalancedParens::find_close(p)`: Accelerated O(1) using RangeMin state machine
- `BalancedParens::find_open(p)`: Currently uses linear scan (acceleration TODO)

**BP Performance** (from benchmarks)
- RangeMin `find_close` is ~40x faster than linear scan for 1M-node trees
- Construction overhead: ~22µs for 10K nodes, ~220µs for 100K nodes
- Deep nesting (worst case): RangeMin ~450ns vs linear ~17µs

### JSON Semi-Indexing

**JSON Module** ([src/json/mod.rs](src/json/mod.rs))
- Converts JSON text to Interest Bits (IB) and Balanced Parentheses (BP) vectors
- Two cursor implementations:
  - `simple`: 3-state machine, marks all structural characters
  - `standard`: 4-state machine, marks structural characters + value starts
- SIMD acceleration with runtime dispatch to best available instruction set:
  - x86_64: AVX2 (32 bytes) > SSE4.2 (16 bytes) > SSE2 (16 bytes)
  - aarch64: NEON (16 bytes, mandatory)

**SIMD Module Structure** ([src/json/simd/](src/json/simd/))
- `x86.rs`: SSE2 baseline (16-byte chunks, universal on x86_64)
- `sse42.rs`: SSE4.2 with PCMPISTRI string instructions (~90% availability)
- `avx2.rs`: AVX2 256-bit processing (32-byte chunks, ~95% availability)
- `bmi2.rs`: BMI2 PDEP/PEXT utilities (note: AMD Zen 1/2 have slow microcode)
- `neon.rs`: ARM NEON (16-byte chunks, mandatory on aarch64)
- `mod.rs`: Runtime CPU feature detection and dispatch

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

**Property tests**: [tests/property_tests.rs](tests/property_tests.rs), [tests/properties.rs](tests/properties.rs), [tests/bp_properties.rs](tests/bp_properties.rs)
- Uses `proptest` for randomized testing
- Verifies rank/select consistency, BP operations correctness
- BP tests verify: roundtrip invariants, RangeMin matches linear scan, tree navigation consistency

**Integration tests**:
- [tests/json_indexing_tests.rs](tests/json_indexing_tests.rs): JSON parsing
- [tests/bp_coverage_tests.rs](tests/bp_coverage_tests.rs): BP edge cases
- [tests/bitread_tests.rs](tests/bitread_tests.rs): Bit-level reading
- [tests/simd_level_tests.rs](tests/simd_level_tests.rs): Cross-level SIMD validation

## `no_std` Support

The library is `no_std` compatible (except in tests):
- Uses `#![cfg_attr(not(test), no_std)]`
- Depends on `alloc` for `Vec<u64>` storage

## Performance Considerations

- Bit positions are 0-indexed from LSB in each u64 word
- Words are stored little-endian (bit 0 is LSB of first word)
- Rank directory lookups are cache-friendly (sequential access)
- Select uses exponential search + binary search for optimal cache behavior
- SIMD implementations process 16-32 bytes per iteration with runtime dispatch

## Key SIMD Implementation Learnings

### Compilation Model
- **`#[target_feature]` is a compiler directive**, not a runtime gate
- All SIMD levels (SSE2, SSE4.2, AVX2) compile into single binary on any x86_64 host
- Each function gets separate code generation with specific instructions
- Running unsupported code without runtime guards causes SIGILL crash
- Runtime dispatch via `is_x86_feature_detected!()` prevents crashes

### Testing Strategy
- **Problem**: Runtime dispatch only tests highest available SIMD level
- **Solution**: [tests/simd_level_tests.rs](tests/simd_level_tests.rs) explicitly calls each implementation
- Force-test SSE2, SSE4.2, AVX2 by calling module functions directly
- Validate all produce identical results against scalar reference
- Critical for catching bugs in lower SIMD levels on modern hardware

### SIMD Instruction Set Hierarchy
| Level   | Width  | Bytes/Iter | Availability | Notes                           |
|---------|--------|------------|--------------|----------------------------------|
| SSE2    | 128bit | 16         | 100%         | Universal baseline on x86_64     |
| SSE4.2  | 128bit | 16         | ~90%         | PCMPISTRI string instructions    |
| AVX2    | 256bit | 32         | ~95%         | 2x width, best price/performance |
| BMI2    | N/A    | N/A        | ~95%         | PDEP/PEXT, but AMD Zen 1/2 slow  |

### BMI2 Considerations
- **Intel Haswell+**: 3-cycle PDEP/PEXT (fast)
- **AMD Zen 1/2**: 18-cycle microcode (slower than scalar)
- **AMD Zen 3+**: 3-cycle hardware (fast)
- Provide utilities but don't force usage - let users opt-in

### no_std Constraints
- `is_x86_feature_detected!()` requires std (not available in no_std)
- Test builds use std for runtime dispatch validation
- Production no_std builds can call specific SIMD modules explicitly
- Default to SSE2 in no_std or provide feature flags for manual selection

### Character Classification Pattern
```rust
// SSE2/SSE4.2: 128-bit, 16-byte masks
let quote_mask = _mm_movemask_epi8(eq_quote) as u16;

// AVX2: 256-bit, 32-byte masks
let quote_mask = _mm256_movemask_epi8(eq_quote) as u32;

// SSE4.2 string search (finds multiple chars in one instruction)
let structural_mask = _mm_cmpistrm(structural_chars, chunk, MODE);
```

### CI/CD Best Practices
- Mirror CI checks locally: `cargo clippy --all-targets --all-features -- -D warnings`
- `-D warnings` treats warnings as errors (enforced in GitHub Actions)
- Test all feature combinations before pushing
- Periodically update Rust toolchain to catch new lints and idiomatic patterns

### Rust Version Updates

**Key toolchain considerations**:
- New Clippy lints can break CI even when code is semantically unchanged
- Rust 1.92.0 introduced `manual_is_multiple_of` lint suggesting `.is_multiple_of()` method
- Example: `bytes.len() % 8 == 0` → `bytes.len().is_multiple_of(8)`
- Regular toolchain updates help adopt new idiomatic patterns early
- Consider `rust-toolchain.toml` for reproducible builds across environments

## jq Query Module

### Current Implementation ([src/jq/](src/jq/))
- `expr.rs`: AST for jq expressions
- `parser.rs`: Recursive descent parser for jq syntax
- `eval.rs`: Expression evaluator using cursor-based navigation
- `mod.rs`: Public API exports

**Supported jq syntax**:
- `.` - Identity
- `.foo` - Field access
- `.[n]` - Array index (positive and negative)
- `.[n:m]`, `.[n:]`, `.[:m]` - Array slicing
- `.[]` - Iterate all elements
- `.foo?` - Optional access (returns nothing instead of error)
- `.foo.bar`, `.foo[0].bar` - Chained/piped expressions

**CLI usage**:
```bash
# Query JSON files
./target/release/succinctly json query '.users[].name' input.json
./target/release/succinctly json query '.users[0]' input.json --raw  # Raw string output
./target/release/succinctly json query '.items[]' input.json --mmap  # Memory-mapped input
```

### Future Implementation Plan
See [docs/jq-implementation-plan.md](docs/jq-implementation-plan.md) for comprehensive roadmap including:
- Comma operator, parentheses, recursive descent
- Arithmetic/comparison operators
- Conditionals (if-then-else, try-catch)
- Builtin functions (type, length, keys, map, select, sort, etc.)
- Variables and user-defined functions

## Performance Optimization Learnings

### O(1) vs O(n) Operations - Critical for Query Performance

**Problem discovered**: Initial jq query implementation was 17x slower than jq (2.74s vs 0.16s for `.unicode[]` on 10MB file).

**Root cause**: `text_position()` called `ib_select1()` which was O(n) linear scan per result.

**Solution**: Add cumulative popcount index for O(log n) select via binary search.

```rust
// JsonIndex now includes:
ib_rank: Vec<u32>,  // Cumulative popcount per word

// Build cumulative index during construction:
fn build_ib_rank(words: &[u64]) -> Vec<u32> {
    let mut rank = Vec::with_capacity(words.len() + 1);
    let mut cumulative: u32 = 0;
    rank.push(0);
    for &word in words {
        cumulative += word.count_ones();
        rank.push(cumulative);
    }
    rank
}

// Binary search for select:
fn ib_select1(&self, k: usize) -> Option<usize> {
    let k32 = k as u32;
    let mut lo = 0usize;
    let mut hi = words.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if self.ib_rank[mid + 1] <= k32 { lo = mid + 1; }
        else { hi = mid; }
    }
    // Then scan within word for exact bit position
}
```

**Result**: 627x speedup (2.76s → 4.4ms), now 5x faster than jq.

### Key Insight: Hierarchical Indices
When you have O(n) operations that get called O(n) times, total complexity becomes O(n²). Solutions:
1. **Cumulative indices**: Store running totals for O(log n) binary search
2. **Hierarchical structure**: L0/L1/L2 checkpoints (like Poppy rank directory)
3. **Sampling**: Trade space for time with sampled indices

### Zero-Copy Output Pattern
For CLI output, use `raw_bytes()` to avoid string allocation:
```rust
// Zero-copy for strings and numbers:
StandardJson::String(s) => out.write_all(s.raw_bytes())?,
StandardJson::Number(n) => out.write_all(n.raw_bytes())?,

// Use BufWriter for buffered I/O:
let mut out = BufWriter::new(stdout.lock());
```

## Benchmark Infrastructure

### JSON SIMD Benchmarks ([benches/json_simd.rs](benches/json_simd.rs))
- Auto-discovers files from `data/bench/generated/`
- Sorts by pattern name and file size
- Limits to files ≤100MB for reasonable runtime
- Compares SIMD implementations: AVX2, SSE4.2, SSE2, NEON, Scalar

**Generating benchmark data**:
```bash
./target/release/succinctly json generate-suite
./target/release/succinctly json generate-suite --max-size 10mb  # Smaller set
```

**Running benchmarks**:
```bash
cargo bench --bench json_simd
cargo bench --bench json_simd -- pattern_comparison  # Compare patterns at 10MB
```

### Benchmark Patterns
| Pattern | Description | Characteristics |
|---------|-------------|-----------------|
| comprehensive | Mixed content | Realistic workload |
| users | User records | Nested objects |
| nested | Deep nesting | Tests BP operations |
| arrays | Large arrays | Tests iteration |
| mixed | Varied structure | Edge cases |
| strings | String-heavy | Tests escape handling |
| numbers | Number-heavy | Tests number parsing |
| literals | true/false/null | Tests literal detection |
| unicode | Unicode strings | Tests UTF-8 handling |
| pathological | Worst-case | Deep nesting, escapes |
