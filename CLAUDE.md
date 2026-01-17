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
| **yaml-semi-indexing** | "YAML index", "YAML parser", "sequence item"| YAML parsing and debugging patterns        |
| **testing**            | "test", "assert", "coverage", "regression"  | Test quality patterns and anti-patterns    |
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
./target/release/succinctly jq -r '.users[] | [.name, .age] | @csv' input.json
./target/release/succinctly jq -r '.users[] | [.name, .age] | @dsv("|")' input.json
./target/release/succinctly jq-locate input.json --offset 42
./target/release/succinctly jq-locate input.json --line 5 --column 10
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
├── yaml/               # YAML semi-indexing (oracle parser)
├── jq/                 # jq query language
└── bin/                # CLI tool
```

### Public API

```rust
use succinctly::bits::BitVec;
use succinctly::trees::BalancedParens;
use succinctly::json::JsonIndex;
use succinctly::yaml::YamlIndex;
use succinctly::jq::{parse, eval};
```

### Core Data Structures

| Structure         | Description                            | Performance   |
|-------------------|----------------------------------------|---------------|
| **BitVec**        | O(1) rank, O(log n) select             | ~3-4% overhead|
| **BalancedParens**| Succinct tree navigation               | ~6% overhead  |
| **JsonIndex**     | JSON semi-indexing with PFSM parser    | ~950 MiB/s    |
| **YamlIndex**     | YAML semi-indexing with oracle parser  | ~250-400 MiB/s|
| **DsvIndex**      | DSV semi-indexing with lightweight rank| 11-169 MiB/s (CLI), 85-1676 MiB/s (API)|

### jq Format Functions

The jq implementation supports format functions for converting values to strings:

| Format       | Syntax              | Description                                       | Example Output      |
|--------------|---------------------|---------------------------------------------------|---------------------|
| **@csv**     | `@csv`              | Comma-separated values (fixed delimiter)          | `a,b,c`             |
| **@tsv**     | `@tsv`              | Tab-separated values (fixed delimiter)            | `a\tb\tc`           |
| **@dsv**     | `@dsv(delimiter)`   | Generic DSV with custom delimiter                 | `a\|b\|c`           |
| **@json**    | `@json`             | JSON format                                       | `{"a":1}`           |
| **@text**    | `@text`             | Convert to string (same as tostring)              | `42`                |
| **@uri**     | `@uri`              | URI percent encoding                              | `hello%20world`     |
| **@base64**  | `@base64`           | Base64 encoding                                   | `aGVsbG8=`          |
| **@base64d** | `@base64d`          | Base64 decoding                                   | `hello`             |
| **@html**    | `@html`             | HTML entity escaping                              | `&lt;script&gt;`    |
| **@sh**      | `@sh`               | Shell quoting                                     | `'hello world'`     |

**@dsv(delimiter) specifics:**
- Custom delimiters: Any single or multi-character string
- Smart quoting: Auto-quotes fields containing delimiter, `"`, or `\n`
- CSV-compatible: `@dsv(",")` produces identical output to `@csv`
- Escape handling: Quotes are escaped as `""`

```bash
# Examples
echo '["a","b","c"]' | jq -r '@dsv("|")'        # Output: a|b|c
echo '["a","b|c","d"]' | jq -r '@dsv("|")'      # Output: a|"b|c"|d
echo '["a","b","c"]' | jq -r '@dsv(";")'        # Output: a;b;c
```

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

| Document                                                          | Purpose                              |
|-------------------------------------------------------------------|--------------------------------------|
| [RELEASE.md](RELEASE.md)                                          | Release process and checklist        |
| [docs/optimisations/](docs/optimisations/)                        | Optimisation techniques reference    |
| [docs/jq-comparison.md](docs/jq-comparison.md)                    | JSON jq benchmark results            |
| [docs/yq-comparison.md](docs/yq-comparison.md)                    | YAML yq benchmark results            |
| [docs/dsv-performance.md](docs/dsv-performance.md)                | DSV input performance benchmarks     |

## Performance Summary

### jq Query Performance (Apple M1 Max)

| Size      | succinctly            | jq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  4.1 ms  (2.3 MiB/s)  |  4.1 ms  (2.3 MiB/s)  | **1.0x**   |
| **100KB** |  6.6 ms (13.0 MiB/s)  |  8.4 ms (10.2 MiB/s)  | **1.3x**   |
| **1MB**   | 28.3 ms (28.5 MiB/s)  | 45.8 ms (17.6 MiB/s)  | **1.6x**   |

To regenerate: `cargo bench --bench jq_comparison`

### yq Query Performance (Apple M1 Max)

| Size      | succinctly            | yq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  4.5 ms  (2.2 MiB/s)  |  7.5 ms  (1.3 MiB/s)  | **1.7x**   |
| **100KB** | 10.6 ms  (8.6 MiB/s)  | 19.8 ms  (4.6 MiB/s)  | **1.9x**   |
| **1MB**   | 72.9 ms (12.7 MiB/s)  |118.6 ms  (7.8 MiB/s)  | **1.6x**   |

### yq Query Performance (AMD Ryzen 9 7950X)

| Size      | succinctly            | yq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  2.0 ms  (4.9 MiB/s)  | 60.2 ms (166 KiB/s)   | **30x**    |
| **100KB** |  7.1 ms (13.0 MiB/s)  | 74.2 ms  (1.2 MiB/s)  | **10.5x**  |
| **1MB**   | 59.7 ms (15.4 MiB/s)  |194.2 ms  (4.7 MiB/s)  | **3.3x**   |

To regenerate: `cargo bench --bench yq_comparison`

### Optimisation Techniques

For detailed documentation on optimisation techniques used in this project, see [docs/optimisations/](docs/optimisations/):

| Category | Document | Key Techniques |
|----------|----------|----------------|
| Bit-level | [bit-manipulation.md](docs/optimisations/bit-manipulation.md) | Popcount, CTZ, PDEP/PEXT |
| SIMD | [simd.md](docs/optimisations/simd.md) | AVX2, AVX-512, NEON |
| Memory | [cache-memory.md](docs/optimisations/cache-memory.md) | Alignment, prefetching |
| Data structures | [hierarchical-structures.md](docs/optimisations/hierarchical-structures.md) | Rank/select indices |
| Parsing | [state-machines.md](docs/optimisations/state-machines.md) | PFSM, lookup tables |

**Key insights** (see [docs/optimisations/README.md](docs/optimisations/README.md) for full details):
- Wider SIMD != automatically faster (AVX-512 JSON was 10% slower than AVX2)
- Algorithmic improvements beat micro-optimisations (cumulative index: 627x speedup)
- Simpler data structures often outperform complex ones due to cache behaviour
- Caching hot values eliminates repeated lookups (type checking: 1-17% improvement)
- Hardware prefetchers beat software prefetch for sequential access (prefetch: +30% regression!)
- SIMD newline scanning + indentation checking enables fast block boundary detection (block scalars: 19-25% improvement!)
- Micro-benchmark wins ≠ real-world improvements (threshold tuning: +8-15% regression despite micro-bench suggesting improvement)

**Recent YAML optimizations:**
- ✅ P2.5 (Cached Type Checking): 1-17% improvement depending on nesting depth
  - See [docs/parsing/yaml.md#p25-cached-type-checking](docs/parsing/yaml.md#p25-cached-type-checking) for details
  - Best for deeply nested YAML (Kubernetes configs, CI/CD files)
- ❌ P2.6 (Software Prefetching): **REJECTED** - 30% regression on large files
  - Modern CPU hardware prefetchers are superior for sequential parsing
  - Software prefetch causes cache pollution and interferes with hardware
  - See [docs/parsing/yaml.md#p26-software-prefetching-for-large-files---rejected-](docs/parsing/yaml.md#p26-software-prefetching-for-large-files---rejected-) for analysis
- ✅ P2.7 (Block Scalar SIMD): **19-25% improvement** on block scalar parsing - **largest Phase 2 optimization!**
  - AVX2 scans 32-byte chunks for newlines, checks indentation using SIMD
  - 100x100 lines: 247 µs → 195 µs (-21%, 1.27x faster)
  - Throughput: 1.39 GiB/s → 1.76 GiB/s (+27%)
  - See [docs/parsing/yaml.md#p27-block-scalar-simd---accepted-](docs/parsing/yaml.md#p27-block-scalar-simd---accepted-) for full analysis
- ❌ P2.8 (SIMD Threshold Tuning): **REJECTED** - 8-15% regression on quoted strings
  - Micro-benchmarks suggested SIMD threshold tuning would help (showed 2-4% gain)
  - End-to-end tests showed severe regressions instead
  - Modern CPUs (branch prediction, inlining) make threshold tuning counterproductive
  - See [docs/parsing/yaml.md#p28-simd-threshold-tuning---rejected-](docs/parsing/yaml.md#p28-simd-threshold-tuning---rejected-) for full analysis
- ❌ P3 (Branchless Character Classification): **REJECTED** - 25-44% regression on simple parsing
  - Replaced `matches!` macros with lookup tables to eliminate branches
  - Micro-benchmarks showed 3-29% improvement for character classification
  - End-to-end tests showed catastrophic 25-44% regressions
  - Modern branch predictors (93-95% accuracy) beat lookup tables
  - `.map_or()` overhead and cache pollution from 256-byte tables dominated any benefit
  - Third consecutive optimization where micro-benchmarks mislead (P2.6, P2.8, P3 all failed)
  - See [docs/parsing/yaml.md#p3-branchless-character-classification---rejected-](docs/parsing/yaml.md#p3-branchless-character-classification---rejected-) for full analysis
- ✅ P4 (Anchor/Alias SIMD): **6-17% improvement** on anchor-heavy workloads
  - AVX2 SIMD scans for anchor name terminators in 32-byte chunks
  - anchors/100: 13.60µs → 11.65µs (-14.6%, 1.17x faster)
  - anchors/1000: 155.5µs → 139.2µs (-10.1%, 1.11x faster)
  - k8s_100: 33.92µs → 31.40µs (-7.4%, 1.08x faster)
  - Micro-benchmarks confirmed: 9-12x faster for 32-64 byte anchor names
  - **First successful optimization since P2.7** - proves SIMD string searching wins when targeting real bottlenecks
  - Best for: Kubernetes manifests, CI/CD configs with many anchors/aliases
  - See [docs/parsing/yaml.md#p4-anchoralias-simd---accepted-](docs/parsing/yaml.md#p4-anchoralias-simd---accepted-) for full analysis
