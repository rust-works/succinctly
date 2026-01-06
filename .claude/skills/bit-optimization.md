# Bit-Level Optimization Patterns

## Byte-Level Lookup Tables

**When to use**: Scanning bits one-at-a-time is slow (64 iterations per word). Use 256-entry lookup tables to process 8 bits at once.

### Pattern: Two-Level Lookup

```rust
// Precompute at compile time
const BYTE_MIN_EXCESS: [i8; 256] = { /* min prefix sum for each byte value */ };
const BYTE_TOTAL_EXCESS: [i8; 256] = { /* total excess (popcount*2 - 8) */ };
const BYTE_FIND_CLOSE: [[u8; 16]; 256] = { /* 2D: byte_value × initial_excess */ };

fn find_close_in_word_fast(word: u64, start_bit: usize, initial_excess: i32) -> Option<usize> {
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

**When to use**: O(n) operation called O(n) times = O(n²). Add index for O(log n).

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

### Result

627x speedup (2.76s → 4.4ms) when select is called per result.

## Hierarchical Skip Pattern

When searching for a position in a bitvector:

1. **L2 blocks** (1024 words): Skip if block's min-excess can't contain result
2. **L1 blocks** (32 words): Skip if block's min-excess can't contain result
3. **L0 words** (1 word): Skip if word's min-excess can't contain result
4. **Bytes** (8 bits): Skip if byte's min-excess can't contain result
5. **Bits**: Final scan within byte

Each level has precomputed statistics enabling O(1) skip decision.
