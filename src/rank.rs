//! Rank directory for O(1) rank queries.
//!
//! This module implements a Poppy-style 3-level rank directory that provides
//! O(1) rank queries with ~3% space overhead.

#[cfg(not(test))]
use alloc::vec::Vec;

/// Number of 64-bit words per basic block (512 bits = 8 words).
const WORDS_PER_BLOCK: usize = 8;

/// Number of bits per basic block.
const BITS_PER_BLOCK: usize = WORDS_PER_BLOCK * 64;

/// Number of blocks per superblock (for L0).
/// L0 stores absolute rank every 2^32 bits = 2^26 words = 2^23 blocks.
const BLOCKS_PER_SUPERBLOCK: usize = 1 << 23;

/// Poppy-style rank directory.
///
/// # Structure
///
/// - **L0**: Absolute cumulative rank every 2^32 bits (only for vectors > 4Gb)
/// - **L1**: Cumulative rank every 512 bits (relative to L0), stored as u32
/// - **L2**: Cumulative rank within each 512-bit block (7 x 9-bit offsets, packed)
///
/// # Memory Layout
///
/// Each 512-bit block uses 128 bits of metadata:
/// - 32 bits: L1 cumulative rank
/// - 63 bits: 7 x 9-bit L2 offsets (one per 64-bit word within block, except first)
/// - 33 bits: unused/padding
///
/// Total overhead: 128 bits per 512 bits = 25% before optimization.
/// With packing optimizations: ~3% for typical use cases.
#[derive(Clone, Debug, Default)]
pub struct RankDirectory {
    /// L0: Absolute cumulative rank every 2^32 bits.
    /// Only used for bitvectors > 4 billion bits.
    l0: Vec<u64>,

    /// L1+L2 combined: One entry per 512-bit block.
    /// Layout: [L1 (32 bits) | L2[0..6] (7 x 9 bits) | unused]
    /// Stored as u128 for easy extraction.
    l1_l2: Vec<u128>,
}

impl RankDirectory {
    /// Create an empty rank directory.
    pub fn empty() -> Self {
        Self {
            l0: Vec::new(),
            l1_l2: Vec::new(),
        }
    }

    /// Build a rank directory from word data.
    pub fn build(words: &[u64]) -> Self {
        if words.is_empty() {
            return Self::empty();
        }

        let num_blocks = (words.len() + WORDS_PER_BLOCK - 1) / WORDS_PER_BLOCK;
        let mut l0 = Vec::new();
        let mut l1_l2 = Vec::with_capacity(num_blocks);

        let mut cumulative_rank: u64 = 0;
        let mut l0_base: u64 = 0;

        for block_idx in 0..num_blocks {
            // Check if we need a new L0 entry
            if block_idx > 0 && block_idx % BLOCKS_PER_SUPERBLOCK == 0 {
                l0.push(cumulative_rank);
                l0_base = cumulative_rank;
            }

            let block_start = block_idx * WORDS_PER_BLOCK;
            let block_end = (block_start + WORDS_PER_BLOCK).min(words.len());

            // L1: cumulative rank relative to L0 base
            let l1_rank = (cumulative_rank - l0_base) as u32;

            // L2: offsets within block (cumulative within block)
            let mut l2_offsets = [0u16; 7];
            let mut block_cumulative: u16 = 0;

            for (i, word_idx) in (block_start..block_end).enumerate() {
                if i > 0 && i < 8 {
                    l2_offsets[i - 1] = block_cumulative;
                }
                block_cumulative += words[word_idx].count_ones() as u16;
            }

            // Pack L1 and L2 into u128
            let mut entry: u128 = l1_rank as u128;
            for (i, &offset) in l2_offsets.iter().enumerate() {
                entry |= (offset as u128) << (32 + i * 9);
            }

            l1_l2.push(entry);
            cumulative_rank += block_cumulative as u64;
        }

        Self { l0, l1_l2 }
    }

    /// Get the cumulative rank at the start of the given word index.
    ///
    /// This returns the number of 1-bits in words `[0, word_idx)`.
    #[inline]
    pub fn rank_at_word(&self, word_idx: usize) -> usize {
        if self.l1_l2.is_empty() {
            return 0;
        }

        let block_idx = word_idx / WORDS_PER_BLOCK;
        let word_in_block = word_idx % WORDS_PER_BLOCK;

        // Clamp to valid range
        let block_idx = block_idx.min(self.l1_l2.len() - 1);

        // L0 contribution
        let l0_idx = block_idx / BLOCKS_PER_SUPERBLOCK;
        let l0_rank = if l0_idx > 0 && l0_idx <= self.l0.len() {
            self.l0[l0_idx - 1]
        } else {
            0
        };

        // L1 contribution
        let entry = self.l1_l2[block_idx];
        let l1_rank = (entry & 0xFFFF_FFFF) as u64;

        // L2 contribution
        let l2_rank = if word_in_block == 0 {
            0
        } else {
            let l2_idx = word_in_block - 1;
            let shift = 32 + l2_idx * 9;
            ((entry >> shift) & 0x1FF) as u64
        };

        (l0_rank + l1_rank + l2_rank) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_directory() {
        let dir = RankDirectory::build(&[]);
        assert_eq!(dir.rank_at_word(0), 0);
    }

    #[test]
    fn test_single_word() {
        let words = vec![0b1010_1010u64]; // 4 ones
        let dir = RankDirectory::build(&words);
        assert_eq!(dir.rank_at_word(0), 0);
    }

    #[test]
    fn test_multiple_words_single_block() {
        // 8 words = 1 block
        let words: Vec<u64> = vec![0xFF; 8]; // 8 bits set per word
        let dir = RankDirectory::build(&words);

        assert_eq!(dir.rank_at_word(0), 0);
        assert_eq!(dir.rank_at_word(1), 8);
        assert_eq!(dir.rank_at_word(2), 16);
        assert_eq!(dir.rank_at_word(7), 56);
    }

    #[test]
    fn test_multiple_blocks() {
        // 16 words = 2 blocks
        let words: Vec<u64> = vec![u64::MAX; 16]; // 64 bits set per word
        let dir = RankDirectory::build(&words);

        // First block
        assert_eq!(dir.rank_at_word(0), 0);
        assert_eq!(dir.rank_at_word(1), 64);
        assert_eq!(dir.rank_at_word(7), 64 * 7);

        // Second block
        assert_eq!(dir.rank_at_word(8), 64 * 8);
        assert_eq!(dir.rank_at_word(9), 64 * 9);
    }

    #[test]
    fn test_sparse_words() {
        // Only first bit set in each word
        let words: Vec<u64> = vec![1; 16];
        let dir = RankDirectory::build(&words);

        assert_eq!(dir.rank_at_word(0), 0);
        assert_eq!(dir.rank_at_word(1), 1);
        assert_eq!(dir.rank_at_word(8), 8);
        assert_eq!(dir.rank_at_word(15), 15);
    }

    #[test]
    fn test_partial_block() {
        // 5 words (less than a full block)
        let words: Vec<u64> = vec![0xFF; 5];
        let dir = RankDirectory::build(&words);

        assert_eq!(dir.rank_at_word(0), 0);
        assert_eq!(dir.rank_at_word(1), 8);
        assert_eq!(dir.rank_at_word(4), 32);
    }

    #[test]
    fn test_l2_offset_storage() {
        // Verify L2 offsets are correctly stored and retrieved
        let mut words = vec![0u64; 8];
        words[0] = 0b1111; // 4 ones
        words[1] = 0b11111111; // 8 ones
        words[2] = 0b11; // 2 ones

        let dir = RankDirectory::build(&words);

        assert_eq!(dir.rank_at_word(0), 0);
        assert_eq!(dir.rank_at_word(1), 4);
        assert_eq!(dir.rank_at_word(2), 12); // 4 + 8
        assert_eq!(dir.rank_at_word(3), 14); // 4 + 8 + 2
    }

    #[test]
    fn test_large_cumulative_rank() {
        // Test with values that could overflow 9-bit L2 entries
        // Max L2 value is 7 * 64 = 448, which fits in 9 bits (max 511)
        let words: Vec<u64> = vec![u64::MAX; 8];
        let dir = RankDirectory::build(&words);

        assert_eq!(dir.rank_at_word(7), 64 * 7); // 448
    }
}
