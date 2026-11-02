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

## Memory-Efficient Construction

### Peak vs Steady-State Memory

**Key insight**: Optimizing construction reduces **peak** memory but may not affect **steady-state** memory (what benchmarks measure).

```rust
// BAD: Double allocation during construction
fn from_vec(data: Vec<u128>) -> Self {
    let layout = Layout::from_size_align(len * 16, 64).unwrap();
    let ptr = alloc::alloc(layout);  // New allocation
    copy_nonoverlapping(data.as_ptr(), ptr, len);  // Copy
    // data dropped here - peak was 2x
}

// GOOD: Builder pattern - single allocation
struct CacheAlignedBuilder {
    ptr: NonNull<u128>,
    len: usize,
    capacity: usize,
}

impl CacheAlignedBuilder {
    fn with_capacity(cap: usize) -> Self {
        let layout = Layout::from_size_align(cap * 16, 64).unwrap();
        let ptr = alloc::alloc(layout);  // Allocate aligned immediately
        Self { ptr, len: 0, capacity: cap }
    }

    fn push(&mut self, value: u128) {
        self.ptr.as_ptr().add(self.len).write(value);
        self.len += 1;
    }

    fn build(self) -> CacheAligned {
        // Transfer ownership, no copy needed
        let result = CacheAligned { ptr: self.ptr, len: self.len };
        mem::forget(self);  // Prevent Drop
        result
    }
}
```

**Benefits**:
- Single allocation (not Vec + aligned copy)
- 50% peak memory reduction during construction
- Cache-aligned from the start

### "Succinct" Doesn't Mean Everything Is Compact

In this codebase:
- **Succinct operations**: rank/select are O(1) with o(n) auxiliary space
- **Position storage is a separate choice**: a node's text positions are not part of the BP
  structure, and how they are stored decides most of the index's size

`YamlIndex` used to keep `bp_to_text` and `bp_to_text_end` as `Vec<u32>` (4 bytes per node each,
~80 MB on a 100 MB input), its biggest component. They are now bitmaps: `OpenPositions` /
`AdvancePositions` (`src/yaml/advance_positions.rs`, P12, #62) and `EndPositions` /
`CompactEndPositions` (`src/yaml/end_positions.rs`). See
[docs/parsing/yaml-succinct.md](../../../docs/parsing/yaml-succinct.md) for the current layout and
why it is bitmaps rather than position vectors.

**Elias-Fano was tried for these arrays and rejected** (#58): `src/bits/elias_fano.rs` measured
34-40x slower than the vectors for the cursor-iteration and random-access patterns the positions
need. It now serves `text::LineIndex` (ADR-0012), where access is rare. See
[compact-index-investigation.md](../../../docs/plan/compact-index-investigation.md) for the
investigation.

## See Also

- [docs/optimizations/bit-manipulation.md](../../../docs/optimizations/bit-manipulation.md) - Bit manipulation techniques
- [docs/optimizations/lookup-tables.md](../../../docs/optimizations/lookup-tables.md) - Lookup table patterns
- [docs/optimizations/hierarchical-structures.md](../../../docs/optimizations/hierarchical-structures.md) - Rank/select structures
- [docs/optimizations/access-patterns.md](../../../docs/optimizations/access-patterns.md) - Sequential vs random access
- [docs/optimizations/history.md](../../../docs/optimizations/history.md) - Historical optimization record
- [docs/plan/compact-index-investigation.md](../../../docs/plan/compact-index-investigation.md) - Elias-Fano for position arrays (rejected, #58)
