# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Skills Reference

Detailed guidance is organized into skills in `.claude/skills/`. Claude will auto-invoke these based on context:

| Skill                  | Triggers                                    | Purpose                                    |
|------------------------|---------------------------------------------|--------------------------------------------|
| **benchmark-docs**     | "benchmark", "update benchmarks"            | Platform-specific benchmark documentation  |
| **markdown-tables**    | "format table", "align table"               | Fixed-width table formatting               |
| **simd-optimization**  | "SIMD", "AVX", "NEON", "vectorization"      | SIMD patterns and learnings                |
| **bit-optimization**   | "rank", "select", "popcount", "lookup table"| Bit-level optimization patterns            |
| **json-semi-indexing** | "JSON index", "semi-index", "cursor"        | JSON parsing implementation details        |
| **commit-msg**         | "commit message", "amend commit"            | Conventional commits format                |

## AI Scratch Directory

Use `.ai/scratch/` for temporary files (git-ignored):

```bash
mkdir -p .ai/scratch
```

## Project Overview

Succinctly is a high-performance Rust library implementing succinct data structures with fast rank and select operations, optimized for both x86_64 (POPCNT) and ARM (NEON) architectures.

## Common Commands

### Building and Testing

```bash
cargo build                              # Standard build
cargo build --features simd              # With SIMD popcount
cargo test                               # Run tests
cargo test --features large-tests        # 1GB bitvector tests
cargo bench                              # Run benchmarks
```

### CLI Tool

```bash
cargo build --release --features cli

./target/release/succinctly json generate 10mb -o benchmark.json
./target/release/succinctly jq '.users[].name' input.json
./target/release/succinctly dev bench jq
```

## Code Architecture

### Module Structure

```
src/
├── lib.rs              # Public API, RankSelect trait
├── bits/               # BitVec, rank/select, popcount
├── trees/              # Balanced parentheses
├── json/               # JSON semi-indexing (PFSM default)
├── jq/                 # jq query language
└── bin/                # CLI tool
```

### Public API

```rust
use succinctly::bits::BitVec;
use succinctly::trees::BalancedParens;
use succinctly::json::JsonIndex;
use succinctly::jq::{parse, eval};
```

### Core Data Structures

| Structure         | Description                            | Overhead |
|-------------------|----------------------------------------|----------|
| **BitVec**        | O(1) rank, O(log n) select             | ~3-4%    |
| **BalancedParens**| Succinct tree navigation               | ~6%      |
| **JsonIndex**     | JSON semi-indexing with PFSM parser    | ~950 MiB/s |

## Feature Flags

| Feature             | Description                    |
|---------------------|--------------------------------|
| `simd`              | Explicit SIMD intrinsics       |
| `portable-popcount` | Portable bitwise algorithm     |
| `large-tests`       | 1GB bitvector tests            |
| `huge-tests`        | 5GB bitvector tests            |
| `cli`               | CLI tool                       |

## Testing Strategy

- **Unit tests**: In each module's `#[cfg(test)] mod tests`
- **Property tests**: `tests/property_tests.rs`, `tests/bp_properties.rs`
- **Integration tests**: `tests/json_indexing_tests.rs`, `tests/simd_level_tests.rs`

## `no_std` Support

The library is `no_std` compatible:
- Uses `#![cfg_attr(not(test), no_std)]`
- Depends on `alloc` for `Vec<u64>` storage

## CI/CD

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo test
./scripts/build.sh
```

## Key Documentation

| Document                                                      | Purpose                              |
|---------------------------------------------------------------|--------------------------------------|
| [RELEASE.md](RELEASE.md)                                      | Release process and checklist        |
| [docs/optimization-summary.md](docs/optimization-summary.md)  | Complete optimization record         |
| [docs/jq-comparison.md](docs/jq-comparison.md)                | Benchmark results                    |

## Performance Summary

### jq Query Performance (Apple M1 Max)

| Size      | succinctly            | jq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  2.4 ms  (3.9 MiB/s)  |  4.3 ms  (2.2 MiB/s)  | **1.79x**  |
| **100KB** |  4.6 ms (18.4 MiB/s)  |  8.2 ms (10.5 MiB/s)  | **1.76x**  |
| **1MB**   | 24.7 ms (32.7 MiB/s)  | 43.9 ms (18.4 MiB/s)  | **1.78x**  |

To regenerate: `./target/release/succinctly dev bench jq`

### Key Optimization Learnings

**What worked:**
- PFSM table-driven JSON parser: 40-77% faster than scalar
- AVX-512 VPOPCNTDQ: 5.2x faster (compute-bound)
- Byte-level lookup tables: 50-90% speedup for BP operations

**What failed:**
- AVX-512 JSON parser: 7-17% slower (memory-bound) - removed
- BMI2 PDEP in BitWriter: 71% slower - reverted
- PFSM batched: 25% slower than production - not deployed

**Key insight**: Wider SIMD != automatically faster. Profile on target hardware, benchmark against production code.

See `.claude/skills/simd-optimization/SKILL.md` and `.claude/skills/bit-optimization/SKILL.md` for details.
