---
name: benchmark-docs
description: Manages benchmark documentation across multiple platforms. Use when updating benchmark results, adding performance data, or documenting jq comparison benchmarks. Triggers on terms like "benchmark", "performance", "jq comparison", "benchmark results", "update benchmarks".
---

# Benchmark Documentation Skill

This skill ensures proper handling of benchmark documentation across multiple platforms (ARM/Apple Silicon and x86_64/Intel/AMD).

**For comprehensive benchmarking instructions**, see [docs/guides/benchmarking.md](../../../docs/guides/benchmarking.md).

This skill focuses on **documentation-specific rules** and multi-platform considerations.

## Critical Rules

**NEVER replace platform-specific benchmarks with each other.**

When adding new benchmark data:

1. **Keep both ARM and x86_64 benchmarks** - Never delete one platform's data when adding another
2. **Add new platform data alongside existing data** - Create separate sections or files per platform
3. **Name files with platform suffix** - e.g., `jq-comparison-m1.jsonl`, `jq-comparison-zen4.jsonl`
4. **Update docs to show both** - README and docs should reference benchmarks from all platforms

## Benchmark File Locations

For complete inventory, see [docs/benchmarks/inventory.md](../../../docs/benchmarks/inventory.md).

Key locations:
- `data/bench/generated/` - Input files (git-ignored)
- `data/bench/results/` - Output (git-ignored)
- `docs/benchmarks/*.md` - Documentation (tracked)
- `README.md` - Summary (tracked)

## Updating Benchmark Documentation

For complete documentation update workflow, see [docs/guides/benchmarking.md](../../../docs/guides/benchmarking.md#updating-documentation).

### Quick Workflow

1. Build the benchmark runner: `cargo build --release --features bench-runner`
2. Run benchmarks: `./target/release/succinctly bench run jq_bench`
3. Review output: `data/bench/results/<timestamp>/jq-bench.md`
4. Update documentation:
   - `docs/benchmarks/jq.md` - Full results with all patterns/sizes
   - `README.md` - Summary highlights only
5. Follow platform parity rules (see below)

## README Table Requirements

### Platform Parity

**Every benchmark table in README.md must have data for BOTH platforms:**
- x86_64 (AMD Ryzen 9 7950X or equivalent)
- ARM (Apple M1 Max or equivalent)

If a table only shows one platform, add the missing platform's data from `docs/benchmarks/jq.md`.

### Pattern Names

**Never abbreviate pattern names in tables.** Use full names:
- `pathological` not `patholog.`
- `comprehensive` not `compreh.`

### Bold Formatting in Tables

When using bold for values in tables, ensure spaces are OUTSIDE the `**` markers:
```markdown
<!-- CORRECT -->
|  **59.6ms** |

<!-- WRONG (won't render as bold) -->
| ** 59.6ms** |
```

See the markdown-tables skill for complete table formatting rules.

## Benchmark Patterns

| Pattern       | Description                    |
|---------------|--------------------------------|
| comprehensive | Mixed content (realistic)      |
| users         | User records (nested objects)  |
| nested        | Deep nesting (tests BP)        |
| arrays        | Large arrays (tests iteration) |
| strings       | String-heavy (tests escapes)   |
| unicode       | Unicode strings                |
| pathological  | Worst-case                     |
| numbers       | Number-heavy documents         |
| literals      | Mix of null, true, false       |
| mixed         | Heterogeneous nested structures|

## Before Running Benchmarks

**CRITICAL: Always compile before benchmarking:**
```bash
cargo build --release --features bench-runner
```

See [docs/guides/benchmarking.md](../../../docs/guides/benchmarking.md#troubleshooting) for environment setup and troubleshooting.

Quick checks:
- Build is up to date: `cargo build --release --features bench-runner`
- CPU load is low: `uptime`
- Test data exists: `ls data/bench/generated/`

## Benchmark Commands Reference

For complete command reference, see [docs/guides/benchmarking.md](../../../docs/guides/benchmarking.md#how-to-run-benchmarks).

**IMPORTANT: Always use the unified benchmark runner (`succinctly bench`), not `dev bench`.**

### Running Benchmarks

**Always build first, then run benchmarks:**

```bash
# Step 1: Build the benchmark runner (required before running benchmarks)
cargo build --release --features bench-runner

# Step 2: Generate test data (if not already present)
./target/release/succinctly json generate-suite
./target/release/succinctly yaml generate-suite

# Step 3: Run benchmarks using the unified runner
./target/release/succinctly bench run jq_bench      # JSON vs jq
./target/release/succinctly bench run yq_bench      # YAML vs yq
./target/release/succinctly bench run jq_comparison # Criterion JSON benchmarks
./target/release/succinctly bench run yq_comparison # Criterion YAML benchmarks
```

### Listing Available Benchmarks

```bash
./target/release/succinctly bench list
```

### Running Multiple Benchmarks

```bash
# Run all JSON benchmarks
./target/release/succinctly bench run jq_bench jq_comparison

# Run all YAML benchmarks
./target/release/succinctly bench run yq_bench yq_comparison yaml_bench
```

## yq Benchmark Query Types

The yq benchmark supports multiple query types to exercise different execution paths:

| Query Type     | Example    | Execution Path | Description                                     |
|----------------|------------|----------------|-------------------------------------------------|
| `identity`     | `.`        | P9 streaming   | Full document streaming output                  |
| `first_element`| `.[0]`     | M2 streaming   | Navigate to first array element                 |
| `iteration`    | `.[]`      | M2 streaming   | Iterate over array elements                     |
| `length`       | `length`   | OwnedValue     | Produces computed value (not cursor-streamable) |

### Running yq Benchmarks

**Always build first:** `cargo build --release --features bench-runner`

```bash
# Run yq CLI benchmark (recommended - includes memory tracking)
./target/release/succinctly bench run yq_bench

# For advanced options, use dev bench yq (after running unified runner)
# Run specific query types
./target/release/succinctly dev bench yq --queries identity
./target/release/succinctly dev bench yq --queries identity,first_element

# Focus on M2 streaming with the navigation pattern
./target/release/succinctly dev bench yq --patterns navigation --sizes 10mb,100mb

# Memory-focused comparison
./target/release/succinctly dev bench yq --memory
```

### Query Type Aliases

Multiple aliases are accepted for each query type:

| Query Type     | Aliases                          |
|----------------|----------------------------------|
| `identity`     | `identity`, `.`                  |
| `first_element`| `first_element`, `first`, `.[0]` |
| `iteration`    | `iteration`, `iter`, `.[]`       |
| `length`       | `length`                         |

## Running bench-compare Benchmarks

For complete instructions, see [docs/guides/benchmarking.md](../../../docs/guides/benchmarking.md#5-cross-parser-benchmarks).

Quick steps:
1. Generate data: `cargo run --release --features cli -- json generate-suite`
2. Run: `cd bench-compare && cargo bench --bench json_parsers`
3. Update documentation (see "Updating Documentation" section below)

## Rust JSON Parser Comparison

The `bench-compare/` subproject benchmarks succinctly against other Rust JSON parsers.

### When to Use Each Parser

| Use Case                          | Best Choice      | Why                                      |
|-----------------------------------|------------------|------------------------------------------|
| Full document traversal           | **sonic-rs**     | Fastest parse + traverse (400+ MiB/s)    |
| Selective field access (jq-style) | **succinctly**   | Lazy evaluation skips unused data        |
| Memory-constrained environments   | **succinctly**   | 17-45x less memory than DOM parsers      |
| Standard DOM parsing              | **serde_json**   | Best ecosystem compatibility             |
| SIMD-accelerated DOM              | **simd-json**    | Fast parsing, moderate memory            |

### Key Learnings

1. **sonic-rs is fastest for full document access** - ~670 MiB/s parse, ~420 MiB/s parse+traverse on ARM
2. **succinctly trades speed for memory** - 46% of input size vs 8-21x for DOM parsers
3. **succinctly wins on selective queries** - jq-style queries are 1.2-6.3x faster than `jq` because unused data isn't parsed
4. **simd-json has highest memory overhead** - 20x input size due to tape-based representation

### Performance Characteristics (ARM, Apple M1 Max)

| Parser      | Parse Only | Parse+Traverse | Memory (100MB) |
|-------------|------------|----------------|----------------|
| sonic-rs    | 687 MiB/s  | 425 MiB/s      | 955 MB (11.9x) |
| succinctly  | 534 MiB/s  | 283 MiB/s      | 37 MB (0.46x)  |
| simd-json   | 182 MiB/s  | 227 MiB/s      | 1654 MB (20.7x)|
| serde_json  | 153 MiB/s  | 139 MiB/s      | 655 MB (8.2x)  |
