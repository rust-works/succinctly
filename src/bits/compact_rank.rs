//! Compact two-level rank directory for O(1) rank queries.
//!
//! Provides ~3.5% space overhead vs 50% for the naive `Vec<u32>` cumulative
//! popcount approach. Used by YAML index structures where multiple bitmaps
//! each need rank support.
//!
//! # Structure
//!
//! - **L1**: Absolute cumulative rank every 128 words (8192 bits).
//!   One `u32` per superblock → 0.39% overhead.
//! - **L2**: Relative cumulative rank every 8 words (512 bits).
//!   One `u16` per block → 3.125% overhead.
//!
//! Total: ~3.5% overhead relative to the bitmap.
//!
//! # Query
//!
//! `rank_at_word(w)` = `l1[w / 128] + l2[w / 8]` — two array lookups, no
//! popcount computation needed.

#[cfg(not(test))]
use alloc::vec::Vec;

/// Words per L1 superblock.
const L1_WORDS: usize = 128;

/// Words per L2 block.
const L2_WORDS: usize = 8;

/// Compact two-level rank directory.
///
/// Stores cumulative popcount at two granularities:
/// - L1: absolute rank per 128 words (u32, supports up to 4 billion bits)
/// - L2: relative rank per 8 words within a superblock (u16, max 8192)
///
/// Query: `rank_at_word(words, w)` = `l1[w/128] + l2[w/8] + popcount(block_words)`
#[derive(Clone, Debug)]
pub struct CompactRank {
    /// Absolute cumulative rank at each superblock boundary.
    /// Entry i = popcount of words [0, i * 128).
    l1: Vec<u32>,
    /// Relative cumulative rank at each block boundary within its superblock.
    /// Entry j = popcount of words [superblock_start, j * 8) where
    /// superblock_start = (j / 16) * 128.
    l2: Vec<u16>,
    /// Total popcount across all words.
    total: u32,
}

impl CompactRank {
    /// Create an empty rank directory.
    pub fn empty() -> Self {
        Self {
            l1: Vec::new(),
            l2: Vec::new(),
            total: 0,
        }
    }

    /// Build a compact rank directory from bitmap words.
    ///
    /// After construction, `rank_at_word(w)` returns the number of 1-bits
    /// in `words[0..w]`.
    pub fn build(words: &[u64]) -> Self {
        if words.is_empty() {
            return Self::empty();
        }

        let num_superblocks = words.len().div_ceil(L1_WORDS);
        let num_blocks = words.len().div_ceil(L2_WORDS);

        let mut l1 = Vec::with_capacity(num_superblocks);
        let mut l2 = Vec::with_capacity(num_blocks);

        let mut absolute_rank: u32 = 0;

        for sb in 0..num_superblocks {
            l1.push(absolute_rank);

            let sb_start = sb * L1_WORDS;
            let sb_end = (sb_start + L1_WORDS).min(words.len());
            let mut relative_rank: u16 = 0;

            // Blocks within this superblock
            let blocks_in_sb = (sb_end - sb_start).div_ceil(L2_WORDS);
            for b in 0..blocks_in_sb {
                l2.push(relative_rank);

                let block_start = sb_start + b * L2_WORDS;
                let block_end = (block_start + L2_WORDS).min(sb_end);
                for &word in &words[block_start..block_end] {
                    let ones = word.count_ones() as u16;
                    relative_rank += ones;
                    absolute_rank += ones as u32;
                }
            }
        }

        Self {
            l1,
            l2,
            total: absolute_rank,
        }
    }

    /// Get the cumulative rank at the start of the given word index.
    ///
    /// Returns the number of 1-bits in `words[0..word_idx]`.
    /// The `words` parameter must be the same bitmap data passed to `build()`.
    /// If `word_idx` exceeds the number of words, returns the total popcount.
    #[inline]
    pub fn rank_at_word(&self, words: &[u64], word_idx: usize) -> usize {
        if self.l1.is_empty() {
            return 0;
        }

        // Boundary case: at or past the end
        if word_idx >= words.len() {
            return self.total as usize;
        }

        let sb_idx = word_idx / L1_WORDS;
        let block_idx = word_idx / L2_WORDS;

        let mut count = self.l1[sb_idx] as usize + self.l2[block_idx] as usize;

        // Add popcount for words within the block up to word_idx
        let block_start = block_idx * L2_WORDS;
        for &word in &words[block_start..word_idx] {
            count += word.count_ones() as usize;
        }

        count
    }

    /// Returns the heap memory usage in bytes.
    pub fn heap_size(&self) -> usize {
        self.l1.len() * 4 + self.l2.len() * 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let words: Vec<u64> = vec![];
        let cr = CompactRank::build(&words);
        assert_eq!(cr.rank_at_word(&words, 0), 0);
    }

    #[test]
    fn test_single_word() {
        let words = vec![0b1010_1010u64]; // 4 ones
        let cr = CompactRank::build(&words);
        assert_eq!(cr.rank_at_word(&words, 0), 0);
        assert_eq!(cr.rank_at_word(&words, 1), 4);
    }

    #[test]
    fn test_multiple_words_single_block() {
        // 8 words = 1 block
        let words: Vec<u64> = vec![0xFF; 8]; // 8 bits per word
        let cr = CompactRank::build(&words);

        assert_eq!(cr.rank_at_word(&words, 0), 0);
        assert_eq!(cr.rank_at_word(&words, 1), 8);
        assert_eq!(cr.rank_at_word(&words, 2), 16);
        assert_eq!(cr.rank_at_word(&words, 7), 56);
        assert_eq!(cr.rank_at_word(&words, 8), 64);
    }

    #[test]
    fn test_multiple_blocks() {
        // 16 words = 2 blocks
        let words: Vec<u64> = vec![u64::MAX; 16]; // 64 bits per word
        let cr = CompactRank::build(&words);

        assert_eq!(cr.rank_at_word(&words, 0), 0);
        assert_eq!(cr.rank_at_word(&words, 1), 64);
        assert_eq!(cr.rank_at_word(&words, 8), 64 * 8);
        assert_eq!(cr.rank_at_word(&words, 15), 64 * 15);
        assert_eq!(cr.rank_at_word(&words, 16), 64 * 16);
    }

    #[test]
    fn test_cross_superblock_boundary() {
        // 256 words = 2 superblocks (128 words each)
        let words: Vec<u64> = vec![1u64; 256]; // 1 bit per word
        let cr = CompactRank::build(&words);

        assert_eq!(cr.rank_at_word(&words, 0), 0);
        assert_eq!(cr.rank_at_word(&words, 128), 128);
        assert_eq!(cr.rank_at_word(&words, 256), 256);
    }

    #[test]
    fn test_sparse_words() {
        let words: Vec<u64> = vec![1; 16]; // 1 bit per word
        let cr = CompactRank::build(&words);

        assert_eq!(cr.rank_at_word(&words, 0), 0);
        assert_eq!(cr.rank_at_word(&words, 1), 1);
        assert_eq!(cr.rank_at_word(&words, 8), 8);
        assert_eq!(cr.rank_at_word(&words, 15), 15);
        assert_eq!(cr.rank_at_word(&words, 16), 16);
    }

    #[test]
    fn test_partial_block() {
        // 5 words (less than a full block)
        let words: Vec<u64> = vec![0xFF; 5]; // 8 bits per word
        let cr = CompactRank::build(&words);

        assert_eq!(cr.rank_at_word(&words, 0), 0);
        assert_eq!(cr.rank_at_word(&words, 1), 8);
        assert_eq!(cr.rank_at_word(&words, 4), 32);
        assert_eq!(cr.rank_at_word(&words, 5), 40);
    }

    #[test]
    fn test_matches_naive_cumulative() {
        // Compare against naive Vec<u32> cumulative popcount
        let words: Vec<u64> = (0..300).map(|i| ((i * 7 + 3) % 256) as u64).collect();

        let cr = CompactRank::build(&words);

        // Build naive
        let mut naive = vec![0u32];
        let mut cum = 0u32;
        for &w in &words {
            cum += w.count_ones();
            naive.push(cum);
        }

        for (i, &expected) in naive.iter().enumerate().take(words.len() + 1) {
            assert_eq!(
                cr.rank_at_word(&words, i),
                expected as usize,
                "mismatch at word {}",
                i
            );
        }
    }

    #[test]
    fn test_large_superblock_boundary() {
        // 128 words exactly = 1 full superblock
        let words: Vec<u64> = vec![0xFF; 128]; // 8 bits per word
        let cr = CompactRank::build(&words);

        assert_eq!(cr.rank_at_word(&words, 0), 0);
        assert_eq!(cr.rank_at_word(&words, 64), 64 * 8);
        assert_eq!(cr.rank_at_word(&words, 128), 128 * 8);
    }

    #[test]
    fn test_overhead() {
        // Verify overhead is ~3.5%
        let words: Vec<u64> = vec![0; 1024]; // 8KB of bitmap
        let cr = CompactRank::build(&words);

        let bitmap_bytes = words.len() * 8;
        let index_bytes = cr.heap_size();
        let overhead_pct = (index_bytes as f64 / bitmap_bytes as f64) * 100.0;

        assert!(
            overhead_pct < 5.0,
            "Overhead {:.1}% exceeds 5% target (bitmap={}, index={})",
            overhead_pct,
            bitmap_bytes,
            index_bytes
        );
    }
}
