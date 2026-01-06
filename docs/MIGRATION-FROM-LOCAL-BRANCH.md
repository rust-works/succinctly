# Migration Guide: Restoring Features from Local Branch

**Date**: 2026-01-06
**Branch divergence**: Local `main` vs `origin/main`
**Common ancestor**: `e10359ba`

---

## Overview

This document describes the features and documentation added to the local branch (commit `a6fc5f5`) that need to be restored after switching to `origin/main`.

The local branch contains:
1. ✅ **2 new documentation files** (high value, no conflicts)
2. ⚠️ **Corpus generation feature** (conflicts with `generate-suite` on origin/main)
3. ⚠️ **Corpus benchmark** (conflicts with new benchmark structure)

---

## Summary of Changes

### Local Branch (1 commit ahead: a6fc5f5)

**Added Files** (no conflicts):
- `docs/optimization-opportunities.md` (693 lines) - CPU optimization analysis
- `docs/runtime-dispatch-implications.md` (567 lines) - Runtime dispatch deep-dive
- `libtest_no_std.rlib` (binary, should be gitignored)
- `tests/snapshots/cli_golden_tests__help_json_generate_corpus.snap`

**Modified Files** (potential conflicts):
- `benches/json_simd.rs` - Added `bench_corpus()` function (+75 lines)
- `src/bin/succinctly/main.rs` - Added `generate-corpus` command (+93 lines)
- `src/bin/succinctly/generators.rs` - Minor tweaks (+8/-6 lines)
- `tests/cli_golden_tests.rs` - Added corpus test (+53 lines)
- `tests/snapshots/cli_golden_tests__help_json.snap` - Updated help text
- `.gitignore` - Added `corpus/` directory

### Origin/Main (36 commits ahead: 906c01a)

**Major changes**:
- Removed `jq` module entirely (~14,000 lines) - **Now re-added!**
- Added `generate-suite` command (different from local's `generate-corpus`)
- Refactored `json_simd.rs` benchmarks to use file discovery pattern
- New benchmark structure: `data/bench/generated/{pattern}/{size}.json`
- Enhanced `BitWriter` functionality
- Improved NEON implementation

**Deleted on origin/main**:
- `bench-compare/` directory
- `benches/balanced_parens.rs`
- `benches/json_pipeline.rs`
- `benches/neon_movemask.rs`
- `docs/performance-analysis.md`
- `docs/jq-implementation-plan.md`
- `examples/size_comparison.rs`
- `tests/bp_properties.rs`

---

## Migration Strategy

### Phase 1: Restore Documentation (ZERO CONFLICTS)

These files can be added directly with no conflicts:

```bash
# Cherry-pick just the documentation from local commit
git checkout a6fc5f5 -- docs/optimization-opportunities.md
git checkout a6fc5f5 -- docs/runtime-dispatch-implications.md

# Commit them
git add docs/optimization-opportunities.md docs/runtime-dispatch-implications.md
git commit -m "Add CPU optimization analysis and runtime dispatch implications

- Comprehensive optimization opportunities for AMD Ryzen 9 7950X (Zen 4)
- Analysis of AVX-512, AVX2, BMI2, and AVX512-VPOPCNTDQ opportunities
- Runtime dispatch implications for embedded and no_std platforms
- Expected performance improvements: 1.5-3.6x for JSON parsing

Documentation identifies:
- Critical issue: Runtime dispatch disabled in production (only active in tests)
- AVX-512 implementation opportunity (64 bytes/iteration)
- AVX512-VPOPCNTDQ for 8x parallel popcount
- BMI2 integration for bit packing (fast on Zen 3+)
- No-std compatibility maintained throughout"
```

**Status**: ✅ SAFE - No conflicts, high value

---

### Phase 2: Adapt Corpus Generation (REQUIRES REFACTORING)

The local `generate-corpus` feature conflicts with origin's `generate-suite`. Both serve similar purposes but have different implementations.

#### Comparison: `generate-corpus` vs `generate-suite`

| Feature | Local `generate-corpus` | Origin `generate-suite` |
|---------|------------------------|-------------------------|
| Output dir | `corpus/` (flat) | `data/bench/generated/{pattern}/` (hierarchical) |
| Sizes | 1KB, 16KB, 128KB, 1MB, 16MB | 1KB, 10KB, 100KB, 1MB, 10MB, 100MB, 1GB |
| Structure | `{pattern}_{size}.json` | `{pattern}/{size}.json` |
| Max size | No limit | Configurable (`--max-size`) |
| Clean option | No | Yes (`--clean`) |
| Patterns | 10 patterns | 10 patterns (same) |
| Purpose | Benchmarking corpus | Comprehensive suite |

**Origin's `generate-suite` is more feature-complete**:
- ✅ Hierarchical organization (better for large suites)
- ✅ Max size limiting (prevents accidental 1GB generation)
- ✅ Clean option (easier iteration)
- ✅ More size options (7 sizes vs 5)

**Recommendation**: **Don't port `generate-corpus`** - Use `generate-suite` instead

The corpus feature was meant for benchmarking. Origin's `generate-suite` already does this better.

---

### Phase 3: Adapt Corpus Benchmark (OPTIONAL)

The local branch added `bench_corpus()` which reads from `corpus/` directory.
Origin/main has `bench_json_files()` which reads from `data/bench/generated/`.

**Local implementation** (a6fc5f5):
```rust
fn bench_corpus(c: &mut Criterion) {
    let corpus_dir = Path::new("corpus");
    // ... reads *.json files from corpus/
    // ... benchmarks all SIMD levels on each file
}
```

**Origin implementation** (906c01a):
```rust
fn bench_json_files(c: &mut Criterion) {
    let base_dir = PathBuf::from("data/bench/generated");
    // ... discovers files in {pattern}/{size}.json structure
    // ... benchmarks all SIMD levels on each file
}
```

**Functional differences**:
- Local: Flat directory structure, simple discovery
- Origin: Hierarchical, pattern grouping, more sophisticated

**Recommendation**: **Use origin's approach** - It's already implemented and more comprehensive.

If you really need the corpus directory pattern:
```bash
# Create corpus as symlink to generated suite
mkdir -p corpus
ln -s ../data/bench/generated/comprehensive/*.json corpus/
```

---

## Restoration Steps

### Step 1: Add Documentation (5 minutes)

```bash
# Ensure you're on origin/main or a branch from it
git checkout -b add-optimization-docs origin/main

# Cherry-pick documentation files
git checkout a6fc5f5 -- docs/optimization-opportunities.md
git checkout a6fc5f5 -- docs/runtime-dispatch-implications.md

# Commit
git add docs/optimization-opportunities.md docs/runtime-dispatch-implications.md
git commit -m "Add CPU optimization analysis and runtime dispatch implications

See docs/optimization-opportunities.md for:
- 8 optimization opportunities for Zen 4 architecture
- AVX-512 implementation roadmap
- AVX512-VPOPCNTDQ for rank operations
- BMI2 integration strategy
- SIMD prefix sum approaches

See docs/runtime-dispatch-implications.md for:
- Runtime dispatch overhead analysis (0.2ns)
- Impact on embedded platforms (none)
- Binary size implications (+5KB)
- Migration patterns for std/no_std"

# Push or create PR
git push origin add-optimization-docs
```

**Result**: Documentation preserved, no feature conflicts.

---

### Step 2: Update .gitignore (1 minute)

The local branch added `corpus/` to `.gitignore`. Origin has `data/bench/generated/`.

```bash
# Check current .gitignore
cat .gitignore

# Add if not present (origin already has data/bench/generated/)
echo "" >> .gitignore
echo "# Benchmark corpus (alternative to data/bench/generated/)" >> .gitignore
echo "corpus/" >> .gitignore

# Also gitignore the binary artifact
echo "" >> .gitignore
echo "# Build artifacts" >> .gitignore
echo "*.rlib" >> .gitignore
```

---

### Step 3: (Optional) Create Corpus Convenience Command

If you prefer the simpler `corpus/` pattern for quick iteration:

**Option A**: Alias in documentation
```bash
# Add to README or docs/
# Quick corpus generation:
cargo run --features cli -- json generate-suite --output corpus --max-size 16mb --clean

# This creates corpus/ with the suite inside
```

**Option B**: Shell script wrapper
```bash
#!/bin/bash
# scripts/generate-corpus.sh

# Wrapper around generate-suite for quick corpus generation
CORPUS_DIR="${1:-corpus}"
MAX_SIZE="${2:-16mb}"

cargo run --features cli -- json generate-suite \
    --output "$CORPUS_DIR" \
    --max-size "$MAX_SIZE" \
    --clean \
    --verify

echo "Corpus generated in $CORPUS_DIR/"
echo "Files: $(find "$CORPUS_DIR" -name '*.json' | wc -l)"
echo "Total size: $(du -sh "$CORPUS_DIR" | cut -f1)"
```

---

## Files Inventory

### Files to Restore

| File | Status | Action | Priority |
|------|--------|--------|----------|
| `docs/optimization-opportunities.md` | ✅ New | Copy directly | HIGH |
| `docs/runtime-dispatch-implications.md` | ✅ New | Copy directly | HIGH |
| `.gitignore` (corpus/) | ⚠️ Modified | Merge changes | LOW |
| `libtest_no_std.rlib` | ❌ Binary | Gitignore instead | N/A |

### Files to Adapt (Optional)

| File | Local Change | Origin Equivalent | Recommendation |
|------|--------------|-------------------|----------------|
| `benches/json_simd.rs` | `bench_corpus()` | `bench_json_files()` | Use origin's version |
| `src/bin/succinctly/main.rs` | `generate-corpus` | `generate-suite` | Use origin's version |
| `src/bin/succinctly/generators.rs` | Minor tweaks | Enhanced version | Use origin's version |
| `tests/cli_golden_tests.rs` | Corpus test | Suite tests | Use origin's version |

### Files Not Needed

These were created locally but aren't needed with origin's approach:
- `tests/snapshots/cli_golden_tests__help_json_generate_corpus.snap` - Not applicable

---

## Testing After Migration

After restoring documentation:

```bash
# 1. Verify docs build correctly
cargo doc --no-deps --open

# 2. Generate benchmark suite (replaces corpus)
cargo run --features cli -- json generate-suite --max-size 100mb --clean

# 3. Run benchmarks on generated suite
cargo bench --bench json_simd

# 4. Verify SIMD implementations
cargo test --test simd_level_tests

# 5. Check that docs are accessible
ls -la docs/
```

Expected output:
```
docs/optimization-opportunities.md        - ✅ Present
docs/runtime-dispatch-implications.md     - ✅ Present
data/bench/generated/                     - ✅ Created by generate-suite
corpus/                                   - ⚠️ Optional (if you created it)
```

---

## Key Optimization Recommendations from Docs

Once documentation is restored, implement these high-priority items:

### 1. Enable Runtime Dispatch (CRITICAL - 5 minutes)

**File**: `src/json/simd/mod.rs`

**Change**:
```diff
- #[cfg(all(target_arch = "x86_64", test))]
+ #[cfg(all(target_arch = "x86_64", feature = "std"))]
  pub fn build_semi_index_standard(json: &[u8]) -> SemiIndex {
      if is_x86_feature_detected!("avx2") {
          avx2::build_semi_index_standard(json)
      } else if is_x86_feature_detected!("sse4.2") {
          sse42::build_semi_index_standard(json)
      } else {
          x86::build_semi_index_standard(json)
      }
  }

- #[cfg(all(target_arch = "x86_64", not(test)))]
+ #[cfg(all(target_arch = "x86_64", not(feature = "std")))]
  pub use x86::build_semi_index_standard;
```

**Impact**: 1.5-1.8x performance improvement immediately
**Risk**: None (code already exists and tested)

### 2. Implement AVX-512 JSON Parser (HIGH - 4-6 hours)

Create `src/json/simd/avx512.rs` following the pattern in `avx2.rs` but with:
- `__m512i` (64-byte chunks)
- `_mm512_cmpeq_epi8_mask` (returns `u64` masks directly)
- Process 64 bytes per iteration

Expected: 1.3-1.5x additional improvement over AVX2

### 3. Add AVX512-VPOPCNTDQ for Rank (HIGH - 2-3 hours)

Update `src/popcount.rs` to use `_mm512_popcnt_epi64` for 8x parallel popcount.

Expected: 2-4x speedup for rank operations on large bitvectors

---

## What Was Lost (Acceptable)

Features from origin/main that conflicted with local changes:

### JQ Module (Re-added on origin)
The `src/jq/` module was removed and then re-added on origin/main. The current origin/main has:
- ✅ `src/jq/mod.rs`
- ✅ `src/jq/eval.rs`
- ✅ `src/jq/expr.rs`
- ✅ `src/jq/parser.rs`
- ✅ `src/jq/value.rs`
- ✅ jq query command in CLI

So **nothing was lost** - jq is present on origin/main.

### Benchmark Suite Approach
Local had a simpler `corpus/` approach. Origin has a more sophisticated `data/bench/generated/` hierarchy.

**Trade-off**: Slightly more complex directory structure, but better organized and more feature-complete.

**Verdict**: ✅ Acceptable - Origin's approach is better

---

## Summary

**Restore Immediately**:
1. ✅ `docs/optimization-opportunities.md` - High-value CPU analysis
2. ✅ `docs/runtime-dispatch-implications.md` - Deep-dive on dispatch

**Don't Restore** (use origin's versions):
3. ❌ `generate-corpus` command → Use `generate-suite` instead
4. ❌ `bench_corpus()` benchmark → Use `bench_json_files()` instead
5. ❌ `libtest_no_std.rlib` → Add to `.gitignore`

**Optional**:
6. ⚙️ Create convenience wrapper/alias for `corpus/` if preferred
7. ⚙️ Update `.gitignore` to exclude `*.rlib` files

**Net Result**:
- ✅ All valuable documentation preserved
- ✅ More comprehensive benchmark suite (origin's)
- ✅ Full jq module available (origin's)
- ✅ No functionality lost
- ✅ Better organized codebase

**Recommended Next Steps**:
1. Restore docs (5 min)
2. Generate benchmark suite (10 min): `cargo run --features cli -- json generate-suite --max-size 100mb`
3. Run benchmarks (varies): `cargo bench --bench json_simd`
4. Implement runtime dispatch fix (5 min) - **Biggest immediate win**

---

## Quick Start Commands

```bash
# 1. Ensure on origin/main
git checkout origin/main -b restore-optimization-docs

# 2. Get documentation from local branch
git checkout a6fc5f5 -- docs/optimization-opportunities.md docs/runtime-dispatch-implications.md

# 3. Update .gitignore
echo -e "\n# Build artifacts\n*.rlib" >> .gitignore

# 4. Commit
git add docs/ .gitignore
git commit -m "Add CPU optimization analysis

- AVX-512 implementation opportunities
- Runtime dispatch analysis (0.2ns overhead)
- Embedded platform compatibility
- Expected 1.5-3.6x performance improvements"

# 5. Generate benchmark suite (use origin's approach)
cargo run --features cli -- json generate-suite --max-size 100mb --clean

# 6. Run benchmarks
cargo bench --bench json_simd

# Done! Documentation restored, benchmarks working with origin's superior approach
```

---

## References

- Local commit: `a6fc5f5` "stuff"
- Origin commit: `906c01a` "stuff"
- Common ancestor: `e10359ba`
- Documentation added: 1,260 lines of optimization analysis
- Features unified: Benchmark generation (origin's approach is better)
