---
name: simd-optimization
description: SIMD optimization patterns and learnings for x86_64 and ARM. Use when implementing SIMD code, optimizing vectorized operations, or debugging SIMD issues. Triggers on terms like "SIMD", "AVX", "SSE", "NEON", "vectorization", "intrinsics".
---

# SIMD Optimization Skill

Patterns and learnings from SIMD optimization in this codebase.

**Comprehensive documentation**: See [docs/optimizations/simd.md](../../../docs/optimizations/simd.md) for full details on SIMD techniques.

## Key Insight: Wider SIMD != Automatically Faster

Two AVX-512 optimizations implemented with dramatically different results:

### AVX512-VPOPCNTDQ: 5.2x Speedup (Compute-Bound)

**Implementation**: `src/bits/popcount.rs`
- Processes 8 u64 words (512 bits) in parallel
- Hardware `_mm512_popcnt_epi64` instruction
- **Result**: 96.8 GiB/s vs 18.5 GiB/s (scalar) = **5.2x faster**

**Why it wins**: Pure compute-bound, embarrassingly parallel, no dependencies

### AVX-512 JSON Parser: 7-17% Slower (Memory-Bound) - REMOVED

- Processed 64 bytes/iteration (vs 32 for AVX2)
- **Result**: 672 MiB/s vs 732 MiB/s (AVX2) = **8.9% slower**

**Why AVX2 won**:
1. Memory-bound workload: Waiting for data from memory, not compute
2. AMD Zen 4 splits AVX-512 into two 256-bit micro-ops
3. State machine overhead: Wider SIMD = more bytes to process sequentially afterward
4. Cache alignment: 32-byte chunks fit cache lines better

## When to Use AVX-512

- Pure compute: math, crypto, compression
- No memory bottlenecks
- No sequential dependencies
- Data-parallel algorithms

## When NOT to Use AVX-512

- Memory-bound workloads
- Sequential state machines
- Complex control flow

## SIMD Instruction Set Hierarchy

| Level  | Width  | Bytes/Iter | Availability | Notes                            |
|--------|--------|------------|--------------|----------------------------------|
| SSE2   | 128bit | 16         | 100%         | Universal baseline on x86_64     |
| SSE4.2 | 128bit | 16         | ~90%         | PCMPISTRI string instructions    |
| AVX2   | 256bit | 32         | ~95%         | 2x width, best price/performance |
| BMI2   | N/A    | N/A        | ~95%         | PDEP/PEXT, but AMD Zen 1/2 slow  |

## Compilation Model

**Key insight**: `#[target_feature]` is a compiler directive, not a runtime gate.

```rust
// All these compile on any x86_64:
#[target_feature(enable = "sse2")]
unsafe fn process_sse2(data: &[u8]) { ... }

#[target_feature(enable = "avx2")]
unsafe fn process_avx2(data: &[u8]) { ... }

// Runtime dispatch (requires std)
fn process(data: &[u8]) {
    if is_x86_feature_detected!("avx2") {
        unsafe { process_avx2(data) }
    } else {
        unsafe { process_sse2(data) }
    }
}
```

## ARM NEON Movemask

**Problem**: NEON lacks x86's `_mm_movemask_epi8`. Variable shifts are slow on M1.

**Solution**: Multiplication trick to pack bits:

```rust
#[inline]
#[target_feature(enable = "neon")]
unsafe fn neon_movemask(v: uint8x16_t) -> u16 {
    let high_bits = vshrq_n_u8::<7>(v);
    let low_u64 = vgetq_lane_u64::<0>(vreinterpretq_u64_u8(high_bits));
    let high_u64 = vgetq_lane_u64::<1>(vreinterpretq_u64_u8(high_bits));

    const MAGIC: u64 = 0x0102040810204080;
    let low_packed = (low_u64.wrapping_mul(MAGIC) >> 56) as u8;
    let high_packed = (high_u64.wrapping_mul(MAGIC) >> 56) as u8;

    (low_packed as u16) | ((high_packed as u16) << 8)
}
```

**Results**: 10-18% improvement on string-heavy and nested JSON patterns.

## SSE2 Unsigned Comparison

SSE2 lacks unsigned byte comparison. Use min trick:

```rust
unsafe fn unsigned_le(a: __m128i, b: __m128i) -> __m128i {
    let min_ab = _mm_min_epu8(a, b);
    _mm_cmpeq_epi8(min_ab, a)  // a <= b iff min(a,b) == a
}
```

## Testing Strategy

**Problem**: Runtime dispatch only tests highest available SIMD level.

**Solution**: Explicitly call each implementation in tests:

```rust
#[test]
fn test_all_simd_levels() {
    let input = b"test input";
    let expected = scalar_impl(input);

    let sse2_result = unsafe { sse2::process(input) };
    let avx2_result = unsafe { avx2::process(input) };

    assert_eq!(sse2_result, expected);
    assert_eq!(avx2_result, expected);
}
```

## no_std Constraints

### `is_x86_feature_detected!` requires std

Alternatives:
- Use `count_ones()` - LLVM optimizes to POPCNT with `-C target-feature=+popcnt`
- Use `#[target_feature(enable = "popcnt")]` on functions
- Keep runtime detection only in `#[cfg(test)]` blocks

### ARM NEON is always available on aarch64

No runtime detection needed:
```rust
#[cfg(target_arch = "aarch64")]
{
    // NEON intrinsics work without feature detection
    unsafe { neon::process(data) }
}
```

## Benchmark Commands

```bash
# Test AVX-512 popcount implementation
cargo test --lib --features simd popcount

# Benchmark popcount strategies
cargo bench --bench popcount_strategies --features simd

# Run comprehensive JSON benchmarks
cargo bench --bench json_simd
```

## Key Takeaways

1. **Profile first, optimize second** - Don't assume wider is better
2. **Understand bottlenecks** - Memory-bound vs compute-bound matters
3. **Measure end-to-end** - Micro-benchmarks can be misleading
4. **Consider architecture** - Zen 4 splits AVX-512, future Zen 5 may not
5. **Amdahl's Law always wins** - Optimize what matters (the slow 80%)
6. **Remove failed optimizations** - Slower code creates technical debt

## See Also

- [docs/optimizations/simd.md](../../../docs/optimizations/simd.md) - Comprehensive SIMD techniques reference
- [docs/optimizations/cache-memory.md](../../../docs/optimizations/cache-memory.md) - Memory-bound vs compute-bound analysis
- [docs/optimizations/branchless.md](../../../docs/optimizations/branchless.md) - SIMD masking techniques
- [docs/optimizations/history.md](../../../docs/optimizations/history.md) - Historical optimization record
