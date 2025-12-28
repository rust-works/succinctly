# Rust no_std SIMD and Bit Manipulation

## Hex Literals vs MAX Constants

`0xFFu64` is NOT all bits set - it's just `255` (8 bits set). For all 64 bits:
- Use `u64::MAX` or `0xFFFF_FFFF_FFFF_FFFFu64`
- `0xFF` = 8 bits, `0xFFFF` = 16 bits, `0xFFFF_FFFF` = 32 bits

## no_std Constraints

### `is_x86_feature_detected!` requires std
This macro is not available in `no_std` crates. Alternatives:
- Use `count_ones()` - LLVM optimizes to POPCNT with `-C target-feature=+popcnt`
- Use `#[target_feature(enable = "popcnt")]` on functions and trust the caller
- Keep runtime detection only in `#[cfg(test)]` blocks (tests use std)

### Tests still have std
`#[cfg(test)]` enables std, so macros like `is_x86_feature_detected!` work in tests even for no_std crates.

## x86 Intrinsics Type Signatures

`_popcnt64` takes `i64` and returns `i32`:
```rust
// Wrong: let result = _popcnt64(*ptr);  // ptr is *const u64
// Right:
let mut sum = 0i32;
sum += _popcnt64(*ptr as i64);
sum as u32
```

## Clippy Lints (Rust 1.92+)

Use `.is_multiple_of()` instead of modulo comparison:
```rust
// Old (triggers clippy::manual_is_multiple_of)
if x % 64 != 0 { ... }

// New
if !x.is_multiple_of(64) { ... }
```

## ARM NEON

NEON is always available on aarch64 - no runtime detection needed:
```rust
#[cfg(target_arch = "aarch64")]
{
    // NEON intrinsics work without feature detection
    unsafe { neon::popcount_512_neon(ptr) }
}
```
