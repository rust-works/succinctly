//! Advance Index for memory-efficient BP-to-text-end position mapping.
//!
//! This module provides `EndPositions`, a space-efficient alternative to `Vec<u32>`
//! for storing scalar end positions in YAML. Containers store 0 as a sentinel
//! (they have no text end position), so only scalar entries carry meaningful data.
//!
//! # Structure
//!
//! For monotonically non-decreasing non-zero positions, uses three bitmaps:
//!
//! - **has_end**: One bit per BP open, set if the node has a non-zero end position
//! - **IB (Interest Bits)**: One bit per text byte, set at each unique end position
//! - **Advance bitmap**: One bit per scalar (node with end pos), set when end position advances
//!
//! For non-monotonic positions, falls back to `Vec<u32>`.
//!
//! # Memory
//!
//! With typical YAML density (N opens, S scalars ≈ 60-70% of N, L text bytes):
//! - Dense `Vec<u32>`: 4N bytes
//! - Compact: N/8 (has_end) + L/8 (IB) + S/8 (advance) + rank/select overhead ≈ 0.8N bytes
//! - **~5x smaller** (when monotonic)

#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::vec::Vec;

use crate::bits::CompactRank;
use crate::util::broadword::select_in_word;

use super::advance_positions::{build_select_samples, SELECT_SAMPLE_RATE};

/// End position storage with automatic optimization.
///
/// Uses memory-efficient encoding when non-zero end positions are monotonically
/// non-decreasing, otherwise falls back to dense `Vec<u32>` storage.
#[derive(Clone, Debug)]
pub enum EndPositions {
    /// Memory-efficient encoding for monotonic end positions.
    Compact(Box<CompactEndPositions>),
    /// Fallback for non-monotonic positions or when compression isn't beneficial.
    Dense(Vec<u32>),
}

/// Compact encoding for scalar end positions using three bitmaps.
///
/// Only non-zero entries (scalars with end positions) are stored in the
/// IB + advance bitmaps. A `has_end` bitmap identifies which BP opens
/// have non-zero end positions.
#[derive(Clone, Debug)]
pub struct CompactEndPositions {
    /// 1 if this BP open has a non-zero end position (is a scalar with end).
    has_end: Vec<u64>,
    /// Rank directory for has_end.
    has_end_rank: CompactRank,
    /// Total number of BP opens.
    num_opens: usize,

    /// Interest bits: one bit per text byte, set at each unique end position.
    ib_words: Vec<u64>,
    /// Number of valid bits in IB (= text length).
    #[allow(dead_code)]
    ib_len: usize,
    /// Rank directory for IB (unused currently; kept for potential rank queries).
    #[allow(dead_code)]
    ib_rank: CompactRank,
    /// Sampled select index for IB.
    ib_select_samples: Vec<u32>,
    /// Total number of unique end positions (1-bits in IB).
    ib_ones: usize,

    /// Advance bitmap: one bit per scalar, set when end position advances.
    advance_words: Vec<u64>,
    /// Number of scalars (nodes with non-zero end positions).
    #[allow(dead_code)]
    num_scalars: usize,
    /// Rank directory for advance.
    advance_rank: CompactRank,
}

impl EndPositions {
    /// Build from a slice of end positions (one per BP open).
    ///
    /// Zero entries are treated as sentinels (containers with no end position).
    /// Automatically chooses compact or dense storage based on monotonicity
    /// of the non-zero entries.
    pub fn build(positions: &[u32], text_len: usize) -> Self {
        if positions.is_empty() {
            return EndPositions::Compact(Box::new(CompactEndPositions::empty(text_len)));
        }

        // Extract non-zero positions and check monotonicity
        let mut non_zero: Vec<u32> = Vec::new();
        let mut is_monotonic = true;
        let mut prev: Option<u32> = None;

        for &pos in positions {
            if pos > 0 {
                if let Some(p) = prev {
                    if pos < p {
                        is_monotonic = false;
                        break;
                    }
                }
                prev = Some(pos);
                non_zero.push(pos);
            }
        }

        if !is_monotonic {
            return EndPositions::Dense(positions.to_vec());
        }

        EndPositions::Compact(Box::new(CompactEndPositions::build(
            positions,
            &non_zero,
            text_len,
        )))
    }

    /// Get the end position for the `open_idx`-th BP open.
    ///
    /// Returns `None` if the entry is 0 (container/no end position) or out of bounds.
    #[inline]
    pub fn get(&self, open_idx: usize) -> Option<usize> {
        match self {
            EndPositions::Compact(c) => c.get(open_idx),
            EndPositions::Dense(v) => v
                .get(open_idx)
                .map(|&pos| pos as usize)
                .filter(|&pos| pos > 0),
        }
    }

    /// Returns the heap memory usage in bytes.
    #[cfg(test)]
    pub fn heap_size(&self) -> usize {
        match self {
            EndPositions::Compact(c) => c.heap_size(),
            EndPositions::Dense(v) => v.len() * 4,
        }
    }

    /// Returns true if using compact storage.
    #[cfg(test)]
    #[inline]
    pub fn is_compact(&self) -> bool {
        matches!(self, EndPositions::Compact(_))
    }
}

impl CompactEndPositions {
    /// Create an empty instance.
    fn empty(text_len: usize) -> Self {
        Self {
            has_end: Vec::new(),
            has_end_rank: CompactRank::empty(),
            num_opens: 0,
            ib_words: Vec::new(),
            ib_len: text_len,
            ib_rank: CompactRank::empty(),
            ib_select_samples: Vec::new(),
            ib_ones: 0,
            advance_words: Vec::new(),
            num_scalars: 0,
            advance_rank: CompactRank::empty(),
        }
    }

    /// Build compact end positions from all positions and the pre-extracted non-zero positions.
    ///
    /// `all_positions` contains one entry per BP open (including zeros for containers).
    /// `non_zero_positions` contains only the non-zero entries, in order, and must be
    /// monotonically non-decreasing.
    fn build(all_positions: &[u32], non_zero_positions: &[u32], text_len: usize) -> Self {
        let num_opens = all_positions.len();
        let num_scalars = non_zero_positions.len();

        // Build has_end bitmap: 1 if BP open has non-zero end position
        let has_end_num_words = num_opens.div_ceil(64);
        let mut has_end = vec![0u64; has_end_num_words];

        for (i, &pos) in all_positions.iter().enumerate() {
            if pos > 0 {
                has_end[i / 64] |= 1u64 << (i % 64);
            }
        }
        let has_end_rank = CompactRank::build(&has_end);

        if non_zero_positions.is_empty() {
            return Self {
                has_end,
                has_end_rank,
                num_opens,
                ib_words: Vec::new(),
                ib_len: text_len,
                ib_rank: CompactRank::empty(),
                ib_select_samples: Vec::new(),
                ib_ones: 0,
                advance_words: Vec::new(),
                num_scalars: 0,
                advance_rank: CompactRank::empty(),
            };
        }

        // Build IB: set bit at each unique end position.
        // End positions can be at text_len (one past last byte for scalars ending at EOF),
        // so allocate one extra bit beyond text_len.
        let ib_num_words = (text_len + 1).div_ceil(64);
        let mut ib_words = vec![0u64; ib_num_words];

        // Build advance bitmap: set bit when end position changes
        let advance_num_words = num_scalars.div_ceil(64);
        let mut advance_words = vec![0u64; advance_num_words];

        let mut prev_pos: Option<u32> = None;
        let mut ib_ones = 0usize;

        for (i, &pos) in non_zero_positions.iter().enumerate() {
            let is_new_position = prev_pos != Some(pos);

            if is_new_position {
                // Set IB bit at this text position
                let word_idx = pos as usize / 64;
                let bit_idx = pos as usize % 64;
                if word_idx < ib_words.len()
                    && (ib_words[word_idx] >> bit_idx) & 1 == 0
                {
                    ib_words[word_idx] |= 1u64 << bit_idx;
                    ib_ones += 1;
                }

                // Set advance bit
                advance_words[i / 64] |= 1u64 << (i % 64);
            }

            prev_pos = Some(pos);
        }

        let ib_rank = CompactRank::build(&ib_words);
        let advance_rank = CompactRank::build(&advance_words);
        let ib_select_samples = build_select_samples(&ib_words, ib_ones);

        Self {
            has_end,
            has_end_rank,
            num_opens,
            ib_words,
            ib_len: text_len,
            ib_rank,
            ib_select_samples,
            ib_ones,
            advance_words,
            num_scalars,
            advance_rank,
        }
    }

    /// Get the end position for the `open_idx`-th BP open.
    ///
    /// Returns `None` if the entry has no end position (container) or out of bounds.
    #[inline]
    pub fn get(&self, open_idx: usize) -> Option<usize> {
        if open_idx >= self.num_opens {
            return None;
        }

        // Check if this BP open has an end position
        let word_idx = open_idx / 64;
        let bit_idx = open_idx % 64;
        if word_idx >= self.has_end.len() || (self.has_end[word_idx] >> bit_idx) & 1 == 0 {
            return None;
        }

        // Count scalars before this open to get scalar index
        let scalar_idx = self.has_end_rank1(open_idx);

        // Count advances up to and including this scalar
        let advance_count = self.advance_rank1(scalar_idx + 1);
        if advance_count == 0 {
            return None;
        }

        // Find the (advance_count - 1)-th unique end position in IB
        self.ib_select1(advance_count - 1)
    }

    /// Count 1-bits in has_end in [0, pos).
    #[inline]
    fn has_end_rank1(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }

        let word_idx = pos / 64;
        let bit_idx = pos % 64;

        let mut count = self
            .has_end_rank
            .rank_at_word(&self.has_end, word_idx.min(self.has_end.len()));

        if word_idx < self.has_end.len() && bit_idx > 0 {
            let mask = (1u64 << bit_idx) - 1;
            count += (self.has_end[word_idx] & mask).count_ones() as usize;
        }

        count
    }

    /// Count 1-bits in advance bitmap in [0, pos).
    #[inline]
    fn advance_rank1(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }

        let word_idx = pos / 64;
        let bit_idx = pos % 64;

        let mut count = self
            .advance_rank
            .rank_at_word(&self.advance_words, word_idx.min(self.advance_words.len()));

        if word_idx < self.advance_words.len() && bit_idx > 0 {
            let mask = (1u64 << bit_idx) - 1;
            count += (self.advance_words[word_idx] & mask).count_ones() as usize;
        }

        count
    }

    /// Select the k-th 1-bit in IB (0-indexed).
    #[inline]
    fn ib_select1(&self, k: usize) -> Option<usize> {
        if k >= self.ib_ones {
            return None;
        }

        // Use select samples to find starting word
        let sample_idx = k / SELECT_SAMPLE_RATE;
        let (start_word, mut remaining) = if sample_idx < self.ib_select_samples.len() {
            let sample_pos = self.ib_select_samples[sample_idx] as usize;
            let skip_ones = sample_idx * SELECT_SAMPLE_RATE;
            (sample_pos / 64, k - skip_ones)
        } else {
            (0, k)
        };

        // Scan from sample position
        for word_idx in start_word..self.ib_words.len() {
            let word = if word_idx == start_word && sample_idx < self.ib_select_samples.len() {
                let sample_pos = self.ib_select_samples[sample_idx] as usize;
                let bit_offset = sample_pos % 64;
                self.ib_words[word_idx] & !((1u64 << bit_offset) - 1)
            } else {
                self.ib_words[word_idx]
            };

            let ones = word.count_ones() as usize;
            if ones > remaining {
                let bit_pos = select_in_word(word, remaining as u32) as usize;
                return Some(word_idx * 64 + bit_pos);
            }
            remaining -= ones;
        }

        None
    }

    /// Returns the heap memory usage in bytes.
    #[cfg(test)]
    pub fn heap_size(&self) -> usize {
        self.has_end.len() * 8
            + self.has_end_rank.heap_size()
            + self.ib_words.len() * 8
            + self.ib_rank.heap_size()
            + self.ib_select_samples.len() * 4
            + self.advance_words.len() * 8
            + self.advance_rank.heap_size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let ep = EndPositions::build(&[], 0);
        assert!(ep.is_compact());
        assert_eq!(ep.get(0), None);
    }

    #[test]
    fn test_all_zeros() {
        // All containers, no scalars
        let positions = vec![0, 0, 0, 0, 0];
        let ep = EndPositions::build(&positions, 100);
        assert!(ep.is_compact());
        for i in 0..5 {
            assert_eq!(ep.get(i), None, "get({}) should be None", i);
        }
    }

    #[test]
    fn test_simple_scalars() {
        // BP opens: container(0), scalar(10), scalar(20), container(0), scalar(30)
        let positions = vec![0, 10, 20, 0, 30];
        let ep = EndPositions::build(&positions, 100);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None); // container
        assert_eq!(ep.get(1), Some(10)); // scalar
        assert_eq!(ep.get(2), Some(20)); // scalar
        assert_eq!(ep.get(3), None); // container
        assert_eq!(ep.get(4), Some(30)); // scalar
        assert_eq!(ep.get(5), None); // out of bounds
    }

    #[test]
    fn test_duplicate_end_positions() {
        // Multiple scalars ending at the same position (unlikely but possible)
        let positions = vec![0, 10, 10, 20, 20, 20];
        let ep = EndPositions::build(&positions, 100);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None);
        assert_eq!(ep.get(1), Some(10));
        assert_eq!(ep.get(2), Some(10));
        assert_eq!(ep.get(3), Some(20));
        assert_eq!(ep.get(4), Some(20));
        assert_eq!(ep.get(5), Some(20));
    }

    #[test]
    fn test_non_monotonic_falls_back_to_dense() {
        // Non-monotonic non-zero positions: 20, 10
        let positions = vec![0, 20, 10, 0, 30];
        let ep = EndPositions::build(&positions, 100);
        assert!(!ep.is_compact()); // Should use Dense fallback

        assert_eq!(ep.get(0), None);
        assert_eq!(ep.get(1), Some(20));
        assert_eq!(ep.get(2), Some(10));
        assert_eq!(ep.get(3), None);
        assert_eq!(ep.get(4), Some(30));
    }

    #[test]
    fn test_all_scalars() {
        // No containers, all scalars
        let positions = vec![5, 10, 15, 20, 25];
        let ep = EndPositions::build(&positions, 100);
        assert!(ep.is_compact());

        for (i, &expected) in positions.iter().enumerate() {
            assert_eq!(
                ep.get(i),
                Some(expected as usize),
                "get({}) failed",
                i
            );
        }
    }

    #[test]
    fn test_single_scalar() {
        let positions = vec![0, 42, 0];
        let ep = EndPositions::build(&positions, 100);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None);
        assert_eq!(ep.get(1), Some(42));
        assert_eq!(ep.get(2), None);
    }

    #[test]
    fn test_memory_savings() {
        // Simulate realistic YAML: ~40% containers (zeros), ~60% scalars
        let mut positions = Vec::with_capacity(1000);
        let mut pos = 5u32;
        for i in 0..1000 {
            if i % 5 < 2 {
                // Container
                positions.push(0);
            } else {
                // Scalar
                positions.push(pos);
                pos += 7 + (i as u32 % 15);
            }
        }

        let text_len = pos as usize + 100;
        let ep = EndPositions::build(&positions, text_len);
        assert!(ep.is_compact());

        let vec_size = positions.len() * 4;
        let ep_size = ep.heap_size();

        eprintln!(
            "Memory: Vec<u32>={} bytes, EndPositions={} bytes ({:.2}x compression)",
            vec_size,
            ep_size,
            vec_size as f64 / ep_size as f64
        );

        // Should achieve meaningful compression
        assert!(
            ep_size < vec_size,
            "EndPositions {} should be smaller than Vec<u32> {}",
            ep_size,
            vec_size
        );

        // Verify all values are correct
        for (i, &expected) in positions.iter().enumerate() {
            let got = ep.get(i);
            if expected == 0 {
                assert_eq!(got, None, "get({}) should be None for container", i);
            } else {
                assert_eq!(
                    got,
                    Some(expected as usize),
                    "get({}) failed: expected {}, got {:?}",
                    i,
                    expected,
                    got
                );
            }
        }
    }

    #[test]
    fn test_realistic_yaml_pattern() {
        // Simulate: mapping(0), key_scalar(5), value_scalar(11), key_scalar(15), value_scalar(22)
        let positions = vec![0, 5, 11, 15, 22];
        let ep = EndPositions::build(&positions, 30);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None); // mapping container
        assert_eq!(ep.get(1), Some(5));
        assert_eq!(ep.get(2), Some(11));
        assert_eq!(ep.get(3), Some(15));
        assert_eq!(ep.get(4), Some(22));
    }

    #[test]
    fn test_large_positions() {
        // Test with positions near u32 range
        let positions = vec![0, 100_000, 200_000, 0, 300_000];
        let ep = EndPositions::build(&positions, 400_000);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None);
        assert_eq!(ep.get(1), Some(100_000));
        assert_eq!(ep.get(2), Some(200_000));
        assert_eq!(ep.get(3), None);
        assert_eq!(ep.get(4), Some(300_000));
    }

    #[test]
    fn test_many_elements_with_samples() {
        // Test with enough elements to exercise select samples (> 256)
        let mut positions = Vec::with_capacity(1000);
        let mut pos = 1u32;
        for i in 0..1000 {
            if i % 3 == 0 {
                positions.push(0); // container
            } else {
                positions.push(pos);
                pos += 5;
            }
        }

        let text_len = pos as usize + 100;
        let ep = EndPositions::build(&positions, text_len);
        assert!(ep.is_compact());

        // Verify all values
        for (i, &expected) in positions.iter().enumerate() {
            let got = ep.get(i);
            if expected == 0 {
                assert_eq!(got, None, "get({}) should be None", i);
            } else {
                assert_eq!(
                    got,
                    Some(expected as usize),
                    "get({}) failed",
                    i
                );
            }
        }
    }

    #[test]
    fn test_real_yaml_compression() {
        // Generate a realistic YAML structure and measure EndPositions savings
        let mut yaml = String::new();
        for i in 0..500 {
            yaml.push_str(&format!("key_{}: value_{}\n", i, i));
        }
        yaml.push_str("nested:\n");
        for i in 0..200 {
            yaml.push_str(&format!("  sub_{}: sub_val_{}\n", i, i));
        }
        yaml.push_str("list:\n");
        for i in 0..300 {
            yaml.push_str(&format!("  - item_{}\n", i));
        }

        let semi = super::super::parser::build_semi_index(yaml.as_bytes()).unwrap();
        let text_len = yaml.len();
        let dense_size = semi.bp_to_text_end.len() * 4;
        let ep = EndPositions::build(&semi.bp_to_text_end, text_len);
        let compact_size = ep.heap_size();

        let num_opens = semi.bp_to_text_end.len();
        let num_nonzero = semi.bp_to_text_end.iter().filter(|&&v| v > 0).count();

        eprintln!(
            "Real YAML ({} bytes, {} opens, {} scalars): Vec<u32>={} bytes, EndPositions={} bytes ({:.2}x compression)",
            text_len, num_opens, num_nonzero, dense_size, compact_size,
            dense_size as f64 / compact_size as f64
        );

        assert!(ep.is_compact(), "Real YAML should use compact encoding");
        assert!(compact_size < dense_size, "Compact should be smaller");

        // Verify correctness: all values match
        for (i, &expected) in semi.bp_to_text_end.iter().enumerate() {
            let got = ep.get(i);
            if expected == 0 {
                assert_eq!(got, None, "open {} should be None", i);
            } else {
                assert_eq!(got, Some(expected as usize), "open {} mismatch", i);
            }
        }
    }

    #[test]
    fn test_end_position_at_text_len() {
        // Edge case: end position equals text_len (scalar ending at EOF).
        // This can happen when the last scalar has no trailing newline.
        let positions = vec![0, 64];
        let ep = EndPositions::build(&positions, 64);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None);
        assert_eq!(ep.get(1), Some(64));
    }

    #[test]
    fn test_end_position_at_text_len_multiple_of_64() {
        // Regression test: text_len is a multiple of 64, and the last scalar
        // ends exactly at text_len. The IB bitmap must accommodate this extra bit.
        let positions = vec![0, 50, 100, 0, 192];
        let ep = EndPositions::build(&positions, 192);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None);
        assert_eq!(ep.get(1), Some(50));
        assert_eq!(ep.get(2), Some(100));
        assert_eq!(ep.get(3), None);
        assert_eq!(ep.get(4), Some(192));
    }
}
