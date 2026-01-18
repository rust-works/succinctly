# Branchless Programming

Modern CPUs use branch prediction to speculatively execute code before knowing which path will be taken. Mispredicted branches cause pipeline flushes costing 10-20 cycles. Branchless code eliminates this penalty for unpredictable branches.

## Overview

| Technique              | Use Case                    | Benefit               |
|------------------------|-----------------------------|-----------------------|
| Conditional move       | Simple if/else              | No misprediction      |
| Arithmetic selection   | Boolean → value             | Single instruction    |
| Lookup tables          | Multi-way branch            | O(1) vs O(branches)   |
| Bit manipulation       | Conditional operations      | Parallel execution    |
| SIMD masking           | Vector conditionals         | All lanes at once     |

---

## Branch Prediction Basics

### How It Works

CPU predicts which way a branch will go and executes speculatively:

```rust
if condition {      // CPU predicts true/false
    do_expensive(); // Executes speculatively
}
// If prediction wrong, flush pipeline (10-20 cycles)
```

### When Branches Are Predictable

**Good** (usually predicted correctly):
- Loop counters: `for i in 0..n`
- Null checks: `if ptr.is_some()`
- Type checks: `if x.is_string()`

**Bad** (unpredictable):
- Data-dependent: `if data[i] > threshold`
- Random: `if random() > 0.5`
- Alternating: `if i % 2 == 0`

---

## Conditional Move (CMOV)

Replace `if/else` with arithmetic that computes both paths.

### The Pattern

```rust
// Branching version
let result = if condition { a } else { b };

// Branchless version (compiler may generate CMOV)
let mask = -(condition as i64);  // 0 or -1 (all bits set)
let result = (a & mask) | (b & !mask);

// Or using Rust's select
let result = if condition { a } else { b };  // Compiler often uses CMOV
```

### When CMOV Helps

```rust
// Unpredictable branch - CMOV wins
fn abs_branchless(x: i64) -> i64 {
    let mask = x >> 63;  // Sign extension: 0 or -1
    (x ^ mask) - mask    // Negates if negative
}

// Same as: if x < 0 { -x } else { x }
```

### When CMOV Hurts

CMOV always evaluates both sides. If one side is expensive:

```rust
// DON'T do this branchless
let result = if likely_true {
    cheap_value
} else {
    expensive_computation()  // Would always run with CMOV
};
```

---

## Arithmetic Selection

Convert boolean conditions to arithmetic operations.

### Boolean to Integer

```rust
// condition as integer: true=1, false=0
let x = condition as u64;

// Negate to get mask: true=-1 (all 1s), false=0
let mask = -(condition as i64) as u64;

// Or equivalently
let mask = 0u64.wrapping_sub(condition as u64);
```

### Min/Max Without Branches

```rust
fn min_branchless(a: u64, b: u64) -> u64 {
    let diff = a.wrapping_sub(b);
    let mask = (diff as i64 >> 63) as u64;  // -1 if a < b, else 0
    a ^ ((a ^ b) & mask)  // a if a < b, else b
}

// Simpler (compiler generates good code):
fn min_simple(a: u64, b: u64) -> u64 {
    std::cmp::min(a, b)  // Uses CMOV on x86
}
```

### Conditional Increment

```rust
// Branching
if condition { count += 1; }

// Branchless
count += condition as u64;
```

---

## Lookup Tables

Replace multi-way branches with array indexing.

### Switch to Table

```rust
// Branching (unpredictable for random input)
fn category(byte: u8) -> u8 {
    match byte {
        b'{' | b'}' => 1,  // Braces
        b'[' | b']' => 2,  // Brackets
        b'"' => 3,         // Quote
        _ => 0,            // Other
    }
}

// Branchless with lookup table
const CATEGORY: [u8; 256] = /* precomputed */;

fn category_branchless(byte: u8) -> u8 {
    CATEGORY[byte as usize]
}
```

### Usage in Succinctly

Character classification in JSON parsing:

```rust
// src/json/pfsm_tables.rs
const CHAR_CLASS: [u8; 256] = /* precomputed */;

fn classify(byte: u8) -> u8 {
    CHAR_CLASS[byte as usize]  // Single array access
}
```

---

## Bit Manipulation Patterns

### Conditional Set/Clear

```rust
// Set bit if condition is true
fn set_if(value: u64, bit: u32, condition: bool) -> u64 {
    let mask = (condition as u64) << bit;
    value | mask
}

// Clear bit if condition is true
fn clear_if(value: u64, bit: u32, condition: bool) -> u64 {
    let mask = !((condition as u64) << bit);
    value & mask
}
```

### Sign Extension for Masking

The sign bit can be extended to create a full mask:

```rust
fn sign_mask(x: i64) -> i64 {
    x >> 63  // Arithmetic shift: 0 if positive, -1 if negative
}

// Use for conditional selection
let result = (a & mask) | (b & !mask);
```

### Unsigned Comparison Trick

For SIMD without unsigned comparison:

```rust
// SSE2 lacks unsigned byte comparison
// Convert to signed by flipping high bit
fn unsigned_lt(a: u8, b: u8) -> bool {
    (a ^ 0x80) < (b ^ 0x80)
}

// SIMD version uses _mm_min_epu8 trick:
// a < b iff min(a,b) == a AND a != b
```

---

## SIMD Branchless Operations

SIMD inherently processes all lanes, making it naturally branchless.

### Vector Masking

```rust
use core::arch::x86_64::*;

unsafe fn select_avx2(mask: __m256i, a: __m256i, b: __m256i) -> __m256i {
    // Select a where mask is all-1s, b where mask is all-0s
    _mm256_blendv_epi8(b, a, mask)
}

unsafe fn conditional_add(values: __m256i, mask: __m256i) -> __m256i {
    // Add 1 only where mask is set
    let ones = _mm256_set1_epi8(1);
    let increment = _mm256_and_si256(ones, mask);
    _mm256_add_epi8(values, increment)
}
```

### NEON Equivalent

```rust
use core::arch::aarch64::*;

unsafe fn select_neon(mask: uint8x16_t, a: uint8x16_t, b: uint8x16_t) -> uint8x16_t {
    vbslq_u8(mask, a, b)  // Bitwise select
}
```

### Nibble Lookup Classification

Replace 13 comparisons with branchless lookup:

```rust
// src/json/simd/neon.rs
unsafe fn classify_nibble(chars: uint8x16_t) -> uint8x16_t {
    let lo = vandq_u8(chars, vdupq_n_u8(0x0F));
    let hi = vshrq_n_u8(chars, 4);

    let lo_table = vld1q_u8(LO_TABLE.as_ptr());
    let hi_table = vld1q_u8(HI_TABLE.as_ptr());

    let lo_result = vqtbl1q_u8(lo_table, lo);  // 16 parallel lookups
    let hi_result = vqtbl1q_u8(hi_table, hi);  // 16 parallel lookups

    vandq_u8(lo_result, hi_result)  // Combine
}
```

**Result**: 2-6% faster than comparison chains.

---

## Loop Optimizations

### Loop Unrolling

Process multiple items per iteration to reduce branch overhead:

```rust
// Before: branch every iteration
for item in data {
    process(item);
}

// After: branch every 4 iterations
let chunks = data.chunks_exact(4);
for chunk in chunks {
    process(chunk[0]);
    process(chunk[1]);
    process(chunk[2]);
    process(chunk[3]);
}
for item in chunks.remainder() {
    process(item);
}
```

### Predicated Loops

Process all elements, mask out invalid results:

```rust
// Instead of early exit
for i in 0..n {
    if data[i] == target {
        return Some(i);  // Unpredictable branch
    }
}

// Branchless scan (for small n)
let mut found_idx = n;  // Sentinel
for i in 0..n {
    let is_match = data[i] == target;
    let candidate = if is_match { i } else { n };
    found_idx = std::cmp::min(found_idx, candidate);
}
if found_idx < n { Some(found_idx) } else { None }
```

---

## When NOT to Use Branchless

### Predictable Branches

Well-predicted branches are nearly free:

```rust
// Loop bounds are perfectly predicted
for i in 0..1000 {  // Keep the branch!
    process(data[i]);
}
```

### Early Exit

When one path is much more common:

```rust
// 99% of calls hit early exit - keep the branch
fn find_rare(data: &[u8]) -> Option<usize> {
    for (i, &b) in data.iter().enumerate() {
        if b == RARE_VALUE {  // 1% chance
            return Some(i);
        }
    }
    None
}
```

### Expensive Alternatives

When branchless requires computing both paths:

```rust
// DON'T make this branchless
let result = if condition {
    simple_value
} else {
    complex_expensive_computation()
};
```

---

## Usage in Succinctly

| Technique            | Location              | Purpose                    |
|----------------------|-----------------------|----------------------------|
| Lookup tables        | `json/pfsm_tables.rs` | Character classification   |
| SIMD masking         | `json/simd/*.rs`      | Vector conditionals        |
| Nibble lookup        | `json/simd/neon.rs`   | Branchless char class      |
| Arithmetic select    | `util/broadword.rs`   | Bit manipulation           |
| Conditional move     | Compiler-generated    | Simple conditionals        |

---

## Key Lessons

1. **Profile first**: Only unpredictable branches need branchless code
2. **Let compiler help**: Modern compilers generate CMOV automatically
3. **Lookup tables scale**: O(1) vs O(branches) for multi-way decisions
4. **SIMD is inherently branchless**: Process all lanes, mask results
5. **Don't over-optimize**: Predictable branches are nearly free

---

## References

- Fog, A. "Optimizing Software in C++" - Branch prediction chapter
- Intel "64 and IA-32 Architectures Optimization Reference Manual"
- Warren, H. S. "Hacker's Delight" - Branchless techniques
- Lemire, D. "Branch Prediction" blog posts
- Langdale, G. & Lemire, D. "Parsing Gigabytes of JSON per Second" (2019)
