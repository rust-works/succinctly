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
| **@yaml**    | `@yaml`             | YAML flow-style encoding (yq)                     | `{a: 1, b: 2}`      |

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
| **10KB**  |  4.1 ms  (2.3 MiB/s)  |  4.1 ms  (2.3 MiB/s)  | **1.0x**   |
| **100KB** |  6.6 ms (13.0 MiB/s)  |  8.4 ms (10.2 MiB/s)  | **1.3x**   |
| **1MB**   | 28.3 ms (28.5 MiB/s)  | 45.8 ms (17.6 MiB/s)  | **1.6x**   |

To regenerate: `cargo bench --bench jq_comparison`

### yq Query Performance (Apple M1 Max)

| Size      | succinctly            | yq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  4.0 ms  (2.5 MiB/s)  |  7.2 ms  (1.3 MiB/s)  | **1.8x**   |
| **100KB** |  8.8 ms (10.5 MiB/s)  | 19.9 ms  (4.6 MiB/s)  | **2.3x**   |
| **1MB**   | 55.0 ms (16.8 MiB/s)  |117.1 ms  (7.9 MiB/s)  | **2.1x**   |

### yq Query Performance (AMD Ryzen 9 7950X)

| Size      | succinctly              | yq                    | Speedup    |
|-----------|-------------------------|-----------------------|------------|
| **10KB**  |  83.5 µs (116.8 MiB/s)  | 57.4 ms (175 KiB/s)   | **687x**   |
| **100KB** | 599 µs (153.6 MiB/s)    | 71.3 ms  (1.3 MiB/s)  | **119x**   |
| **1MB**   | 4.24 ms (217.6 MiB/s)   |191.1 ms  (4.8 MiB/s)  | **45x**    |

To regenerate: `cargo bench --bench yq_comparison`

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
- ✅ P10 (Type Preservation): **Full yq compatibility + 27-304% performance improvement**
  - Fixed quoted string type preservation: `"1.0"` stays as string, not converted to number `1`
  - Added early-exit for quoted strings, skipping expensive `parse::<i64>()` and `parse::<f64>()` calls
  - 10KB: 108µs → 83.5µs (90.5 → 116.8 MiB/s, **+29%**)
  - 100KB: 828µs → 599µs (111.1 → 153.6 MiB/s, **+38%**)
  - 1MB: 6.35ms → 4.24ms (145.6 → 217.6 MiB/s, **+49%**)
  - Created comprehensive test suite: 32 tests including 8 direct byte-for-byte comparisons with system `yq`
  - **Key achievement**: `succinctly yq` is now a drop-in replacement for `yq` for supported arguments
  - See [docs/parsing/yaml.md#p10-type-preservation---accepted-](docs/parsing/yaml.md#p10-type-preservation---accepted-) for full analysis
