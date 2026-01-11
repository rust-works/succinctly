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
| `docs/jq-comparison.md`        | Curated benchmark documentation| tracked      |
| `docs/jq-comparison-m1.jsonl`  | Apple M1 Max raw data          | tracked      |
| `docs/jq-comparison-zen4.jsonl`| AMD Zen 4 raw data             | tracked      |
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

1. **`docs/jq-comparison.md`** - Full benchmark results with all patterns and sizes
2. **`README.md`** - Highlights only (summary table with key patterns/sizes)
   - Link to `docs/jq-comparison.md` for detailed results

### Step 4: Guidelines for Curating Results

- Keep platform-specific sections (ARM vs x86_64)
- Don't replace one platform's data with another
- README should highlight best-case speedups and representative patterns
- `docs/jq-comparison.md` should have comprehensive results

## README Table Requirements

### Platform Parity

**Every benchmark table in README.md must have data for BOTH platforms:**
- x86_64 (AMD Ryzen 9 7950X or equivalent)
- ARM (Apple M1 Max or equivalent)

If a table only shows one platform, add the missing platform's data from `docs/jq-comparison.md`.

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
```
