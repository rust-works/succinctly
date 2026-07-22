//! Lightweight DSV index using simple cumulative rank (like JsonIndex).
//!
//! This is faster than full BitVec with RankDirectory + SelectIndex.

#[cfg(not(test))]
use alloc::vec::Vec;

/// Lightweight DSV index with simple cumulative rank per word.
///
/// Instead of full BitVec with 3-level RankDirectory and SelectIndex,
/// this uses simple cumulative popcount per word like JsonIndex.
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DsvIndexLightweight {
    /// Marker bits (delimiter or newline positions)
    pub markers: Vec<u64>,
    /// Cumulative popcount for markers: rank[i] = total 1-bits in words[0..i)
    pub markers_rank: Vec<u32>,

    /// Newline bits
    pub newlines: Vec<u64>,
    /// Cumulative popcount for newlines: rank[i] = total 1-bits in words[0..i)
    pub newlines_rank: Vec<u32>,

    /// Total byte length of indexed text
    pub text_len: usize,
}

/// Build cumulative rank array.
/// Returns a vector where entry i = total 1-bits in words[0..i).
///
/// The `u32` accumulator is safe because `new` asserts
/// `text_len <= u32::MAX` (#188), and set bits <= text_len. Widening both
/// rank arrays to `u64` would double two hot per-word structures (~12.5% of
/// input combined) for inputs the index cannot represent anyway.
fn build_rank(words: &[u64]) -> Vec<u32> {
    let mut rank = Vec::with_capacity(words.len() + 1);
    let mut cumulative: u32 = 0;
    rank.push(0); // rank[0] = 0 (no words before word 0)
    for &word in words {
        cumulative += word.count_ones();
        rank.push(cumulative);
    }
    rank
}

impl DsvIndexLightweight {
    /// Create a new lightweight index.
    ///
    /// # Panics
    ///
    /// Panics if `text_len` exceeds `u32::MAX` bytes (just under 4 GiB): the
    /// cumulative rank arrays store counts as `u32` (#188). Larger inputs
    /// would previously truncate silently. (The fields are `pub`, so a direct
    /// struct literal can bypass this check; all crate builders funnel through
    /// `new`.)
    pub fn new(markers: Vec<u64>, newlines: Vec<u64>, text_len: usize) -> Self {
        assert!(
            u32::try_from(text_len).is_ok(),
            "DsvIndexLightweight supports inputs up to u32::MAX (4294967295) bytes; \
             got {text_len} bytes (#188)"
        );
        let markers_rank = build_rank(&markers);
        let newlines_rank = build_rank(&newlines);

        Self {
            markers,
            markers_rank,
            newlines,
            newlines_rank,
            text_len,
        }
    }

    /// Count 1-bits in markers at positions [0, i).
    #[inline]
    pub fn markers_rank1(&self, i: usize) -> usize {
        if i == 0 {
            return 0;
        }
        if i >= self.text_len {
            return *self.markers_rank.last().unwrap_or(&0) as usize;
        }

        let word_idx = i / 64;
        let bit_idx = i % 64;

        // Get cumulative rank from array
        let cumulative = self.markers_rank[word_idx] as usize;

        // Add partial word count
        if word_idx < self.markers.len() {
            let word = self.markers[word_idx];
            let mask = (1u64 << bit_idx) - 1;
            cumulative + (word & mask).count_ones() as usize
        } else {
            cumulative
        }
    }

    /// Find position of k-th 1-bit in markers (0-indexed).
    #[inline]
    pub fn markers_select1(&self, k: usize) -> Option<usize> {
        let total_ones = *self.markers_rank.last().unwrap_or(&0) as usize;
        if k >= total_ones {
            return None;
        }

        // Find the word containing the k-th bit: the last word whose
        // cumulative rank is <= k. partition_point is duplicate-stable, unlike
        // binary_search, which returns an arbitrary index among the equal
        // entries produced by zero-marker words (issue #196). rank[0] == 0, so
        // the partition point is always >= 1.
        let word_idx = self.markers_rank.partition_point(|&r| r as usize <= k) - 1;

        if word_idx >= self.markers.len() {
            return None;
        }

        // Count how many ones we need to skip in this word
        let rank_before = self.markers_rank[word_idx] as usize;
        let remaining = k - rank_before;

        // Find the position within the word
        let word = self.markers[word_idx];
        let bit_pos = select_in_word(word, remaining as u32) as usize;

        let result = word_idx * 64 + bit_pos;
        if result < self.text_len {
            Some(result)
        } else {
            None
        }
    }

    /// Count 1-bits in newlines at positions [0, i).
    #[inline]
    pub fn newlines_rank1(&self, i: usize) -> usize {
        if i == 0 {
            return 0;
        }
        if i >= self.text_len {
            return *self.newlines_rank.last().unwrap_or(&0) as usize;
        }

        let word_idx = i / 64;
        let bit_idx = i % 64;

        let cumulative = self.newlines_rank[word_idx] as usize;

        if word_idx < self.newlines.len() {
            let word = self.newlines[word_idx];
            let mask = (1u64 << bit_idx) - 1;
            cumulative + (word & mask).count_ones() as usize
        } else {
            cumulative
        }
    }

    /// Find position of k-th 1-bit in newlines (0-indexed).
    #[inline]
    pub fn newlines_select1(&self, k: usize) -> Option<usize> {
        let total_ones = *self.newlines_rank.last().unwrap_or(&0) as usize;
        if k >= total_ones {
            return None;
        }

        // See markers_select1: duplicate-stable word lookup (issue #196).
        let word_idx = self.newlines_rank.partition_point(|&r| r as usize <= k) - 1;

        if word_idx >= self.newlines.len() {
            return None;
        }

        let rank_before = self.newlines_rank[word_idx] as usize;
        let remaining = k - rank_before;

        let word = self.newlines[word_idx];
        let bit_pos = select_in_word(word, remaining as u32) as usize;

        let result = word_idx * 64 + bit_pos;
        if result < self.text_len {
            Some(result)
        } else {
            None
        }
    }

    /// Total number of markers.
    #[inline]
    pub fn marker_count(&self) -> usize {
        *self.markers_rank.last().unwrap_or(&0) as usize
    }

    /// Total number of newlines (row count).
    #[inline]
    pub fn row_count(&self) -> usize {
        *self.newlines_rank.last().unwrap_or(&0) as usize
    }

    /// Check if index is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.text_len == 0
    }
}

/// Select k-th set bit in a word using CTZ loop.
#[inline]
fn select_in_word(mut val: u64, mut k: u32) -> u32 {
    loop {
        if val == 0 {
            return 64;
        }
        let t = val.trailing_zeros();
        if k == 0 {
            return t;
        }
        k -= 1;
        val &= val - 1; // Clear lowest set bit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rank() {
        let markers = vec![0b1010_1010u64];
        let newlines = vec![0b0000_0001u64];
        let index = DsvIndexLightweight::new(markers, newlines, 64);

        assert_eq!(index.markers_rank1(0), 0);
        assert_eq!(index.markers_rank1(1), 0);
        assert_eq!(index.markers_rank1(2), 1);
        assert_eq!(index.markers_rank1(8), 4);
    }

    #[test]
    fn test_select() {
        let markers = vec![0b1010_1010u64];
        let newlines = vec![0b0000_0001u64];
        let index = DsvIndexLightweight::new(markers, newlines, 64);

        assert_eq!(index.markers_select1(0), Some(1));
        assert_eq!(index.markers_select1(1), Some(3));
        assert_eq!(index.markers_select1(2), Some(5));
        assert_eq!(index.markers_select1(3), Some(7));
    }

    #[test]
    #[should_panic(expected = "up to u32::MAX")]
    #[cfg(target_pointer_width = "64")]
    fn test_text_len_guard_panics() {
        // text_len is a plain parameter, so exercising the #188 guard needs no
        // 4 GiB allocation.
        let _ = DsvIndexLightweight::new(vec![], vec![], u32::MAX as usize + 1);
    }

    /// Issue #196: a word with zero markers produces duplicate entries in the
    /// cumulative rank array, and select1 must still find the right word.
    #[test]
    fn test_select1_zero_marker_word_regression_196() {
        let markers = vec![0b1u64, 0, 0b11];
        let newlines = vec![0b1u64, 0, 0b10];
        let index = DsvIndexLightweight::new(markers, newlines, 192);

        assert_eq!(index.markers_rank, vec![0, 1, 1, 3]);
        assert_eq!(index.markers_select1(0), Some(0));
        assert_eq!(index.markers_select1(1), Some(128));
        assert_eq!(index.markers_select1(2), Some(129));
        assert_eq!(index.markers_select1(3), None);

        assert_eq!(index.newlines_rank, vec![0, 1, 1, 2]);
        assert_eq!(index.newlines_select1(0), Some(0));
        assert_eq!(index.newlines_select1(1), Some(129));
        assert_eq!(index.newlines_select1(2), None);
    }

    #[test]
    fn test_select1_leading_zero_word() {
        let markers = vec![0u64, 0b101];
        let newlines = vec![0u64, 0b10];
        let index = DsvIndexLightweight::new(markers, newlines, 128);

        assert_eq!(index.markers_rank, vec![0, 0, 2]);
        assert_eq!(index.markers_select1(0), Some(64));
        assert_eq!(index.markers_select1(1), Some(66));
        assert_eq!(index.markers_select1(2), None);

        assert_eq!(index.newlines_select1(0), Some(65));
        assert_eq!(index.newlines_select1(1), None);
    }
}
