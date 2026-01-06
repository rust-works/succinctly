# SIMD Implementation Patterns

## Compilation Model

**Key insight**: `#[target_feature]` is a compiler directive, not a runtime gate.

- All SIMD levels (SSE2, SSE4.2, AVX2) compile into single binary on any x86_64 host
- Each function gets separate code generation with specific instructions
- Running unsupported code without runtime guards causes SIGILL crash
- Runtime dispatch via `is_x86_feature_detected!()` prevents crashes

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

## SIMD Instruction Set Hierarchy

| Level   | Width  | Bytes/Iter | Availability | Notes                           |
|---------|--------|------------|--------------|----------------------------------|
| SSE2    | 128bit | 16         | 100%         | Universal baseline on x86_64     |
| SSE4.2  | 128bit | 16         | ~90%         | PCMPISTRI string instructions    |
| AVX2    | 256bit | 32         | ~95%         | 2x width, best price/performance |
| BMI2    | N/A    | N/A        | ~95%         | PDEP/PEXT, but AMD Zen 1/2 slow  |

### BMI2 Considerations

- **Intel Haswell+**: 3-cycle PDEP/PEXT (fast)
- **AMD Zen 1/2**: 18-cycle microcode (slower than scalar)
- **AMD Zen 3+**: 3-cycle hardware (fast)
- Provide utilities but don't force usage - let users opt-in

## Testing Strategy

**Problem**: Runtime dispatch only tests highest available SIMD level.

**Solution**: Explicitly call each implementation in tests:

```rust
#[test]
fn test_all_simd_levels() {
    let input = b"test input";
    let expected = scalar_impl(input);

    // Force-test each level
    let sse2_result = unsafe { sse2::process(input) };
    let avx2_result = unsafe { avx2::process(input) };

    assert_eq!(sse2_result, expected);
    assert_eq!(avx2_result, expected);
}
```

## Character Classification Pattern

### x86 SSE2/SSE4.2 (128-bit, 16 bytes)

```rust
let quote_mask = _mm_movemask_epi8(eq_quote) as u16;
```

### x86 AVX2 (256-bit, 32 bytes)

```rust
let quote_mask = _mm256_movemask_epi8(eq_quote) as u32;
```

### SSE4.2 String Search

```rust
// Find multiple chars in one instruction
let structural_mask = _mm_cmpistrm(structural_chars, chunk, MODE);
```

## ARM NEON Movemask

**Problem**: NEON lacks x86's `_mm_movemask_epi8`. Variable shifts are slow on M1.

**Solution**: Multiplication trick to pack bits:

```rust
#[inline]
#[target_feature(enable = "neon")]
unsafe fn neon_movemask(v: uint8x16_t) -> u16 {
    // Step 1: Shift right by 7 to get 0 or 1 in each byte
    let high_bits = vshrq_n_u8::<7>(v);

    // Step 2: Extract as two u64 values
    let low_u64 = vgetq_lane_u64::<0>(vreinterpretq_u64_u8(high_bits));
    let high_u64 = vgetq_lane_u64::<1>(vreinterpretq_u64_u8(high_bits));

    // Step 3: Pack 8 bytes into 8 bits using multiplication
    // Magic number positions each byte at different bit position
    const MAGIC: u64 = 0x0102040810204080;
    let low_packed = (low_u64.wrapping_mul(MAGIC) >> 56) as u8;
    let high_packed = (high_u64.wrapping_mul(MAGIC) >> 56) as u8;

    (low_packed as u16) | ((high_packed as u16) << 8)
}
```

**Why this works**: Variable shifts (`vshlq_u8` with vector shift amounts) are ~10 cycles on M1. Fixed shifts + scalar multiply is ~3 cycles total.

**Results**: 10-18% improvement on string-heavy and nested JSON patterns.

## SSE2 Unsigned Comparison

SSE2 lacks unsigned byte comparison. Use min trick:

```rust
unsafe fn unsigned_le(a: __m128i, b: __m128i) -> __m128i {
    let min_ab = _mm_min_epu8(a, b);
    _mm_cmpeq_epi8(min_ab, a)  // a <= b iff min(a,b) == a
}
```

## Multi-Architecture Module Pattern

```rust
// src/simd/mod.rs
#[cfg(target_arch = "aarch64")]
pub mod neon;

#[cfg(target_arch = "x86_64")]
pub mod x86;

#[cfg(target_arch = "aarch64")]
pub use neon::process;

#[cfg(target_arch = "x86_64")]
pub use x86::process;

// Fallback for other platforms
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub use super::scalar::process;
```
