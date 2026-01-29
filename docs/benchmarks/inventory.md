# Benchmark Inventory

This document provides a comprehensive index of all benchmark reports in the project.

## Overview

The project contains **7 main benchmark documentation files** with approximately **120+ distinct benchmark report sections**.

| File | Description | Sections |
|------|-------------|----------|
| [jq.md](jq.md) | JSON query performance vs system jq | ~40 |
| [yq.md](yq.md) | YAML query performance vs system yq | ~35 |
| [dsv.md](dsv.md) | CSV/TSV parsing performance | ~8 |
| [cross-language.md](cross-language.md) | Multi-parser JSON comparison | ~15 |
| [rust-parsers.md](rust-parsers.md) | Rust JSON parser comparison | ~12 |
| [rust-yaml-parsers.md](rust-yaml-parsers.md) | Rust YAML parser comparison | ~10 |
| [../parsing/yaml.md](../parsing/yaml.md) | YAML optimization phases | ~15 |
| [../parsing/json.md](../parsing/json.md) | JSON optimization phases | ~2 |

---

## [jq.md](jq.md) - JSON Query Benchmarks

Comprehensive benchmarks comparing `succinctly jq .` vs `jq .` for JSON formatting/printing.

### Platforms Covered
- AMD Ryzen 9 7950X (Zen 4, x86_64)
- ARM Neoverse-V2 (AWS Graviton 4)
- ARM Neoverse-V1 (AWS Graviton 3)
- Apple M1 Max (ARM)

### Benchmark Sections

#### AMD Ryzen 9 7950X (Zen 4, x86_64)
- Pattern: arrays
- Pattern: comprehensive
- Pattern: literals
- Pattern: mixed
- Pattern: nested
- Pattern: numbers
- Pattern: pathological
- Pattern: strings
- Pattern: unicode
- Pattern: users

#### ARM Neoverse-V2 (AWS Graviton 4)
- Summary - Comprehensive Pattern
- Pattern: comprehensive
- Pattern: users
- Pattern: nested
- Pattern: arrays
- Pattern: strings
- Key Findings

#### ARM Neoverse-V1 (AWS Graviton 3)
- Summary - Comprehensive Pattern (all sizes 1KB-100MB)
- Pattern: arrays
- Pattern: comprehensive
- Pattern: literals
- Pattern: mixed
- Pattern: nested
- Pattern: numbers
- Pattern: pathological
- Pattern: strings
- Pattern: unicode
- Pattern: users
- Key Findings

#### Apple M1 Max (ARM)
- Pattern: arrays
- Pattern: comprehensive
- Pattern: literals
- Pattern: mixed
- Pattern: nested
- Pattern: numbers
- Pattern: pathological
- Pattern: strings
- Pattern: unicode
- Pattern: users

---

## [yq.md](yq.md) - YAML Query Benchmarks

Benchmarks comparing `succinctly yq .` vs `yq .` (Mike Farah's yq) for YAML formatting/printing.

### Platforms Covered
- ARM Neoverse-V2 (AWS Graviton 4)
- ARM Neoverse-V1 (AWS Graviton 3)
- Apple M1 Max
- AMD Ryzen 9 7950X (Zen 4)

### Benchmark Sections

#### Summary Results
- ARM (Neoverse-V2 / Graviton 4) - yq Identity Comparison (comprehensive pattern)
- ARM (Neoverse-V1 / Graviton 3) - succinctly yq Performance
- ARM (Apple M1 Max) - yq Identity Comparison (comprehensive pattern)
- x86_64 (AMD Ryzen 9 7950X) - yq Identity Comparison (comprehensive pattern)
- x86_64 (AMD Ryzen 9 7950X) - Key Achievements

#### Detailed Results by Pattern (ARM - Neoverse-V2 / Graviton 4)
- Pattern: comprehensive
- Pattern: nested
- Pattern: pathological
- Pattern: users
- Pattern: sequences
- Pattern: strings
- Pattern: numbers
- Pattern: unicode
- Pattern: mixed

#### Detailed Results by Pattern (ARM - Neoverse-V1 / Graviton 3)
- Pattern: comprehensive
- Pattern: config
- Pattern: mixed
- Pattern: navigation
- Pattern: nested
- Pattern: numbers
- Pattern: pathological
- Pattern: sequences
- Pattern: strings
- Pattern: unicode
- Pattern: users

#### Detailed Results by Pattern (ARM - Apple M1 Max)
- Pattern: comprehensive
- Pattern: nested
- Pattern: pathological
- Pattern: users
- Pattern: sequences
- Pattern: strings
- Pattern: numbers
- Pattern: unicode
- Pattern: mixed

#### Detailed Results by Pattern (x86_64 - AMD Ryzen 9 7950X)
- Pattern: comprehensive
- Pattern: config
- Pattern: mixed
- Pattern: navigation
- Pattern: nested
- Pattern: numbers
- Pattern: pathological
- Pattern: sequences
- Pattern: strings
- Pattern: unicode
- Pattern: users

#### Selection Benchmarks (Lazy Evaluation)
- Results (ARM Neoverse-V2 / Graviton 4) - 5% Slice Selection
- Single Field Extraction (first, middle, end positions)
- Key findings

#### BP Select1 Micro-benchmark (P11)
- Results (Apple M1 Max, 10K queries each)
- Results (ARM Neoverse-V2, 10K queries each)
- Results (AMD Ryzen 9 7950X with BMI2 PDEP, 10K queries each)
- Key Observations
- Why It Matters

#### M2 Streaming Navigation Benchmarks
- Query Types and Execution Paths
- Benchmark Results (Apple M1 Max, navigation pattern) - 10MB file
- Benchmark Results (Apple M1 Max, navigation pattern) - 100MB file
- Key insights
- Running M2-Focused Benchmarks
- When M2 Streaming Helps
- When OwnedValue is Required

---

## [dsv.md](dsv.md) - CSV/TSV Parsing Benchmarks

Performance benchmarks for DSV parsing via `succinctly jq --input-dsv`.

### Platforms Covered
- Apple M1 Max (ARM)
- AMD Ryzen 9 7950X (x86_64)
- ARM Neoverse-V2 (AWS Graviton 4)
- ARM Neoverse-V1 (AWS Graviton 3)

### Benchmark Sections

#### Summary
- Platform comparison table (CLI Throughput, API Iteration, Parse Speed)

#### CLI Throughput (End-to-End)
- ARM (M1 Max) - 10MB Files (10 patterns)
- x86_64 (Ryzen 9) - 10MB Files (10 patterns)
- ARM Neoverse-V2 (Graviton 4) - 10MB Files (10 patterns)
- ARM Neoverse-V1 (Graviton 3) - 10MB Files (10 patterns) with full results by pattern

#### Query Comparison
- Single column selection (.[0]) vs full output (.) - x86_64

#### Library API Performance (x86_64)
- Sequential Iteration (3 patterns)
- Random Access (2 patterns)
- Parse Speed (Index Build Only) (2 patterns)

#### Memory Usage
- Memory overhead by file size (1MB, 10MB, 100MB)

#### Optimization Impact
- NEON PMULL Prefix XOR (2026-01-22)
- Lightweight Index (2026-01-12)

---

## [cross-language.md](cross-language.md) - Cross-Parser JSON Benchmarks

Comprehensive benchmarks comparing **succinctly** against other Rust JSON parsers.

### Parsers Compared
- serde_json (Standard DOM parser)
- simd-json (SIMD-accelerated parser)
- sonic-rs (SIMD + arena-based parser)
- succinctly (Semi-index with balanced parentheses)

### Platforms Covered
- x86_64 (AMD Zen 4)
- Apple Silicon (M-series)
- ARM Neoverse-V2 (AWS Graviton 4) - references to end-to-end benchmarks
- ARM Neoverse-V1 (AWS Graviton 3) - references to end-to-end benchmarks

### Benchmark Sections

#### Parse Only Performance
- Parse Only - Apple Silicon (M-series) - All sizes (1KB to 100MB)
- Parse Only - x86_64 (AMD Zen 4) - 10MB and 100MB

#### Parse + Traverse Performance
- Parse + Traverse - Apple Silicon (M-series) - All sizes (1KB to 100MB)
- Parse + Traverse - x86_64 (AMD Zen 4) - 10MB and 100MB

#### Peak Memory Usage
- Peak Memory Usage - Apple Silicon (M-series) - All sizes (1KB to 100MB)

#### Detailed x86_64 Results
- Parse Only - Comprehensive Pattern (x86_64 Zen 4) - All sizes (1KB to 1GB)
- Parse Only - 10MB Detailed Comparison
- Parse Only - 100MB Detailed Comparison
- Parse + Traverse - Comprehensive Pattern (x86_64 Zen 4) - All sizes (1KB to 1GB)
- Parse + Traverse - 10MB Detailed Comparison

#### Memory Overhead Comparison
- Summary (x86_64 AMD Zen 4)
- Detailed Measurements by Size:
  - 1KB JSON
  - 10KB JSON
  - 100KB JSON
  - 1MB JSON
  - 10MB JSON
  - 100MB JSON

#### Pattern-Specific Benchmarks
- Pattern: Pathological (Deeply Nested) - Pending
- Pattern: Numbers (Numeric Arrays) - Pending
- Pattern: Strings (String-Heavy) - Pending

---

## [rust-parsers.md](rust-parsers.md) - Rust JSON Parser Comparison

Benchmark comparison of succinctly against other popular Rust JSON parsers.

### Libraries Compared
- succinctly (Semi-index, streaming)
- sonic-rs (DOM, arena-based)
- serde_json (DOM, standard)
- simd-json (DOM, SIMD-accelerated)

### Platform
- x86_64 (AMD Ryzen 9 7950X, Zen 4)

### Benchmark Sections

#### Parse-Only Performance
- Summary Table (1MB file)
- Detailed Results by File Size:
  - 1KB Files
  - 10KB Files
  - 100KB Files
  - 1MB Files
  - 10MB Files
  - 100MB Files

#### Peak Memory Usage
- Summary Table (all sizes)
- Memory Efficiency (vs JSON size)
- Memory Comparison (1MB file)

#### Key Findings
- Parse Performance
- Memory Efficiency
- Use Case Recommendations

---

## [rust-yaml-parsers.md](rust-yaml-parsers.md) - Rust YAML Parser Comparison

Benchmark comparison of succinctly against serde_yaml, the standard Rust YAML parser.

### Libraries Compared
- serde_yaml (Standard YAML parser, based on yaml-rust2)
- succinctly (Semi-index, streaming)

### Platform
- x86_64 (AMD Ryzen 9 7950X, Zen 4)

### Benchmark Sections

#### Parse-Only Performance
- Summary Table (1MB file)
- Detailed Results by File Size:
  - 1KB Files
  - 10KB Files
  - 100KB Files
  - 1MB Files
  - 10MB Files

#### Parse + Traverse Performance
- Summary Table (1MB file)
- Detailed Results by File Size (1KB to 10MB)

#### YAML to JSON Conversion
- Summary Table (1MB file)
- Detailed Results by File Size (1KB to 10MB)

#### Peak Memory Usage
- Summary Table (all sizes)
- Memory Efficiency (vs YAML size)

#### Key Findings
- Parse Performance (8-14x faster)
- Parse + Traverse Performance (4-6x faster)
- YAML to JSON Conversion (2.8-3.9x faster)
- Memory Efficiency (10-39x less memory)
- Use Case Recommendations

---

## [../parsing/json.md](../parsing/json.md) - JSON Optimization Phases

JSON parsing implementation and optimization benchmark results.

### Benchmark Sections

#### O1: Sequential Cursor for IB Select - ACCEPTED ✅
- Micro-benchmark Results (select_patterns, 10K queries)
- End-to-end jq queries (criterion, 10KB-1MB)
- CLI benchmark results (`succinctly dev bench jq`)
- Key Design Decisions
- Comparison with YAML O1

#### O2: Gap-Skipping via Binary Search - ACCEPTED ✅
- Micro-benchmark Results (select_patterns, 10K queries)
- CLI benchmark results (`succinctly dev bench jq`)
- Key Design Decisions
- Why neutral impact (forward-gap path rarely hit)

---

## [../parsing/yaml.md](../parsing/yaml.md) - YAML Optimization Phases

YAML parsing implementation and optimization benchmark results.

### Benchmark Sections

#### Baseline and General Optimizations
- Benchmark Results (Apple M1 Max ARM64 NEON)
  - String Scanning
  - Indentation Scanning
  - Unquoted Structural Scanning (Chunked Skip)
- AMD Ryzen 9 7950X - Baseline Results (2026-01-17)
  - Simple Key-Value Pairs
  - Nested Structures
  - Sequences
  - Quoted Strings (Regular)
  - Long Quoted Strings (100 strings each)
  - Large Files
- AMD Ryzen 9 7950X - P0+ Optimized Results (2026-01-17)
  - Performance Improvements vs Baseline
  - Selected Benchmark Improvements
  - Implementation Details

#### Optimization Phases (Accepted)

##### P2.5: Cached Type Checking - IMPLEMENTED ✅
- Benchmark Results (multiple nesting depths)
- Performance impact by workload type
- Best use cases

##### P2.7: Block Scalar SIMD - ACCEPTED ✅
- Micro-benchmark Results
- End-to-end Results
- Throughput Improvements
- ARM64 NEON Results (Apple M1 Max) - 2026-01-22

##### P4: Anchor/Alias SIMD - ACCEPTED ✅
- Micro-benchmark Results (32-128 byte anchor names)
- End-to-end Results
  - anchors/100
  - anchors/1000
  - k8s benchmarks
- ARM64 NEON Results (Apple M1 Max) - 2026-01-22

##### P9: Direct YAML-to-JSON Streaming - ACCEPTED ✅
- Before/After Comparison
- Performance Improvements by File Size (10KB, 100KB, 1MB)
- Bottleneck Analysis

##### P10: Type Preservation - ACCEPTED ✅
- Correctness fix (quoted strings preserved)
- Performance impact
- Test suite results

##### P11: BP Select1 for yq-locate - ACCEPTED ✅
- Covered in [yq.md](yq.md) BP Select1 Micro-benchmark section

##### P12: Advance Index for bp_to_text - ACCEPTED ✅
- Memory compression measurements
- End-to-end yq benchmarks (users/1mb, sequences/1mb, nested/1mb)
- yaml_bench improvements
- Cache locality improvements

##### P12-A: Build Regression Mitigation - ACCEPTED ✅
- A1: Inline zero-filling results
- A2: Combined monotonicity check results
- A4: Lazy newline index results
- Improvements by content type (long strings, block scalars, simple_kv, nested)

##### O1: Sequential Cursor for AdvancePositions - ACCEPTED ✅
- End-to-end yq benchmarks (users/ workload, 1KB to 1MB)
- Impact by file size
- Best use cases

##### O2: Gap-Skipping via advance_rank1 - ACCEPTED ✅
- End-to-end yq benchmarks (nested/1kb, users/10kb, strings/100kb)
- Impact analysis

#### Optimization Phases (Rejected)

##### P1: YFSM (YAML Finite State Machine) - REJECTED ❌
- Micro-benchmark results
- Why rejected (too simple grammar, P0+ already optimal)

##### P2.6: Software Prefetching for Large Files - REJECTED ❌
- Benchmark results showing 30% regression
- Analysis of hardware vs software prefetching

##### P2.8: SIMD Threshold Tuning - REJECTED ❌
- Micro-benchmark results (2-4% gain)
- End-to-end results (8-15% regression)
- Analysis of mismatch

##### P3: Branchless Character Classification - REJECTED ❌
- Micro-benchmark results (3-29% improvement)
- End-to-end results (25-44% regression)
- Analysis of branch prediction vs lookup tables

##### P5: Flow Collection Fast Path - REJECTED ❌
- Micro-benchmark analysis (8-14x SIMD wins for 64-128 bytes)
- Real-world size distribution analysis (90% < 30 bytes)
- Why rejected during analysis phase

##### P6: BMI2 Operations (PDEP/PEXT) - REJECTED ❌
- Grammar compatibility analysis
- Comparison with DSV approach
- Why quote indexing doesn't work for YAML

##### P7: Newline Index - REJECTED ❌
- Use case analysis (CLI feature vs parsing optimization)
- Comparison with JSON's NewlineIndex
- Why it's not a parsing optimization

##### P8: AVX-512 Variants - REJECTED ❌
- Micro-benchmark design flaw analysis
- Realistic size results (AMD Ryzen 9 7950X, 64B-256B)
- Memory bandwidth bottleneck analysis
- Comparison with JSON AVX-512 precedent

#### Additional ARM Benchmarks

##### P4: NEON classify_yaml_chars Port - REJECTED ❌
- Benchmark results
- Analysis of why NEON port wasn't beneficial

##### P4: Pure Broadword (SWAR) Classification - TESTED, NEUTRAL ⚖️
- Test results
- Performance comparison

##### Portable Broadword Module - IMPLEMENTED ✅
- Implementation details
- Platform support

##### Broadword vs Scalar Performance - MEASURED ✅
- Performance measurements
- Use cases

---

## Finding Specific Benchmarks

### By Platform
- **AMD Ryzen 9 7950X (x86_64)**: [jq.md](jq.md), [yq.md](yq.md), [dsv.md](dsv.md), [cross-language.md](cross-language.md), [rust-parsers.md](rust-parsers.md), [yaml.md](../parsing/yaml.md)
- **ARM Neoverse-V2 (Graviton 4)**: [jq.md](jq.md), [yq.md](yq.md), [dsv.md](dsv.md)
- **ARM Neoverse-V1 (Graviton 3)**: [jq.md](jq.md), [yq.md](yq.md), [dsv.md](dsv.md)
- **Apple M1 Max**: [jq.md](jq.md), [yq.md](yq.md), [dsv.md](dsv.md), [cross-language.md](cross-language.md), [yaml.md](../parsing/yaml.md)

### By Format
- **JSON**: [jq.md](jq.md), [cross-language.md](cross-language.md), [rust-parsers.md](rust-parsers.md)
- **YAML**: [yq.md](yq.md), [rust-yaml-parsers.md](rust-yaml-parsers.md), [yaml.md](../parsing/yaml.md)
- **DSV/CSV/TSV**: [dsv.md](dsv.md)

### By Comparison Type
- **vs External Tools**: [jq.md](jq.md) (vs jq), [yq.md](yq.md) (vs yq)
- **vs Rust Parsers**: [cross-language.md](cross-language.md), [rust-parsers.md](rust-parsers.md), [rust-yaml-parsers.md](rust-yaml-parsers.md)
- **Optimization Phases**: [yaml.md](../parsing/yaml.md)
- **Memory Benchmarks**: [cross-language.md](cross-language.md), [rust-parsers.md](rust-parsers.md), [rust-yaml-parsers.md](rust-yaml-parsers.md), [yq.md](yq.md)

### By Feature
- **Lazy Evaluation**: [yq.md](yq.md) - Selection Benchmarks
- **Streaming Navigation**: [yq.md](yq.md) - M2 Streaming Navigation Benchmarks
- **SIMD Optimizations**: [yaml.md](../parsing/yaml.md) - P0+, P2.7, P4
- **BP Select1**: [yq.md](yq.md) - BP Select1 Micro-benchmark

---

## Benchmark Methodology

All benchmarks follow these principles:

### Tools
- **Criterion.rs**: Statistical accuracy for micro-benchmarks
- **CLI benchmarking**: End-to-end performance with real tools
- **Memory tracking**: Custom allocators for peak memory measurement

### Conditions
- **Warm cache**: All benchmarks run with data in cache
- **Multiple runs**: Statistical sampling for reliability
- **MD5 validation**: Output correctness verification
- **Multiple file sizes**: Typically 1KB, 10KB, 100KB, 1MB, 10MB, 100MB
- **Multiple patterns**: Comprehensive, users, nested, pathological, strings, etc.

### Reporting
- **Throughput**: MiB/s or GB/s
- **Wall time**: Total elapsed time
- **Memory**: Peak RSS (Resident Set Size)
- **Speedup**: Relative to baseline or competitor
- **Memory ratio**: Overhead vs input size

---

## Reproducing Benchmarks

### Generate Test Data
```bash
# JSON
cargo run --release --features cli -- json generate-suite

# YAML
cargo run --release --features cli -- yaml generate-suite

# DSV
cargo run --release --features cli -- dsv generate-suite
```

### Run Benchmarks
```bash
# CLI benchmarks
./target/release/succinctly dev bench jq
./target/release/succinctly dev bench yq
./target/release/succinctly dev bench dsv

# Criterion benchmarks
cargo bench --bench jq_comparison
cargo bench --bench yq_comparison
cargo bench --bench yq_select
cargo bench --bench yaml_bench
cargo bench --bench json_parsers

# Cross-parser comparison
cd bench-compare
cargo bench --bench json_parsers
```

---

## Related Documentation

- [README.md](README.md) - Overview of benchmark results
- [../optimizations/](../optimizations/) - Optimization techniques used
- [../architecture/semi-indexing.md](../architecture/semi-indexing.md) - Semi-indexing architecture
- [../../CLAUDE.md](../../CLAUDE.md) - Project overview and development guide
