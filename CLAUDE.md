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

### Semi-Indexing Architecture

Succinctly uses **semi-indexing** rather than traditional DOM parsing. Instead of building a complete in-memory tree, it creates a lightweight structural index (~3-6% overhead) and extracts values lazily. This enables:

- **17-46x less memory** than DOM parsers
- **Fast queries** because only accessed values are materialized
- **Streaming output** without intermediate allocations

**Trade-off**: Semi-indexing performs minimal validation compared to full parsers (jq, yq). The benchmark comparisons to jq/yq reflect both architectural advantages and reduced validation work. See [docs/architecture/semi-indexing.md](docs/architecture/semi-indexing.md) for details.

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

# JSON operations
./target/release/succinctly json generate 10mb -o benchmark.json
./target/release/succinctly jq '.users[].name' input.json
./target/release/succinctly jq -r '.users[] | [.name, .age] | @csv' input.json
./target/release/succinctly jq -r '.users[] | [.name, .age] | @dsv("|")' input.json
./target/release/succinctly jq-locate input.json --offset 42
./target/release/succinctly jq-locate input.json --line 5 --column 10

# YAML operations
./target/release/succinctly yq '.users[].name' config.yaml
./target/release/succinctly yq -o json '.' config.yaml         # Output as JSON
./target/release/succinctly yq '.spec.containers[]' k8s.yaml
./target/release/succinctly yq --doc 0 '.' multi-doc.yaml      # First document only
./target/release/succinctly yq-locate config.yaml --offset 42
./target/release/succinctly yq-locate config.yaml --line 5 --column 10

# DSV/CSV operations
./target/release/succinctly jq --input-dsv ',' '.[] | select(.[0] == "Alice")' data.csv
./target/release/succinctly dsv generate users 1000 -o users.csv

# Benchmarks
./target/release/succinctly dev bench jq
./target/release/succinctly dev bench yq
./target/release/succinctly dev bench yq --queries all --memory  # M2 streaming comparison
./target/release/succinctly dev bench dsv
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
├── dsv/                # DSV/CSV semi-indexing (BMI2/SIMD)
├── jq/                 # jq query language and evaluator
└── bin/                # CLI tool (jq, yq, jq-locate, yq-locate)
```

### Public API

```rust
use succinctly::bits::BitVec;
use succinctly::trees::BalancedParens;
use succinctly::json::JsonIndex;
use succinctly::yaml::YamlIndex;
use succinctly::dsv::DsvIndex;
use succinctly::jq::{parse, eval};
```

### Core Data Structures

| Structure         | Description                            | Performance (x86_64 Zen 4) |
|-------------------|----------------------------------------|----------------------------|
| **BitVec**        | O(1) rank, O(log n) select             | ~3-4% overhead             |
| **BalancedParens**| Succinct tree navigation               | ~6% overhead               |
| **JsonIndex**     | JSON semi-indexing with PFSM parser    | ~880 MiB/s                 |
| **YamlIndex**     | YAML semi-indexing with oracle parser  | ~250-400 MiB/s             |
| **DsvIndex**      | DSV semi-indexing with lightweight rank| 85-1676 MiB/s (API)        |

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
| **@yaml**    | `@yaml`             | YAML flow-style encoding (yq)                     | `{a: 1, b: 2}`      |
| **@props**   | `@props`            | Java properties format (yq)                       | `key = value`       |

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

### jq Assignment Operators

The jq implementation supports assignment operators for modifying JSON in-place:

| Operator   | Syntax               | Description                                          | Example                    |
|------------|----------------------|------------------------------------------------------|----------------------------|
| **=**      | `.path = value`      | Simple assignment                                    | `.a = 42`                  |
| **\|=**    | `.path \|= filter`   | Update assignment (applies filter to current value)  | `.a \|= . + 1`             |
| **+=**     | `.path += value`     | Compound add (equivalent to `.path \|= . + value`)   | `.count += 10`             |
| **-=**     | `.path -= value`     | Compound subtract                                    | `.health -= 25`            |
| ***=**     | `.path *= value`     | Compound multiply                                    | `.scale *= 2`              |
| **/=**     | `.path /= value`     | Compound divide                                      | `.total /= 4`              |
| **%=**     | `.path %= value`     | Compound modulo                                      | `.index %= 10`             |
| **//=**    | `.path //= value`    | Alternative assignment (sets only if null/false)     | `.default //= "fallback"`  |
| **del()**  | `del(.path)`         | Delete field or array element                        | `del(.temporary)`          |

```bash
# Examples
echo '{"a": 1}' | succinctly jq '.a = 42'           # {"a": 42}
echo '{"x": 5}' | succinctly jq '.x |= . * 2'       # {"x": 10}
echo '{"n": 10}' | succinctly jq '.n += 5'          # {"n": 15}
echo '{"a": null}' | succinctly jq '.a //= "default"'  # {"a": "default"}
echo '{"a": 1, "b": 2}' | succinctly jq 'del(.a)'   # {"b": 2}
echo '[1, 2, 3]' | succinctly jq '.[] |= . * 2'     # [2, 4, 6]
```

### jq Position-Based Navigation (succinctly extension)

Succinctly extends jq with position-based navigation builtins that allow jumping directly to a node at a specific byte offset or line/column position. This is unique to succinctly and not available in standard jq or yq.

| Builtin                    | Description                                           | Example                |
|----------------------------|-------------------------------------------------------|------------------------|
| **at_offset(n)**           | Jump to node at byte offset `n` (0-indexed)           | `at_offset(10)`        |
| **at_position(line; col)** | Jump to node at line/column (1-indexed, semicolon-separated) | `at_position(2; 3)` |

**Use cases:**
- IDE integration: Jump to node under cursor position
- Error investigation: Navigate to specific byte offset from error message
- Programmatic navigation: Build tools that work with document positions

```bash
# Jump to node at byte offset 10 (inside "Alice" string)
echo '{"name": "Alice", "age": 30}' | succinctly jq 'at_offset(10)'
# Output: "Alice"

# Jump to node at line 1, column 1 (the root object)
echo '{"name": "Alice"}' | succinctly jq 'at_position(1; 1)'
# Output: {"name": "Alice"}

# Navigate from offset position - get array at offset 10, then access element
echo '{"users": [{"name": "Alice"}, {"name": "Bob"}]}' | succinctly jq 'at_offset(10) | .[1].name'
# Output: "Bob"

# Works with multiline JSON - line 2, column 3 is the "name" key
echo '{
  "name": "Alice",
  "age": 30
}' | succinctly jq 'at_position(2; 3)'
# Output: "name"

# Works with YAML via yq
succinctly yq 'at_offset(6)' config.yaml

# Combine with other jq operations
echo '{"data": {"nested": {"value": 42}}}' | succinctly jq 'at_offset(9) | .nested.value'
# Output: 42
```

**Notes:**
- `at_offset(n)` uses 0-indexed byte offset (same as `jq-locate --offset`)
- `at_position(line; col)` uses 1-indexed line and column (same as `jq-locate --line --column`)
- Returns error if offset/position is out of bounds or doesn't correspond to a valid node
- Use `at_offset(n)?` or `at_position(l; c)?` for optional (returns empty on invalid position)

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
| [docs/guides/release.md](docs/guides/release.md)                  | Release process and checklist        |
| [docs/optimizations/](docs/optimizations/)                        | Optimization techniques reference    |
| [docs/benchmarks/jq.md](docs/benchmarks/jq.md)                    | JSON jq benchmark results            |
| [docs/benchmarks/yq.md](docs/benchmarks/yq.md)                    | YAML yq benchmark results            |
| [docs/benchmarks/dsv.md](docs/benchmarks/dsv.md)                  | DSV input performance benchmarks     |

## Performance Summary

### jq Query Performance (Apple M1 Max)

| Size      | succinctly            | jq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  4.1 ms  (2.3 MiB/s)  |  4.8 ms  (2.0 MiB/s)  | **1.2x**   |
| **100KB** |  6.6 ms (12.9 MiB/s)  |  9.0 ms  (9.4 MiB/s)  | **1.4x**   |
| **1MB**   | 29.6 ms (27.3 MiB/s)  | 45.3 ms (17.8 MiB/s)  | **1.5x**   |

To regenerate: `cargo bench --bench jq_comparison`

### jq Query Performance (ARM Neoverse-V2)

| Size      | succinctly            | jq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  2.2 ms (4.3 MiB/s)   |  2.7 ms  (3.5 MiB/s)  | **1.2x**   |
| **100KB** |  4.7 ms (17.1 MiB/s)  |  6.8 ms (12.5 MiB/s)  | **1.5x**   |
| **1MB**   | 28.4 ms (28.4 MiB/s)  | 47.3 ms (17.1 MiB/s)  | **1.7x**   |

### jq Query Performance (ARM Neoverse-V1)

| Size      | succinctly            | jq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  1.4 ms  (6.7 MiB/s)  |  2.6 ms  (3.6 MiB/s)  | **1.8x**   |
| **100KB** |  4.0 ms (21.5 MiB/s)  |  6.4 ms (13.3 MiB/s)  | **1.6x**   |
| **1MB**   | 27.9 ms (29.0 MiB/s)  | 45.6 ms (17.7 MiB/s)  | **1.6x**   |

### yq Query Performance (Apple M1 Max)

| Size       | succinctly             | yq                     | Speedup     | Mem Ratio  |
|------------|------------------------|------------------------|-------------|------------|
| **10KB**   |   6.4 ms  (1.5 MiB/s)  |   9.2 ms  (1.1 MiB/s)  | **1.4x**    | **0.51x**  |
| **100KB**  |   7.1 ms (13.0 MiB/s)  |  20.9 ms  (4.4 MiB/s)  | **2.9x**    | **0.33x**  |
| **1MB**    |  22.8 ms (40.4 MiB/s)  | 115.3 ms  (8.0 MiB/s)  | **5.1x**    | **0.12x**  |
| **10MB**   | 165.3 ms (56.5 MiB/s)  |   1.03 s  (9.1 MiB/s)  | **6.2x**    | **0.04x**  |
| **100MB**  |   1.52 s (61.1 MiB/s)  |   9.84 s  (9.4 MiB/s)  | **6.5x**    | **0.03x**  |

### yq Query Performance (ARM Neoverse-V2)

| Size      | succinctly              | yq                    | Speedup    |
|-----------|-------------------------|-----------------------|------------|
| **10KB**  | 2.0 ms  (4.9 MiB/s)     | 5.5 ms (1.8 MiB/s)    | **2.7x**   |
| **100KB** | 3.7 ms (24.9 MiB/s)     | 21.6 ms (4.3 MiB/s)   | **5.8x**   |
| **1MB**   | 19.5 ms (47.3 MiB/s)    | 154.6 ms (6.0 MiB/s)  | **7.9x**   |

### yq Query Performance (ARM Neoverse-V1)

Note: System `yq` not installed; showing succinctly-only performance.

| Size      | succinctly              |
|-----------|-------------------------|
| **10KB**  | 1.27 ms  (7.7 MiB/s)    |
| **100KB** | 2.87 ms (32.0 MiB/s)    |
| **1MB**   | 17.7 ms (52.1 MiB/s)    |

### yq Query Performance (AMD Ryzen 9 7950X)

| Size      | succinctly              | yq                    | Speedup    |
|-----------|-------------------------|-----------------------|------------|
| **10KB**  | 2.7 ms   (3.4 MiB/s)    | 64.7 ms (140 KiB/s)   | **24x**    |
| **100KB** | 4.4 ms  (20.5 MiB/s)    | 77.9 ms (1.2 MiB/s)   | **18x**    |
| **1MB**   | 18.9 ms (49.0 MiB/s)    |207.1 ms (4.5 MiB/s)   | **11x**    |

To regenerate: `succinctly dev bench yq` (includes memory) or `cargo bench --bench yq_comparison` (time only)

### M2 Streaming Navigation Performance (Apple M1 Max, 100MB navigation file)

| Query       | Path          | succinctly | yq       | Speedup     | succ Mem | yq Mem  |
|-------------|---------------|------------|----------|-------------|----------|---------|
| `.`         | P9 streaming  | 1.78s      | 11.54s   | **6.5x**    | 254 MB   | 7 GB    |
| `.[0]`      | M2 streaming  | 444ms      | 5.84s    | **13.2x**   | 254 MB   | 5 GB    |
| `.[]`       | M2 streaming  | 2.53s      | 13.30s   | **5.3x**    | 269 MB   | 8 GB    |
| `length`    | OwnedValue    | 445ms      | 5.84s    | **13.1x**   | 254 MB   | 5 GB    |

M2 streaming (`.[0]`) is **4.0x faster** than identity (`.`), with **3-5% of yq's memory**.

To benchmark: `succinctly dev bench yq --queries all --memory`

### Optimization Techniques

For detailed documentation on optimization techniques used in this project, see [docs/optimizations/](docs/optimizations/):

| Category | Document | Key Techniques |
|----------|----------|----------------|
| Bit-level | [bit-manipulation.md](docs/optimizations/bit-manipulation.md) | Popcount, CTZ, PDEP/PEXT |
| SIMD | [simd.md](docs/optimizations/simd.md) | AVX2, AVX-512, NEON |
| Memory | [cache-memory.md](docs/optimizations/cache-memory.md) | Alignment, prefetching |
| Data structures | [hierarchical-structures.md](docs/optimizations/hierarchical-structures.md) | Rank/select indices |
| Parsing | [state-machines.md](docs/optimizations/state-machines.md) | PFSM, lookup tables |

**Key insights** (see [docs/optimizations/README.md](docs/optimizations/README.md) for full details):
- Wider SIMD != automatically faster (AVX-512 JSON was 10% slower than AVX2)
- Algorithmic improvements beat micro-optimizations (cumulative index: 627x speedup; YAML streaming: 2.3x speedup)
- Simpler data structures often outperform complex ones due to cache behaviour
- Caching hot values eliminates repeated lookups (type checking: 1-17% improvement)
- Hardware prefetchers beat software prefetch for sequential access (prefetch: +30% regression!)
- SIMD newline scanning + indentation checking enables fast block boundary detection (block scalars: 19-25% improvement!)
- Micro-benchmark wins ≠ real-world improvements (threshold tuning: +8-15% regression despite micro-bench suggesting improvement)
- Eliminating phases beats optimizing them (YAML streaming: removed DOM conversion entirely for 2.3x gain)

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
- ❌ P5 (Flow Collection Fast Path): **REJECTED** - aborted at analysis stage before implementation
  - Micro-benchmarks showed 8-14x SIMD wins for 64-128 byte flow collections
  - Real YAML flow collections are typically 10-30 bytes (too small for SIMD to win)
  - SIMD only wins at 32+ bytes, but 90% of real flow collections are < 30 bytes
  - **Lessons from P2.6/P2.8/P3 applied** - rejected during analysis to avoid predicted regression
  - Key insight: Micro-benchmark wins only translate when optimizing inputs that exist in real workloads
  - See [docs/parsing/yaml.md#p5-flow-collection-fast-path---rejected-](docs/parsing/yaml.md#p5-flow-collection-fast-path---rejected-) for full analysis
- ❌ P6 (BMI2 Operations): **REJECTED** - YAML's grammar prevents DSV-style quote indexing
  - **Primary reason**: YAML cannot use BMI2 like DSV does (grammar incompatibility)
    - DSV: Quotes are context-free (`""` escape), can build global quote index in one pass using BMI2 `toggle64`
    - YAML: Backslash escaping (`\"`) requires escape preprocessing BEFORE quote detection
    - Circular dependency: Need escape mask to build quote mask, but escape handling is what we're optimizing
    - Multi-pass approach (escape → quote → parse) slower than current early-exit SIMD
  - **Wrong approach tested**: Micro-benchmarks tested per-string parsing (not index building)
    - DSV BMI2 builds global document index (scans everything once)
    - Tested YAML per-string parsing (early-exit SIMD wins on short strings)
    - Apples-to-oranges comparison revealed fundamental grammar difference
  - **Key insight**: BMI2 quote indexing only works when escaping is context-free (CSV's `""`, not YAML's `\"`)
  - **Additional context-sensitivity**: YAML has two quote types (`"` vs `'`), block scalars (`|` `>`), and context-dependent quote meaning
  - See [docs/parsing/yaml.md#p6-bmi2-operations-pdeppext---rejected-](docs/parsing/yaml.md#p6-bmi2-operations-pdeppext---rejected-) for full analysis
- ❌ P7 (Newline Index): **REJECTED** - use case mismatch (CLI feature, not parsing optimization)
  - **Discovery**: JSON's `NewlineIndex` is NOT built during parsing
    - Only used in `jq-locate` CLI tool for `--line X --column Y` → byte offset conversion
    - Never built during JSON parsing, jq queries, or benchmarks
    - Zero performance impact on actual JSON processing
  - **YAML's current approach**: Lazy O(n) `current_line()` only on error paths
    - 4 call sites, all in error reporting (TabIndentation, UnexpectedToken, etc.)
    - Comment: "Only called on error paths, so we pay the cost only when needed"
    - Benchmarks use valid YAML (no errors, so `current_line()` never called)
  - **Two possible interpretations**:
    - Option A (CLI feature): Add `yq-locate` tool with on-demand NewlineIndex (no benchmark impact)
    - Option B (misguided): Build index during parsing (O(n) overhead with zero benefit for valid YAML)
  - **Why Option B would fail**: Adds build cost to hot path (parsing) to optimize cold path (errors/CLI)
  - **Pattern recognition**: Not all JSON features are optimizations - NewlineIndex is CLI UX, not performance
  - See [docs/parsing/yaml.md#p7-newline-index---rejected-](docs/parsing/yaml.md#p7-newline-index---rejected-) for full analysis
- ❌ P8 (AVX-512 Variants): **REJECTED** - benchmark design flaw + JSON precedent (7% slower at realistic sizes)
  - **Primary reason**: Memory-bound workload doesn't benefit from wider SIMD
    - JSON AVX-512 was **7-17% slower** than AVX2 and removed from codebase
    - YAML is similarly memory-bound (sequential text parsing)
    - Memory bandwidth bottleneck prevents wider vectors from helping
  - **Micro-benchmark design flaw**:
    - Measured loop iterations instead of work performed
    - AVX2: 8 iterations for 256B (32B chunks)
    - AVX-512: 4 iterations for 256B (64B chunks)
    - "2x speedup" was artificial - half the iterations, not twice the efficiency
  - **Realistic size results** (AMD Ryzen 9 7950X):
    - 64B: **7% slower** (18.59ns vs 17.34ns) - memory bandwidth bottleneck
    - 128B: **Neutral** (18.94ns vs 18.95ns) - break-even point
    - 256B+: "Wins" misleading (fewer iterations, not faster per-byte)
  - **Zen 4 limitation**: Splits 512-bit ops into two 256-bit paths (overhead without benefit)
  - **Pattern recognition**: P5/P6/P7/P8 all rejected for mismatch (size/grammar/use case/benchmark)
  - **Key lesson**: Wider SIMD ≠ faster for memory-bound workloads (AVX2 already saturates RAM bandwidth)
  - See [docs/parsing/yaml.md#p8-avx-512-variants---rejected-](docs/parsing/yaml.md#p8-avx-512-variants---rejected-) for full analysis
- ✅ P9 (Direct YAML-to-JSON Streaming): **2.3x improvement** on `yq` identity queries - **largest Phase 2 optimization!**
  - Eliminated intermediate OwnedValue DOM by streaming directly from YAML cursor to JSON
  - 10KB: 257µs → 108µs (38.1 → 90.5 MiB/s, **+137%**)
  - 100KB: 1.93ms → 828µs (47.7 → 111.1 MiB/s, **+133%**)
  - Single-pass YAML→JSON escape transcoding without intermediate string allocation
  - **Bottleneck shift**: Parsing now 40% of time (was 15%), DOM conversion eliminated
  - Best for: `yq '.'` identity queries, format conversion, streaming output
  - See [docs/parsing/yaml.md#p9-direct-yaml-to-json-streaming---accepted-](docs/parsing/yaml.md#p9-direct-yaml-to-json-streaming---accepted-) for full analysis
- ✅ P10 (Type Preservation): **Full yq compatibility** (correctness fix)
  - Fixed quoted string type preservation: `"1.0"` stays as string, not converted to number `1`
  - Added early-exit for quoted strings, skipping expensive `parse::<i64>()` and `parse::<f64>()` calls
  - Current performance: 10KB: 1.63ms, 100KB: 2.78ms, 1MB: 13.2ms (with correct output)
  - Created comprehensive test suite: 32 tests including 8 direct byte-for-byte comparisons with system `yq`
  - **Key achievement**: `succinctly yq` is now a drop-in replacement for `yq` for supported arguments
  - See [docs/parsing/yaml.md#p10-type-preservation---accepted-](docs/parsing/yaml.md#p10-type-preservation---accepted-) for full analysis
- ✅ P11 (BP Select1 for yq-locate): **2.5-5.9x faster** select1 queries, fixes issue #26
  - Added zero-cost generic `SelectSupport` trait to `BalancedParens<W, S>` (NoSelect for JSON, WithSelect for YAML)
  - `find_bp_at_text_pos()` now uses O(1) sampled select1 instead of O(log n) binary search on rank1
  - **Micro-benchmark speedups** (10K queries):
    - 1K opens: 326µs vs 820µs (**2.5x** faster)
    - 10K opens: 318µs vs 1.31ms (**4.1x** faster)
    - 100K opens: 308µs vs 1.68ms (**5.4x** faster)
    - 1M opens: 356µs vs 2.10ms (**5.9x** faster)
  - **End-to-end yq benchmarks**: 1MB identity query 3.1% faster (14.45ms → 14.00ms)
  - **Trade-off**: 2-4% regression in yaml_bench (SelectIndex build cost) but benefits `yq-locate` use case
  - **Zero-cost for JSON**: Uses `NoSelect` (ZST) - no memory or runtime overhead
  - **Fixes GitHub issue #26**: YAML `at_offset` and `yq-locate` now return correct nodes
  - See [docs/parsing/yaml.md#p11-bp-select1-for-yq-locate---accepted-](docs/parsing/yaml.md#p11-bp-select1-for-yq-locate---accepted-) for full analysis
- ✅ P12 (Advance Index for bp_to_text): **20-25% faster** yq identity queries on 1MB files, fixes issue #62
  - Replaced `Vec<u32>` with memory-efficient `BpToTextPositions` enum (IB + Advance bitmaps)
  - **~1.5× memory compression** for bp_to_text structure (measured with 33% duplicates)
  - **Automatic fallback** to `Vec<u32>` for non-monotonic positions (explicit keys)
  - **End-to-end yq benchmarks**:
    - users/1mb: **-24.6%** time (+32.7% throughput)
    - sequences/1mb: **-24.8%** time (+33.0% throughput)
    - nested/1mb: **-21.5%** time (+27.4% throughput)
  - **yaml_bench improvements**: sequences/1000 -5.0%, large/1mb -3.2%, block_scalars -3.5%
  - **Minor regression**: ~1.5-2% on tiny (10-element) quoted string benchmarks
  - Better cache locality from compact bitmap representation
  - See [docs/parsing/yaml.md#p12-advance-index-for-memory-efficient-bp_to_text---accepted-](docs/parsing/yaml.md#p12-advance-index-for-memory-efficient-bp_to_text---accepted-) for full analysis
