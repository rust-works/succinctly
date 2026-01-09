//! Bitvector with rank/select support.
//!
//! This module provides `BitVec`, the main data structure for storing bits
//! with efficient rank and select operations.

#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::bits::popcount::{popcount_word, popcount_words};
use crate::bits::rank::RankDirectory;
use crate::bits::select::SelectIndex;
use crate::util::broadword::select_in_word;
use crate::{Config, RankSelect};

/// A bitvector with rank/select support.
///
/// The bitvector stores bits in 64-bit words and maintains auxiliary data
/// structures for O(1) rank queries and O(log n) select queries.
///
/// # Memory Layout
///
/// - **words**: Raw bit storage as `Vec<u64>`
/// - **rank_dir**: Poppy-style 3-level rank directory (~3% overhead)
/// - **select_idx**: Sampled select index (~1-3% overhead depending on sample rate)
///
/// # Example
///
/// ```
/// use succinctly::{BitVec, RankSelect};
///
/// let words = vec![0b1010_1010u64; 8];
/// let bv = BitVec::from_words(words, 512);
///
/// assert_eq!(bv.rank1(8), 4);
/// assert_eq!(bv.select1(0), Some(1));
/// ```
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct BitVec {
    /// Raw bit storage
    words: Vec<u64>,
    /// Number of valid bits
    len: usize,
    /// Total number of 1-bits (cached)
    ones_count: usize,
    /// Rank directory for O(1) rank queries
    rank_dir: RankDirectory,
    /// Select index for faster select queries
    select_idx: SelectIndex,
}

impl BitVec {
    /// Create a bitvector from raw u64 words.
    ///
    /// # Arguments
    ///
    /// * `words` - The raw bit data as 64-bit words (little-endian bit order)
    /// * `len` - The number of valid bits (may be less than `words.len() * 64`)
    ///
    /// # Panics
    ///
    /// Panics if `len > words.len() * 64`.
    pub fn from_words(words: Vec<u64>, len: usize) -> Self {
        Self::with_config(words, len, Config::default())
    }

    /// Create a bitvector with custom configuration.
    pub fn with_config(mut words: Vec<u64>, len: usize, config: Config) -> Self {
        assert!(
            len <= words.len().saturating_mul(64),
            "len {} exceeds capacity {}",
            len,
            words.len().saturating_mul(64)
        );

        // Mask out unused bits in the last word
        if len > 0 {
            let last_word_bits = len % 64;
            if last_word_bits > 0 && !words.is_empty() {
                let last_idx = words.len() - 1;
                let mask = (1u64 << last_word_bits) - 1;
                words[last_idx] &= mask;
            }
        }

        // Count total ones using the selected popcount implementation
        let ones_count: usize = popcount_words(&words) as usize;

        // Build rank directory
        let rank_dir = RankDirectory::build(&words);

        // Build select index
        let select_idx = SelectIndex::build(&words, ones_count, config.select_sample_rate);

        Self {
            words,
            len,
            ones_count,
            rank_dir,
            select_idx,
        }
    }

    /// Create an empty bitvector.
    pub fn new() -> Self {
        Self {
            words: Vec::new(),
            len: 0,
            ones_count: 0,
            rank_dir: RankDirectory::empty(),
            select_idx: SelectIndex::empty(),
        }
    }

    /// Number of bits in the bitvector.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the bitvector is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Total number of 1-bits in the bitvector.
    #[inline]
    pub fn count_ones(&self) -> usize {
        self.ones_count
    }

    /// Total number of 0-bits in the bitvector.
    #[inline]
    pub fn count_zeros(&self) -> usize {
        self.len - self.ones_count
    }

    /// Access the bit at position `i`.
    ///
    /// # Panics
    ///
    /// Panics if `i >= len`.
    #[inline]
    pub fn get(&self, i: usize) -> bool {
        assert!(i < self.len, "index {} out of bounds (len={})", i, self.len);
        let word_idx = i / 64;
        let bit_idx = i % 64;
        (self.words[word_idx] >> bit_idx) & 1 == 1
    }

    /// Access the bit at position `i` without bounds checking.
    ///
    /// # Safety
    ///
    /// Caller must ensure `i < len`.
    #[inline]
    pub unsafe fn get_unchecked(&self, i: usize) -> bool {
        let word_idx = i / 64;
        let bit_idx = i % 64;
        // SAFETY: Caller guarantees i is within bounds, so word_idx is valid
        unsafe { (*self.words.get_unchecked(word_idx) >> bit_idx) & 1 == 1 }
    }

    /// Get the raw word at the given index.
    #[inline]
    pub fn word(&self, idx: usize) -> u64 {
        self.words[idx]
    }

    /// Number of 64-bit words in the bitvector.
    #[inline]
    pub fn word_count(&self) -> usize {
        self.words.len()
    }

    /// Get a slice of all words.
    #[inline]
    pub fn words(&self) -> &[u64] {
        &self.words
    }
}

impl Default for BitVec {
    fn default() -> Self {
        Self::new()
    }
}

impl RankSelect for BitVec {
    /// Count 1-bits in positions `[0, i)`.
    ///
    /// Returns 0 if `i == 0`, and `count_ones()` if `i >= len`.
    #[inline]
    fn rank1(&self, i: usize) -> usize {
        if i == 0 {
            return 0;
        }
        if i >= self.len {
            return self.ones_count;
        }

        let word_idx = i / 64;
        let bit_idx = i % 64;

        // Get cumulative rank from directory
        let dir_rank = self.rank_dir.rank_at_word(word_idx);

        // Add partial word count using the selected popcount implementation
        let word = self.words[word_idx];
        let mask = (1u64 << bit_idx) - 1;
        let partial = popcount_word(word & mask) as usize;

        dir_rank + partial
    }

    /// Find position of the k-th 1-bit (0-indexed).
    ///
    /// Returns `None` if there are fewer than `k+1` ones.
    fn select1(&self, k: usize) -> Option<usize> {
        if k >= self.ones_count {
            return None;
        }

        // Use select index to jump to approximate position
        let (start_word, mut remaining) = self.select_idx.jump_to(k);

        // Scan words from the starting position
        for word_idx in start_word..self.words.len() {
            let word = self.words[word_idx];
            let pop = popcount_word(word) as usize;

            if pop > remaining {
                // Found the target word
                let bit_pos = select_in_word(word, remaining as u32) as usize;
                let result = word_idx * 64 + bit_pos;
                return if result < self.len {
                    Some(result)
                } else {
                    None
                };
            }

            remaining -= pop;
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_words_empty() {
        let bv = BitVec::from_words(vec![], 0);
        assert_eq!(bv.len(), 0);
        assert_eq!(bv.count_ones(), 0);
        assert!(bv.is_empty());
    }

    #[test]
    fn test_from_words_single() {
        let bv = BitVec::from_words(vec![0b1010_1010], 8);
        assert_eq!(bv.len(), 8);
        assert_eq!(bv.count_ones(), 4);
        assert!(!bv.get(0));
        assert!(bv.get(1));
        assert!(!bv.get(2));
        assert!(bv.get(3));
    }

    #[test]
    fn test_get_all_positions() {
        let bv = BitVec::from_words(vec![0b1100_0011], 8);
        assert!(bv.get(0));
        assert!(bv.get(1));
        assert!(!bv.get(2));
        assert!(!bv.get(3));
        assert!(!bv.get(4));
        assert!(!bv.get(5));
        assert!(bv.get(6));
        assert!(bv.get(7));
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn test_get_out_of_bounds() {
        let bv = BitVec::from_words(vec![0xFF], 8);
        bv.get(8);
    }

    #[test]
    fn test_count_ones_all_zeros() {
        let bv = BitVec::from_words(vec![0, 0, 0], 192);
        assert_eq!(bv.count_ones(), 0);
        assert_eq!(bv.count_zeros(), 192);
    }

    #[test]
    fn test_count_ones_all_ones() {
        let bv = BitVec::from_words(vec![u64::MAX, u64::MAX], 128);
        assert_eq!(bv.count_ones(), 128);
        assert_eq!(bv.count_zeros(), 0);
    }

    #[test]
    fn test_partial_word() {
        // Only 10 bits valid in a 64-bit word
        let bv = BitVec::from_words(vec![0b11111_11111], 10);
        assert_eq!(bv.len(), 10);
        assert_eq!(bv.count_ones(), 10);
    }

    #[test]
    fn test_partial_word_masks_unused() {
        // Word has all bits set, but only 10 are valid
        let bv = BitVec::from_words(vec![u64::MAX], 10);
        assert_eq!(bv.count_ones(), 10);
    }

    #[test]
    fn test_rank1_empty() {
        let bv = BitVec::from_words(vec![], 0);
        assert_eq!(bv.rank1(0), 0);
    }

    #[test]
    fn test_rank1_at_zero() {
        let bv = BitVec::from_words(vec![0b1111], 4);
        assert_eq!(bv.rank1(0), 0);
    }

    #[test]
    fn test_rank1_simple() {
        // Bits: 1 0 1 1 0 0 1 0 (LSB first)
        let bv = BitVec::from_words(vec![0b0100_1101], 8);
        assert_eq!(bv.rank1(1), 1); // [0,1) = bit 0 = 1
        assert_eq!(bv.rank1(2), 1); // [0,2) = bits 0,1 = 1,0 = 1
        assert_eq!(bv.rank1(3), 2); // [0,3) = bits 0,1,2 = 1,0,1 = 2
        assert_eq!(bv.rank1(4), 3); // [0,4) = bits 0,1,2,3 = 1,0,1,1 = 3
        assert_eq!(bv.rank1(8), 4); // all 8 bits
    }

    #[test]
    fn test_rank1_word_boundary() {
        let bv = BitVec::from_words(vec![u64::MAX, u64::MAX], 128);
        assert_eq!(bv.rank1(64), 64);
        assert_eq!(bv.rank1(65), 65);
        assert_eq!(bv.rank1(128), 128);
    }

    #[test]
    fn test_rank1_beyond_len() {
        let bv = BitVec::from_words(vec![u64::MAX], 64);
        assert_eq!(bv.rank1(100), 64);
    }

    #[test]
    fn test_rank0_simple() {
        let bv = BitVec::from_words(vec![0b0100_1101], 8);
        assert_eq!(bv.rank0(4), 1); // 4 - rank1(4) = 4 - 3 = 1
        assert_eq!(bv.rank0(8), 4); // 8 - rank1(8) = 8 - 4 = 4
    }

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
        // Bits: 1 0 1 1 0 0 1 0 (positions 0, 2, 3, 6 have 1s)
        let bv = BitVec::from_words(vec![0b0100_1101], 8);
        assert_eq!(bv.select1(0), Some(0)); // 0th one at position 0
        assert_eq!(bv.select1(1), Some(2)); // 1st one at position 2
        assert_eq!(bv.select1(2), Some(3)); // 2nd one at position 3
        assert_eq!(bv.select1(3), Some(6)); // 3rd one at position 6
        assert_eq!(bv.select1(4), None); // No 4th one
    }

    #[test]
    fn test_select1_word_boundary() {
        let bv = BitVec::from_words(vec![u64::MAX, u64::MAX], 128);
        assert_eq!(bv.select1(63), Some(63));
        assert_eq!(bv.select1(64), Some(64));
        assert_eq!(bv.select1(127), Some(127));
        assert_eq!(bv.select1(128), None);
    }

    #[test]
    fn test_rank_select_roundtrip() {
        let bv = BitVec::from_words(vec![0xAAAA_AAAA_AAAA_AAAA, 0x5555_5555_5555_5555], 128);

        // For every set bit, select(rank(i)) should be <= i
        for i in 0..128 {
            if bv.get(i) {
                let rank = bv.rank1(i);
                let select = bv.select1(rank);
                assert_eq!(select, Some(i), "roundtrip failed at i={}", i);
            }
        }
    }

    #[test]
    fn test_large_bitvector() {
        // 1024 bits with alternating pattern 0xAAAA = bits at odd positions (1,3,5,...)
        let words: Vec<u64> = (0..16).map(|_| 0xAAAA_AAAA_AAAA_AAAA).collect();
        let bv = BitVec::from_words(words, 1024);

        assert_eq!(bv.count_ones(), 512);
        assert_eq!(bv.rank1(512), 256);
        // 255th one (0-indexed): in word 7 (255/32=7), 31st one in that word
        // Position = 7*64 + (1 + 31*2) = 448 + 63 = 511
        assert_eq!(bv.select1(255), Some(511));
    }

    // ========================================================================
    // Partial word edge case tests (1-64 bits)
    // ========================================================================

    #[test]
    fn test_single_bit_bitvector() {
        // len = 1, single bit set
        let bv = BitVec::from_words(vec![1u64], 1);
        assert_eq!(bv.len(), 1);
        assert_eq!(bv.count_ones(), 1);
        assert!(bv.get(0));
        assert_eq!(bv.rank1(0), 0);
        assert_eq!(bv.rank1(1), 1);
        assert_eq!(bv.select1(0), Some(0));
        assert_eq!(bv.select1(1), None);

        // len = 1, single bit unset
        let bv = BitVec::from_words(vec![0u64], 1);
        assert_eq!(bv.count_ones(), 0);
        assert_eq!(bv.count_zeros(), 1);
        assert!(!bv.get(0));
        assert_eq!(bv.select1(0), None);
    }

    #[test]
    fn test_63_bit_bitvector() {
        // Boundary case: 63 bits (just under one word)
        let bv = BitVec::from_words(vec![u64::MAX], 63);
        assert_eq!(bv.len(), 63);
        assert_eq!(bv.count_ones(), 63);
        assert_eq!(bv.count_zeros(), 0);
        assert_eq!(bv.rank1(63), 63);
        assert_eq!(bv.select1(62), Some(62));
        assert_eq!(bv.select1(63), None);
    }

    #[test]
    fn test_65_bit_bitvector() {
        // Boundary case: 65 bits (just over one word)
        let bv = BitVec::from_words(vec![u64::MAX, 1u64], 65);
        assert_eq!(bv.len(), 65);
        assert_eq!(bv.count_ones(), 65);
        assert_eq!(bv.rank1(64), 64);
        assert_eq!(bv.rank1(65), 65);
        assert_eq!(bv.select1(64), Some(64));
        assert_eq!(bv.select1(65), None);
    }

    #[test]
    fn test_various_partial_lengths() {
        // Test lengths 1, 7, 8, 15, 16, 31, 32, 33, 63, 64, 65
        for len in [
            1usize, 7, 8, 15, 16, 31, 32, 33, 63, 64, 65, 100, 127, 128, 129,
        ] {
            let num_words = len.div_ceil(64);
            let words: Vec<u64> = vec![u64::MAX; num_words];
            let bv = BitVec::from_words(words, len);

            assert_eq!(bv.len(), len, "len mismatch for {}", len);
            assert_eq!(bv.count_ones(), len, "count_ones mismatch for len={}", len);
            assert_eq!(bv.rank1(len), len, "rank1(len) mismatch for len={}", len);

            if len > 0 {
                assert_eq!(
                    bv.select1(len - 1),
                    Some(len - 1),
                    "select1 last bit for len={}",
                    len
                );
            }
            assert_eq!(bv.select1(len), None, "select1 beyond for len={}", len);
        }
    }

    // ========================================================================
    // Extreme bit position tests (first/last bit)
    // ========================================================================

    #[test]
    fn test_first_bit_only() {
        // First bit set in various sized bitvectors
        for num_words in [1, 2, 8, 16] {
            let mut words = vec![0u64; num_words];
            words[0] = 1; // First bit set
            let len = num_words * 64;
            let bv = BitVec::from_words(words, len);

            assert_eq!(bv.count_ones(), 1);
            assert_eq!(bv.select1(0), Some(0));
            assert_eq!(bv.rank1(1), 1);
            assert_eq!(bv.rank1(len), 1);
        }
    }

    #[test]
    fn test_last_bit_only() {
        // Last bit set in various sized bitvectors
        for num_words in [1, 2, 8, 16] {
            let mut words = vec![0u64; num_words];
            let len = num_words * 64;
            words[num_words - 1] = 1u64 << 63; // Last bit of last word
            let bv = BitVec::from_words(words, len);

            assert_eq!(bv.count_ones(), 1);
            assert_eq!(bv.select1(0), Some(len - 1));
            assert_eq!(bv.rank1(len - 1), 0);
            assert_eq!(bv.rank1(len), 1);
        }
    }

    #[test]
    fn test_last_bit_partial_word() {
        // Last bit set where len is not word-aligned
        for len in [10usize, 33, 63, 100] {
            let num_words = len.div_ceil(64);
            let mut words = vec![0u64; num_words];
            let bit_in_last_word = (len - 1) % 64;
            words[num_words - 1] = 1u64 << bit_in_last_word;
            let bv = BitVec::from_words(words, len);

            assert_eq!(bv.count_ones(), 1, "count_ones for len={}", len);
            assert_eq!(bv.select1(0), Some(len - 1), "select1 for len={}", len);
            assert_eq!(bv.rank1(len - 1), 0, "rank1(len-1) for len={}", len);
            assert_eq!(bv.rank1(len), 1, "rank1(len) for len={}", len);
        }
    }

    #[test]
    fn test_first_and_last_bit_only() {
        // Only first and last bits set
        let len = 1024;
        let num_words = len / 64;
        let mut words = vec![0u64; num_words];
        words[0] = 1; // First bit
        words[num_words - 1] = 1u64 << 63; // Last bit

        let bv = BitVec::from_words(words, len);

        assert_eq!(bv.count_ones(), 2);
        assert_eq!(bv.select1(0), Some(0));
        assert_eq!(bv.select1(1), Some(len - 1));
        assert_eq!(bv.rank1(1), 1);
        assert_eq!(bv.rank1(len - 1), 1);
        assert_eq!(bv.rank1(len), 2);
    }

    // ========================================================================
    // Cross-block boundary tests
    // ========================================================================

    #[test]
    fn test_block_boundary_512_bits() {
        // One rank block = 8 words = 512 bits
        // Test with ones at block boundary
        let mut words = vec![0u64; 16]; // 1024 bits = 2 blocks
        words[7] = 1u64 << 63; // Bit 511 (last bit of block 0)
        words[8] = 1u64; // Bit 512 (first bit of block 1)

        let bv = BitVec::from_words(words, 1024);

        assert_eq!(bv.count_ones(), 2);
        assert_eq!(bv.rank1(511), 0);
        assert_eq!(bv.rank1(512), 1);
        assert_eq!(bv.rank1(513), 2);
        assert_eq!(bv.select1(0), Some(511));
        assert_eq!(bv.select1(1), Some(512));
    }

    #[test]
    fn test_select_crossing_blocks() {
        // Pattern where select must cross block boundaries
        // 8 words per block, set one bit per word
        let words: Vec<u64> = (0..64).map(|i| 1u64 << (i % 64)).collect();
        let bv = BitVec::from_words(words, 64 * 64);

        assert_eq!(bv.count_ones(), 64);

        // Test select at block boundaries
        assert_eq!(bv.select1(7), Some(7 * 64 + 7)); // Last in block 0
        assert_eq!(bv.select1(8), Some(8 * 64 + 8)); // First in block 1
        assert_eq!(bv.select1(63), Some(63 * 64 + 63)); // Last one
    }

    #[test]
    fn test_rank_at_every_word_boundary() {
        // Test rank at every word boundary for a multi-block bitvector
        let words: Vec<u64> = (0..32)
            .map(|i| if i % 2 == 0 { u64::MAX } else { 0 })
            .collect();
        let bv = BitVec::from_words(words, 32 * 64);

        // Even words have all ones (64 each), odd words have none
        // Cumulative: word 0=64, word 2=128, word 4=192, ...
        for word_idx in 0..32 {
            let pos = word_idx * 64;
            let expected = (word_idx / 2 + word_idx % 2) * 64;
            assert_eq!(
                bv.rank1(pos),
                expected,
                "rank1({}) at word {} boundary",
                pos,
                word_idx
            );
        }
    }

    // ========================================================================
    // Configuration/sample rate tests
    // ========================================================================

    #[test]
    fn test_with_sample_rate_64() {
        let words: Vec<u64> = vec![0xAAAA_AAAA_AAAA_AAAA; 16];
        let config = Config {
            select_sample_rate: 64,
        };
        let bv = BitVec::with_config(words, 1024, config);

        // Should work the same regardless of sample rate
        assert_eq!(bv.count_ones(), 512);
        assert_eq!(bv.select1(0), Some(1));
        assert_eq!(bv.select1(255), Some(511));
        assert_eq!(bv.select1(511), Some(1023));
    }

    #[test]
    fn test_with_sample_rate_1() {
        // Sample every single one
        let words: Vec<u64> = vec![0xAAAA_AAAA_AAAA_AAAA; 8];
        let config = Config {
            select_sample_rate: 1,
        };
        let bv = BitVec::with_config(words, 512, config);

        assert_eq!(bv.count_ones(), 256);
        // With sample rate 1, every position should be directly indexed
        for k in 0..256 {
            assert!(bv.select1(k).is_some(), "select1({}) should be Some", k);
        }
    }

    #[test]
    fn test_with_large_sample_rate() {
        // Sample rate larger than total ones
        let words: Vec<u64> = vec![1u64; 8]; // 8 ones total
        let config = Config {
            select_sample_rate: 1024,
        };
        let bv = BitVec::with_config(words, 512, config);

        assert_eq!(bv.count_ones(), 8);
        for k in 0..8 {
            assert_eq!(bv.select1(k), Some(k * 64), "select1({}) failed", k);
        }
    }

    // ========================================================================
    // Empty bitvector edge cases
    // ========================================================================

    #[test]
    fn test_empty_all_operations() {
        let bv = BitVec::new();

        assert_eq!(bv.len(), 0);
        assert_eq!(bv.count_ones(), 0);
        assert_eq!(bv.count_zeros(), 0);
        assert!(bv.is_empty());

        // Rank operations
        assert_eq!(bv.rank1(0), 0);
        assert_eq!(bv.rank0(0), 0);
        assert_eq!(bv.rank1(100), 0); // Beyond len

        // Select operations
        assert_eq!(bv.select1(0), None);
    }

    // ========================================================================
    // get_unchecked safety test
    // ========================================================================

    #[test]
    fn test_get_unchecked_valid() {
        let bv = BitVec::from_words(vec![0b1010_1010u64], 8);

        // SAFETY: We know indices 0-7 are valid
        unsafe {
            assert!(!bv.get_unchecked(0));
            assert!(bv.get_unchecked(1));
            assert!(!bv.get_unchecked(2));
            assert!(bv.get_unchecked(3));
            assert!(!bv.get_unchecked(4));
            assert!(bv.get_unchecked(5));
            assert!(!bv.get_unchecked(6));
            assert!(bv.get_unchecked(7));
        }
    }

    // ========================================================================
    // Rank/select consistency tests
    // ========================================================================

    #[test]
    fn test_rank_select_consistency_sparse() {
        // Sparse: one bit every 100 positions
        let mut words = vec![0u64; 32]; // 2048 bits
        for i in 0..20 {
            let pos = i * 100;
            if pos < 2048 {
                words[pos / 64] |= 1u64 << (pos % 64);
            }
        }
        let bv = BitVec::from_words(words, 2048);

        // Verify rank(select(k)) = k for valid k
        for k in 0..bv.count_ones() {
            if let Some(pos) = bv.select1(k) {
                assert_eq!(bv.rank1(pos), k, "rank1(select1({})) should be {}", k, k);
            }
        }
    }
}
