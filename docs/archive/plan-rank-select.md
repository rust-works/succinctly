# Implementation Plan: Cross-Platform High-Performance Rank/Select

## Recommended Approach: Poppy with Sampled Select

Based on the research, **Poppy** (with CS-Poppy's sampled select) offers the best balance:

| Criterion                 | Poppy + Sampled Select | SPIDER        | Rank9     |
|---------------------------|------------------------|---------------|-----------|
| Space overhead            | ~3.5%                  | ~3.8%         | 25%       |
| Rank performance          | Excellent              | Best for ≥8Gb | Excellent |
| Select performance        | Very good              | Best          | Moderate  |
| Implementation complexity | Medium                 | High          | Low       |
| Cross-platform            | Yes                    | Yes           | Yes       |
| No PDEP/PEXT required     | Yes                    | Yes           | Yes       |

**Rationale**: Poppy is well-documented, proven in production (sdsl-lite), and provides excellent performance without PDEP/PEXT. SPIDER is newer and faster for very large datasets, but more complex. We can add SPIDER optimizations later.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        Public API                                │
│  RankSelect trait + BitVec struct                               │
└─────────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────────────────────────────────────────┐
│                    Algorithm Layer                               │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │   Poppy     │  │  Sampled    │  │  Broadword Select       │  │
│  │   Rank      │  │  Select     │  │  (select-in-word)       │  │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────────────────────────────────────────┐
│                    Primitives Layer                              │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │  popcount   │  │  select_in  │  │  SELECT_IN_BYTE_TABLE   │  │
│  │  (scalar)   │  │  _word      │  │  (2KB lookup table)     │  │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────────────────────────────────────────┐
│              Platform-Specific Acceleration (optional)          │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │ x86 AVX2    │  │ ARM NEON    │  │ ARM SVE (future)        │  │
│  │ popcount    │  │ popcount    │  │ popcount + BDEP/BEXT    │  │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

## Phase 1: Core Data Structures

### 1.1 Bit Vector Storage

```rust
/// Raw bit storage as 64-bit words
pub struct BitVec {
    words: Vec<u64>,
    len: usize,  // Number of valid bits
}
```

### 1.2 Poppy Rank Directory

Three-level structure with ~3% space overhead:

```
Level 0 (L0): Superblocks - cumulative rank every 2^32 bits
Level 1 (L1): Blocks      - cumulative rank every 512 bits (relative to L0)
Level 2 (L2): Subblocks   - cumulative rank every 64 bits (relative to L1, packed)
```

```rust
/// Poppy rank directory
pub struct RankDirectory {
    /// L0: Absolute cumulative rank every 2^32 bits
    /// Only needed for bitvectors > 4 billion bits
    l0: Vec<u64>,

    /// L1+L2 interleaved: One L1 entry + 7 L2 entries per 512 bits
    /// L1: 32-bit cumulative rank (relative to L0)
    /// L2: 7 x 9-bit subblock ranks (packed into remaining 64 bits)
    /// Total: 128 bits per 512-bit block = 25% overhead before packing
    /// With packing: ~3% overhead
    l1_l2: Vec<u128>,
}
```

**Memory layout** (per 512-bit block):
```
┌────────────────────────────────────────────────────────────────┐
│ L1 (32 bits)  │ L2[0] │ L2[1] │ L2[2] │ L2[3] │ L2[4] │ L2[5] │ L2[6] │
│ cumulative    │ 9-bit │ 9-bit │ 9-bit │ 9-bit │ 9-bit │ 9-bit │ 9-bit │
│ rank          │ offset│ offset│ offset│ offset│ offset│ offset│ offset│
└────────────────────────────────────────────────────────────────┘
                                  128 bits total
```

### 1.3 Sampled Select Index

```rust
/// Select acceleration structure
pub struct SelectIndex {
    /// Position of every SAMPLE_RATE-th 1-bit
    samples1: Vec<u64>,
    /// Position of every SAMPLE_RATE-th 0-bit (optional)
    samples0: Option<Vec<u64>>,
    /// Sample rate (default: 256)
    sample_rate: u32,
}
```

## Phase 2: Primitive Operations

### 2.1 Popcount

```rust
// Cross-platform: uses hardware instruction on all modern CPUs
#[inline]
fn popcount(x: u64) -> u32 {
    x.count_ones()
}

// Block popcount (512 bits = 8 words)
#[inline]
fn popcount_block(words: &[u64; 8]) -> u32 {
    words.iter().map(|w| w.count_ones()).sum()
}
```

### 2.2 Select-in-Word (Broadword/SWAR)

```rust
const L8: u64 = 0x0101_0101_0101_0101;
const H8: u64 = 0x8080_8080_8080_8080;

/// Select the k-th set bit (0-indexed) in a 64-bit word
#[inline]
fn select_in_word(x: u64, k: u32) -> u32 {
    // SWAR byte popcounts
    let byte_sums = {
        let x = x - ((x >> 1) & 0x5555_5555_5555_5555);
        let x = (x & 0x3333_3333_3333_3333) + ((x >> 2) & 0x3333_3333_3333_3333);
        (x + (x >> 4)) & 0x0F0F_0F0F_0F0F_0F0F
    };

    // Prefix sums via multiplication
    let prefix = byte_sums.wrapping_mul(L8);

    // Find target byte
    let k_broadcast = (k as u64 + 1) * L8;
    let ge_mask = ((prefix | H8) - k_broadcast) & H8;
    let byte_idx = (ge_mask.wrapping_mul(L8) >> 56) as u32;

    // Select within byte using lookup table
    let byte_offset = byte_idx * 8;
    let target_byte = ((x >> byte_offset) & 0xFF) as u8;
    let prefix_before = if byte_idx == 0 {
        0
    } else {
        ((prefix >> ((byte_idx - 1) * 8)) & 0xFF) as u32
    };

    byte_offset + SELECT_IN_BYTE_TABLE[(target_byte as usize) * 8 + (k - prefix_before) as usize]
}
```

### 2.3 Select-in-Byte Lookup Table

```rust
/// Precomputed: position of k-th set bit in each byte value
/// Table size: 256 * 8 = 2KB
static SELECT_IN_BYTE_TABLE: [u8; 2048] = /* generated at build time */;
```

## Phase 3: Rank Implementation

```rust
impl BitVec {
    /// Count 1-bits in positions [0, i)
    pub fn rank1(&self, i: usize) -> usize {
        if i == 0 { return 0; }
        if i >= self.len { return self.count_ones(); }

        let word_idx = i / 64;
        let bit_idx = i % 64;

        // L0 contribution (for very large bitvectors)
        let l0_idx = word_idx / (1 << 26);  // 2^32 bits = 2^26 words
        let l0_rank = if l0_idx > 0 { self.rank_dir.l0[l0_idx - 1] } else { 0 };

        // L1+L2 contribution
        let block_idx = word_idx / 8;  // 512 bits = 8 words per block
        let l1_l2 = self.rank_dir.l1_l2[block_idx];

        let l1_rank = (l1_l2 & 0xFFFF_FFFF) as u64;  // Lower 32 bits

        // L2: which subblock within the 512-bit block?
        let subblock_idx = word_idx % 8;
        let l2_rank = if subblock_idx == 0 {
            0
        } else {
            // Extract 9-bit L2 entry
            let shift = 32 + (subblock_idx - 1) * 9;
            ((l1_l2 >> shift) & 0x1FF) as u64
        };

        // Count remaining bits in the partial word
        let word = self.words[word_idx];
        let mask = (1u64 << bit_idx) - 1;  // Bits [0, bit_idx)
        let partial_rank = (word & mask).count_ones() as u64;

        (l0_rank + l1_rank + l2_rank + partial_rank) as usize
    }
}
```

## Phase 4: Select Implementation

```rust
impl BitVec {
    /// Find position of the k-th 1-bit (0-indexed)
    pub fn select1(&self, k: usize) -> Option<usize> {
        let total_ones = self.count_ones();
        if k >= total_ones { return None; }

        // Jump using samples
        let sample_idx = k / self.select_idx.sample_rate as usize;
        let (mut pos, mut remaining) = if sample_idx == 0 {
            (0, k)
        } else {
            let sample_pos = self.select_idx.samples1[sample_idx - 1] as usize;
            (sample_pos, k - sample_idx * self.select_idx.sample_rate as usize)
        };

        // Scan blocks using rank directory
        let mut block_idx = pos / 512;
        while remaining > 0 {
            let block_pop = self.block_popcount(block_idx);
            if block_pop <= remaining {
                remaining -= block_pop;
                block_idx += 1;
                pos = block_idx * 512;
            } else {
                break;
            }
        }

        // Scan words within block
        let start_word = pos / 64;
        for word_idx in start_word..(start_word + 8).min(self.words.len()) {
            let word = self.words[word_idx];
            let pop = word.count_ones() as usize;
            if pop > remaining {
                return Some(word_idx * 64 + select_in_word(word, remaining as u32) as usize);
            }
            remaining -= pop;
        }

        None
    }
}
```

## Phase 5: Platform-Specific Optimization (Optional)

### 5.1 NEON Popcount for Large Blocks

```rust
#[cfg(target_arch = "aarch64")]
mod neon {
    use core::arch::aarch64::*;

    #[target_feature(enable = "neon")]
    pub unsafe fn popcount_512(ptr: *const u8) -> u32 {
        let v0 = vld1q_u8(ptr);
        let v1 = vld1q_u8(ptr.add(16));
        let v2 = vld1q_u8(ptr.add(32));
        let v3 = vld1q_u8(ptr.add(48));

        let c0 = vcntq_u8(v0);
        let c1 = vcntq_u8(v1);
        let c2 = vcntq_u8(v2);
        let c3 = vcntq_u8(v3);

        let sum = vaddq_u8(vaddq_u8(c0, c1), vaddq_u8(c2, c3));
        vaddvq_u8(sum) as u32
    }
}
```

### 5.2 x86 AVX2 Popcount

```rust
#[cfg(target_arch = "x86_64")]
mod avx2 {
    use core::arch::x86_64::*;

    #[target_feature(enable = "avx2", enable = "popcnt")]
    pub unsafe fn popcount_512(ptr: *const u64) -> u32 {
        let mut sum = 0u32;
        for i in 0..8 {
            sum += _popcnt64(*ptr.add(i)) as u32;
        }
        sum
    }
}
```

## Phase 6: Public API

```rust
/// Trait for rank/select operations
pub trait RankSelect {
    fn rank1(&self, i: usize) -> usize;
    fn rank0(&self, i: usize) -> usize { i - self.rank1(i) }
    fn select1(&self, k: usize) -> Option<usize>;
    fn select0(&self, k: usize) -> Option<usize>;
}

/// Configuration for building indices
#[derive(Clone, Debug)]
pub struct Config {
    /// Sample rate for select acceleration (default: 256)
    pub select_sample_rate: u32,
    /// Whether to build select0 index (default: false)
    pub build_select0: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            select_sample_rate: 256,
            build_select0: false,
        }
    }
}

/// Bitvector with rank/select support
pub struct BitVec {
    words: Vec<u64>,
    len: usize,
    rank_dir: RankDirectory,
    select_idx: SelectIndex,
}

impl BitVec {
    /// Create from raw bits
    pub fn new(bits: impl IntoIterator<Item = bool>) -> Self;

    /// Create from u64 words
    pub fn from_words(words: Vec<u64>, len: usize) -> Self;

    /// Create with custom configuration
    pub fn with_config(words: Vec<u64>, len: usize, config: Config) -> Self;

    /// Number of bits
    pub fn len(&self) -> usize;

    /// Total number of 1-bits
    pub fn count_ones(&self) -> usize;

    /// Access bit at position i
    pub fn get(&self, i: usize) -> bool;
}

impl RankSelect for BitVec { /* ... */ }
```

## Implementation Order

### Milestone 1: Minimal Working Implementation
1. [ ] `BitVec` storage with `from_words()` and `get()`
2. [ ] `popcount()` using `u64::count_ones()`
3. [ ] `SELECT_IN_BYTE_TABLE` generation (build script or const fn)
4. [ ] `select_in_word()` broadword implementation
5. [ ] Basic `RankDirectory` (L1+L2 only, no L0)
6. [ ] `rank1()` implementation
7. [ ] Unit tests for rank (see Test Plan below)

### Milestone 2: Select Support
8. [ ] `SelectIndex` with sampling
9. [ ] `select1()` implementation
10. [ ] Unit tests for select
11. [ ] Property tests: rank/select invariants

### Milestone 3: Completeness
12. [ ] L0 support for bitvectors > 4 billion bits
13. [ ] `rank0()` and `select0()`
14. [ ] Edge case handling (empty, all-zeros, all-ones)
15. [ ] `Config` for customization

### Milestone 4: Performance
16. [ ] Criterion benchmarks
17. [ ] NEON popcount (optional feature)
18. [ ] AVX2 popcount (optional feature)
19. [ ] Profile and optimize hot paths

### Milestone 5: Polish
20. [ ] Documentation with examples
21. [ ] README with usage guide
22. [ ] CI for x86_64 and aarch64
23. [ ] Publish to crates.io

## Test Plan

### Unit Tests

#### BitVec Basic Operations (`tests/bitvec_tests.rs`)

```rust
#[cfg(test)]
mod bitvec_tests {
    use super::*;

    #[test]
    fn test_from_words_empty() {
        let bv = BitVec::from_words(vec![], 0);
        assert_eq!(bv.len(), 0);
        assert_eq!(bv.count_ones(), 0);
    }

    #[test]
    fn test_from_words_single() {
        let bv = BitVec::from_words(vec![0b1010_1010], 8);
        assert_eq!(bv.len(), 8);
        assert_eq!(bv.get(0), false);
        assert_eq!(bv.get(1), true);
        assert_eq!(bv.get(7), true);
    }

    #[test]
    fn test_get_out_of_bounds() {
        let bv = BitVec::from_words(vec![0xFF], 8);
        // Should panic or return false depending on API choice
    }

    #[test]
    fn test_count_ones_all_zeros() {
        let bv = BitVec::from_words(vec![0, 0, 0], 192);
        assert_eq!(bv.count_ones(), 0);
    }

    #[test]
    fn test_count_ones_all_ones() {
        let bv = BitVec::from_words(vec![u64::MAX, u64::MAX], 128);
        assert_eq!(bv.count_ones(), 128);
    }

    #[test]
    fn test_partial_word() {
        // Only 10 bits valid in a 64-bit word
        let bv = BitVec::from_words(vec![0b11111_11111], 10);
        assert_eq!(bv.len(), 10);
        assert_eq!(bv.count_ones(), 10);
    }
}
```

#### Rank Tests (`tests/rank_tests.rs`)

```rust
#[cfg(test)]
mod rank_tests {
    use super::*;

    #[test]
    fn test_rank1_empty() {
        let bv = BitVec::from_words(vec![], 0);
        assert_eq!(bv.rank1(0), 0);
    }

    #[test]
    fn test_rank1_at_zero() {
        let bv = BitVec::from_words(vec![0b1111], 4);
        assert_eq!(bv.rank1(0), 0);  // rank1(0) counts bits in [0, 0) = empty
    }

    #[test]
    fn test_rank1_simple() {
        // Bits: 1 0 1 1 0 0 1 0
        let bv = BitVec::from_words(vec![0b0100_1101], 8);
        assert_eq!(bv.rank1(1), 1);  // [0,1) = bit 0 = 1
        assert_eq!(bv.rank1(2), 1);  // [0,2) = bits 0,1 = 1,0 = 1
        assert_eq!(bv.rank1(4), 3);  // [0,4) = bits 0,1,2,3 = 1,0,1,1 = 3
        assert_eq!(bv.rank1(8), 4);  // all 8 bits
    }

    #[test]
    fn test_rank1_word_boundary() {
        let bv = BitVec::from_words(vec![u64::MAX, u64::MAX], 128);
        assert_eq!(bv.rank1(64), 64);
        assert_eq!(bv.rank1(65), 65);
        assert_eq!(bv.rank1(128), 128);
    }

    #[test]
    fn test_rank1_block_boundary() {
        // 512 bits = 8 words per block
        let words: Vec<u64> = (0..16).map(|_| u64::MAX).collect();
        let bv = BitVec::from_words(words, 1024);
        assert_eq!(bv.rank1(512), 512);
        assert_eq!(bv.rank1(513), 513);
    }

    #[test]
    fn test_rank1_sparse() {
        // Single bit set at position 500
        let mut words = vec![0u64; 16];
        words[7] = 1 << 52;  // Position 7*64 + 52 = 500
        let bv = BitVec::from_words(words, 1024);
        assert_eq!(bv.rank1(500), 0);
        assert_eq!(bv.rank1(501), 1);
        assert_eq!(bv.rank1(1024), 1);
    }

    #[test]
    fn test_rank0_simple() {
        let bv = BitVec::from_words(vec![0b0100_1101], 8);
        assert_eq!(bv.rank0(4), 1);  // 4 - rank1(4) = 4 - 3 = 1
        assert_eq!(bv.rank0(8), 4);  // 8 - rank1(8) = 8 - 4 = 4
    }
}
```

#### Select Tests (`tests/select_tests.rs`)

```rust
#[cfg(test)]
mod select_tests {
    use super::*;

    #[test]
    fn test_select1_empty() {
        let bv = BitVec::from_words(vec![], 0);
        assert_eq!(bv.select1(0), None);
    }

    #[test]
    fn test_select1_all_zeros() {
        let bv = BitVec::from_words(vec![0, 0], 128);
        assert_eq!(bv.select1(0), None);
    }

    #[test]
    fn test_select1_simple() {
        // Bits: 1 0 1 1 0 0 1 0 (positions 0,2,3,6 have 1s)
        let bv = BitVec::from_words(vec![0b0100_1101], 8);
        assert_eq!(bv.select1(0), Some(0));  // 0th one at position 0
        assert_eq!(bv.select1(1), Some(2));  // 1st one at position 2
        assert_eq!(bv.select1(2), Some(3));  // 2nd one at position 3
        assert_eq!(bv.select1(3), Some(6));  // 3rd one at position 6
        assert_eq!(bv.select1(4), None);     // No 4th one
    }

    #[test]
    fn test_select1_word_boundary() {
        let bv = BitVec::from_words(vec![u64::MAX, u64::MAX], 128);
        assert_eq!(bv.select1(63), Some(63));
        assert_eq!(bv.select1(64), Some(64));
        assert_eq!(bv.select1(127), Some(127));
    }

    #[test]
    fn test_select1_across_blocks() {
        // 512-bit blocks, query across boundary
        let words: Vec<u64> = (0..16).map(|_| u64::MAX).collect();
        let bv = BitVec::from_words(words, 1024);
        assert_eq!(bv.select1(511), Some(511));
        assert_eq!(bv.select1(512), Some(512));
    }

    #[test]
    fn test_select0_simple() {
        // Bits: 1 0 1 1 0 0 1 0 (positions 1,4,5,7 have 0s)
        let bv = BitVec::from_words(vec![0b0100_1101], 8);
        assert_eq!(bv.select0(0), Some(1));  // 0th zero at position 1
        assert_eq!(bv.select0(1), Some(4));  // 1st zero at position 4
        assert_eq!(bv.select0(3), Some(7));  // 3rd zero at position 7
        assert_eq!(bv.select0(4), None);     // No 4th zero
    }
}
```

#### Select-in-Word Tests (`tests/broadword_tests.rs`)

```rust
#[cfg(test)]
mod broadword_tests {
    use super::*;

    #[test]
    fn test_select_in_word_first_bit() {
        assert_eq!(select_in_word(0b1, 0), 0);
        assert_eq!(select_in_word(0b10, 0), 1);
        assert_eq!(select_in_word(0b100, 0), 2);
    }

    #[test]
    fn test_select_in_word_multiple() {
        let word = 0b1010_1010u64;
        assert_eq!(select_in_word(word, 0), 1);
        assert_eq!(select_in_word(word, 1), 3);
        assert_eq!(select_in_word(word, 2), 5);
        assert_eq!(select_in_word(word, 3), 7);
    }

    #[test]
    fn test_select_in_word_all_ones() {
        let word = u64::MAX;
        for k in 0..64 {
            assert_eq!(select_in_word(word, k), k);
        }
    }

    #[test]
    fn test_select_in_word_high_bits() {
        let word = 1u64 << 63;
        assert_eq!(select_in_word(word, 0), 63);
    }

    #[test]
    fn test_select_in_word_not_found() {
        assert_eq!(select_in_word(0, 0), 64);  // No bits set
        assert_eq!(select_in_word(0b1, 1), 64);  // Only 1 bit, asking for 2nd
    }

    #[test]
    fn test_select_in_byte_table() {
        // Verify table correctness for all byte values
        for byte in 0u8..=255 {
            let pop = byte.count_ones();
            for k in 0..pop {
                let pos = SELECT_IN_BYTE_TABLE[(byte as usize) * 8 + k as usize];
                // Verify the k-th bit is actually set at position pos
                assert!((byte >> pos) & 1 == 1,
                    "byte={:08b}, k={}, pos={}", byte, k, pos);
            }
        }
    }
}
```

### Property-Based Tests (`tests/properties.rs`)

```rust
use proptest::prelude::*;

proptest! {
    /// rank1(select1(k)) == k for valid k
    #[test]
    fn prop_rank_of_select(
        words in prop::collection::vec(any::<u64>(), 1..100),
        k_ratio in 0.0..1.0f64
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        let ones = bv.count_ones();

        if ones > 0 {
            let k = ((k_ratio * ones as f64) as usize).min(ones - 1);
            if let Some(pos) = bv.select1(k) {
                prop_assert_eq!(bv.rank1(pos + 1) - 1, k,
                    "rank1(select1({}) + 1) - 1 should equal {}", k, k);
            }
        }
    }

    /// select1(rank1(i)) == i when bit i is set
    #[test]
    fn prop_select_of_rank(
        words in prop::collection::vec(any::<u64>(), 1..100),
        i_ratio in 0.0..1.0f64
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words.clone(), len);
        let i = (i_ratio * len as f64) as usize;

        if i < len && bv.get(i) {
            let rank = bv.rank1(i);
            prop_assert_eq!(bv.select1(rank), Some(i),
                "select1(rank1({})) should equal {}", i, i);
        }
    }

    /// rank1(n) + rank0(n) == n
    #[test]
    fn prop_rank_sum(
        words in prop::collection::vec(any::<u64>(), 1..100),
        i_ratio in 0.0..1.0f64
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        let i = (i_ratio * len as f64) as usize;

        if i <= len {
            prop_assert_eq!(bv.rank1(i) + bv.rank0(i), i);
        }
    }

    /// rank1 is monotonically increasing
    #[test]
    fn prop_rank_monotonic(
        words in prop::collection::vec(any::<u64>(), 1..50),
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);

        let mut prev_rank = 0;
        for i in 0..=len {
            let rank = bv.rank1(i);
            prop_assert!(rank >= prev_rank,
                "rank1({}) = {} < rank1({}) = {}", i, rank, i-1, prev_rank);
            prev_assert!(rank <= prev_rank + 1,
                "rank1 jumped by more than 1");
            prev_rank = rank;
        }
    }

    /// select1 returns strictly increasing positions
    #[test]
    fn prop_select_monotonic(
        words in prop::collection::vec(any::<u64>(), 1..50),
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        let ones = bv.count_ones();

        let mut prev_pos = None;
        for k in 0..ones {
            let pos = bv.select1(k);
            if let (Some(prev), Some(curr)) = (prev_pos, pos) {
                prop_assert!(curr > prev,
                    "select1({}) = {} <= select1({}) = {}", k, curr, k-1, prev);
            }
            prev_pos = pos;
        }
    }
}
```

### Comparison Tests (`tests/reference_tests.rs`)

```rust
/// Simple O(n) reference implementation for validation
fn reference_rank1(words: &[u64], len: usize, i: usize) -> usize {
    let mut count = 0;
    for bit_pos in 0..i.min(len) {
        let word_idx = bit_pos / 64;
        let bit_idx = bit_pos % 64;
        if (words[word_idx] >> bit_idx) & 1 == 1 {
            count += 1;
        }
    }
    count
}

fn reference_select1(words: &[u64], len: usize, k: usize) -> Option<usize> {
    let mut count = 0;
    for bit_pos in 0..len {
        let word_idx = bit_pos / 64;
        let bit_idx = bit_pos % 64;
        if (words[word_idx] >> bit_idx) & 1 == 1 {
            if count == k {
                return Some(bit_pos);
            }
            count += 1;
        }
    }
    None
}

proptest! {
    #[test]
    fn prop_rank_matches_reference(
        words in prop::collection::vec(any::<u64>(), 1..100),
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words.clone(), len);

        // Test at various positions
        for i in (0..=len).step_by(7) {
            let expected = reference_rank1(&words, len, i);
            let actual = bv.rank1(i);
            prop_assert_eq!(actual, expected, "rank1({}) mismatch", i);
        }
    }

    #[test]
    fn prop_select_matches_reference(
        words in prop::collection::vec(any::<u64>(), 1..100),
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words.clone(), len);
        let ones = bv.count_ones();

        // Test at various k values
        for k in (0..ones).step_by(7) {
            let expected = reference_select1(&words, len, k);
            let actual = bv.select1(k);
            prop_assert_eq!(actual, expected, "select1({}) mismatch", k);
        }
    }
}
```

### Edge Case Tests (`tests/edge_cases.rs`)

```rust
#[cfg(test)]
mod edge_cases {
    use super::*;

    #[test]
    fn test_single_bit_set() {
        for pos in [0, 1, 62, 63, 64, 127, 511, 512] {
            let word_idx = pos / 64;
            let bit_idx = pos % 64;
            let mut words = vec![0u64; word_idx + 1];
            words[word_idx] = 1 << bit_idx;
            let len = (word_idx + 1) * 64;

            let bv = BitVec::from_words(words, len);

            assert_eq!(bv.count_ones(), 1, "pos={}", pos);
            assert_eq!(bv.select1(0), Some(pos), "pos={}", pos);
            assert_eq!(bv.rank1(pos), 0, "pos={}", pos);
            assert_eq!(bv.rank1(pos + 1), 1, "pos={}", pos);
        }
    }

    #[test]
    fn test_alternating_bits() {
        let word = 0xAAAA_AAAA_AAAA_AAAAu64;  // 1010...
        let bv = BitVec::from_words(vec![word], 64);

        assert_eq!(bv.count_ones(), 32);
        assert_eq!(bv.select1(0), Some(1));
        assert_eq!(bv.select1(31), Some(63));
    }

    #[test]
    fn test_very_sparse() {
        // 1 bit set in 10000 bits
        let mut words = vec![0u64; 157];  // 157 * 64 = 10048 bits
        words[78] = 1 << 32;  // Position 78*64 + 32 = 5024
        let bv = BitVec::from_words(words, 10000);

        assert_eq!(bv.count_ones(), 1);
        assert_eq!(bv.select1(0), Some(5024));
        assert_eq!(bv.rank1(5024), 0);
        assert_eq!(bv.rank1(5025), 1);
    }

    #[test]
    fn test_very_dense() {
        // Only 1 bit NOT set in 10000 bits
        let mut words: Vec<u64> = vec![u64::MAX; 157];
        words[78] &= !(1 << 32);  // Clear position 5024
        let bv = BitVec::from_words(words, 10000);

        assert_eq!(bv.count_ones(), 9999);
        assert_eq!(bv.select0(0), Some(5024));
    }

    #[test]
    fn test_exact_block_boundary() {
        // Exactly 512 bits (one block)
        let words = vec![u64::MAX; 8];
        let bv = BitVec::from_words(words, 512);

        assert_eq!(bv.rank1(512), 512);
        assert_eq!(bv.select1(511), Some(511));
    }

    #[test]
    fn test_just_past_block_boundary() {
        // 513 bits (one block + 1 bit)
        let mut words = vec![u64::MAX; 9];
        words[8] = 1;
        let bv = BitVec::from_words(words, 513);

        assert_eq!(bv.rank1(513), 513);
        assert_eq!(bv.select1(512), Some(512));
    }

    #[test]
    fn test_max_l2_value() {
        // L2 entries are 9 bits, max value 511
        // This happens at the 7th subblock of an all-ones block
        let words = vec![u64::MAX; 8];
        let bv = BitVec::from_words(words, 512);

        // Verify internal L2 values don't overflow
        // (This tests the directory building, not the query)
        assert_eq!(bv.rank1(448), 448);  // 7 * 64 = 448
    }
}
```

### Benchmark Tests (`benches/rank_select.rs`)

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

fn generate_bitvec(size: usize, density: f64, seed: u64) -> BitVec {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut words = Vec::with_capacity((size + 63) / 64);

    for _ in 0..(size + 63) / 64 {
        let mut word = 0u64;
        for bit in 0..64 {
            if rng.gen::<f64>() < density {
                word |= 1 << bit;
            }
        }
        words.push(word);
    }

    BitVec::from_words(words, size)
}

fn generate_queries(count: usize, max: usize, seed: u64) -> Vec<usize> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..count).map(|_| rng.gen_range(0..max)).collect()
}

fn bench_rank(c: &mut Criterion) {
    let mut group = c.benchmark_group("rank1");

    for size in [1_000_000, 10_000_000, 100_000_000] {
        for density in [0.01, 0.1, 0.5, 0.9] {
            let bv = generate_bitvec(size, density, 42);
            let queries = generate_queries(10000, size, 123);

            group.bench_with_input(
                BenchmarkId::new(
                    format!("{:.0}M/{:.0}%", size as f64 / 1e6, density * 100.0),
                    ""
                ),
                &(&bv, &queries),
                |b, (bv, queries)| {
                    b.iter(|| {
                        let mut sum = 0usize;
                        for &q in queries.iter() {
                            sum += bv.rank1(black_box(q));
                        }
                        sum
                    })
                },
            );
        }
    }
    group.finish();
}

fn bench_select(c: &mut Criterion) {
    let mut group = c.benchmark_group("select1");

    for size in [1_000_000, 10_000_000, 100_000_000] {
        for density in [0.01, 0.1, 0.5, 0.9] {
            let bv = generate_bitvec(size, density, 42);
            let ones = bv.count_ones();
            let queries = generate_queries(10000, ones, 123);

            group.bench_with_input(
                BenchmarkId::new(
                    format!("{:.0}M/{:.0}%", size as f64 / 1e6, density * 100.0),
                    ""
                ),
                &(&bv, &queries),
                |b, (bv, queries)| {
                    b.iter(|| {
                        let mut sum = 0usize;
                        for &q in queries.iter() {
                            if let Some(pos) = bv.select1(black_box(q)) {
                                sum += pos;
                            }
                        }
                        sum
                    })
                },
            );
        }
    }
    group.finish();
}

fn bench_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("construction");

    for size in [1_000_000, 10_000_000] {
        let words: Vec<u64> = (0..(size + 63) / 64)
            .map(|i| i as u64 * 0x1234_5678_9ABC_DEF0)
            .collect();

        group.bench_with_input(
            BenchmarkId::new(format!("{:.0}M", size as f64 / 1e6), ""),
            &words,
            |b, words| {
                b.iter(|| {
                    BitVec::from_words(black_box(words.clone()), size)
                })
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_rank, bench_select, bench_construction);
criterion_main!(benches);
```

### CI Configuration (`.github/workflows/ci.yml`)

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

env:
  CARGO_TERM_COLOR: always

jobs:
  test:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
          - os: macos-latest
            target: aarch64-apple-darwin

    runs-on: ${{ matrix.os }}

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-action@stable
        with:
          targets: ${{ matrix.target }}

      - name: Build
        run: cargo build --verbose

      - name: Run unit tests
        run: cargo test --verbose

      - name: Run property tests
        run: cargo test --test properties --verbose

      - name: Run with all features
        run: cargo test --all-features --verbose

  bench:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-action@stable

      - name: Run benchmarks (smoke test)
        run: cargo bench --bench rank_select -- --quick

  miri:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-action@nightly
        with:
          components: miri

      - name: Run Miri
        run: cargo +nightly miri test --lib

  coverage:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-action@stable
        with:
          components: llvm-tools-preview

      - name: Install cargo-llvm-cov
        run: cargo install cargo-llvm-cov

      - name: Generate coverage
        run: cargo llvm-cov --all-features --lcov --output-path lcov.info

      - name: Upload coverage
        uses: codecov/codecov-action@v3
        with:
          files: lcov.info
```

### Test Summary

| Category        | Count | Purpose                            |
|-----------------|-------|------------------------------------|
| Unit tests      | ~40   | Individual function correctness    |
| Property tests  | ~10   | Invariants hold for random inputs  |
| Reference tests | ~5    | Match simple O(n) implementation   |
| Edge case tests | ~15   | Boundary conditions, extremes      |
| Benchmark tests | ~12   | Performance across sizes/densities |

**Total: ~80+ tests**

### Large Bitvector Testing Strategy (1G+ bits)

Testing with 1-billion-bit vectors requires special consideration for memory, time, and CI constraints.

#### Challenges

| Challenge            | Impact                                      | Mitigation                         |
|----------------------|---------------------------------------------|------------------------------------|
| Memory (125MB+ data) | CI runners may OOM; local dev machines vary | Feature-gate, separate test binary |
| Construction time    | Building index takes seconds                | Run as benchmark, not unit test    |
| Reference comparison | O(n) reference is too slow                  | Spot-check with sampling           |
| CI timeout           | Full test suite too slow                    | Separate workflow, nightly runs    |

#### Test Categories for Large Vectors

```rust
// tests/large_vectors.rs - gated behind feature flag
#![cfg(feature = "large-tests")]

use succinctly::BitVec;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

const ONE_BILLION: usize = 1_000_000_000;
const SAMPLE_QUERIES: usize = 10_000;

/// Generate a large bitvector with controlled density
fn generate_large_bitvec(size: usize, density: f64, seed: u64) -> BitVec {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let word_count = (size + 63) / 64;
    let mut words = Vec::with_capacity(word_count);

    // Use threshold comparison for speed (vs per-bit random)
    let threshold = (density * u64::MAX as f64) as u64;
    for _ in 0..word_count {
        let mut word = 0u64;
        for bit in 0..64 {
            if rng.gen::<u64>() < threshold {
                word |= 1 << bit;
            }
        }
        words.push(word);
    }

    BitVec::from_words(words, size)
}

#[test]
fn test_1g_construction() {
    let bv = generate_large_bitvec(ONE_BILLION, 0.5, 42);
    assert_eq!(bv.len(), ONE_BILLION);
    // Sanity check: ~500M ones expected
    let ones = bv.count_ones();
    assert!(ones > 490_000_000 && ones < 510_000_000,
        "Expected ~50% density, got {}", ones as f64 / ONE_BILLION as f64);
}

#[test]
fn test_1g_rank_consistency() {
    let bv = generate_large_bitvec(ONE_BILLION, 0.5, 42);
    let mut rng = ChaCha8Rng::seed_from_u64(123);

    // Spot-check rank at random positions
    for _ in 0..SAMPLE_QUERIES {
        let i = rng.gen_range(0..ONE_BILLION);
        let rank = bv.rank1(i);

        // Verify monotonicity locally
        if i > 0 {
            let prev_rank = bv.rank1(i - 1);
            assert!(rank >= prev_rank && rank <= prev_rank + 1,
                "rank1({}) = {}, rank1({}) = {}", i, rank, i-1, prev_rank);
        }

        // Verify rank + rank0 = i
        assert_eq!(bv.rank1(i) + bv.rank0(i), i);
    }
}

#[test]
fn test_1g_select_consistency() {
    let bv = generate_large_bitvec(ONE_BILLION, 0.5, 42);
    let ones = bv.count_ones();
    let mut rng = ChaCha8Rng::seed_from_u64(456);

    for _ in 0..SAMPLE_QUERIES {
        let k = rng.gen_range(0..ones);
        if let Some(pos) = bv.select1(k) {
            // Verify select/rank round-trip
            assert_eq!(bv.rank1(pos + 1) - 1, k,
                "rank1(select1({}) + 1) - 1 != {}", k, k);

            // Verify bit at position is set
            assert!(bv.get(pos), "select1({}) = {} but bit not set", k, pos);
        }
    }
}

#[test]
fn test_1g_boundary_conditions() {
    let bv = generate_large_bitvec(ONE_BILLION, 0.5, 42);

    // First and last positions
    assert_eq!(bv.rank1(0), 0);
    let total = bv.rank1(ONE_BILLION);
    assert_eq!(total, bv.count_ones());

    // L0 superblock boundaries (every 2^32 bits)
    // For 1G bits, we have positions near 0 only (no L0 boundary)
    // Test would be different for 5G+ vectors

    // Block boundaries (every 512 bits)
    for block in [0, 1, 100, 1000, 1_000_000] {
        let pos = block * 512;
        if pos < ONE_BILLION {
            let _ = bv.rank1(pos);
            let _ = bv.rank1(pos + 1);
        }
    }
}

#[test]
fn test_1g_sparse() {
    // 0.1% density = ~1M ones in 1G bits
    let bv = generate_large_bitvec(ONE_BILLION, 0.001, 42);
    let ones = bv.count_ones();

    assert!(ones > 900_000 && ones < 1_100_000,
        "Expected ~1M ones, got {}", ones);

    // Select should still work efficiently
    if let Some(pos) = bv.select1(ones / 2) {
        assert!(bv.get(pos));
    }
}

#[test]
fn test_1g_dense() {
    // 99.9% density = ~1M zeros in 1G bits
    let bv = generate_large_bitvec(ONE_BILLION, 0.999, 42);
    let zeros = ONE_BILLION - bv.count_ones();

    assert!(zeros > 900_000 && zeros < 1_100_000,
        "Expected ~1M zeros, got {}", zeros);

    // Select0 should work efficiently
    if let Some(pos) = bv.select0(zeros / 2) {
        assert!(!bv.get(pos));
    }
}
```

#### L0 Superblock Testing (4G+ bits)

The L0 level activates for bitvectors > 2^32 bits. Testing requires ~500MB+ RAM:

```rust
#![cfg(feature = "huge-tests")]

const FIVE_BILLION: usize = 5_000_000_000;

#[test]
fn test_5g_l0_boundary() {
    // This test requires ~625MB for data + ~20MB for indices
    let bv = generate_large_bitvec(FIVE_BILLION, 0.5, 42);

    // L0 boundary at 2^32 = 4,294,967,296
    let l0_boundary = 1usize << 32;

    // Query around L0 boundary
    let rank_before = bv.rank1(l0_boundary - 1);
    let rank_at = bv.rank1(l0_boundary);
    let rank_after = bv.rank1(l0_boundary + 1);

    assert!(rank_at >= rank_before);
    assert!(rank_after >= rank_at);
    assert!(rank_after <= rank_at + 1);

    // Select across L0 boundary
    let k = rank_at;  // First one at or after L0 boundary
    if let Some(pos) = bv.select1(k) {
        assert!(pos >= l0_boundary - 64);  // Within one word of boundary
    }
}
```

#### Memory-Mapped Large Vector Tests

For vectors too large to fit in memory, test with memory-mapped files:

```rust
#![cfg(feature = "mmap-tests")]

use memmap2::MmapMut;
use std::fs::OpenOptions;
use tempfile::NamedTempFile;

#[test]
fn test_mmap_10g_bitvec() {
    const TEN_BILLION: usize = 10_000_000_000;
    const BYTES_NEEDED: usize = TEN_BILLION / 8;  // 1.25 GB

    let file = NamedTempFile::new().unwrap();
    file.as_file().set_len(BYTES_NEEDED as u64).unwrap();

    let mmap = unsafe {
        MmapMut::map_mut(file.as_file()).unwrap()
    };

    // Initialize with pattern (faster than random)
    let words: &mut [u64] = unsafe {
        std::slice::from_raw_parts_mut(
            mmap.as_ptr() as *mut u64,
            BYTES_NEEDED / 8
        )
    };
    for (i, word) in words.iter_mut().enumerate() {
        *word = i as u64 * 0x1234_5678_9ABC_DEF0;  // Deterministic pattern
    }

    // Build index over mmap'd data
    let bv = BitVec::from_mmap(mmap, TEN_BILLION);

    // Spot-check queries
    assert!(bv.rank1(TEN_BILLION / 2) > 0);
}
```

#### CI Configuration for Large Tests

```yaml
# .github/workflows/large-tests.yml
name: Large Vector Tests

on:
  schedule:
    - cron: '0 3 * * *'  # Nightly at 3 AM UTC
  workflow_dispatch:      # Manual trigger

jobs:
  large-tests:
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-action@stable

      - name: Run 1G tests
        run: cargo test --features large-tests --release -- --test-threads=1
        timeout-minutes: 30
        env:
          RUST_MIN_STACK: 8388608  # 8MB stack for deep recursion

  huge-tests:
    runs-on: ubuntu-latest
    # Only run on main branch to save resources
    if: github.ref == 'refs/heads/main'

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-action@stable

      - name: Run 5G tests (L0 boundary)
        run: cargo test --features huge-tests --release -- --test-threads=1
        timeout-minutes: 60
```

#### Cargo.toml Feature Flags

```toml
[features]
default = []
# ... existing features ...

# Test features (not for production)
large-tests = []   # 1G bitvector tests (~125MB RAM)
huge-tests = []    # 5G bitvector tests (~625MB RAM)
mmap-tests = ["memmap2", "tempfile"]  # Memory-mapped tests

[dev-dependencies]
# ... existing dev-dependencies ...
memmap2 = { version = "0.9", optional = true }
tempfile = { version = "3.10", optional = true }
```

#### Local Development Workflow

```bash
# Quick tests (default, no large vectors)
cargo test

# Include 1G tests (requires ~200MB free RAM)
cargo test --features large-tests --release -- --test-threads=1

# Include 5G tests (requires ~700MB free RAM, ~2 min)
cargo test --features huge-tests --release -- --test-threads=1

# Memory-mapped tests (uses disk, less RAM)
cargo test --features mmap-tests --release
```

#### Performance Assertions for Large Vectors

```rust
#[test]
#[cfg(feature = "large-tests")]
fn test_1g_rank_latency() {
    let bv = generate_large_bitvec(ONE_BILLION, 0.5, 42);
    let queries: Vec<usize> = (0..10_000)
        .map(|i| (i * 100_000) % ONE_BILLION)
        .collect();

    let start = std::time::Instant::now();
    let mut sum = 0usize;
    for &q in &queries {
        sum += bv.rank1(q);
    }
    let elapsed = start.elapsed();

    // Prevent optimization
    assert!(sum > 0);

    // Target: < 50ns per query average
    let ns_per_query = elapsed.as_nanos() / queries.len() as u128;
    assert!(ns_per_query < 100,
        "rank1 too slow on 1G vector: {}ns/query", ns_per_query);
}
```

#### Test Matrix Summary

| Test Category     | Size   | RAM Required | Run Time | CI Frequency |
|-------------------|--------|--------------|----------|--------------|
| Unit tests        | < 1K   | < 1 MB       | < 1s     | Every PR     |
| Property tests    | < 100K | < 10 MB      | < 10s    | Every PR     |
| Benchmark (smoke) | < 100M | < 50 MB      | < 30s    | Every PR     |
| Large tests       | 1G     | ~200 MB      | ~2 min   | Nightly      |
| Huge tests        | 5G     | ~700 MB      | ~5 min   | Nightly/main |
| Mmap tests        | 10G+   | < 100 MB     | ~10 min  | Weekly       |

## File Structure

```
src/
├── lib.rs              # Public API, re-exports
├── bitvec.rs           # BitVec struct and basic ops
├── rank.rs             # RankDirectory, rank1/rank0
├── select.rs           # SelectIndex, select1/select0
├── broadword.rs        # select_in_word, constants
├── table.rs            # SELECT_IN_BYTE_TABLE
└── simd/
    ├── mod.rs          # Feature detection, dispatch
    ├── scalar.rs       # Fallback implementations
    ├── neon.rs         # ARM NEON (cfg-gated)
    └── avx2.rs         # x86 AVX2 (cfg-gated)

benches/
└── rank_select.rs      # Criterion benchmarks

tests/
├── rank_tests.rs       # Unit tests for rank
├── select_tests.rs     # Unit tests for select
└── properties.rs       # Property-based tests
```

## Cargo.toml Features

```toml
[features]
default = []
# Enable SIMD acceleration (auto-detected at compile time)
simd = []
# Enable runtime SIMD detection (slight overhead)
runtime-dispatch = []
# Include select0 support (increases index size)
select0 = []

[dependencies]
# No required dependencies for core functionality

[dev-dependencies]
criterion = "0.5"
proptest = "1.0"
rand = "0.8"

[[bench]]
name = "rank_select"
harness = false
```

## Performance Targets

| Operation | Size | Target Latency | Notes                       |
|-----------|------|----------------|-----------------------------|
| rank1     | 10^6 | < 20 ns        | Single cache line access    |
| rank1     | 10^8 | < 30 ns        | L1+L2 lookup + partial word |
| select1   | 10^6 | < 50 ns        | Sample jump + short scan    |
| select1   | 10^8 | < 100 ns       | Sample jump + block scan    |

## Testing Strategy

1. **Unit tests**: Known values, edge cases
2. **Property tests**: `rank1(select1(k)) == k`, `select1(rank1(i)) == i`
3. **Fuzzing**: Random bitvectors, random queries
4. **Comparison**: Verify against simple O(n) reference implementation
5. **Cross-platform CI**: Test on x86_64-linux, aarch64-linux, aarch64-darwin

## Future Enhancements (Post-1.0)

- [ ] SPIDER-style metadata interleaving for rank
- [ ] Prediction-based select (SPIDER algorithm)
- [ ] RRR compressed representation
- [ ] Mutable bitvector with index maintenance
- [ ] Memory-mapped file support
- [ ] WASM support
- [ ] SVE2 BDEP/BEXT when hardware available
