# Optimization Patterns Skill

**Use when**: Optimizing code, reviewing performance, analyzing regressions, or planning optimizations.

**Triggers**: "optimize", "performance", "regression", "slow", "speedup", "benchmark", "inline", "SIMD", "fast path"

## Core Principles

### 1. Measure First, Optimize Second

- **Always profile before optimizing** - bottleneck is rarely where you expect
- Use bisection testing to identify exact regression point
- Benchmark end-to-end, not just isolated functions
- Compare against what you're replacing, not theoretical best

**Tools:**
```bash
# Bisect performance regression
python3 /tmp/bench_commits.py  # Reliable timing across commits

# Profile hot paths
perf stat -e branch-misses,branches ./binary
cargo bench --bench <name>
```

### 1a. CRITICAL: Always Capture Full Benchmark Output

**NEVER filter benchmark output directly.** Always capture to a file first, then analyze.

```bash
# WRONG - loses data if interrupted or filtered incorrectly
cargo bench --bench json_validate_bench 2>&1 | grep "time:"

# CORRECT - capture everything, then analyze
cargo bench --bench json_validate_bench 2>&1 | tee /tmp/bench_$(date +%Y%m%d_%H%M%S).txt
# Then analyze from file:
grep -E "(time:|thrpt:|change:)" /tmp/bench_*.txt
```

**Why this matters:**
- Benchmarks take minutes to run - losing results wastes time
- Criterion outputs multi-line results that grep can miss
- You need full context to understand regressions
- Comparison requires both before/after data preserved

**Workflow for A/B comparison:**
```bash
# 1. Benchmark current state
cargo bench --bench <name> 2>&1 | tee /tmp/bench_after.txt

# 2. Stash/checkout baseline
git stash  # or git checkout <baseline>

# 3. Benchmark baseline
cargo bench --bench <name> 2>&1 | tee /tmp/bench_before.txt

# 4. Restore working state
git stash pop  # or git checkout -

# 5. Compare results from files
diff /tmp/bench_before.txt /tmp/bench_after.txt
```

### 2. Algorithm > Micro-Optimization

**Evidence from this codebase:**
- Cumulative index: **627x** speedup (algorithmic)
- Lazy string slicing: **8.5%** speedup (fast-path checks)
- Inline hints: **failed** (compiler already optimal)

**Pattern:** Simple checks to avoid work beat trying to make work faster.

### 3. Fast Paths Are Your Friend

Add early-exit checks for common cases before expensive operations:

```rust
// GOOD: Check for identity cases first
if start.is_none() && end.is_none() {
    return value;  // [:] - no work needed
}

// THEN do expensive work
let len = s_str.chars().count();  // Unicode iteration
```

**Common fast paths:**
- Identity operations (no-op)
- Empty results (early return)
- Full range after bounds resolution
- Null/None propagation

### 4. Trust the Compiler on Generics

**DON'T** use `#[inline(always)]` on generic functions unless you have strong evidence:

```rust
// DANGER: Can cause code explosion in generic-heavy codebases
#[inline(always)]  // ← Don't do this!
fn helper<T: Clone>(...) { ... }
```

**Why it fails:**
- Generic functions monomorphize per type
- Forced inlining into other generics = exponential code bloat
- LLVM optimizer chokes on massive functions
- Tests slow down or hang (seen: 60s vs 2s)

**Instead:**
```rust
// GOOD: Let compiler decide
fn helper<T: Clone>(...) { ... }

// OK: Hint, not force (compiler can ignore)
#[inline]
fn helper<T: Clone>(...) { ... }
```

**Evidence from jq optimization:**
- File: 18K lines, 227 generic functions
- `#[inline(always)]`: **Hangs tests** ✗
- `#[cold]` only: **-0.3%** ✗
- `#[inline]` hint: **-1.3%** ✗
- No hints: **Already optimal** ✓

### 5. Wider SIMD ≠ Faster

Memory-bound workloads don't benefit from AVX-512:

**Evidence:**
- JSON parsing: AVX-512 was **7-17% slower** than AVX2
- YAML parsing: AVX-512 was **7% slower**
- Pattern: Sequential text parsing saturates RAM bandwidth at AVX2

**Decision tree:**
```
Is workload compute-bound or memory-bound?
├── Compute-bound → Consider AVX-512
└── Memory-bound → Stick with AVX2/NEON
```

### 6. Simpler Data Structures Win

Cache behavior > theoretical complexity:

- DSV lightweight index (simple array): **5-9x faster** than 3-level BitVec
- Reason: Better cache locality, no indirection

### 7. Micro-Benchmarks Can Mislead

**Known failures where micro-benchmarks showed gains but end-to-end regressed:**

| Optimization | Micro-bench | End-to-end | Root Cause |
|--------------|-------------|------------|------------|
| Software prefetch | - | **-30%** | HW prefetcher already handles sequential |
| SIMD threshold tuning | +2-4% | **-8 to -15%** | Branch prediction + inlining better |
| Branchless lookup tables | +3-29% | **-25 to -44%** | Cache pollution, branch predictor wins |
| AVX-512 loop | +2x iterations | **-7%** | Measured fewer calls, not more work |

**Lesson:** Only trust end-to-end benchmarks on real workloads.

## Optimization Workflow

### 1. Identify Regression

```bash
# Bisect commits to find regression point
git bisect start HEAD v0.5.0
git bisect run python3 /tmp/bench_commits.py
```

### 2. Analyze Root Cause

- Profile the slow code path
- Compare code diff at regression point
- Form hypothesis about bottleneck

**Warning:** Code inspection can mislead - verify with measurement.

### 3. Design Fix

**Prefer in order:**
1. **Avoid work** - Fast-path checks for common cases
2. **Reduce work** - Better algorithm or data structure
3. **Parallelize work** - SIMD, threading (if applicable)
4. **Make work faster** - Micro-optimizations (last resort)

### 4. Implement and Measure

```bash
# Build and benchmark
cargo build --release --features cli
python3 /tmp/bench_commits.py  # Compare vs baseline

# Run full test suite
cargo test --release
```

### 5. Know When to Stop

- **Ship wins** - Don't chase diminishing returns
- **Revert failures** - Failed optimizations add complexity
- **Document learnings** - Update docs/optimizations/README.md

## Common Pitfalls

### ❌ Forcing Inlining on Large Generic Functions

**Problem:**
```rust
#[inline(always)]  // Code explosion in 18K-line file!
fn find_field<W: Clone>(...) { ... }
```

**Solution:** Remove the attribute. Compiler knows best.

### ❌ Optimizing Without Profiling

**Problem:** Guessing bottleneck based on code reading.

**Solution:** Profile first. Bottleneck is often not where you expect.

### ❌ Micro-Benchmarking in Isolation

**Problem:** Optimizing a helper function without end-to-end testing.

**Solution:** Benchmark the full use case, not isolated primitives.

### ❌ Ignoring Cache Effects

**Problem:** Choosing "better" O(n) algorithm that's slower due to cache misses.

**Solution:** Profile with cache counters, prefer simple sequential access.

## Success Patterns

### ✓ Fast-Path Identity Checks

```rust
// Common in slicing, iteration, filtering
if is_identity_operation() {
    return input;  // No work needed
}
// ... expensive work ...
```

### ✓ Early Exit on Empty

```rust
if input.is_empty() {
    return default;
}
// ... expensive processing ...
```

### ✓ Algorithmic Improvements

- Replace O(n²) with O(n log n)
- Add cumulative index for O(1) queries
- Use simpler data structures for better cache

### ✓ Let Compiler Optimize

Don't add `#[inline]`, `#[cold]`, or other hints unless profiling shows compiler is wrong (rare).

## Files

**Documentation:**
- [docs/optimizations/README.md](../../docs/optimizations/README.md) - All optimization learnings
- [docs/optimizations/*.md](../../docs/optimizations/) - Specific technique guides

**Recent examples:**
- jq lazy string slicing: [src/jq/eval.rs](../../src/jq/eval.rs) lines 278-327

## References

- [Optimization README](../../docs/optimizations/README.md) - Complete catalog
- [SIMD Skill](simd-optimization/SKILL.md) - SIMD-specific patterns
- [Bit Optimization Skill](bit-optimization/SKILL.md) - Bit manipulation
