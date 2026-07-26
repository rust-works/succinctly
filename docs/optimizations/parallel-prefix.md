# Parallel Prefix Operations

[Home](../../) > [Docs](../) > [Optimizations](./) > Parallel Prefix

Parallel prefix (scan) operations compute cumulative results across a sequence. While inherently sequential in nature, clever algorithms can parallelize portions of the computation.

## Overview

| Operation      | Sequential | Parallel (SIMD) | Use Case                    |
|----------------|------------|-----------------|-----------------------------|
| Prefix Sum     | O(n)       | O(n/w + log w)  | Running totals              |
| Prefix XOR     | O(n)       | O(log w) / word | Quote state tracking        |
| Prefix Max/Min | O(n)       | O(n/w + log w)  | Range queries               |

Where `w` = SIMD width (e.g., 64 bits for u64, 256 bits for AVX2).

---

## The Sequential Problem

Given array `a[0..n]`, compute `prefix[i] = a[0] ⊕ a[1] ⊕ ... ⊕ a[i]`

Where `⊕` is any associative operation (sum, XOR, max, etc.).

```rust
fn prefix_sum(data: &[u64]) -> Vec<u64> {
    let mut result = Vec::with_capacity(data.len());
    let mut sum = 0;
    for &x in data {
        sum += x;
        result.push(sum);
    }
    result
}
```

**Problem**: Each step depends on the previous - hard to parallelize.

---

## Within-Word Parallel Prefix

For operations within a single machine word, use doubling strategy.

### Prefix XOR (Quote State)

Compute cumulative XOR of all bits in a word:

```rust
// src/util/simd/quote_mask.rs — one copy, shared by every DSV backend (#182)
// After this, bit[i] = XOR of bits 0..=i
fn prefix_xor(mut y: u64) -> u64 {
    y ^= y << 1;   // XOR adjacent pairs
    y ^= y << 2;   // XOR adjacent quads
    y ^= y << 4;   // XOR adjacent bytes
    y ^= y << 8;   // XOR adjacent u16s
    y ^= y << 16;  // XOR adjacent u32s
    y ^= y << 32;  // XOR halves
    y
}
```

**How it works**:
```
Initial:     [a, b, c, d, e, f, g, h]
After << 1:  [a, a^b, b^c, c^d, d^e, e^f, f^g, g^h]
After << 2:  [a, a^b, a^b^c, a^b^c^d, b^c^d^e, c^d^e^f, d^e^f^g, e^f^g^h]
...continues until each position has XOR of all positions up to it
```

**Application in DSV parsing**: Track whether each position is inside quotes:

```rust
// src/util/simd/quote_mask.rs — shared tail for the AVX2/SSE2/NEON backends.
// Each backend supplies only `quote_xor`: AVX2/SSE2 use the shift chain above,
// NEON uses a single PMULL (carryless multiply by all-ones).
fn toggle64_from_prefix_xor(carry: u64, quote_mask: u64, quote_xor: u64) -> (u64, u64) {
    // Broadcast the carry bit to all 64 lanes, then complement: 1 = outside quotes
    let inside = quote_xor ^ 0u64.wrapping_sub(carry & 1);
    (!inside, next_carry(carry, quote_mask))
}
```

### Prefix Sum in Word

Similar doubling for addition:

```rust
fn prefix_sum_word(mut x: u64) -> u64 {
    x += x << 8;
    x += x << 16;
    x += x << 32;
    x
}
// Note: Overflow behavior must be considered
```

---

## Carry Propagation with PDEP

BMI2's PDEP instruction enables efficient carry propagation:

```rust
// src/util/simd/x86.rs — the deposit is the only BMI2-specific step. Its SVE2
// twin (`util/simd/sve2.rs::toggle64_sve2`) swaps PDEP for BDEP and is otherwise
// the same call.
unsafe fn toggle64_bmi2(carry: u64, quote_mask: u64) -> (u64, u64) {
    // Scatter alternating pattern to quote positions
    let addend = _pdep_u64(ODDS_MASK << (carry & 1), quote_mask);
    toggle64_from_deposit(carry, quote_mask, addend)
}

// src/util/simd/quote_mask.rs — shared adder tail (#182)
fn toggle64_from_deposit(carry: u64, quote_mask: u64, addend: u64) -> (u64, u64) {
    // Use addition to propagate carries (fills between quote pairs)
    let c = carry & 1;
    let result = ((addend << 1) | c).wrapping_add(!quote_mask);

    // Carry for the next chunk is quote-count parity, NOT the adder's
    // overflow: `addend << 1` drops an opener deposited at bit 63 (#149).
    (result, next_carry(carry, quote_mask))
}
```

**Result**: 10x faster than prefix_xor, 50-100x faster than scalar. Gated on
*fast* BMI2 — AMD Zen 1/2 run PDEP in ~18-cycle microcode and take the AVX2
prefix-xor arm instead (#182).

Both families share `next_carry`, so the chunk-boundary carry has exactly one
definition. It previously had five, and the bit-63 bug lived in two of them
(#149).

---

## SIMD Prefix Sum

### The Challenge

SIMD processes lanes independently - no cross-lane operations:

```
Lane 0: a₀ → prefix₀
Lane 1: a₁ → prefix₁  // Can't access a₀!
Lane 2: a₂ → prefix₂  // Can't access a₀, a₁!
...
```

### Within-Lane + Cross-Lane

Process in two phases:

```rust
unsafe fn prefix_sum_avx2(data: &[i32]) -> Vec<i32> {
    // Phase 1: Local prefix sums within 256-bit chunks
    let mut local_sums = vec![];
    let mut chunk_totals = vec![];

    for chunk in data.chunks(8) {
        let v = _mm256_loadu_si256(chunk.as_ptr() as *const __m256i);

        // Local prefix sum within vector
        let local = local_prefix_sum_256(v);
        local_sums.push(local);

        // Extract total for this chunk
        chunk_totals.push(_mm256_extract_epi32(local, 7));
    }

    // Phase 2: Prefix sum of chunk totals (sequential)
    let mut running_total = 0;
    for (local, &chunk_total) in local_sums.iter_mut().zip(&chunk_totals) {
        // Add running total to all elements in chunk
        let offset = _mm256_set1_epi32(running_total);
        *local = _mm256_add_epi32(*local, offset);
        running_total += chunk_total;
    }

    // Store results...
}
```

### Why This Is Limited

The sequential phase (prefix sum of chunk totals) becomes the bottleneck:
- O(n/w) chunks need sequential processing
- Amdahl's law limits speedup

**Result in succinctly**: NEON batched popcount for cumulative index was 25% slower than scalar because prefix sum is inherently sequential.

---

## Cumulative Index Alternative

Instead of computing prefix sums at query time, precompute them:

```rust
struct CumulativeIndex {
    // cumulative[i] = popcount(words[0..i])
    cumulative: Vec<u32>,
}

impl CumulativeIndex {
    fn build(words: &[u64]) -> Self {
        let mut cumulative = Vec::with_capacity(words.len());
        let mut sum = 0u32;
        for &word in words {
            sum += word.count_ones();
            cumulative.push(sum);
        }
        Self { cumulative }
    }

    // O(1) rank query
    fn rank(&self, word_idx: usize) -> u32 {
        if word_idx == 0 { 0 } else { self.cumulative[word_idx - 1] }
    }
}
```

**Trade-off**: O(n) space and build time, but O(1) query time.
**Result**: 627x speedup for workloads with many queries.

---

## State Machine Parallelization

### The Fundamental Limit

Finite state machines have inherent data dependency:

```
state[i+1] = transition(state[i], input[i])
```

Each state depends on all previous inputs - cannot parallelize naively.

### Speculative Execution

Compute transitions from all possible initial states:

```rust
// PFSM: Parallel Finite State Machine
// For each byte, store transitions from all states
const TRANSITION_TABLE: [u32; 256] = /* packed transitions */;

fn pfsm_transition(byte: u8) -> [u8; 4] {
    let packed = TRANSITION_TABLE[byte as usize];
    [
        (packed >> 0) as u8 & 0xFF,   // Next state if current = 0
        (packed >> 8) as u8 & 0xFF,   // Next state if current = 1
        (packed >> 16) as u8 & 0xFF,  // Next state if current = 2
        (packed >> 24) as u8 & 0xFF,  // Next state if current = 3
    ]
}
```

**Limitation**: Still need to chain transitions sequentially. NEON PFSM shuffle composition was 47% slower than scalar.

### Prior Art

- **Mytkowicz et al. (2014)**: "Data-Parallel Finite-State Machines"
- **Langdale & Lemire (2019)**: PFSM in simdjson

---

## Usage in Succinctly

| Technique            | Location                   | Purpose               | Result        |
|----------------------|----------------------------|-----------------------|---------------|
| prefix_xor           | `util/simd/quote_mask.rs`  | Quote state tracking  | Baseline      |
| PDEP toggle          | `util/simd/x86.rs`         | Fast quote masking    | 10x faster    |
| BDEP toggle          | `util/simd/sve2.rs`        | Fast quote masking    | 10x faster    |
| Cumulative index     | `json/light.rs`            | Precomputed prefix    | 627x faster   |
| PFSM tables          | `json/pfsm_tables.rs`      | State parallelization | 33-77% faster |

### Failed Attempts

| Technique              | Location            | Result     | Reason                       |
|------------------------|---------------------|------------|------------------------------|
| NEON batched popcount  | Rejected            | -25%       | Prefix sum sequential        |
| NEON PFSM shuffle      | Rejected            | -47%       | State dependency chain       |

---

## Key Lessons

1. **Within-word is parallelizable**: Doubling strategy gives O(log w)
2. **Cross-word is sequential**: Prefix sum of chunk totals is the bottleneck
3. **Precompute when possible**: Cumulative index trades space for time
4. **State machines are hard**: Speculative execution helps but doesn't eliminate dependency
5. **Profile carefully**: "Parallel" doesn't always mean faster

---

## See Also

- [Bit Manipulation](bit-manipulation.md) — PDEP/PEXT for DSV quote masking
- [SIMD](simd.md) — vectorised prefix XOR on AVX2/NEON

## References

- Blelloch, G. "Prefix Sums and Their Applications" (1990)
- Mytkowicz, T. et al. "Data-Parallel Finite-State Machines" (2014)
- Harris, M. "Parallel Prefix Sum (Scan) with CUDA" (2007)
- Intel "Optimization Reference Manual" - Parallel algorithms
- Langdale, G. & Lemire, D. "Parsing Gigabytes of JSON per Second" (2019)
