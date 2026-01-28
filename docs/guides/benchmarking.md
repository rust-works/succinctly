# Benchmarking Guide

This guide provides complete information for running, interpreting, and documenting benchmarks in the succinctly project.

## Table of Contents

- [Overview](#overview)
- [Benchmark Inventory](#benchmark-inventory)
- [Types of Benchmarks](#types-of-benchmarks)
- [When to Run Benchmarks](#when-to-run-benchmarks)
- [How to Run Benchmarks](#how-to-run-benchmarks)
- [Data Generation](#data-generation)
- [Platforms and Hardware](#platforms-and-hardware)
- [CI/CD Integration](#cicd-integration)
- [Interpreting Results](#interpreting-results)
- [Updating Documentation](#updating-documentation)
- [Troubleshooting](#troubleshooting)

---

## Overview

Succinctly uses multiple types of benchmarks to measure performance:

1. **Micro-benchmarks**: Fine-grained measurements of specific operations (rank, select, parsing)
2. **End-to-end benchmarks**: Full pipeline measurements (CLI tool vs competitors)
3. **Cross-parser benchmarks**: Comparison with other Rust JSON parsers
4. **Optimization benchmarks**: Before/after measurements for specific optimizations

All benchmarks use **Criterion.rs** for statistical accuracy with warm cache conditions.

### Benchmark Philosophy

- **Measure first, optimize later**: Always profile before optimizing
- **Micro ≠ macro**: Micro-benchmark wins don't always translate to end-to-end improvements
- **Test on target hardware**: Performance varies significantly across architectures
- **Document everything**: Keep detailed records in `docs/benchmarks/` and `docs/parsing/`

---

## Benchmark Inventory

For a complete index of all benchmark reports by file and section, see:

**[docs/benchmarks/inventory.md](../benchmarks/inventory.md)** - Complete inventory of ~110+ benchmark report sections across 6 files

---

## Types of Benchmarks

### 1. Core Data Structure Benchmarks

Located in `benches/`:

| Benchmark               | Purpose                                    | Data Source      |
|-------------------------|--------------------------------------------|------------------|
| `rank_select`           | BitVec rank/select operations              | Generated inline |
| `balanced_parens`       | Tree navigation operations                 | Generated inline |
| `elias_fano`            | Elias-Fano encoding for monotone sequences | Generated inline |
| `popcount_strategies`   | Popcount implementations                   | Generated inline |

### 2. JSON Benchmarks

| Benchmark               | Purpose                                    | Data Source      |
|-------------------------|--------------------------------------------|------------------|
| `json_simd`             | SIMD implementations comparison            | Generated files  |
| `pfsm_vs_simd`          | Table-based vs SIMD parsing                | Generated files  |
| `pfsm_vs_scalar`        | Table-based vs scalar parsing              | Generated files  |
| `json_pipeline`         | Full pipeline (index + navigate + print)   | Generated files  |
| `jq_comparison`         | succinctly jq vs system jq (CLI)           | Generated files  |

### 3. YAML Benchmarks

| Benchmark               | Purpose                                    | Data Source      |
|-------------------------|--------------------------------------------|------------------|
| `yaml_bench`            | YAML parsing throughput                    | Generated inline |
| `yaml_type_stack_micro` | YAML type stack operations                 | Generated inline |
| `yaml_anchor_micro`     | YAML anchor parsing                        | Generated inline |
| `yaml_transcode_micro`  | YAML→JSON transcoding                      | Generated inline |
| `yq_comparison`         | succinctly yq vs system yq (CLI)           | Generated files  |
| `yq_select`             | Partial selection queries (lazy eval)      | Generated files  |
| `bp_select_micro`       | BP select1 performance                     | Generated inline |

### 4. DSV Benchmarks

| Benchmark               | Purpose                                    | Data Source      |
|-------------------------|--------------------------------------------|------------------|
| `dsv_bench`             | DSV/CSV parsing and access                 | Generated files  |

### 5. Cross-Parser Benchmarks

Located in `bench-compare/benches/`:

| Benchmark               | Purpose                                    | Parsers Compared                           |
|-------------------------|--------------------------------------------|--------------------------------------------|
| `json_parsers`          | Compare against other Rust JSON parsers    | serde_json, simd-json, sonic-rs, succinctly|

**Why separate?** Avoids adding competitor dependencies to the main crate.

### 6. ARM-Specific Benchmarks

| Benchmark               | Purpose                                    | Data Source      |
|-------------------------|--------------------------------------------|------------------|
| `neon_movemask`         | NEON movemask implementations              | Generated inline |

---

## When to Run Benchmarks

### Required: Pre-Release

**Before every release**, run full benchmark suite on all platforms:

```bash
# On x86_64 (Zen 4 or similar)
./scripts/run-full-benchmarks.sh

# On ARM64 (Graviton or Apple Silicon)
./scripts/run-full-benchmarks.sh
```

Update `docs/benchmarks/*.md` with new results if significant changes.

### Recommended: After Optimizations

After implementing a performance optimization:

1. **Run relevant micro-benchmarks** to verify the optimization works
2. **Run end-to-end benchmarks** to verify real-world impact
3. **Document results** in `docs/parsing/yaml.md` or `docs/optimizations/`

Example: After SIMD optimization for YAML:
```bash
# Micro-benchmark
cargo bench --bench yaml_bench

# End-to-end
cargo bench --bench yq_comparison
```

### Optional: During Development

Run targeted benchmarks when:
- Modifying hot paths (parsing, rank/select)
- Investigating performance regressions
- Exploring new optimization ideas

### CI/CD: Automated

GitHub Actions runs `rank_select` benchmarks on every PR/push for smoke testing:
- x86_64 (ubuntu-latest)
- ARM64-Linux (ubuntu-24.04-arm)
- ARM64-macOS (macos-latest)

**Note**: CI does not run full benchmark suite (too time-consuming).

---

## How to Run Benchmarks

### Unified Benchmark Runner (Recommended)

The project includes a unified benchmark runner that provides discovery, listing, and running all benchmarks with automatic metadata tracking.

```bash
# Build with bench-runner feature
cargo build --release --features bench-runner

# List all available benchmarks
./target/release/succinctly bench list

# List by category
./target/release/succinctly bench list --category core
./target/release/succinctly bench list --category json

# Run specific benchmarks
./target/release/succinctly bench run rank_select yaml_bench

# Run all benchmarks in a category
./target/release/succinctly bench run --category core

# Run all benchmarks
./target/release/succinctly bench run --all

# Dry run (see what would execute)
./target/release/succinctly bench run --dry-run --all
```

**Output Location**: Results are saved to `data/bench/results/<timestamp>_<commit>/` with:
- `metadata.json` - Git, system, and toolchain information
- `summary.json` - Run summary with pass/fail status
- `stdout/` - Raw output from each benchmark

### Quick Start (Traditional)

```bash
# 1. Build release binary
cargo build --release --features cli

# 2. Generate test data
cargo run --release --features cli -- json generate-suite
cargo run --release --features cli -- yaml generate-suite
cargo run --release --features cli -- dsv generate-suite

# 3. Run all benchmarks
cargo bench

# 4. Run cross-parser comparison
cd bench-compare
cargo bench
```

### Running Specific Benchmarks

#### Core Data Structures
```bash
cargo bench --bench rank_select        # BitVec operations
cargo bench --bench balanced_parens    # Tree navigation
cargo bench --bench popcount_strategies # Popcount implementations
```

#### JSON Benchmarks
```bash
cargo bench --bench json_simd          # SIMD comparison
cargo bench --bench pfsm_vs_simd       # PFSM vs SIMD
cargo bench --bench pfsm_vs_scalar     # PFSM vs scalar
cargo bench --bench json_pipeline      # Full pipeline
cargo bench --bench jq_comparison      # vs system jq
```

#### YAML Benchmarks
```bash
cargo bench --bench yaml_bench         # Parsing throughput
cargo bench --bench yq_comparison      # vs system yq
cargo bench --bench yq_select          # Lazy evaluation
cargo bench --bench bp_select_micro    # BP select1
```

#### DSV Benchmarks
```bash
cargo bench --bench dsv_bench          # CSV/TSV parsing
```

#### Cross-Parser Comparison
```bash
cd bench-compare

# All benchmarks
cargo bench --bench json_parsers

# Specific groups
cargo bench --bench json_parsers -- "parse_only"
cargo bench --bench json_parsers -- "parse_traverse"
cargo bench --bench json_parsers -- "traverse_only"
cargo bench --bench json_parsers -- "peak_memory"
```

### Running with Specific Features

```bash
# Default (hardware popcount)
cargo bench --bench rank_select

# Explicit SIMD
cargo bench --bench rank_select --features simd

# Portable popcount (broadword)
cargo bench --bench rank_select --features portable-popcount
```

### Running with Native CPU Optimizations

For best performance on your CPU:

```bash
# x86_64 with AVX2/BMI2
RUSTFLAGS="-C target-cpu=native" cargo bench

# Verify features detected
RUSTFLAGS="-C target-cpu=native" cargo rustc -- --print=cfg | grep target_feature
```

### Filtering Benchmarks

```bash
# Run only benchmarks matching pattern
cargo bench -- "rank1"                 # Only rank1 benchmarks
cargo bench -- "10mb"                  # Only 10MB file benchmarks
cargo bench --bench yaml_bench -- "simple_kv"  # Only simple_kv pattern

# Save baseline for comparison
cargo bench -- --save-baseline main

# Compare against baseline
cargo bench -- --baseline main
```

### Controlling Sample Size

```bash
# Quick run (fewer samples)
cargo bench -- --quick

# Specific sample size
cargo bench -- --sample-size 10

# Warm-up iterations
cargo bench -- --warm-up-time 1
```

---

## Data Generation

### JSON Test Data

Generate test files in `data/bench/generated/`:

```bash
# Individual sizes
cargo run --release --features cli -- json generate 1kb -o data/bench/generated/comprehensive/1kb.json
cargo run --release --features cli -- json generate 10kb -o data/bench/generated/comprehensive/10kb.json
cargo run --release --features cli -- json generate 100kb -o data/bench/generated/comprehensive/100kb.json
cargo run --release --features cli -- json generate 1mb -o data/bench/generated/comprehensive/1mb.json
cargo run --release --features cli -- json generate 10mb -o data/bench/generated/comprehensive/10mb.json
cargo run --release --features cli -- json generate 100mb -o data/bench/generated/comprehensive/100mb.json

# All sizes at once
cargo run --release --features cli -- json generate-suite
```

### YAML Test Data

```bash
# All patterns and sizes
cargo run --release --features cli -- yaml generate-suite

# Individual pattern/size
cargo run --release --features cli -- yaml generate comprehensive 1mb -o test.yaml
```

### DSV Test Data

```bash
# All patterns and sizes
cargo run --release --features cli -- dsv generate-suite

# Individual pattern/size
cargo run --release --features cli -- dsv generate users 1000 -o users.csv
```

### Available Patterns

| Format | Patterns |
|--------|----------|
| JSON   | comprehensive, users, nested, arrays, strings, numbers, pathological, mixed, literals, unicode |
| YAML   | comprehensive, users, nested, sequences, strings, numbers, pathological, mixed, unicode, config, navigation |
| DSV    | users, numeric, tabular, mixed, quoted, strings, wide, long, multiline, pathological |

### Standard Sizes

- 1KB, 10KB, 100KB (small files)
- 1MB, 10MB (medium files)
- 100MB (large files)
- 1GB (extra large, rarely used)

---

## Platforms and Hardware

### Primary Benchmark Platforms

#### 1. AMD Ryzen 9 7950X (Zen 4, x86_64)
- **Location**: Local development machine
- **OS**: Linux 6.6.87.2-microsoft-standard-WSL2 (WSL2)
- **SIMD**: AVX2, BMI2 PDEP (3-cycle)
- **Use for**: x86_64 baseline, BMI2 optimizations
- **Access**: Local

#### 2. ARM Neoverse-V2 (AWS Graviton 4)
- **Location**: AWS EC2 instance
- **OS**: Linux 6.14.0-1018-aws
- **SIMD**: NEON (128-bit), SVE2 (128-bit), SVEBITPERM (BDEP/BEXT)
- **Use for**: ARM64 server performance, SVE2 features
- **Access**: Cloud instance (manual provisioning)

#### 3. ARM Neoverse-V1 (AWS Graviton 3)
- **Location**: AWS EC2 instance
- **OS**: Linux 6.14.0-1018-aws
- **SIMD**: NEON (128-bit), SVE (256-bit)
- **Use for**: ARM64 baseline, SVE support
- **Access**: Cloud instance (manual provisioning)

#### 4. Apple M1 Max (Apple Silicon)
- **Location**: Local development machine
- **OS**: macOS Darwin 25.1.0
- **SIMD**: ARM NEON (128-bit)
- **Use for**: Consumer ARM64 performance
- **Access**: Local

### CI/CD Platforms

GitHub Actions runs benchmarks on:
- **ubuntu-latest** (x86_64)
- **ubuntu-24.04-arm** (ARM64)
- **macos-latest** (Apple Silicon M-series)

### Checking CPU Features

#### Linux
```bash
# All features
lscpu | grep -E "Model name|Architecture|CPU|Flags"
cat /proc/cpuinfo | grep flags

# Specific features
grep -q avx2 /proc/cpuinfo && echo "AVX2: supported"
grep -q popcnt /proc/cpuinfo && echo "POPCNT: supported"
grep -q bmi2 /proc/cpuinfo && echo "BMI2: supported"
grep -q sve2 /proc/cpuinfo && echo "SVE2: supported"
grep -q svebitperm /proc/cpuinfo && echo "SVEBITPERM: supported"
```

#### macOS
```bash
sysctl -n machdep.cpu.brand_string
sysctl -n machdep.cpu.features
system_profiler SPHardwareDataType
```

### Performance Characteristics by Platform

| Platform | Popcount | select_in_word | JSON Parse | YAML Parse | Notes |
|----------|----------|----------------|------------|------------|-------|
| AMD Zen 4 | Hardware | BMI2 PDEP (fast) | ~880 MiB/s | ~250-400 MiB/s | 3-cycle PDEP |
| ARM Graviton 4 | Hardware | SVE2 BDEP | ~550 MiB/s | ~250 MiB/s | SVE2 BDEP slower than BMI2 |
| ARM Graviton 3 | Hardware | Broadword | ~550 MiB/s | ~250 MiB/s | No SVE2 BDEP |
| Apple M1 Max | Hardware | Broadword | ~550 MiB/s | ~250 MiB/s | No SVE/SVE2 |

---

## CI/CD Integration

### Automated Benchmarks

The CI workflow (`.github/workflows/ci.yml`) runs benchmarks on every push/PR:

**Test Matrix**:
- x86_64 (ubuntu-latest)
- ARM64-Linux (ubuntu-24.04-arm)
- ARM64-macOS (macos-latest)

**Benchmarks Run**:
```yaml
- cargo bench --bench rank_select -- --noplot
- cargo bench --bench rank_select --features simd -- --noplot
- cargo bench --bench rank_select --features portable-popcount -- --noplot
```

**Artifacts**: Uploaded as `benchmark-results-{arch}` with 3 text files per platform.

### What CI Does NOT Run

CI intentionally skips:
- End-to-end comparison benchmarks (require system jq/yq)
- Cross-parser benchmarks (require external dependencies)
- Large file benchmarks (100MB+, too time-consuming)
- All JSON/YAML/DSV benchmarks (require generated data)

These must be run **manually before releases**.

### Adding Benchmarks to CI

To add a benchmark to CI, edit `.github/workflows/ci.yml`:

```yaml
- name: Run benchmarks (new_bench)
  run: cargo bench --bench new_bench -- --noplot | tee bench-new.txt
```

**Considerations**:
- Keep runtime under 5 minutes per platform
- Use `--noplot` to skip graph generation
- Use `tee` to save results to artifact
- Avoid requiring external tools or large files

---

## Interpreting Results

### Understanding Criterion Output

```
test bench_name ... bench:  1,234 ns/iter (+/- 56)
                              ^^^^^           ^^^^
                              mean           std dev
```

**Key metrics**:
- **Mean**: Average time per iteration
- **Std dev**: Standard deviation (lower = more consistent)
- **Throughput**: MiB/s, GB/s (for data-processing benchmarks)
- **Speedup**: Relative to baseline (e.g., 1.5x, 2.0x)

### Performance Expectations

#### Core Operations (x86_64 Zen 4)

| Operation | Expected Performance | Notes |
|-----------|---------------------|-------|
| rank1 | ~1-2 ns | O(1) with 3-level directory |
| select1 | ~10-20 ns | O(log n) binary search + scan |
| find_close | ~5-10 ns | O(1) with RangeMin |
| popcount (hardware) | ~1-2 ns | Single instruction |
| popcount (portable) | ~10-15 ns | Broadword algorithm |

#### Parsing Throughput (x86_64 Zen 4)

| Parser | Expected Throughput | Notes |
|--------|---------------------|-------|
| JSON (PFSM) | 800-900 MiB/s | Table-driven |
| YAML (oracle) | 250-400 MiB/s | Context-sensitive |
| DSV (API iteration) | 85-1676 MiB/s | Depends on pattern |
| DSV (parse only) | 1.3-3.7 GB/s | Index building |

#### Comparison Benchmarks (Expected Speedups)

| Benchmark | Expected Speedup | vs |
|-----------|------------------|-----|
| jq (x86_64) | 1.1-2.6x | System jq |
| jq (ARM) | 1.0-2.9x | System jq |
| yq (x86_64) | 7-27x | System yq |
| yq (ARM) | 1.3-9.9x | System yq |

### When to Investigate

**Investigate regressions when**:
- Core operations >10% slower than baseline
- End-to-end >5% slower than baseline
- Memory usage >10% higher than baseline
- Speedup vs competitors drops >20%

**Common causes**:
- Branch mispredictions (check with `perf stat`)
- Cache misses (check with `perf stat`)
- Memory bandwidth saturation
- Compiler regression (try different rustc versions)
- System load (ensure clean benchmark environment)

### Micro-Benchmark vs End-to-End

**Critical lesson**: Micro-benchmark wins don't always translate to real-world gains.

**Examples of misleading micro-benchmarks**:
- **P2.8 (SIMD Threshold Tuning)**: +2-4% micro, **-8-15% end-to-end** ❌
- **P3 (Branchless Character Classification)**: +3-29% micro, **-25-44% end-to-end** ❌
- **P2.6 (Software Prefetching)**: Looked promising, **-30% end-to-end** ❌

**Always validate with end-to-end benchmarks** before claiming an optimization works.

---

## Updating Documentation

### When to Update Docs

**Required**:
- After implementing optimization (document in `docs/parsing/yaml.md` or `docs/optimizations/`)
- Before release (update `docs/benchmarks/*.md` if significant changes)

**Optional**:
- After micro-optimizations (if <5% improvement)
- During exploration (use `docs/plan/` for proposals)

### Which Files to Update

#### 1. Optimization Phase Results

**File**: `docs/parsing/yaml.md` (or `docs/parsing/json.md`, `docs/parsing/dsv.md`)

**When**: After implementing an optimization phase

**Format**:
```markdown
### P#: Optimization Name - ACCEPTED ✅ / REJECTED ❌

**Goal**: Brief description of what you tried to optimize

**Approach**: Implementation details

**Benchmark Results** (Platform Name, Date):

| Benchmark | Before | After | Improvement |
|-----------|--------|-------|-------------|
| test/100  | 123µs  | 100µs | **-18.7%** (1.23x faster) |

**Key Findings**:
- Bulleted summary
- Include why accepted/rejected

**See Also**: [yq.md](../benchmarks/yq.md) for end-to-end results
```

#### 2. End-to-End Comparison Results

**Files**: `docs/benchmarks/jq.md`, `docs/benchmarks/yq.md`, `docs/benchmarks/dsv.md`

**When**: Before major releases or after significant optimizations

**Format**: Follow existing table format:

```markdown
### Platform Name (CPU Details)

| Size      | succinctly   | competitor   | Speedup       | succ Mem | comp Mem | Mem Ratio |
|-----------|--------------|--------------|---------------|----------|----------|-----------|
| **1MB**   |   20.5 ms    |  111.2 ms    | **5.4x**      |   10 MB  |   82 MB  | **0.12x** |
```

**Include**:
- Platform details (CPU model, OS, SIMD features)
- Date of benchmark run
- Tool versions (jq/yq version)
- Speedup and memory ratio

#### 3. Cross-Parser Comparison

**Files**: `docs/benchmarks/cross-language.md`, `docs/benchmarks/rust-parsers.md`

**When**: After major changes to JSON parsing or before releases

**Format**: Follow existing table format with all parsers in same table

#### 4. Benchmark Inventory

**File**: `docs/benchmarks/inventory.md`

**When**: Adding new benchmark reports or reorganizing docs

**Update**: Add new sections to the inventory index

### Versioning and Dating

**Always include**:
- **Date**: When benchmark was run (e.g., "2026-01-17")
- **Platform**: CPU model and OS version
- **Tool versions**: For comparison benchmarks (jq-1.6, yq v4.48.1)
- **Build flags**: Any special RUSTFLAGS or features

**Example**:
```markdown
**Date**: 2026-01-17
**Platform**: AMD Ryzen 9 7950X (Zen 4, x86_64)
**OS**: Linux 6.6.87.2-microsoft-standard-WSL2
**Build**: `RUSTFLAGS="-C target-cpu=native" cargo build --release`
```

### Formatting Tables

Use fixed-width alignment for readability:

```markdown
| Size      | succinctly   | competitor   | Speedup       |
|-----------|--------------|--------------|---------------|
| **1KB**   |    2.5 ms    |   58.8 ms    | **24x**       |
| **10KB**  |    2.6 ms    |   59.9 ms    | **23x**       |
```

**Tips**:
- Bold the size column for emphasis
- Bold speedup numbers that exceed baseline
- Right-align numeric columns
- Use consistent units (ms, µs, ns, MiB/s)

---

## Troubleshooting

### Benchmark Won't Run

#### Error: "No such file or directory"

**Cause**: Missing test data

**Fix**:
```bash
# Generate all test data
cargo run --release --features cli -- json generate-suite
cargo run --release --features cli -- yaml generate-suite
cargo run --release --features cli -- dsv generate-suite
```

#### Error: "Command 'jq' not found"

**Cause**: Comparison benchmarks require system tools

**Fix**:
```bash
# Ubuntu/Debian
sudo apt-get install jq

# macOS
brew install jq

# For yq
sudo wget https://github.com/mikefarah/yq/releases/latest/download/yq_linux_amd64 -O /usr/local/bin/yq
sudo chmod +x /usr/local/bin/yq
```

#### Error: "Benchmark takes too long"

**Cause**: Large file benchmarks (100MB+) can take 10+ minutes

**Fix**: Use `--quick` flag or filter to smaller sizes
```bash
cargo bench -- --quick
cargo bench -- "1mb"  # Only 1MB files
```

### Inconsistent Results

#### High Standard Deviation

**Causes**:
- System load (other processes running)
- Thermal throttling
- Power management

**Fix**:
```bash
# Close other applications
# Disable CPU frequency scaling (Linux)
sudo cpupower frequency-set --governor performance

# Run with higher sample size
cargo bench -- --sample-size 100
```

#### Results Vary Between Runs

**Causes**:
- Cache state differences
- Memory allocator non-determinism
- Background processes

**Fix**:
- Run multiple times and average
- Use `--warm-up-time` for longer warm-up
- Check for background processes (`top`, `htop`)

### Cross-Parser Benchmark Issues

#### Error: "Failed to parse with simd-json"

**Cause**: simd-json requires mutable input

**Fix**: Already handled in `bench-compare`, but verify test data is valid JSON

#### Memory Measurements Don't Match Docs

**Cause**: Allocator differences or OS differences

**Fix**: Use tracking allocator in `bench-compare` (already implemented)

### CI Benchmark Failures

#### Timeout

**Cause**: Benchmark takes too long on CI runners

**Fix**: Reduce sample size or filter benchmarks in `.github/workflows/ci.yml`

#### Artifact Upload Failed

**Cause**: File size too large or network issue

**Fix**: Use `--noplot` to skip graphs, check file size

---

## Advanced Topics

### Profiling with Perf

```bash
# Build with debug symbols
cargo build --release

# Record profile
perf record --call-graph=dwarf ./target/release/succinctly jq '.users[]' large.json

# Analyze
perf report
```

### Flame Graphs

```bash
# Install flamegraph
cargo install flamegraph

# Generate flame graph
cargo flamegraph --bench yaml_bench -- --bench
```

### Comparing Multiple Baselines

```bash
# Save baseline before optimization
git checkout main
cargo bench -- --save-baseline main

# Implement optimization
git checkout feature-branch

# Compare
cargo bench -- --baseline main
```

### Custom Benchmark Filters

```bash
# Multiple patterns (OR)
cargo bench -- "rank1|select1"

# Case insensitive
cargo bench -- "(?i)RANK"

# Exclude pattern
cargo bench -- --skip "100mb"
```

---

## See Also

- [docs/benchmarks/inventory.md](../benchmarks/inventory.md) - Complete benchmark report index
- [docs/benchmarks/README.md](../benchmarks/README.md) - Benchmark results overview
- [docs/guides/developer.md](developer.md) - General development workflow
- [docs/guides/release.md](release.md) - Release process (includes benchmark requirements)
- [docs/optimizations/history.md](../optimizations/history.md) - Optimization history and lessons learned
