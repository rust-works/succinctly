---
name: benchmark-docs
description: Manages benchmark documentation across multiple platforms. Use when updating benchmark results, adding performance data, or documenting jq comparison benchmarks. Triggers on terms like "benchmark", "performance", "jq comparison", "benchmark results", "update benchmarks".
---

# Benchmark Documentation Skill

This skill ensures proper handling of benchmark documentation across multiple platforms (ARM/Apple Silicon and x86_64/Intel/AMD).

## Critical Rules

**NEVER replace platform-specific benchmarks with each other.**

When adding new benchmark data:

1. **Keep both ARM and x86_64 benchmarks** - Never delete one platform's data when adding another
2. **Add new platform data alongside existing data** - Create separate sections or files per platform
3. **Name files with platform suffix** - e.g., `jq-comparison-m1.jsonl`, `jq-comparison-zen4.jsonl`
4. **Update docs to show both** - README and docs should reference benchmarks from all platforms

## Benchmark File Locations

| Location                       | Purpose                        | Git Status   |
|--------------------------------|--------------------------------|--------------|
| `data/bench/generated/`        | Input JSON files               | git-ignored  |
| `data/bench/results/`          | Benchmark output (JSONL, MD)   | git-ignored  |
| `docs/benchmarks/jq.md`        | Curated benchmark documentation| tracked      |
| `docs/benchmarks/yq.md`        | yq comparison benchmarks       | tracked      |
| `docs/benchmarks/dsv.md`       | DSV performance benchmarks     | tracked      |
| `README.md`                    | Summary benchmarks             | tracked      |

## Updating Benchmark Documentation

### Step 1: Run Benchmarks

```bash
# Build release binary
cargo build --release --features cli

# Run full benchmark suite
./target/release/succinctly dev bench jq

# Or run specific patterns/sizes
./target/release/succinctly dev bench jq --patterns nested,strings --sizes 1kb,10kb,100kb,1mb
```

### Step 2: Review Output

Check `data/bench/results/jq-bench.md` for the generated markdown tables.

### Step 3: Update Documentation

Update in **two places**:

1. **`docs/benchmarks/jq.md`** - Full benchmark results with all patterns and sizes
2. **`README.md`** - Highlights only (summary table with key patterns/sizes)
   - Link to `docs/benchmarks/jq.md` for detailed results

### Step 4: Guidelines for Curating Results

- Keep platform-specific sections (ARM vs x86_64)
- Don't replace one platform's data with another
- README should highlight best-case speedups and representative patterns
- `docs/benchmarks/jq.md` should have comprehensive results

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

Always check for heavy CPU activity that could skew results:

```bash
# Check CPU activity (macOS)
top -l 1 -n 5 | head -20

# Check CPU activity (Linux)
top -bn1 | head -20

# Alternative: check load average
uptime
```

Wait for CPU load to settle (ideally below 1.0) before running benchmarks.

## Benchmark Commands Reference

```bash
# Generate benchmark data
./target/release/succinctly json generate-suite
./target/release/succinctly json generate-suite --max-size 10mb

# Run jq comparison benchmarks (recommended)
./target/release/succinctly dev bench jq

# Run criterion benchmarks
cargo bench --bench json_simd
cargo bench --bench balanced_parens
cargo bench --bench jq_comparison

# JSON parser comparison (in bench-compare/)
cd bench-compare && cargo bench --bench json_parsers
```

## Running bench-compare Benchmarks

The `bench-compare/` subproject requires test data to be generated first.

### Step 1: Generate Test Data

```bash
# From project root, generate all sizes
cargo run --release --features cli -- json generate 1kb -o data/bench/generated/comprehensive/1kb.json
cargo run --release --features cli -- json generate 10kb -o data/bench/generated/comprehensive/10kb.json
cargo run --release --features cli -- json generate 100kb -o data/bench/generated/comprehensive/100kb.json
cargo run --release --features cli -- json generate 1mb -o data/bench/generated/comprehensive/1mb.json
cargo run --release --features cli -- json generate 10mb -o data/bench/generated/comprehensive/10mb.json
cargo run --release --features cli -- json generate 100mb -o data/bench/generated/comprehensive/100mb.json
```

### Step 2: Run Benchmarks

```bash
cd bench-compare
cargo bench --bench json_parsers
```

### Step 3: Update Documentation

After running, manually extract results from criterion output and update:
- `bench-compare/README.md` - Full results with all sizes
- `.claude/skills/benchmark-docs/SKILL.md` - Summary table

### Key Points

- Benchmarks compare: serde_json, simd-json, sonic-rs, succinctly
- Test files must exist in `data/bench/generated/comprehensive/`
- 1GB file is excluded (too large for criterion's sample requirements)
- Memory measurements use a custom peak-tracking allocator

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
