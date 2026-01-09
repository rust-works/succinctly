# Implementation Plan: Balanced Parentheses Operations

## Overview

This plan covers implementing balanced parentheses (BP) operations for succinct tree navigation.
The implementation follows a two-phase approach:

1. **Phase 1**: Broadword `findUnmatchedClose` for single words (O(log w) per word)
2. **Phase 2**: RangeMin structure for O(1) operations on vectors

## Background

In balanced parentheses encoding:
- `1` = open parenthesis `(`
- `0` = close parenthesis `)`
- A valid BP sequence has matching open/close pairs

### Key Operations

| Operation       | Description                                                     |
|-----------------|-----------------------------------------------------------------|
| `find_close(i)` | Find position of `)` matching `(` at position `i`               |
| `find_open(i)`  | Find position of `(` matching `)` at position `i`               |
| `enclose(i)`    | Find position of `(` enclosing the `(` at position `i` (parent) |

### Excess Function

The **excess** at position `i` is: `rank1(i) - rank0(i) = 2*rank1(i) - i`

- Represents "depth" in the tree
- `find_close(i)` finds smallest `j > i` where `excess(j) = excess(i) - 1`

---

## Phase 1: Broadword Word-Level Operations

### 1.1 Core Algorithm: `find_unmatched_close_in_word`

Based on Vigna's paper ["Broadword Implementation of Parenthesis Queries"](https://arxiv.org/abs/1301.5468).

**File**: `src/bp.rs` (new file)

```rust
//! Balanced parentheses operations.

/// Find position of first unmatched close parenthesis in a 64-bit word.
///
/// Returns bit position (0-63) of the first `)` that doesn't have a matching `(`
/// to its left within the word. Returns 64 if no unmatched close exists.
///
/// Bits are: 1 = open `(`, 0 = close `)`
///
/// Based on: Vigna, "Broadword Implementation of Parenthesis Queries", 2013
#[inline]
pub fn find_unmatched_close_in_word(x: u64) -> u32 {
    // ... implementation from haskell-works
}
```

**Algorithm outline** (from hw-balancedparens):
1. Compute byte-level excess using SWAR
2. Find first byte where cumulative excess goes negative
3. Binary search within that byte

### 1.2 Helper: Byte-Level Constants

```rust
/// L8: 0x0101_0101_0101_0101 - 1 in each byte's LSB
const L8: u64 = 0x0101_0101_0101_0101;

/// H8: 0x8080_8080_8080_8080 - 1 in each byte's MSB
const H8: u64 = 0x8080_8080_8080_8080;
```

Already exist in `broadword.rs`.

### 1.3 Word-Level `find_close`

```rust
/// Find matching close parenthesis within a single word.
///
/// Given an open parenthesis at bit position `p` (0-indexed),
/// returns the position of its matching close.
///
/// Returns `None` if the matching close is beyond this word.
#[inline]
pub fn find_close_in_word(word: u64, p: u32) -> Option<u32> {
    if p >= 64 {
        return None;
    }

    // Shift so position p is at bit 0, then find first unmatched close
    let shifted = word >> p;

    // If bit at p is 0 (close), it matches itself
    if shifted & 1 == 0 {
        return Some(p);
    }

    // Find first unmatched close after position 0
    let result = find_unmatched_close_in_word(shifted >> 1);

    if result < 64 - p - 1 {
        Some(p + 1 + result)
    } else {
        None // Match is in a later word
    }
}
```

### 1.4 Vector-Level Linear Scan

```rust
/// Find matching close parenthesis in a bitvector.
///
/// Scans word-by-word from position `p` until finding the matching close.
/// Time: O(distance to matching close / 64)
pub fn find_close(words: &[u64], p: usize) -> Option<usize> {
    let word_idx = p / 64;
    let bit_idx = p % 64;

    // Check if match is in the same word
    if let Some(local) = find_close_in_word(words[word_idx], bit_idx as u32) {
        return Some(word_idx * 64 + local as usize);
    }

    // Track excess as we scan forward
    let first_word = words[word_idx] >> bit_idx;
    let mut excess = 2 * first_word.count_ones() as i32 - (64 - bit_idx) as i32;

    // Scan subsequent words
    for (i, &word) in words[word_idx + 1..].iter().enumerate() {
        let word_excess = 2 * word.count_ones() as i32 - 64;

        // Check if this word contains the match (excess will go to -1 at some point)
        if excess + /* min excess in word */ <= 0 {
            // Binary search within word
            // ...
        }

        excess += word_excess;
    }

    None
}
```

### 1.5 Tests for Phase 1

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_close_simple() {
        // "()" = 0b10
        assert_eq!(find_close_in_word(0b10, 0), Some(1));
    }

    #[test]
    fn test_find_close_nested() {
        // "(())" = 0b0110
        assert_eq!(find_close_in_word(0b0110, 0), Some(3));
        assert_eq!(find_close_in_word(0b0110, 1), Some(2));
    }

    #[test]
    fn test_find_close_already_close() {
        // ")" = 0b0
        assert_eq!(find_close_in_word(0b0, 0), Some(0));
    }

    #[test]
    fn test_find_unmatched_close() {
        // "(()))" = 0b00110
        // First unmatched close is at position 4
        assert_eq!(find_unmatched_close_in_word(0b00110), 4);
    }
}
```

---

## Phase 2: RangeMin for O(1) Operations

### 2.1 Data Structure

```rust
/// Balanced parentheses with O(1) navigation operations.
///
/// Stores auxiliary min-excess data at three levels for fast find_close/find_open.
pub struct BalancedParens {
    /// The underlying bit vector (1=open, 0=close)
    words: Vec<u64>,
    /// Number of valid bits
    len: usize,

    /// L0: Per-word min excess and cumulative excess
    l0_min_excess: Vec<i8>,
    l0_cum_excess: Vec<i16>,

    /// L1: Per-32-word block (2048 bits)
    l1_min_excess: Vec<i16>,
    l1_cum_excess: Vec<i16>,

    /// L2: Per-1024-word block (65536 bits) - only for very large vectors
    l2_min_excess: Vec<i16>,
    l2_cum_excess: Vec<i16>,
}
```

### 2.2 Construction

```rust
impl BalancedParens {
    /// Build from a bitvector representing balanced parentheses.
    pub fn new(words: Vec<u64>, len: usize) -> Self {
        let l0_len = words.len();
        let l1_len = (words.len() + 31) / 32;
        let l2_len = (words.len() + 1023) / 1024;

        // Build L0: per-word statistics
        let mut l0_min_excess = Vec::with_capacity(l0_len);
        let mut l0_cum_excess = Vec::with_capacity(l0_len);

        for &word in &words {
            let (min_e, total_e) = word_min_excess(word);
            l0_min_excess.push(min_e);
            l0_cum_excess.push(total_e);
        }

        // Build L1: per-32-word block statistics
        // ... aggregate from L0

        // Build L2: per-1024-word block statistics
        // ... aggregate from L1

        Self { words, len, l0_min_excess, l0_cum_excess, l1_min_excess, l1_cum_excess, l2_min_excess, l2_cum_excess }
    }
}

/// Compute minimum excess and total excess for a single word.
fn word_min_excess(word: u64) -> (i8, i16) {
    let mut excess: i8 = 0;
    let mut min_excess: i8 = 0;

    for i in 0..64 {
        if (word >> i) & 1 == 1 {
            excess += 1; // open
        } else {
            excess -= 1; // close
            min_excess = min_excess.min(excess);
        }
    }

    (min_excess, excess as i16)
}
```

### 2.3 O(1) find_close Algorithm

```rust
impl BalancedParens {
    /// Find matching close parenthesis. O(1) time.
    pub fn find_close(&self, p: usize) -> Option<usize> {
        if p >= self.len {
            return None;
        }

        let word_idx = p / 64;
        let bit_idx = p % 64;

        // Is position p a close paren? Then it matches itself.
        if (self.words[word_idx] >> bit_idx) & 1 == 0 {
            return Some(p);
        }

        // Start with excess = 1 (we just saw an open paren)
        // Need to find first position where excess reaches 0

        // Phase 1: Search within current word
        // Phase 2: Search L0 blocks to find target word
        // Phase 3: Search L1 blocks if needed
        // Phase 4: Search within target word

        self.find_close_from(p, 1)
    }

    fn find_close_from(&self, p: usize, initial_excess: i32) -> Option<usize> {
        // Use hierarchical search with L0/L1/L2 indices
        // Similar to rm2FindClose in haskell-works RangeMin.hs
        todo!()
    }
}
```

### 2.4 Additional Operations

```rust
impl BalancedParens {
    /// Find matching open parenthesis. O(1) time.
    pub fn find_open(&self, p: usize) -> Option<usize> {
        // Mirror of find_close, searching backwards
        todo!()
    }

    /// Find enclosing open parenthesis (parent node). O(1) time.
    pub fn enclose(&self, p: usize) -> Option<usize> {
        // Find open paren at excess = current_excess - 1
        todo!()
    }

    /// Navigate to first child in tree.
    pub fn first_child(&self, p: usize) -> Option<usize> {
        if self.is_open(p) && p + 1 < self.len && self.is_open(p + 1) {
            Some(p + 1)
        } else {
            None
        }
    }

    /// Navigate to next sibling in tree.
    pub fn next_sibling(&self, p: usize) -> Option<usize> {
        let close = self.find_close(p)?;
        if close + 1 < self.len && self.is_open(close + 1) {
            Some(close + 1)
        } else {
            None
        }
    }

    /// Navigate to parent node.
    pub fn parent(&self, p: usize) -> Option<usize> {
        self.enclose(p)
    }

    /// Check if position p is an open parenthesis.
    #[inline]
    pub fn is_open(&self, p: usize) -> bool {
        (self.words[p / 64] >> (p % 64)) & 1 == 1
    }

    /// Check if position p is a close parenthesis.
    #[inline]
    pub fn is_close(&self, p: usize) -> bool {
        !self.is_open(p)
    }
}
```

---

## File Structure

```
src/
├── lib.rs           # Add: mod bp; pub use bp::BalancedParens;
├── bp.rs            # NEW: All balanced parentheses code
│   ├── find_unmatched_close_in_word()
│   ├── find_close_in_word()
│   ├── word_min_excess()
│   └── BalancedParens struct + impl
├── broadword.rs     # Existing: L8, H8, select_in_word
└── ... (existing files)
```

---

## Implementation Order

### Step 1: Create `src/bp.rs` with Phase 1
1. Add module declaration to `lib.rs`
2. Implement `find_unmatched_close_in_word` from Vigna's algorithm
3. Implement `find_close_in_word` wrapper
4. Add comprehensive tests
5. Verify against haskell-works test cases

### Step 2: Add vector-level linear scan
1. Implement `find_close` for `&[u64]`
2. Implement `find_open` for `&[u64]`
3. Add tests with multi-word patterns

### Step 3: Implement Phase 2 RangeMin
1. Add `BalancedParens` struct with L0/L1/L2 storage
2. Implement `new()` construction
3. Implement hierarchical `find_close`
4. Implement `find_open` and `enclose`
5. Add navigation helpers (first_child, next_sibling, parent)

### Step 4: Benchmarks
1. Add benchmarks comparing Phase 1 vs Phase 2
2. Benchmark against typical JSON/XML tree depths
3. Compare with linear scan baseline

---

## Testing Strategy

### Unit Tests
- Single-word patterns: `()`, `(())`, `((()))`, `()()`
- Edge cases: empty, all open, all close
- Large patterns spanning multiple words

### Property-Based Tests
- Generate random valid BP sequences
- Verify `find_close(find_open(i)) == i` for close positions
- Verify `find_open(find_close(i)) == i` for open positions
- Verify tree navigation consistency

### Correctness Tests
- Port test cases from haskell-works
- Compare against naive O(n) implementation

---

## Space/Time Complexity

| Structure          | Space Overhead | find_close | find_open | enclose |
|--------------------|----------------|------------|-----------|---------|
| None (linear scan) | 0%             | O(n)       | O(n)      | O(n)    |
| Phase 1 only       | 0%             | O(n)       | O(n)      | O(n)    |
| RangeMin           | ~6%            | O(1)       | O(1)      | O(1)    |

---

## References

1. Vigna, ["Broadword Implementation of Parenthesis Queries"](https://arxiv.org/abs/1301.5468), 2013
2. Sadakane & Navarro, ["Fully-Functional Succinct Trees"](https://arxiv.org/abs/0905.0768), SODA 2010
3. [SDSL-lite bp_support_sada](https://github.com/simongog/sdsl-lite/blob/master/include/sdsl/bp_support_sada.hpp)
4. [haskell-works succinct-core BalancedParens](https://github.com/haskell-works/succinct)
