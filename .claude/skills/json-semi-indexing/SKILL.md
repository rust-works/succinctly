---
name: json-semi-indexing
description: JSON semi-indexing implementation details and patterns. Use when working on JSON parsing, semi-index construction, or cursor navigation. Triggers on terms like "JSON index", "semi-index", "interest bits", "balanced parentheses", "JSON cursor".
---

# JSON Semi-Indexing Skill

Implementation details for JSON semi-indexing using succinct data structures.

**Comprehensive documentation**: See [docs/parsing/json.md](../../../docs/parsing/json.md) for full parsing architecture.

## Quick Reference

### Semi-Index Structure

- **Interest Bits (IB)**: Marks structural positions (opens, leaves)
- **Balanced Parentheses (BP)**: Encodes tree structure for navigation

See [docs/parsing/json.md](../../../docs/parsing/json.md) for details on:
- State machine outputs (Phi values)
- PFSM table-driven parser (950 MiB/s on x86_64 AMD Zen 4)
- SIMD character classification

## Key SIMD Intrinsics

**ARM NEON**:
```rust
vceqq_u8(chunk, splat)      // Equality comparison
vcleq_u8(chunk, max)        // Unsigned less-than-or-equal
vandq_u8(a, b)              // Bitwise AND
vqtbl1q_u8(table, indices)  // Nibble lookup (16 parallel)
```

**x86 SSE2/AVX2**:
```rust
_mm_cmpeq_epi8(chunk, splat)   // Equality comparison
_mm_movemask_epi8(cmp)         // Extract MSBs as u16 bitmask
_mm256_movemask_epi8(cmp)      // AVX2: Extract as u32 bitmask
```

## Multi-Architecture Dispatch

```rust
// src/json/simd/mod.rs
#[cfg(target_arch = "x86_64")]
if is_x86_feature_detected!("avx2") {
    avx2::build_semi_index_standard(json)
} else if is_x86_feature_detected!("sse4.2") {
    sse42::build_semi_index_standard(json)
} else {
    x86::build_semi_index_standard(json)  // SSE2 fallback
}

#[cfg(target_arch = "aarch64")]
neon::build_semi_index_standard(json)
```

## Testing Patterns

### SIMD vs Scalar Comparison

Always verify SIMD produces identical results to scalar:
```rust
fn compare_results(json: &[u8]) {
    let scalar = standard::build_semi_index(json);
    let simd = simd::build_semi_index_standard(json);
    assert_eq!(scalar.ib, simd.ib);
    assert_eq!(scalar.bp, simd.bp);
    assert_eq!(scalar.state, simd.state);
}
```

### Boundary Testing

Test at SIMD chunk boundaries (multiples of 16/32):
- Escapes spanning chunk boundaries
- State transitions at position 15/16 or 31/32
- Inputs of length 15, 16, 17, 31, 32, 33, etc.

## Performance Reference

| Size      | succinctly            | jq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  2.4 ms  (3.9 MiB/s)  |  4.3 ms  (2.2 MiB/s)  | **1.79x**  |
| **100KB** |  4.6 ms (18.4 MiB/s)  |  8.2 ms (10.5 MiB/s)  | **1.76x**  |
| **1MB**   | 24.7 ms (32.7 MiB/s)  | 43.9 ms (18.4 MiB/s)  | **1.78x**  |

## See Also

- [docs/parsing/json.md](../../../docs/parsing/json.md) - Full JSON parsing documentation
- [docs/parsing/dsv.md](../../../docs/parsing/dsv.md) - DSV parsing (similar techniques)
- [docs/optimizations/state-machines.md](../../../docs/optimizations/state-machines.md) - PFSM technique details
- [docs/optimizations/simd.md](../../../docs/optimizations/simd.md) - SIMD patterns
- [docs/optimizations/lookup-tables.md](../../../docs/optimizations/lookup-tables.md) - Table-driven parsing
