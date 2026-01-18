# Bit Manipulation Techniques

Bit manipulation optimizations operate on individual bits or groups of bits within machine words, exploiting the parallel nature of bitwise operations to achieve constant-time or sub-linear complexity.

## Overview

| Technique                | Use Case                      | Complexity | Speedup        |
|--------------------------|-------------------------------|------------|----------------|
| Population Count         | Counting set bits             | O(1)       | 10-50x vs naive|
| CTZ/CLZ                  | Finding bit positions         | O(1)       | 8-64x vs loop  |
| Broadword Select         | Finding k-th set bit          | O(1)       | 3-10x vs linear|
| Bit Clearing             | Iterating set bits            | O(k)       | ~2x vs masking |
| PDEP/PEXT                | Bit scattering/gathering      | O(1)       | 10-100x        |

---

## Population Count (Popcount)

**Problem**: Count the number of 1-bits in a word.

**Naive approach**: O(64) loop checking each bit.

### Portable Broadword Algorithm

Uses SWAR (SIMD-Within-A-Register) to count bits in parallel:

```rust
fn popcount_word(mut x: u64) -> u32 {
    // Count bits in pairs (2-bit groups)
    x = x - ((x >> 1) & 0x5555_5555_5555_5555);
    // Sum pairs into nibbles (4-bit groups)
    x = (x & 0x3333_3333_3333_3333) + ((x >> 2) & 0x3333_3333_3333_3333);
    // Sum nibbles into bytes (8-bit groups)
    x = (x + (x >> 4)) & 0x0F0F_0F0F_0F0F_0F0F;
    // Multiply by 0x0101... to sum all bytes into top byte
    ((x.wrapping_mul(0x0101_0101_0101_0101)) >> 56) as u32
}
```

**Magic constants**:
- `0x5555...` = `01010101...` - masks alternating bits
- `0x3333...` = `00110011...` - masks alternating pairs
- `0x0F0F...` = `00001111...` - masks alternating nibbles
- `0x0101...` = sum all bytes via multiplication

**Performance**: ~10 instructions, constant time.

### Hardware POPCNT

Modern CPUs provide single-cycle popcount:

```rust
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::_popcnt64;

fn popcount_hw(x: u64) -> u32 {
    unsafe { _popcnt64(x as i64) as u32 }
}
```

**Rust**: Use `x.count_ones()` which compiles to `POPCNT` when available.

### Prior Art

- **Wegner (1960)**: First efficient popcount algorithm
- **HAKMEM Memo 169 (1972)**: MIT AI Lab optimizations
- **Knuth TAOCP Vol 4A**: Comprehensive coverage of bitwise tricks

---

## Count Trailing/Leading Zeros (CTZ/CLZ)

**Problem**: Find position of lowest/highest set bit.

### Hardware Instructions

```rust
fn ctz(x: u64) -> u32 {
    x.trailing_zeros()  // Compiles to TZCNT/BSF on x86
}

fn clz(x: u64) -> u32 {
    x.leading_zeros()   // Compiles to LZCNT/BSR on x86
}
```

**Usage in succinctly**: Finding next set bit for iteration:

```rust
// src/util/broadword.rs
fn select_in_word(mut val: u64, mut k: u32) -> u32 {
    loop {
        let t = val.trailing_zeros();
        if k == 0 { return t; }
        k -= 1;
        val &= val - 1;  // Clear lowest set bit
    }
}
```

### Clear Lowest Set Bit

The expression `x & (x - 1)` clears the lowest set bit:

```
x     = 0b10110100
x - 1 = 0b10110011
x & (x-1) = 0b10110000
```

**Application**: Iterate over set bits efficiently:

```rust
while x != 0 {
    let pos = x.trailing_zeros();
    process(pos);
    x &= x - 1;
}
```

---

## Broadword Select-in-Word

**Problem**: Find position of the k-th set bit in a word.

### CTZ Loop Method

Simple and often fastest on modern CPUs:

```rust
fn select_in_word(mut val: u64, mut k: u32) -> u32 {
    loop {
        let t = val.trailing_zeros();
        if k == 0 { return t; }
        k -= 1;
        val &= val - 1;
    }
}
```

**Complexity**: O(k) but with single-cycle TZCNT, often faster than broadword.

### Vigna's Broadword Select

Constant-time algorithm using parallel operations:

```rust
fn select_broadword(x: u64, k: u32) -> u32 {
    // Compute byte popcounts in parallel
    let s = x - ((x >> 1) & 0x5555_5555_5555_5555);
    let s = (s & 0x3333_3333_3333_3333) + ((s >> 2) & 0x3333_3333_3333_3333);
    let s = ((s + (s >> 4)) & 0x0F0F_0F0F_0F0F_0F0F).wrapping_mul(0x0101_0101_0101_0101);

    // Find byte containing k-th bit using parallel comparison
    let b = ((s + 0x0101_0101_0101_0101_u64.wrapping_mul(!k as u64)) >> 7)
        & 0x0101_0101_0101_0101;
    let byte_idx = b.trailing_zeros() / 8;

    // Extract byte and use lookup table
    let byte = (x >> (byte_idx * 8)) as u8;
    let remaining = k - ((s >> ((byte_idx - 1) * 8)) as u32 & 0xFF);
    byte_idx * 8 + SELECT_IN_BYTE_TABLE[(byte as usize) * 8 + remaining as usize]
}
```

### Prior Art

- **Vigna (2008)**: "Broadword Implementation of Rank/Select Queries"
- **Gog & Petri (2014)**: sdsl-lite library optimizations

---

## PDEP and PEXT (BMI2)

**PDEP** (Parallel Deposit): Scatter bits to positions marked by mask.
**PEXT** (Parallel Extract): Gather bits from positions marked by mask.

### PDEP for Quote Masking

Used in DSV parsing to compute regions inside quotes:

```rust
// src/dsv/simd/bmi2.rs
fn toggle64_bmi2(quote_mask: u64, carry: u64) -> (u64, u64) {
    // Scatter alternating 0/1 pattern to quote positions
    let addend = _pdep_u64(0x5555_5555_5555_5555 << carry, quote_mask);

    // Use addition with carry propagation to fill between quotes
    let comp = !quote_mask;
    let shifted = (addend << 1) | carry;
    let (result, overflow) = shifted.overflowing_add(comp);

    (result ^ comp, overflow as u64)
}
```

**Result**: 10x faster than prefix_xor, 50-100x faster than scalar.

### PEXT for Bit Gathering

```rust
// Extract bits at positions marked by mask
let result = _pext_u64(data, mask);
```

**Application**: Compress sparse bit patterns, extract structural characters.

### When NOT to Use PDEP

**Failed optimization in succinctly**: BMI2 PDEP for BitWriter was 71% slower (3.4x regression).

```rust
// DON'T DO THIS - PDEP is slow for consecutive bits
let deposited = _pdep_u64(bits, mask);  // 3-cycle latency
// Simple shift+OR is faster for ranges:
let result = (bits << offset) | existing;  // 2 cycles total
```

**Lesson**: PDEP excels at sparse bit operations, not consecutive ranges.

### Prior Art

- **Intel BMI2 (2013)**: Haswell introduced PDEP/PEXT
- **Langdale & Lemire (2019)**: simdjson uses PEXT for JSON parsing

---

## Bit Reversal and Rotation

### Bit Reversal

Reverse bits in a word (useful for certain tree encodings):

```rust
fn reverse_bits(mut x: u64) -> u64 {
    x = ((x & 0x5555_5555_5555_5555) << 1) | ((x >> 1) & 0x5555_5555_5555_5555);
    x = ((x & 0x3333_3333_3333_3333) << 2) | ((x >> 2) & 0x3333_3333_3333_3333);
    x = ((x & 0x0F0F_0F0F_0F0F_0F0F) << 4) | ((x >> 4) & 0x0F0F_0F0F_0F0F_0F0F);
    x.swap_bytes()
}
```

### Rotation

```rust
fn rotate_left(x: u64, n: u32) -> u64 {
    x.rotate_left(n)  // Single instruction on modern CPUs
}
```

---

## Usage in Succinctly

| Technique      | Location                     | Purpose                           |
|----------------|------------------------------|-----------------------------------|
| POPCNT         | `bits/popcount.rs`           | Rank queries, index building      |
| CTZ loop       | `util/broadword.rs`          | Select-in-word                    |
| Bit clearing   | `util/broadword.rs`          | Set bit iteration                 |
| PDEP toggle    | `dsv/simd/bmi2.rs`           | Quote region masking (10x faster) |
| Broadword      | `bits/popcount.rs`           | Portable popcount fallback        |

---

## Key Lessons

1. **Hardware beats software**: Use `count_ones()`, `trailing_zeros()` which compile to single instructions
2. **Profile before optimizing**: CTZ loop often beats "optimal" broadword due to branch prediction
3. **Know your instructions**: PDEP is for sparse patterns, not consecutive bits
4. **Bit clearing idiom**: `x &= x - 1` is fundamental for iteration

---

## References

- Knuth, D. E. "The Art of Computer Programming, Volume 4A: Combinatorial Algorithms"
- Warren, H. S. "Hacker's Delight" (2nd ed., 2012)
- Anderson, S. "Bit Twiddling Hacks" (Stanford Graphics)
- Vigna, S. "Broadword Implementation of Rank/Select Queries" (2008)
