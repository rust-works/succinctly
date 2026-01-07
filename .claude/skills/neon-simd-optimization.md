# NEON SIMD Optimization Patterns for JSON Parsing

## Overview

This skill documents learnings from optimizing the NEON JSON parser in `src/json/simd/neon.rs`. The key insight is that **processing larger chunks (32 bytes vs 16 bytes)** can yield significant improvements, especially for string-heavy workloads.

## Key Optimization: 32-byte Processing

### Pattern

When processing data with NEON (128-bit/16-byte vectors), consider loading two vectors and combining their classification results into 32-bit masks:

```rust
// Instead of processing 16 bytes at a time:
while offset + 16 <= data.len() {
    let chunk = vld1q_u8(data.as_ptr().add(offset));
    let class = classify_chars(chunk);  // Returns u16 masks
    state = process_chunk(class, state, ...);
    offset += 16;
}

// Process 32 bytes at a time:
while offset + 32 <= data.len() {
    let chunk_lo = vld1q_u8(data.as_ptr().add(offset));
    let chunk_hi = vld1q_u8(data.as_ptr().add(offset + 16));
    let class_lo = classify_chars(chunk_lo);  // u16 masks
    let class_hi = classify_chars(chunk_hi);  // u16 masks
    let class32 = CharClass32::from_pair(class_lo, class_hi);  // u32 masks
    state = process_chunk_32(class32, state, ...);
    offset += 32;
}
```

### Why It Works

1. **Reduced loop overhead**: Fewer iterations means less branch prediction, loop counter updates
2. **Better fast-path amortization**: String scanning can skip up to 32 characters at once
3. **Improved instruction-level parallelism**: Two NEON loads can execute in parallel

### Benchmark Results (Apple M1)

| Pattern | Improvement |
|---------|-------------|
| nested (deep nesting) | -41% time |
| strings (string-heavy) | -37% time |
| unicode | -15% time |
| comprehensive (mixed) | -9% time |
| numbers | -5% time |
| arrays | -5% time |

String-heavy patterns benefit most because the `InString` fast-path can batch-write more zeros.

## Implementation Details

### CharClass32 Structure

```rust
struct CharClass32 {
    quotes: u32,        // Combined from two u16
    backslashes: u32,
    opens: u32,
    closes: u32,
    delims: u32,
    value_chars: u32,
    string_special: u32,
}

impl CharClass32 {
    #[inline]
    fn from_pair(lo: CharClass, hi: CharClass) -> Self {
        Self {
            quotes: (lo.quotes as u32) | ((hi.quotes as u32) << 16),
            // ... same pattern for other fields
        }
    }
}
```

### State Machine Adaptation

The 32-byte processing function is nearly identical to the 16-byte version, but uses:
- `u32` instead of `u16` for bit masks
- `1u32 << i` instead of `1u16 << i` for bit testing
- Loop bound of 32 instead of 16

## What Doesn't Work on ARM

### SIMD Prefix Sum (Rejected)

Attempted to batch popcount using NEON `vcntq_u8`, but it was **25% slower** than scalar:

```rust
// This approach FAILED:
let v = vld1q_u8(ptr);
let counts = vcntq_u8(v);  // Per-byte popcount
// 14 lane extractions needed to get individual word counts
// Sequential dependency chain for prefix sum remains
```

**Why it failed:**
- Lane extraction (`vgetq_lane`) is expensive
- Prefix sums have inherent sequential dependencies
- Apple Silicon's scalar execution is very efficient

### Lesson Learned

SIMD batching helps for **independent** operations, not for operations with **sequential dependencies** like prefix sums. The bottleneck is the data dependency chain, not the computation itself.

## When to Apply This Pattern

**Good candidates for 32-byte processing:**
- Character classification loops
- String scanning with fast-paths
- Any loop where the inner work is relatively light

**Poor candidates:**
- Prefix sum computations
- Operations requiring cross-lane dependencies
- Very small inputs (< 64 bytes)

## Files Modified

- `src/json/simd/neon.rs`: Added `CharClass32`, `process_chunk_standard_32()`, modified main loop

## Benchmark Commands

```bash
# Run NEON benchmarks
cargo bench --bench json_simd -- "NEON"

# Compare patterns at 10MB
cargo bench --bench json_simd -- "pattern_comparison_10mb"

# Single benchmark with change detection
cargo bench --bench json_simd -- "json_indexing/NEON/comprehensive/10mb" --noplot
```
