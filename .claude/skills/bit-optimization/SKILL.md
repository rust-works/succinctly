---
name: bit-optimization
description: Bit-level optimization patterns for succinct data structures. Use when optimizing rank/select, popcount, or balanced parentheses operations. Triggers on terms like "rank", "select", "popcount", "bit manipulation", "lookup table", "optimization".
---

# Bit-Level Optimization Patterns

Learnings from optimizing succinct data structure operations.

**Comprehensive documentation**: See [docs/optimizations/](../../../docs/optimizations/) for full technique reference:
- [bit-manipulation.md](../../../docs/optimizations/bit-manipulation.md) - Popcount, CTZ, PDEP/PEXT
- [lookup-tables.md](../../../docs/optimizations/lookup-tables.md) - Byte-level lookup tables
- [hierarchical-structures.md](../../../docs/optimizations/hierarchical-structures.md) - Rank/select indices

## Byte-Level Lookup Tables

**When to use**: Scanning bits one-at-a-time is slow (64 iterations per word). Use 256-entry lookup tables to process 8 bits at once.

### Pattern: Two-Level Lookup

```rust
// Precompute at compile time
const BYTE_MIN_EXCESS: [i8; 256] = { /* min prefix sum for each byte value */ };
const BYTE_TOTAL_EXCESS: [i8; 256] = { /* total excess (popcount*2 - 8) */ };
const BYTE_FIND_CLOSE: [[u8; 16]; 256] = { /* 2D: byte_value x initial_excess */ };

fn find_close_in_word_fast(word: u64, initial_excess: i32) -> Option<usize> {
    let bytes = word.to_le_bytes();
    let mut excess = initial_excess;

    for (i, &byte_val) in bytes.iter().enumerate() {
        // Level 1: Can the result be in this byte?
        let min_excess_in_byte = BYTE_MIN_EXCESS[byte_val as usize] as i32;
        if excess + min_excess_in_byte <= 0 {
            // Level 2: Find exact position within byte
            if excess <= 16 {
                let match_pos = BYTE_FIND_CLOSE[byte_val as usize][(excess - 1) as usize];
                return Some(i * 8 + match_pos as usize);
            }
            // Fallback for excess > 16: bit-by-bit scan of this byte only
        }
        excess += BYTE_TOTAL_EXCESS[byte_val as usize] as i32;
    }
    None
}
```

### Key Insights

1. **Two-level lookup**: First check if result is in this byte (fast), then find exact position
2. **Compile-time computation**: Use `const` blocks for zero runtime overhead
3. **Hierarchical skip**: Skip entire bytes/words when result can't be there
4. **Fallback for edge cases**: Handle out-of-range lookup indices with bit scan

### Results

- 50-90% speedup for BP operations
- ~90% speedup for construction
- Deep nesting: 221ns vs 450ns (51% improvement)

## Cumulative Index Pattern

**When to use**: O(n) operation called O(n) times = O(n^2). Add index for O(log n).

```rust
// Build cumulative popcount index
fn build_rank_index(words: &[u64]) -> Vec<u32> {
    let mut rank = Vec::with_capacity(words.len() + 1);
    let mut cumulative: u32 = 0;
    rank.push(0);
    for &word in words {
        cumulative += word.count_ones();
        rank.push(cumulative);
    }
    rank
}

// Binary search for select
fn select1(&self, k: usize) -> Option<usize> {
    let k32 = k as u32;
    let mut lo = 0;
    let mut hi = self.words.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if self.rank[mid + 1] <= k32 { lo = mid + 1; }
        else { hi = mid; }
    }
    // Scan within word for exact bit position
    let word_rank = self.rank[lo];
    let remaining = k32 - word_rank;
    select_in_word(self.words[lo], remaining as u32).map(|b| lo * 64 + b as usize)
}
```

**Result**: 627x speedup (2.76s to 4.4ms) when select is called per result.

## Exponential Search for Sequential Select

**When to use**: Select queries during iteration (`.users[]`) access sequential positions.

### Pattern: Galloping from Hint

```rust
fn select_with_hint(ib_rank: &[u32], k: usize, hint: usize) -> Option<usize> {
    let k32 = k as u32;
    let n = ib_rank.len() - 1;
    let hint = hint.min(n.saturating_sub(1));

    // Check direction from hint
    let (lo, hi) = if ib_rank[hint + 1] <= k32 {
        // Gallop forward: 1, 2, 4, 8, ... until overshoot
        let mut bound = 1;
        let mut prev = hint;
        loop {
            let next = (hint + bound).min(n);
            if next >= n || ib_rank[next + 1] > k32 {
                break (prev, next);
            }
            prev = next;
            bound *= 2;
        }
    } else {
        // Gallop backward
        // ...
    };

    // Binary search within narrowed range [lo, hi]
}
```

### Results

| Access Pattern | Binary Search | Exponential Search | Speedup               |
|----------------|---------------|--------------------|-----------------------|
| Sequential     | 335 us        | 102 us             | **3.3x**              |
| Random         | 780 us        | 1.07 ms            | **0.7x** (regression) |

### Trade-off

Exponential search adds ~37% overhead for random access. Consider providing two methods:
- `select1_from(k, hint)` - Fast for sequential (iteration)
- `select1_binary(k)` - Fast for random (indexed lookup)

## Hierarchical Skip Pattern

When searching for a position in a bitvector:

1. **L2 blocks** (1024 words): Skip if block's min-excess can't contain result
2. **L1 blocks** (32 words): Skip if block's min-excess can't contain result
3. **L0 words** (1 word): Skip if word's min-excess can't contain result
4. **Bytes** (8 bits): Skip if byte's min-excess can't contain result
5. **Bits**: Final scan within byte

Each level has precomputed statistics enabling O(1) skip decision.

## Critical Lesson: Benchmark Against Production

**Problem**: PFSM batched processing appeared to be 40% improvement, but was actually 25% slower than production.

### What Happened

| Implementation           | Throughput    | Role                      |
|--------------------------|---------------|---------------------------|
| `pfsm_optimized.rs`      | 516-578 MiB/s | **Production** (deployed) |
| `pfsm_simd.rs` (batched) | 398-461 MiB/s | New attempt               |
| `pfsm.rs` (basic)        | 282-344 MiB/s | Reference implementation  |

The batched approach was benchmarked against `pfsm.rs` (basic reference), showing 40% improvement. But `pfsm_optimized.rs` was already deployed and was 25% faster.

### The Rule

**ALWAYS benchmark new optimizations against ALL existing implementations:**

```rust
// RIGHT: Compare against everything, including production
group.bench_function("basic", |b| b.iter(|| pfsm::process(json)));
group.bench_function("batched", |b| b.iter(|| pfsm_simd::process(json)));
group.bench_function("production", |b| b.iter(|| pfsm_optimized::process(json)));
```

### Checklist Before Declaring "Optimization Success"

- [ ] Benchmark includes current production implementation
- [ ] New code is faster than production, not just faster than a reference
- [ ] Multiple input sizes tested (small, medium, large)
- [ ] Multiple patterns tested (if applicable)
- [ ] End-to-end benchmark confirms improvement carries through

## See Also

- [docs/optimizations/bit-manipulation.md](../../../docs/optimizations/bit-manipulation.md) - Bit manipulation techniques
- [docs/optimizations/lookup-tables.md](../../../docs/optimizations/lookup-tables.md) - Lookup table patterns
- [docs/optimizations/hierarchical-structures.md](../../../docs/optimizations/hierarchical-structures.md) - Rank/select structures
- [docs/optimizations/access-patterns.md](../../../docs/optimizations/access-patterns.md) - Sequential vs random access
- [docs/optimizations/history.md](../../../docs/optimizations/history.md) - Historical optimization record
