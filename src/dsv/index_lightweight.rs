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
    pub fn new(markers: Vec<u64>, newlines: Vec<u64>, text_len: usize) -> Self {
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

        // Binary search to find the word containing the k-th bit
        let target_rank = (k + 1) as u32;
        let word_idx = match self.markers_rank.binary_search(&target_rank) {
            Ok(idx) => idx.saturating_sub(1),
            Err(idx) => idx.saturating_sub(1),
        };

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

        let target_rank = (k + 1) as u32;
        let word_idx = match self.newlines_rank.binary_search(&target_rank) {
            Ok(idx) => idx.saturating_sub(1),
            Err(idx) => idx.saturating_sub(1),
        };

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
}
