//! Bitvector with rank/select support.
//!
//! This module provides `BitVec`, the main data structure for storing bits
//! with efficient rank and select operations.

#[cfg(not(test))]
use alloc::vec::Vec;

use crate::broadword::select_in_word;
use crate::rank::RankDirectory;
use crate::select::SelectIndex;
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

        // Count total ones
        let ones_count: usize = words.iter().map(|w| w.count_ones() as usize).sum();

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

        // Add partial word count
        let word = self.words[word_idx];
        let mask = (1u64 << bit_idx) - 1;
        let partial = (word & mask).count_ones() as usize;

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
            let pop = word.count_ones() as usize;

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

    /// Find position of the k-th 0-bit (0-indexed).
    ///
    /// Returns `None` if there are fewer than `k+1` zeros.
    fn select0(&self, k: usize) -> Option<usize> {
        let zeros = self.count_zeros();
        if k >= zeros {
            return None;
        }

        // Linear scan for select0 (could be optimized with separate index)
        let mut remaining = k;

        for word_idx in 0..self.words.len() {
            let word = self.words[word_idx];
            let inverted = !word;

            // Handle partial last word
            let valid_bits = if word_idx == self.words.len() - 1 && self.len % 64 != 0 {
                self.len % 64
            } else {
                64
            };

            let zeros_in_word = if valid_bits == 64 {
                inverted.count_ones() as usize
            } else {
                let mask = (1u64 << valid_bits) - 1;
                (inverted & mask).count_ones() as usize
            };

            if zeros_in_word > remaining {
                // Found the target word - find k-th zero (= k-th one in inverted)
                let search_word = if valid_bits == 64 {
                    inverted
                } else {
                    inverted & ((1u64 << valid_bits) - 1)
                };
                let bit_pos = select_in_word(search_word, remaining as u32) as usize;
                return Some(word_idx * 64 + bit_pos);
            }

            remaining -= zeros_in_word;
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
    fn test_select0_simple() {
        // Bits: 1 0 1 1 0 0 1 0 (positions 1, 4, 5, 7 have 0s)
        let bv = BitVec::from_words(vec![0b0100_1101], 8);
        assert_eq!(bv.select0(0), Some(1)); // 0th zero at position 1
        assert_eq!(bv.select0(1), Some(4)); // 1st zero at position 4
        assert_eq!(bv.select0(2), Some(5)); // 2nd zero at position 5
        assert_eq!(bv.select0(3), Some(7)); // 3rd zero at position 7
        assert_eq!(bv.select0(4), None); // No 4th zero
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
}
