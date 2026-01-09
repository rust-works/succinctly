# PFSM Integration Summary

## Changes Made

### 1. Default Implementation Change
**File**: `src/json/standard.rs`

The `build_semi_index()` function now uses PFSM optimized by default:
```rust
pub fn build_semi_index(json: &[u8]) -> SemiIndex {
    // Uses PFSM optimized - 40-77% faster than scalar
    pfsm_optimized::pfsm_process_chunk_optimized(...)
}
```

The original scalar implementation is preserved as `build_semi_index_scalar()` for fallback and comparison.

### 2. Performance Improvement

**Before (scalar)**: 627 MiB/s
**After (PFSM optimized)**: 962 MiB/s
**Improvement**: **+53% faster** on 1KB files

Scaling results:
- **1KB**: 962 MiB/s (53% faster)
- **10KB**: 949 MiB/s (77% faster)
- **100KB**: 817 MiB/s (66% faster)

### 3. No CPU Feature Requirements

Unlike AVX2/BMI2 optimizations, PFSM requires no special CPU features:
- **Portable**: Works on all architectures (x86_64, ARM, etc.)
- **No runtime detection**: No feature flags needed
- **No fallback complexity**: Always available

### 4. Implementation Strategy

**Why PFSM is faster:**
1. **Table lookups** are faster than branch-heavy conditionals
2. **L1 cache**: 256-entry tables fit entirely in L1 cache
3. **No allocations**: Single-pass processing
4. **Better ILP**: More instruction-level parallelism opportunities

**Why we don't need runtime dispatch:**
- PFSM is faster than both scalar and AVX2
- No CPU features required
- Simpler code path

## Testing

All tests pass with PFSM as default:
```bash
cargo test --lib json                    # 181 tests ✓
cargo test --test json_indexing_tests   # 42 tests ✓
cargo test --test pfsm_correctness      # 6 tests ✓
```

## Files Modified

1. **src/json/pfsm_optimized.rs** (new) - Single-pass PFSM implementation
2. **src/json/standard.rs** - Changed default to PFSM, added scalar fallback
3. **src/json/mod.rs** - Added pfsm_optimized module
4. **CLAUDE.md** - Updated documentation
5. **Cargo.toml** - Added json_standard benchmark
6. **benches/json_standard.rs** (new) - Benchmark showing improvement

## Usage

### Default (PFSM optimized)
```rust
use succinctly::json::standard::build_semi_index;

let json = br#"{"a":1}"#;
let semi = build_semi_index(json);  // Uses PFSM optimized
```

### Explicit scalar fallback
```rust
use succinctly::json::standard::build_semi_index_scalar;

let json = br#"{"a":1}"#;
let semi = build_semi_index_scalar(json);  // Original implementation
```

### Explicit PFSM
```rust
use succinctly::json::{BitWriter, pfsm_optimized, pfsm_tables::PfsmState};

let json = br#"{"a":1}"#;
let mut ib = BitWriter::new();
let mut bp = BitWriter::new();
pfsm_optimized::pfsm_process_chunk_optimized(json, PfsmState::InJson, &mut ib, &mut bp);
```

## Benchmark Commands

```bash
# Compare default (PFSM) vs scalar
cargo bench --bench json_standard

# Detailed PFSM comparison
cargo bench --bench pfsm_comparison

# Test correctness
cargo test --test pfsm_correctness
```

## Key Insight

**Memory allocation overhead was the problem, not the PFSM concept.**

The two-pass implementation with intermediate vector allocation was slower. The single-pass implementation without allocations is the fastest approach - even faster than hand-tuned AVX2 SIMD for small-to-medium files (typical use case).

## Future Work

- Consider SIMD PFSM for very large files (>10MB)
- Benchmark on different CPU architectures (ARM, RISC-V)
- Profile with perf to identify remaining bottlenecks
