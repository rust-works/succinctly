# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Documentation Formatting Rules

**Markdown tables MUST use fixed-width columns with space padding.** This is mandatory for readability in plain text editors and diffs.

```markdown
<!-- CORRECT: Fixed-width columns with padding -->
| Size     | succinctly          | jq                  | Speedup    |
|----------|---------------------|---------------------|------------|
| **10KB** | 2.2 ms (4.3 MiB/s)  | 4.1 ms (2.3 MiB/s)  | **1.86x**  |
| **1MB**  | 24.5 ms (33 MiB/s)  | 44 ms (18 MiB/s)    | **1.79x**  |

<!-- WRONG: Unpadded columns -->
| Size | succinctly | jq | Speedup |
|------|------------|-----|---------|
| **10KB** | 2.2 ms (4.3 MiB/s) | 4.1 ms (2.3 MiB/s) | **1.86x** |
```

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

# Query JSON files (jq-compatible interface, output matches jq by default)
./target/release/succinctly jq '.users[].name' input.json
./target/release/succinctly jq -r '.users[0]' input.json
./target/release/succinctly jq '.items[]' input.json

# Preserve original input formatting (numbers, escapes)
./target/release/succinctly jq --preserve-input . input.json
SUCCINCTLY_PRESERVE_INPUT=1 ./target/release/succinctly jq . input.json
```

## Code Architecture

### Module Structure

```
src/
├── lib.rs              # Public API, RankSelect trait, Config
├── bits/               # Bitvector implementations
│   ├── mod.rs          # Re-exports
│   ├── bitvec.rs       # BitVec with rank/select
│   ├── rank.rs         # 3-level Poppy rank directory
│   ├── select.rs       # Sampled select index
│   └── popcount.rs     # Population count strategies
├── trees/              # Tree encodings
│   ├── mod.rs          # Re-exports
│   └── bp.rs           # Balanced parentheses with RangeMin
├── util/               # Internal utilities
│   ├── mod.rs
│   ├── broadword.rs    # Bit manipulation primitives
│   ├── table.rs        # Lookup tables
│   └── simd/           # SIMD utilities
├── json/               # JSON semi-indexing
├── jq/                 # jq query language
├── binary.rs           # Binary I/O
└── bin/                # CLI tool
```

### Public API

```rust
// Recommended imports
use succinctly::bits::BitVec;
use succinctly::trees::BalancedParens;
use succinctly::json::JsonIndex;
use succinctly::jq::{parse, eval};

// Convenience re-exports (also available at root)
use succinctly::{BitVec, BalancedParens, RankSelect};

// Backward compatibility (deprecated, use trees:: instead)
use succinctly::bp::BalancedParens;
```

### Core Data Structures

**BitVec** ([src/bits/bitvec.rs](src/bits/bitvec.rs))
- Main bitvector with rank/select support
- Memory layout: raw words (`Vec<u64>`), rank directory (~3% overhead), select index (~1-3% overhead)
- Uses 3-level Poppy-style rank directory (L0/L1/L2) for O(1) rank queries
- Select uses sampled index with configurable sample rate (default: 256)

**RankSelect trait** ([src/lib.rs](src/lib.rs))
- `rank1(i)`: Count 1-bits in `[0, i)` - O(1)
- `select1(k)`: Find position of k-th 1-bit - O(log n) with acceleration
- `rank0(i)`: Count 0-bits in `[0, i)` - O(1) (computed as `i - rank1(i)`)

### Balanced Parentheses

**BalancedParens** ([src/trees/bp.rs](src/trees/bp.rs))
- Succinct tree navigation using balanced parentheses encoding (1=open, 0=close)
- RangeMin structure with hierarchical min-excess indices (~6% overhead)
- Tree operations: `find_close`, `find_open`, `enclose`, `first_child`, `next_sibling`, `parent`, `excess`

### JSON Semi-Indexing

**JSON Module** ([src/json/mod.rs](src/json/mod.rs))
- Converts JSON text to Interest Bits (IB) and Balanced Parentheses (BP) vectors
- Two cursor implementations: `simple` (3-state) and `standard` (4-state)
- **Default**: PFSM optimized (table-driven, single-pass) - 40-77% faster than scalar
- Multiple acceleration strategies available for comparison

**PFSM Optimized** ([src/json/pfsm_optimized.rs](src/json/pfsm_optimized.rs)) **← DEFAULT (950 MiB/s)**
- **Single-pass table-driven parser** - fastest implementation
- 256-entry lookup tables for state transitions and output bits
- No intermediate allocations - processes data in one pass
- **Performance**: 40-77% faster than standard scalar, beats AVX2 SIMD
- **Why it's fast**:
  - Fewer branches than conditional state machine
  - Excellent cache locality (tables fit in L1)
  - Better instruction-level parallelism
  - No memory allocation overhead
- Used by default in `standard::build_semi_index()`

**SIMD Module Structure** ([src/json/simd/](src/json/simd/))
- `x86.rs`: SSE2 baseline (16 bytes/iteration, universal on x86_64)
- `sse42.rs`: SSE4.2 with PCMPISTRI (16 bytes/iteration, ~90% availability)
- `avx2.rs`: AVX2 256-bit processing (32 bytes/iteration, ~95% availability)
- `neon.rs`: ARM NEON (16 bytes/iteration, mandatory on aarch64)
- `mod.rs`: Runtime CPU feature detection and dispatch

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

**Before running benchmarks**, always check for heavy CPU activity that could skew results:

```bash
# Check CPU activity (macOS)
top -l 1 -n 5 | head -20

# Check CPU activity (Linux)
top -bn1 | head -20

# Alternative: check load average
uptime
```

Wait for CPU load to settle before running benchmarks. Ideally, load average should be below 1.0 for reliable results.

```bash
# Generate benchmark data
./target/release/succinctly json generate-suite
./target/release/succinctly json generate-suite --max-size 10mb

# Run benchmarks
cargo bench --bench json_simd
cargo bench --bench balanced_parens
cargo bench --bench jq_comparison
```

### Benchmark Patterns
| Pattern       | Description                    |
|---------------|--------------------------------|
| comprehensive | Mixed content (realistic)      |
| users         | User records (nested objects)  |
| nested        | Deep nesting (tests BP)        |
| arrays        | Large arrays (tests iteration) |
| strings       | String-heavy (tests escapes)   |
| unicode       | Unicode strings                |
| pathological  | Worst-case                     |

## CI/CD

```bash
# Mirror CI checks locally
cargo clippy --all-targets --all-features -- -D warnings
cargo test
./scripts/build.sh
```

## AVX-512 Performance Learnings

### Key Insight: Wider SIMD ≠ Automatically Faster

Two AVX-512 optimizations implemented with dramatically different results:

#### ✅ AVX512-VPOPCNTDQ: 5.2x Speedup (Compute-Bound)

**Implementation**: [src/bits/popcount.rs](src/bits/popcount.rs)
- Processes 8 u64 words (512 bits) in parallel
- Hardware `_mm512_popcnt_epi64` instruction
- **Result**: 96.8 GiB/s vs 18.5 GiB/s (scalar) = **5.2x faster**

**Why it wins**: Pure compute-bound, embarrassingly parallel, no dependencies

**Real-world impact**: Minimal - popcount is only ~1.6% of BitVec construction time

#### ❌ AVX-512 JSON Parser: 7-17% Slower than AVX2 (Memory-Bound) - REMOVED

**Previous Implementation**: ~~`src/json/simd/avx512.rs`~~ (removed 2026-01-07)
- Processed 64 bytes/iteration (vs 32 for AVX2)
- **Result**: 672 MiB/s vs 732 MiB/s (AVX2) = **8.9% slower**
- **Action**: Implementation removed; runtime dispatch now prioritizes AVX2

**Why AVX2 won**:
1. **Memory-bound workload**: Waiting for data from memory, not compute
2. **AMD Zen 4 architecture**: Splits AVX-512 into two 256-bit micro-ops (not native 512-bit)
3. **State machine overhead**: Wider SIMD = more bytes to process sequentially afterward
4. **Cache alignment**: 32-byte chunks fit cache lines better than 64-byte
5. **Amdahl's Law**: Speeding up 20% (SIMD classification) doesn't help when 80% (state machine + memory) dominates

### Performance Comparison Table

| Workload            | AVX-512 Result     | Status      | Reason                                    |
|---------------------|--------------------|-------------|-------------------------------------------|
| Popcount            | **5.2x faster** ✓  | **KEPT**    | Compute-bound, parallel, no dependencies  |
| JSON parsing        | **7-17% slower** ✗ | **REMOVED** | Memory-bound, sequential state machine    |
| Rank queries        | **Minimal impact** | N/A         | Popcount only 1.6% of total time          |
| BitVec construction | **~1% faster**     | N/A         | Dominated by memory allocation + indexing |

### Architectural Insights

**AMD Zen 4 AVX-512 characteristics**:
- ✓ No frequency throttling (unlike early Intel AVX-512)
- ✗ Not native 512-bit (two 256-bit execution units)
- ✗ AVX-512 ops split into 2x 256-bit micro-ops
- → Additional latency from operation splitting
- → AVX2 uses full-width units directly

**When to use AVX-512**:
- ✓ Pure compute: math, crypto, compression
- ✓ No memory bottlenecks
- ✓ No sequential dependencies
- ✓ Data-parallel algorithms
- ✗ Memory-bound workloads
- ✗ Sequential state machines
- ✗ Complex control flow

### Benchmarking Best Practices

Based on this work:

1. **Always profile on target hardware** - Theoretical ≠ actual performance
2. **Test multiple SIMD widths** - Wider isn't always better
3. **Measure end-to-end impact** - Not just micro-optimizations
4. **Consider Amdahl's Law** - Optimize the bottleneck, not the fast path
5. **Understand workload characteristics** - Compute-bound vs memory-bound matters more than instruction width

### Documentation

**Quick Reference**: [docs/optimization-summary.md](docs/optimization-summary.md) - **Complete record of all optimizations** (what worked, what failed, exact performance numbers)

Historical analysis available in `docs/archive/`.

### Production Recommendations

**For popcount operations**:
- Use AVX512-VPOPCNTDQ when available (Intel Ice Lake+, AMD Zen 4+)
- Runtime dispatch automatically selects best implementation
- Minimal real-world impact due to Amdahl's Law

**For JSON parsing**:
- **Use AVX2 as highest priority** (despite current runtime dispatch favoring AVX-512)
- AVX2 is 3-6% faster on Zen 4
- Broader compatibility (2013+ vs 2019+/2022+)
- Lower power consumption
- Keep AVX-512 as reference implementation and future optimization target

### Command Reference

```bash
# Test AVX-512 popcount implementation
cargo test --lib --features simd popcount

# Benchmark popcount strategies (includes AVX-512 VPOPCNTDQ)
cargo bench --bench popcount_strategies --features simd

# Generate JSON benchmark suite
cargo run --release --features cli -- json generate-suite

# Run comprehensive JSON benchmarks (AVX2, SSE4.2, SSE2, Scalar)
cargo bench --bench json_simd
```

### Key Takeaways for Future Optimizations

1. **Profile first, optimize second** - Don't assume wider/smarter is better
2. **Understand bottlenecks** - Memory-bound vs compute-bound matters
3. **Measure end-to-end** - Micro-benchmarks can be misleading
4. **Consider architecture** - Zen 4 splits AVX-512, future Zen 5 may not
5. **Amdahl's Law always wins** - Optimize what matters (the slow 80%), not what's easy (the fast 20%)
6. **Remove failed optimizations** - Slower code creates technical debt

### Successful Optimizations

#### PFSM (Parallel Finite State Machine) for JSON Parsing (2026-01-07)

Table-driven state machine approach ported from haskellworks hw-json-simd library.

**Implementation**: [src/json/pfsm.rs](src/json/pfsm.rs), [src/json/pfsm_tables.rs](src/json/pfsm_tables.rs)

**Two-Stage Pipeline**:
1. **Stage 1 - State Machine**: Sequential byte-by-byte processing with 256-entry lookup tables
   - TRANSITION_TABLE: Maps (byte, state) → next_state
   - PHI_TABLE: Maps (byte, state) → output bits (IB/OP/CL)

2. **Stage 2 - Bit Extraction**: Parallel extraction of interest bits and balanced parentheses
   - **BMI2+AVX2 path**: Process 8 phi values at once using PEXT/PDEP
   - **Scalar fallback**: Byte-by-byte extraction for remaining bytes

**Performance Results** (Comprehensive Pattern, 1MB):

| Implementation     | Throughput    | vs PFSM     |
|--------------------|---------------|-------------|
| **PFSM BMI2+AVX2** | **679 MiB/s** | baseline    |
| Standard AVX2      | 546 MiB/s     | -20% slower |
| Standard Scalar    | 494 MiB/s     | -27% slower |

**Cross-Pattern Performance** (10KB files):

| Pattern       | PFSM      | Standard Scalar | Standard AVX2 | Speedup vs Scalar | Speedup vs AVX2 |
|---------------|-----------|-----------------|---------------|-------------------|-----------------|
| comprehensive | 696 MiB/s | 437 MiB/s       | 666 MiB/s     | **+59%**          | **+5%**         |
| users         | 681 MiB/s | 454 MiB/s       | 704 MiB/s     | **+50%**          | -3%             |
| nested        | 824 MiB/s | 573 MiB/s       | 913 MiB/s     | **+44%**          | -10%            |
| arrays        | 667 MiB/s | 458 MiB/s       | 649 MiB/s     | **+46%**          | **+3%**         |

**Why PFSM Wins**:
- Table lookups are cache-friendly (256 entries = 1-2KB per table)
- State machine separates control flow from bit extraction
- BMI2 PEXT/PDEP used correctly for sparse bit operations (not consecutive like BitWriter)
- Amortizes table lookup overhead across the entire input
- Benefits from modern CPU branch prediction

**Command Reference**:

```bash
# Test PFSM implementation
cargo test --lib json::pfsm

# Benchmark PFSM vs standard implementations
cargo bench --bench pfsm_comparison

# End-to-end benchmark across all JSON patterns
cargo bench --bench pfsm_end_to_end

# Compare succinctly jq vs system jq
cargo bench --bench jq_comparison
```

### jq Query Performance (Apple M1 Max)

End-to-end comparison of `succinctly jq -c . file.json` vs `jq -c . file.json`:

| Size      | succinctly            | jq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  2.4 ms  (3.9 MiB/s)  |  4.3 ms  (2.2 MiB/s)  | **1.79x**  |
| **100KB** |  4.6 ms (18.4 MiB/s)  |  8.2 ms (10.5 MiB/s)  | **1.76x**  |
| **1MB**   | 24.7 ms (32.7 MiB/s)  | 43.9 ms (18.4 MiB/s)  | **1.78x**  |

### Failed Optimizations

See [docs/optimization-summary.md](docs/optimization-summary.md) for detailed analysis:
- **AVX-512 JSON parser**: -7-17% slower (removed 2026-01-07)
- **BMI1 mask iteration**: -25-31% slower (reverted 2026-01-07)
- **BMI2 PDEP in BitWriter**: -71% slower (reverted 2026-01-07)
- **PFSM Batched** (`pfsm_simd.rs`): -25% slower than production `pfsm_optimized` (2026-01-08)
  - Was 40% faster than basic `pfsm.rs`, but `pfsm_optimized.rs` already solves this better
  - Lesson: Always benchmark against production code, not just reference implementations
