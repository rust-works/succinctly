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

## Bit Position Conventions

- Bit positions are 0-indexed from LSB in each u64 word
- Words are stored little-endian (bit 0 is LSB of first word)
- `word.to_le_bytes()` gives bytes in memory order for byte-level processing

## Compile-Time Lookup Tables

Use `const` blocks for zero-overhead lookup tables:
```rust
const BYTE_POPCOUNT: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut i: usize = 0;
    while i < 256 {
        table[i] = (i as u8).count_ones() as u8;
        i += 1;
    }
    table
};
```

Note: `for` loops aren't allowed in const contexts; use `while`.

## Zero-Copy Output Patterns

For CLI output, avoid allocation:
```rust
// Zero-copy for strings and numbers:
out.write_all(value.raw_bytes())?;

// Use BufWriter for buffered I/O:
let mut out = BufWriter::new(stdout.lock());
```
